//! `http`: requests to the domains a plugin's manifest allows, run by `ureq` on a small worker
//! pool shared by all plugins; the answer reaches the plugin as `event.http-done` through its
//! [`Inbox`].
//!
//! The allowlist is matched against the host of the parsed URL (the WHATWG parser of the `url`
//! crate, which also normalizes IPv4 forms such as `127.1`), never against the text. The
//! request is sent to the `http::Uri` parsed from that URL's serialization, and the host is
//! checked to be the same in both, so the host that was checked is the host `ureq` connects
//! to. URLs with a user name or password are refused; an IPv4 host matches only an entry
//! written as that address; IPv6 hosts are never allowed (an entry cannot name one).
//!
//! Ruling Z: a host matched by a name entry may not resolve to a local address (loopback,
//! private, unique-local, link-local, CGNAT, unspecified, multicast): [`PublicOnly`] checks
//! the resolved addresses of every connection, every hop included, so a DNS answer cannot
//! turn an allowed name into a door to the user's machine or network. Only a host written as
//! an IP literal (allowed by an exact IP entry) or `localhost` may reach local addresses.
//! Proxies from the environment are not used, so that the target is what gets resolved.
//!
//! `ureq` follows no redirect itself: the host follows at most [`MAX_REDIRECTS`] and checks
//! every hop like the first URL. The 30 s timeout starts at `fetch` and spans the whole
//! exchange, waiting for a worker, every hop and the body included. A plugin has at most
//! [`MAX_CONCURRENT`] requests; one counts from `fetch` until its answer is delivered, so the
//! answers waiting for a busy plugin are bounded too. Ruling Y: when the plugin's store goes
//! (the plugin stopped or was unloaded), its queued requests are not sent and the answers of
//! running ones are discarded.

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use ureq::Agent;
use ureq::config::Config;
use ureq::http::header::{CONTENT_TYPE, LOCATION};
use ureq::http::{self as h, HeaderName, HeaderValue, Method, Uri};
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use url::{Host, Url};

use super::{HostState, Inbox};
use crate::bindings::tomakast::plugin::http::Host as HttpHost;
use crate::bindings::tomakast::plugin::types::{
    Event, HttpDone, HttpRequest, HttpResponse, KeyValue,
};
use crate::manifest::DomainPattern;

/// Test-only: answers checked requests instead of the network: (method, url) -> (status, body).
#[cfg(any(test, feature = "testing"))]
pub type Responder = Arc<dyn Fn(&str, &str) -> Result<(u16, Vec<u8>), String> + Send + Sync>;

/// Requests one plugin may have running (or answered and not yet delivered).
pub const MAX_CONCURRENT: usize = 4;
/// Largest response body.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// The whole exchange, every redirect and the body included.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// Redirects the host follows.
pub const MAX_REDIRECTS: usize = 5;

/// Worker threads shared by all plugins, and how long an idle one waits before it exits.
const MAX_WORKERS: usize = 8;
const WORKER_IDLE: Duration = Duration::from_secs(60);

const DENIED: &str = "the http permission was not granted (no allowed domains)";

const METHODS: [Method; 7] = [
    Method::GET,
    Method::HEAD,
    Method::POST,
    Method::PUT,
    Method::PATCH,
    Method::DELETE,
    Method::OPTIONS,
];

/// Headers a plugin may not set: the host's own framing, and `host`, which would let a
/// request reach another site behind an allowed server than the URL names.
const FORBIDDEN_HEADERS: [&str; 11] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "upgrade",
    "te",
    "trailer",
    "expect",
    "proxy-authorization",
    "proxy-connection",
];

/// Dropped when a redirect goes to another origin.
const CREDENTIAL_HEADERS: [&str; 2] = ["authorization", "cookie"];

/// The HTTP state of one plugin store. Dropping it (with the store) drops the plugin's queued
/// requests and discards the answers of running ones.
pub(crate) struct Http {
    /// Names this store's queue in the shared pool.
    owner: u64,
    allow: Arc<[DomainPattern]>,
    in_flight: usize,
    next_id: u64,
    inbox: Arc<Inbox>,
    /// Cleared when the store goes; the plugin's tasks check it before and after the request.
    alive: Arc<AtomicBool>,
    pool: Arc<Pool>,
    agent: Agent,
    /// Test-only: answers instead of `execute`, after the checks.
    #[cfg(any(test, feature = "testing"))]
    responder: Option<Responder>,
}

impl Http {
    pub(crate) fn new(allow: Vec<DomainPattern>, inbox: Arc<Inbox>) -> Http {
        Http::with_parts(allow, inbox, Arc::clone(Pool::global()), agent().clone())
    }

    fn with_parts(
        allow: Vec<DomainPattern>,
        inbox: Arc<Inbox>,
        pool: Arc<Pool>,
        agent: Agent,
    ) -> Http {
        static OWNERS: AtomicU64 = AtomicU64::new(1);
        Http {
            owner: OWNERS.fetch_add(1, Ordering::Relaxed),
            allow: allow.into(),
            in_flight: 0,
            next_id: 1,
            inbox,
            alive: Arc::new(AtomicBool::new(true)),
            pool,
            agent,
            #[cfg(any(test, feature = "testing"))]
            responder: None,
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn set_responder(&mut self, responder: Responder) {
        self.responder = Some(responder);
    }

    fn fetch(&mut self, request: HttpRequest) -> Result<u64, String> {
        if self.allow.is_empty() {
            return Err(DENIED.into());
        }
        if self.in_flight >= MAX_CONCURRENT {
            return Err(format!(
                "at most {MAX_CONCURRENT} requests may run at a time"
            ));
        }
        let prepared = prepare(&self.allow, request)?;
        let deadline = Instant::now() + TIMEOUT;
        let id = self.next_id;
        self.next_id += 1;
        self.in_flight += 1;
        let allow = Arc::clone(&self.allow);
        let inbox = Arc::clone(&self.inbox);
        let alive = Arc::clone(&self.alive);
        let agent = self.agent.clone();
        #[cfg(any(test, feature = "testing"))]
        let responder = self.responder.clone();
        self.pool.submit(
            self.owner,
            Box::new(move |start: Result<(), String>| {
                // Ruling Y: nothing is sent for a plugin that is gone.
                if !alive.load(Ordering::Acquire) {
                    return;
                }
                let response = start.and_then(|()| {
                    catch_unwind(AssertUnwindSafe(|| {
                        #[cfg(any(test, feature = "testing"))]
                        if let Some(responder) = &responder {
                            return responder(prepared.method.as_str(), prepared.url.as_str()).map(
                                |(status, body)| HttpResponse {
                                    status,
                                    headers: Vec::new(),
                                    body,
                                },
                            );
                        }
                        execute(&agent, &allow, prepared, deadline)
                    }))
                    .unwrap_or_else(|_| Err("the request failed".into()))
                });
                // …and the answer of a request it started is discarded.
                if alive.load(Ordering::Acquire) {
                    inbox.push(Event::HttpDone(HttpDone {
                        request: id,
                        response,
                    }));
                }
            }),
        );
        Ok(id)
    }

    /// An answer is being delivered: its request no longer counts.
    pub(crate) fn delivered(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

impl HttpHost for HostState {
    fn fetch(&mut self, request: HttpRequest) -> Result<u64, String> {
        self.record_import("http.fetch");
        self.http.fetch(request)
    }
}

/// A checked request, ready to send.
struct Prepared {
    method: Method,
    url: Url,
    uri: Uri,
    headers: Vec<(HeaderName, HeaderValue)>,
    body: Option<Vec<u8>>,
}

fn prepare(allow: &[DomainPattern], request: HttpRequest) -> Result<Prepared, String> {
    let method = METHODS
        .into_iter()
        .find(|m| m.as_str().eq_ignore_ascii_case(&request.method))
        .ok_or("the method must be GET, HEAD, POST, PUT, PATCH, DELETE or OPTIONS")?;
    if request.body.is_some() && matches!(method, Method::GET | Method::HEAD) {
        return Err(format!("a {method} request has no body"));
    }
    let url = Url::parse(&request.url).map_err(|e| format!("not a valid URL: {e}"))?;
    let (url, uri) = check(allow, url)?;
    let mut headers = Vec::with_capacity(request.headers.len());
    for KeyValue { key, value } in request.headers {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| "a header name is not valid".to_string())?;
        if FORBIDDEN_HEADERS.contains(&name.as_str()) {
            return Err(format!("the {name} header is set by the launcher"));
        }
        let value = HeaderValue::from_str(&value)
            .map_err(|_| format!("the value of the {name} header is not valid"))?;
        headers.push((name, value));
    }
    Ok(Prepared {
        method,
        url,
        uri,
        headers,
        body: request.body,
    })
}

/// `url` without its fragment and the `Uri` the request goes to, if `allow` permits its
/// host.
fn check(allow: &[DomainPattern], mut url: Url) -> Result<(Url, Uri), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http and https URLs are allowed".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLs with a user name or password are not allowed".into());
    }
    let allowed = match url.host() {
        Some(Host::Domain(name)) => allow.iter().any(|p| p.matches(name)),
        Some(Host::Ipv4(ip)) => allow.iter().any(|p| p.matches_ip(ip)),
        Some(Host::Ipv6(_)) | None => false,
    };
    if !allowed {
        let host = url.host_str().unwrap_or_default();
        return Err(format!("the host {host:?} is not allowed"));
    }
    url.set_fragment(None);
    let uri: Uri = url
        .as_str()
        .parse()
        .map_err(|_| "the URL cannot be requested".to_string())?;
    // The host the client connects to must be the host that was checked.
    let same = uri.scheme_str() == Some(url.scheme())
        && uri.host() == url.host_str()
        && uri.port_u16().or(url.port_or_known_default()) == url.port_or_known_default();
    if !same {
        return Err("the URL is ambiguous".into());
    }
    Ok((url, uri))
}

/// Same scheme, host and port.
fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host() == b.host()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Sends `request`, following and checking redirects, and reads the body.
/// Sends `request`, following and checking redirects, and reads the body, all before
/// `deadline` (a request that waited for a worker past it fails at once).
fn execute(
    agent: &Agent,
    allow: &[DomainPattern],
    request: Prepared,
    deadline: Instant,
) -> Result<HttpResponse, String> {
    let Prepared {
        mut method,
        mut url,
        mut uri,
        mut headers,
        mut body,
    } = request;
    let mut redirects = 0;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!("timed out after {} s", TIMEOUT.as_secs()));
        }
        let response = send(agent, &method, &uri, &headers, body.as_deref(), left)?;
        let status = response.status().as_u16();
        let location = match status {
            301 | 302 | 303 | 307 | 308 => response.headers().get(LOCATION),
            _ => None,
        };
        let Some(location) = location else {
            return read(response);
        };
        if redirects == MAX_REDIRECTS {
            return Err(format!("more than {MAX_REDIRECTS} redirects"));
        }
        redirects += 1;
        let next = location
            .to_str()
            .ok()
            .and_then(|location| url.join(location).ok())
            .ok_or("a redirect has an invalid location")?;
        let (next, next_uri) = check(allow, next).map_err(|e| format!("redirect refused: {e}"))?;
        follow(status, &url, &next, &mut method, &mut body, &mut headers);
        url = next;
        uri = next_uri;
    }
}

/// Rewrites the request for a redirect with `status` from `from` to `to`: 303, and 301/302
/// after a POST, continue as a GET without the body; another origin gets no credentials.
fn follow(
    status: u16,
    from: &Url,
    to: &Url,
    method: &mut Method,
    body: &mut Option<Vec<u8>>,
    headers: &mut Vec<(HeaderName, HeaderValue)>,
) {
    let to_get = (status == 303 && *method != Method::HEAD)
        || (matches!(status, 301 | 302) && *method == Method::POST);
    if to_get {
        *method = Method::GET;
        *body = None;
        headers.retain(|(name, _)| *name != CONTENT_TYPE);
    }
    if !same_origin(from, to) {
        headers.retain(|(name, _)| !CREDENTIAL_HEADERS.contains(&name.as_str()));
    }
}

fn send(
    agent: &Agent,
    method: &Method,
    uri: &Uri,
    headers: &[(HeaderName, HeaderValue)],
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<h::Response<ureq::Body>, String> {
    let mut builder = h::Request::builder()
        .method(method.clone())
        .uri(uri.clone());
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let invalid = |e: h::Error| format!("the request is not valid: {e}");
    let result = match body {
        Some(body) => {
            let request = builder.body(body).map_err(invalid)?;
            let request = agent
                .configure_request(request)
                .timeout_global(Some(timeout))
                .build();
            agent.run(request)
        }
        None => {
            let request = builder.body(()).map_err(invalid)?;
            let request = agent
                .configure_request(request)
                .timeout_global(Some(timeout))
                .build();
            agent.run(request)
        }
    };
    result.map_err(|e| format!("the request failed: {e}"))
}

fn read(mut response: h::Response<ureq::Body>) -> Result<HttpResponse, String> {
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| KeyValue {
            key: name.as_str().to_owned(),
            value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
        })
        .collect();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES as u64)
        .read_to_vec()
        .map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => "the response body is larger than 4 MiB".into(),
            other => format!("reading the response failed: {other}"),
        })?;
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

/// The client shared by all plugins.
fn agent() -> &'static Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT.get_or_init(|| build_agent(DefaultResolver::default()))
}

/// A client over `resolver`, guarded by [`PublicOnly`]: no redirects of its own, status codes
/// as answers, no proxy from the environment.
fn build_agent(resolver: impl Resolver) -> Agent {
    let config = Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .proxy(None)
        .user_agent("tomakast-plugin")
        .build();
    Agent::with_parts(config, DefaultConnector::new(), PublicOnly(resolver))
}

/// Ruling Z: refuses a connection to a name whose resolved addresses include a local one.
#[derive(Debug)]
struct PublicOnly<R>(R);

impl<R: Resolver> Resolver for PublicOnly<R> {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let addrs = self.0.resolve(uri, config, timeout)?;
        if may_reach_local(uri.host().unwrap_or_default()) {
            return Ok(addrs);
        }
        if addrs.iter().any(|addr| is_local(addr.ip())) {
            return Err(ureq::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the host resolves to a local or private address",
            )));
        }
        Ok(addrs)
    }
}

/// An IP literal (it passed the allowlist only through an exact IP entry) or `localhost`.
fn may_reach_local(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.parse::<IpAddr>().is_ok() || host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
}

/// Loopback, private (RFC 1918, fc00::/7), link-local, CGNAT (100.64/10), unspecified,
/// multicast or broadcast. An IPv6 address that carries an IPv4 address counts as that IPv4
/// address: IPv4-mapped `::ffff:a.b.c.d`, IPv4-compatible `::a.b.c.d`, NAT64 `64:ff9b::/96`
/// and 6to4 `2002::/16` (a network that routes them reaches the embedded address). The
/// local-use NAT64 prefix `64:ff9b:1::/48` is local as a whole.
fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_local_v4(v4),
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return true;
            }
            if let Some(v4) = embedded_v4(v6) {
                return is_local_v4(v4);
            }
            let seg = v6.segments();
            v6.is_multicast()
                || seg[0] & 0xfe00 == 0xfc00
                || seg[0] & 0xffc0 == 0xfe80
                || seg[0] & 0xffc0 == 0xfec0
                || (seg[0] == 0x64 && seg[1] == 0xff9b && seg[2] == 1)
        }
    }
}

/// The IPv4 address an IPv6 address carries: IPv4-mapped (`::ffff:0:0/96`), IPv4-compatible
/// (`::/96`, after `::` and `::1` are handled), NAT64 (`64:ff9b::/96`) or 6to4 (`2002::/16`,
/// the 32 bits after the prefix).
fn embedded_v4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    let seg = v6.segments();
    let v4 = |hi: u16, lo: u16| Some(Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo)));
    match seg {
        [0, 0, 0, 0, 0, 0xffff, hi, lo] | [0, 0, 0, 0, 0, 0, hi, lo] => v4(hi, lo),
        [0x64, 0xff9b, 0, 0, 0, 0, hi, lo] => v4(hi, lo),
        [0x2002, hi, lo, ..] => v4(hi, lo),
        _ => None,
    }
}

fn is_local_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || a == 0
        || (a == 100 && b & 0xc0 == 64)
}

/// Work for the pool: called with `Ok` on a worker, or with `Err(reason)` when no worker could
/// be started, so that it can fail (and free its slot).
type Task = Box<dyn FnOnce(Result<(), String>) + Send>;

/// A small pool of worker threads: they start when work comes, wait on a condvar while idle
/// (parked) and exit after [`WORKER_IDLE`] without work. Waiting tasks are kept per plugin
/// store and served round-robin (Ruling AM), so one plugin's backlog does not delay another
/// plugin's next request. A request's 30 s still start at `fetch`.
struct Pool {
    state: Mutex<PoolState>,
    work: Condvar,
    max_workers: usize,
    idle_exit: Duration,
    /// Starts a worker thread running [`Pool::work_loop`]; replaced in tests.
    spawn: fn(Arc<Pool>) -> std::io::Result<()>,
}

#[derive(Default)]
struct PoolState {
    /// Waiting tasks per owner, in turn order; an owner with no waiting task has no entry.
    queues: VecDeque<(u64, VecDeque<Task>)>,
    /// Waiting tasks over all owners.
    queued: usize,
    workers: usize,
    idle: usize,
}

impl PoolState {
    fn push(&mut self, owner: u64, task: Task) {
        match self.queues.iter_mut().find(|(o, _)| *o == owner) {
            Some((_, tasks)) => tasks.push_back(task),
            None => self.queues.push_back((owner, VecDeque::from([task]))),
        }
        self.queued += 1;
    }

    /// The first task of the owner whose turn it is; that owner goes to the back of the line.
    fn pop_next(&mut self) -> Option<Task> {
        let (owner, mut tasks) = self.queues.pop_front()?;
        let task = tasks.pop_front()?;
        if !tasks.is_empty() {
            self.queues.push_back((owner, tasks));
        }
        self.queued -= 1;
        Some(task)
    }

    /// The task `owner` queued last, taken back.
    fn pop_last(&mut self, owner: u64) -> Option<Task> {
        let at = self.queues.iter().position(|(o, _)| *o == owner)?;
        let task = self.queues[at].1.pop_back()?;
        if self.queues[at].1.is_empty() {
            self.queues.remove(at);
        }
        self.queued -= 1;
        Some(task)
    }
}

impl Pool {
    fn global() -> &'static Arc<Pool> {
        static POOL: OnceLock<Arc<Pool>> = OnceLock::new();
        POOL.get_or_init(|| Pool::new(MAX_WORKERS, WORKER_IDLE))
    }

    fn new(max_workers: usize, idle_exit: Duration) -> Arc<Pool> {
        Arc::new(Pool {
            state: Mutex::new(PoolState::default()),
            work: Condvar::new(),
            max_workers,
            idle_exit,
            spawn: |pool| {
                std::thread::Builder::new()
                    .name("plugin-http".into())
                    .spawn(move || pool.work_loop())
                    .map(drop)
            },
        })
    }

    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues `task` of plugin store `owner`.
    fn submit(self: &Arc<Self>, owner: u64, task: Task) {
        let mut state = self.lock();
        state.push(owner, task);
        if state.queued > state.idle && state.workers < self.max_workers {
            match (self.spawn)(Arc::clone(self)) {
                Ok(()) => state.workers += 1,
                Err(err) => {
                    tracing::warn!(target: "plugin", %err, "could not start an HTTP worker");
                    // With no worker at all the task would wait forever: fail it now.
                    if state.workers == 0
                        && let Some(task) = state.pop_last(owner)
                    {
                        drop(state);
                        task(Err("no HTTP worker could be started".into()));
                        return;
                    }
                }
            }
        }
        self.work.notify_one();
    }

    fn work_loop(&self) {
        loop {
            let task = {
                let mut state = self.lock();
                loop {
                    if let Some(task) = state.pop_next() {
                        break task;
                    }
                    state.idle += 1;
                    let (next, wait) = self
                        .work
                        .wait_timeout(state, self.idle_exit)
                        .unwrap_or_else(PoisonError::into_inner);
                    state = next;
                    state.idle -= 1;
                    if wait.timed_out() && state.queued == 0 {
                        state.workers -= 1;
                        return;
                    }
                }
            };
            // A task delivers its own failure; a panic must not cost the pool a worker.
            if catch_unwind(AssertUnwindSafe(|| task(Ok(())))).is_err() {
                tracing::error!(target: "plugin", "an HTTP task panicked");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn pattern(s: &str) -> DomainPattern {
        DomainPattern::parse(s).unwrap()
    }

    fn allowed(allow: &[&str], url: &str) -> Result<String, String> {
        let allow: Vec<DomainPattern> = allow.iter().map(|s| pattern(s)).collect();
        let url = Url::parse(url).map_err(|e| e.to_string())?;
        check(&allow, url).map(|(_, uri)| uri.to_string())
    }

    #[test]
    fn hosts_are_matched_after_parsing() {
        let ok = |allow: &[&str], url: &str| allowed(allow, url).unwrap();
        assert_eq!(
            ok(&["example.com"], "https://example.com/a#frag"),
            "https://example.com/a"
        );
        assert_eq!(
            ok(&["example.com"], "https://EXAMPLE.com:8443/a"),
            "https://example.com:8443/a"
        );
        assert_eq!(
            ok(&["example.com"], "http://example.com./"),
            "http://example.com./"
        );
        assert_eq!(
            ok(&["*.example.org"], "https://api.example.org/"),
            "https://api.example.org/"
        );
        // A backslash is a path separator in http URLs: the host stays example.com.
        assert_eq!(
            ok(&["example.com"], "http://example.com\\@evil.test/"),
            "http://example.com/@evil.test/"
        );
        assert_eq!(
            ok(&["xn--e1afmkfd.xn--p1ai"], "https://пример.рф/"),
            "https://xn--e1afmkfd.xn--p1ai/"
        );

        let refused = |allow: &[&str], url: &str| {
            assert!(allowed(allow, url).is_err(), "{url} with {allow:?}");
        };
        refused(&["example.com"], "https://example.com@evil.test/");
        refused(&["example.com"], "https://user:pw@example.com/");
        refused(&["example.com"], "https://user@example.com/");
        refused(&["example.com"], "https://evil.test#@example.com/");
        refused(&["example.com"], "https://evil.test/?@example.com");
        refused(&["example.com"], "https://example.com.evil.test/");
        refused(&["example.com"], "https://evilexample.com/");
        refused(&["example.com"], "https://sub.example.com/");
        refused(&["*.example.org"], "https://example.org/");
        refused(&["example.com"], "ftp://example.com/");
        refused(&["example.com"], "file:///etc/passwd");
        refused(&["example.com"], "ws://example.com/");
    }

    /// Ruling Q: an IP host matches only an entry written as that address.
    #[test]
    fn ip_literal_hosts_need_an_exact_ip_entry() {
        assert!(allowed(&["127.0.0.1"], "http://127.0.0.1:8080/").is_ok());
        // Other spellings of the same address are the same address.
        assert!(allowed(&["127.0.0.1"], "http://127.1/").is_ok());
        assert!(allowed(&["127.0.0.1"], "http://0x7f.0.0.1/").is_ok());
        assert!(allowed(&["127.0.0.1"], "http://2130706433/").is_ok());
        // A wildcard or a name never matches an address.
        assert!(allowed(&["*.0.0.1"], "http://127.0.0.1/").is_err());
        assert!(allowed(&["*.0.1"], "http://127.0.1/").is_err());
        assert!(allowed(&["localhost"], "http://127.0.0.1/").is_err());
        assert!(allowed(&["127.0.0.1"], "http://127.0.0.2/").is_err());
        assert!(allowed(&["127.0.0.1"], "http://[::1]/").is_err());
        assert!(allowed(&["127.0.0.1"], "http://[::ffff:127.0.0.1]/").is_err());
    }

    fn request(
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> HttpRequest {
        HttpRequest {
            method: method.into(),
            url: url.into(),
            headers: headers
                .iter()
                .map(|(k, v)| KeyValue {
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            body: body.map(<[u8]>::to_vec),
        }
    }

    #[test]
    fn requests_are_checked_before_they_are_sent() {
        let allow = [pattern("example.com")];
        let prepare = |r: HttpRequest| prepare(&allow, r).map(|_| ());
        let url = "https://example.com/";
        assert!(prepare(request("get", url, &[("accept", "text/plain")], None)).is_ok());
        assert!(prepare(request("POST", url, &[("content-type", "a/b")], Some(b"x"))).is_ok());
        for method in ["CONNECT", "TRACE", "BREW", "GET\r\nX: y", ""] {
            assert!(
                prepare(request(method, url, &[], None)).is_err(),
                "{method:?}"
            );
        }
        assert!(prepare(request("GET", url, &[], Some(b"x"))).is_err());
        for (name, value) in [
            ("Host", "evil.test"),
            ("content-length", "1"),
            ("Transfer-Encoding", "chunked"),
            ("x-a", "line\r\nInjected: yes"),
            ("bad name", "v"),
            ("", "v"),
        ] {
            let refused = prepare(request("GET", url, &[(name, value)], None));
            assert!(refused.is_err(), "{name:?}: {value:?}");
        }
    }

    /// The request after one redirect: `(method, body?, header names)`.
    fn after(status: u16, method: Method, from: &str, to: &str) -> (Method, bool, Vec<String>) {
        let (from, to) = (Url::parse(from).unwrap(), Url::parse(to).unwrap());
        let mut method = method;
        let mut body = Some(b"x".to_vec());
        let mut headers: Vec<(HeaderName, HeaderValue)> =
            ["authorization", "cookie", "content-type", "accept"]
                .into_iter()
                .map(|n| (HeaderName::from_static(n), HeaderValue::from_static("v")))
                .collect();
        follow(status, &from, &to, &mut method, &mut body, &mut headers);
        let names = headers.iter().map(|(n, _)| n.as_str().to_owned()).collect();
        (method, body.is_some(), names)
    }

    #[test]
    fn redirects_rewrite_the_method_and_drop_credentials_across_origins() {
        let all = ["authorization", "cookie", "content-type", "accept"];
        let a = "https://a.example/x";
        // Same origin: 307/308 keep everything; 303 and POST-after-301/302 become a GET.
        assert_eq!(
            after(307, Method::POST, a, "https://a.example/y"),
            (Method::POST, true, all.map(String::from).to_vec())
        );
        assert_eq!(
            after(308, Method::PUT, a, "https://a.example/y"),
            (Method::PUT, true, all.map(String::from).to_vec())
        );
        let get = (
            Method::GET,
            false,
            vec![
                "authorization".to_string(),
                "cookie".into(),
                "accept".into(),
            ],
        );
        assert_eq!(after(303, Method::POST, a, "https://a.example/y"), get);
        assert_eq!(after(302, Method::POST, a, "https://a.example/y"), get);
        assert_eq!(after(301, Method::POST, a, "https://a.example/y"), get);
        assert_eq!(
            after(303, Method::PUT, a, "https://a.example/y").0,
            Method::GET
        );
        assert_eq!(
            after(303, Method::HEAD, a, "https://a.example/y").0,
            Method::HEAD
        );
        assert_eq!(
            after(302, Method::PUT, a, "https://a.example/y").0,
            Method::PUT
        );
        // Another host, port or scheme drops authorization and cookie.
        for other in [
            "https://b.example/y",
            "https://a.example:8443/y",
            "http://a.example/y",
        ] {
            let (method, body, names) = after(307, Method::POST, a, other);
            assert_eq!((method, body), (Method::POST, true), "{other}");
            assert_eq!(names, ["content-type", "accept"], "{other}");
        }
    }

    #[test]
    fn pool_workers_exit_when_idle() {
        let pool = Pool::new(2, Duration::from_millis(100));
        let (tx, rx) = mpsc::channel();
        for i in 0..5 {
            let tx = tx.clone();
            pool.submit(
                i as u64,
                Box::new(move |start| {
                    assert!(start.is_ok());
                    std::thread::sleep(Duration::from_millis(20));
                    tx.send(i).unwrap();
                }),
            );
        }
        let mut done: Vec<i32> = (0..5)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        done.sort_unstable();
        assert_eq!(done, [0, 1, 2, 3, 4]);
        assert!(pool.lock().workers <= 2);
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(pool.lock().workers, 0, "idle workers did not exit");
        // And the pool starts again.
        pool.submit(1, Box::new(move |_| tx.send(9).unwrap()));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), 9);
    }

    /// Ruling AM: the pool serves the plugins' queues in turn, not in submission order, so a
    /// plugin that queued many requests does not make another plugin's one wait behind all
    /// of them.
    #[test]
    fn plugins_take_turns_for_workers() {
        let pool = Pool::new(1, WORKER_IDLE);
        let (release, wait) = mpsc::channel::<()>();
        // Holds the only worker until everything is queued.
        pool.submit(
            0,
            Box::new(move |_| {
                let _ = wait.recv();
            }),
        );
        let order = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let task = |owner: u64, n: u32| -> Task {
            let order = Arc::clone(&order);
            let tx = tx.clone();
            Box::new(move |_| {
                order.lock().unwrap().push(format!("{owner}.{n}"));
                tx.send(()).unwrap();
            })
        };
        for n in 1..=3 {
            pool.submit(1, task(1, n));
        }
        for n in 1..=3 {
            pool.submit(2, task(2, n));
        }
        pool.submit(3, task(3, 1));
        release.send(()).unwrap();
        for _ in 0..7 {
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        assert_eq!(
            *order.lock().unwrap(),
            ["1.1", "2.1", "3.1", "1.2", "2.2", "1.3", "2.3"]
        );
    }

    /// Finding 6: with no worker at all, a task fails at once instead of waiting forever.
    #[test]
    fn a_task_fails_when_no_worker_can_start() {
        let pool = Arc::new(Pool {
            spawn: |_| Err(std::io::Error::other("no threads")),
            ..Arc::into_inner(Pool::new(2, WORKER_IDLE)).unwrap()
        });
        let (tx, rx) = mpsc::channel();
        pool.submit(1, Box::new(move |start| tx.send(start).unwrap()));
        let start = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(start.unwrap_err().contains("no HTTP worker"));
        assert_eq!(pool.lock().queued, 0);
    }

    #[test]
    fn local_addresses_are_classified() {
        for local in [
            "127.0.0.1",
            "127.255.0.9",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.0.1",
            "100.64.0.1",
            "100.127.255.255",
            "0.0.0.0",
            "0.1.2.3",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
            // I-7: IPv4 addresses embedded in IPv6 count as themselves.
            "64:ff9b::127.0.0.1",
            "64:ff9b::10.0.0.1",
            "64:ff9b::c0a8:101",
            "64:ff9b:1::8.8.8.8",
            "2002:7f00:1::1",
            "2002:c0a8:101::",
            "2002:a00:1:ffff::1",
            "::127.0.0.1",
            "::10.1.2.3",
            "::192.168.1.1",
        ] {
            assert!(is_local(local.parse().unwrap()), "{local}");
        }
        for public in [
            "8.8.8.8",
            "172.32.0.1",
            "100.128.0.1",
            "100.63.255.255",
            "192.169.0.1",
            "2001:4860:4860::8888",
            "::ffff:8.8.8.8",
            "64:ff9b::8.8.8.8",
            "2002:808:808::1",
            "::8.8.8.8",
        ] {
            assert!(!is_local(public.parse().unwrap()), "{public}");
        }
        assert!(may_reach_local("127.0.0.1"));
        assert!(may_reach_local("[::1]"));
        assert!(may_reach_local("LocalHost."));
        assert!(!may_reach_local("rebind.test"));
        assert!(!may_reach_local("localhost.evil.test"));
    }

    /// A tiny HTTP server on 127.0.0.1 that counts connections and answers `ok` after `hold`.
    fn server(hold: Duration) -> (u16, Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = Arc::clone(&accepted);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                count.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                        line.clear();
                    }
                    std::thread::sleep(hold);
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                    );
                });
            }
        });
        (port, accepted)
    }

    /// Resolves the names it knows to fixed addresses, like a rebinding DNS server would.
    #[derive(Debug)]
    struct Mapped(Vec<(&'static str, IpAddr)>);

    impl Resolver for Mapped {
        fn resolve(
            &self,
            uri: &Uri,
            config: &Config,
            timeout: NextTimeout,
        ) -> Result<ResolvedSocketAddrs, ureq::Error> {
            let host = uri.host().unwrap_or_default();
            let Some(&(_, ip)) = self.0.iter().find(|(name, _)| *name == host) else {
                return DefaultResolver::default().resolve(uri, config, timeout);
            };
            let mut addrs = self.empty();
            addrs.push(std::net::SocketAddr::new(ip, uri.port_u16().unwrap_or(80)));
            Ok(addrs)
        }
    }

    fn get(allow: &[&str], url: &str) -> Prepared {
        let allow: Vec<DomainPattern> = allow.iter().map(|s| pattern(s)).collect();
        prepare(&allow, request("GET", url, &[], None)).unwrap()
    }

    /// Ruling Z: an allowed name that resolves to a local address is refused before any byte
    /// is sent; `localhost` and IP-literal entries still reach local addresses.
    #[test]
    fn names_resolving_to_local_addresses_are_refused() {
        let (port, accepted) = server(Duration::ZERO);
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        let agent = build_agent(Mapped(vec![
            ("rebind.test", loopback),
            ("localhost", loopback),
        ]));
        let deadline = || Instant::now() + TIMEOUT;
        let url = format!("http://rebind.test:{port}/");
        let refused = execute(
            &agent,
            &[pattern("rebind.test")],
            get(&["rebind.test"], &url),
            deadline(),
        );
        let error = refused.unwrap_err();
        assert!(error.contains("local or private address"), "{error}");
        assert_eq!(
            accepted.load(Ordering::SeqCst),
            0,
            "a refused name was connected to"
        );

        // The twins: `localhost` and an IP literal still reach the loopback address.
        let to_local = format!("http://localhost:{port}/");
        let allow = [pattern("localhost"), pattern("127.0.0.1")];
        let ok = execute(&agent, &allow, get(&["localhost"], &to_local), deadline()).unwrap();
        assert_eq!((ok.status, ok.body.as_slice()), (200, b"ok".as_slice()));
        let literal = format!("http://127.0.0.1:{port}/");
        let ok = execute(&agent, &allow, get(&["127.0.0.1"], &literal), deadline()).unwrap();
        assert_eq!(ok.status, 200);
        assert_eq!(accepted.load(Ordering::SeqCst), 2);
    }

    /// Finding 7: the budget starts at `fetch`; a task that waited past it sends nothing.
    #[test]
    fn a_request_past_its_deadline_is_not_sent() {
        let (port, accepted) = server(Duration::ZERO);
        let url = format!("http://localhost:{port}/");
        let past = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        let error = execute(
            agent(),
            &[pattern("localhost")],
            get(&["localhost"], &url),
            past,
        )
        .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert_eq!(accepted.load(Ordering::SeqCst), 0);
    }

    fn http_for(inbox: &Arc<Inbox>, pool: &Arc<Pool>) -> Http {
        Http::with_parts(
            vec![pattern("localhost")],
            Arc::clone(inbox),
            Arc::clone(pool),
            agent().clone(),
        )
    }

    fn fetch(http: &mut Http, port: u16) {
        let url = format!("http://localhost:{port}/");
        http.fetch(request("GET", &url, &[], None)).unwrap();
    }

    /// Ruling Y: a stopped plugin's queued request is never sent, and the answer of a running
    /// one is discarded.
    #[test]
    fn a_dropped_plugin_sends_nothing_and_gets_nothing() {
        let (port, accepted) = server(Duration::from_millis(300));
        let inbox = Arc::new(Inbox::default());
        let pool = Pool::new(1, WORKER_IDLE);

        // Queued: the only worker is busy until released.
        let (release, wait) = mpsc::channel::<()>();
        pool.submit(
            0,
            Box::new(move |_| {
                let _ = wait.recv();
            }),
        );
        let mut http = http_for(&inbox, &pool);
        fetch(&mut http, port);
        drop(http);
        release.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            accepted.load(Ordering::SeqCst),
            0,
            "a queued request was sent"
        );

        // Running: the server holds the answer for 300 ms.
        let mut http = http_for(&inbox, &pool);
        fetch(&mut http, port);
        // Waits for the server to take it (up to 2 s: a loaded machine is slow to connect).
        let start = std::time::Instant::now();
        while accepted.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
        drop(http);
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(
            inbox.len(),
            0,
            "the answer of a dropped plugin was delivered"
        );

        // The twin: a live plugin gets its answer.
        let mut http = http_for(&inbox, &pool);
        fetch(&mut http, port);
        let deadline = Instant::now() + Duration::from_secs(5);
        while inbox.len() == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(inbox.len(), 1);
    }
}
