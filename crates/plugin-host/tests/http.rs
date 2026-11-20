//! Plugin HTTP: `http.fetch` against a local server on 127.0.0.1, allowed in the test through
//! a `localhost` allowlist entry. No external network.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use support::harness::*;
use support::server::Server;
use tomakast_plugin_host::{DomainPattern, Responder};

const SINK: &str = "kitchen-sink/indexed";

fn allow_localhost(package: &mut tomakast_plugin_host::Package) {
    package.manifest.permissions.http = vec![DomainPattern::parse("localhost").unwrap()];
}

fn fetch(h: &mut Harness, url: &str) -> String {
    send(h, &format!("GET {url}"))
}

/// `fetch:<request>` through the kitchen-sink: `<METHOD> <url>[|<header>=<value>]…[|body=…]`.
fn send(h: &mut Harness, request: &str) -> String {
    let effects = h
        .run(SINK, &format!("fetch:{request}"))
        .expect("run answers");
    metadata(&effects)["result"].clone()
}

fn allow_localhost_and_ip(package: &mut tomakast_plugin_host::Package) {
    package.manifest.permissions.http = ["localhost", "127.0.0.1"]
        .map(|d| DomainPattern::parse(d).unwrap())
        .to_vec();
}

fn done(h: &mut Harness) -> BTreeMap<String, String> {
    next_detail(h, "http-done", Duration::from_secs(10)).expect("http-done arrives")
}

#[test]
fn http_get_allowed_domain_roundtrip() {
    let server = Server::start();
    let mut h = harness_with(allow_localhost, BTreeMap::new());
    assert_eq!(fetch(&mut h, &server.url("localhost", "/hello")), "Ok(1)");
    let answer = done(&mut h);
    assert_eq!(answer["request"], "1");
    assert_eq!(answer["status"], "200", "{answer:?}");
    assert_eq!(answer["body"], "hello");
    assert_eq!(answer["x-test"], "yes");
    assert_eq!(server.accepted(), 1);

    // A redirect to an allowed host is followed by the host.
    assert_eq!(
        fetch(&mut h, &server.url("localhost", "/redirect-in")),
        "Ok(2)"
    );
    let answer = done(&mut h);
    assert_eq!(
        (answer["status"].as_str(), answer["body"].as_str()),
        ("200", "hello")
    );
    // Status codes are answers, not errors.
    assert_eq!(fetch(&mut h, &server.url("localhost", "/nope")), "Ok(3)");
    assert_eq!(done(&mut h)["status"], "404");
}

#[test]
fn http_denied_domain_errors_without_request() {
    let server = Server::start();
    let mut h = harness_with(allow_localhost, BTreeMap::new());
    // The same server under a name that is not on the list: an IP literal, a look-alike.
    for url in [
        server.url("127.0.0.1", "/hello"),
        server.url("localhost.localdomain", "/hello"),
        format!("http://localhost@127.0.0.1:{}/hello", server.port),
        format!("http://user:pw@localhost:{}/hello", server.port),
        server.url("[::1]", "/hello"),
        format!("ftp://localhost:{}/hello", server.port),
        "file:///etc/passwd".to_string(),
    ] {
        let refused = fetch(&mut h, &url);
        assert!(refused.starts_with("Err"), "{url}: {refused}");
    }
    // Without the permission nothing is allowed.
    let mut bare = harness();
    let refused = fetch(&mut bare, &server.url("localhost", "/hello"));
    assert!(
        refused.starts_with("Err") && refused.contains("permission"),
        "{refused}"
    );
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(server.accepted(), 0, "a refused fetch sent a request");
    assert_eq!(
        next_detail(&mut h, "http-done", Duration::from_millis(100)),
        None
    );

    // The allowed twin reaches the server.
    assert_eq!(fetch(&mut h, &server.url("localhost", "/hello")), "Ok(1)");
    assert_eq!(done(&mut h)["status"], "200");
    assert_eq!(server.accepted(), 1);
}

#[test]
fn http_redirect_to_unlisted_domain_is_blocked() {
    let server = Server::start();
    let mut h = harness_with(allow_localhost, BTreeMap::new());
    assert_eq!(
        fetch(&mut h, &server.url("localhost", "/redirect-out")),
        "Ok(1)"
    );
    let answer = done(&mut h);
    let error = answer.get("error").expect("an error answer");
    assert!(error.contains("not allowed"), "{answer:?}");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(server.accepted(), 1, "the redirect target was requested");

    // At most 5 redirects.
    assert_eq!(fetch(&mut h, &server.url("localhost", "/loop")), "Ok(2)");
    let answer = done(&mut h);
    assert!(answer["error"].contains("redirect"), "{answer:?}");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        server.accepted(),
        1 + 6,
        "the first request and five redirects"
    );
}

#[test]
fn http_body_cap() {
    let server = Server::start();
    let mut h = harness_with(allow_localhost, BTreeMap::new());
    assert_eq!(fetch(&mut h, &server.url("localhost", "/max")), "Ok(1)");
    let answer = done(&mut h);
    assert_eq!(
        answer.get("body-len").map(String::as_str),
        Some("4194304"),
        "{answer:?}"
    );
    assert_eq!(fetch(&mut h, &server.url("localhost", "/big")), "Ok(2)");
    let answer = done(&mut h);
    let error = answer.get("error").expect("an error answer");
    assert!(error.contains("4 MiB"), "{answer:?}");
}

#[test]
fn http_concurrency_cap() {
    let server = Server::start();
    let mut h = harness_with(allow_localhost, BTreeMap::new());
    for i in 1..=4 {
        assert_eq!(
            fetch(&mut h, &server.url("localhost", "/hold")),
            format!("Ok({i})")
        );
    }
    let refused = fetch(&mut h, &server.url("localhost", "/hold"));
    assert!(
        refused.starts_with("Err") && refused.contains('4'),
        "{refused}"
    );
    for _ in 0..4 {
        assert_eq!(done(&mut h)["body"], "held");
    }
    // Delivered answers free their slots.
    assert_eq!(fetch(&mut h, &server.url("localhost", "/hello")), "Ok(5)");
    assert_eq!(done(&mut h)["body"], "hello");
}

/// Finding 3: credentials stay with their origin. Both hosts are allowed; the redirect from
/// `localhost` to `127.0.0.1` drops `authorization` and `cookie` and keeps other headers.
#[test]
fn cross_origin_redirect_drops_credentials() {
    let server = Server::start();
    let mut h = harness_with(allow_localhost_and_ip, BTreeMap::new());
    let credentials = "|authorization=Bearer secret|cookie=session=1|x-keep=yes";
    let url = server.url("localhost", "/to-ip");
    assert_eq!(send(&mut h, &format!("GET {url}{credentials}")), "Ok(1)");
    let echo = done(&mut h)["body"].clone();
    assert!(echo.contains("x-keep: yes"), "{echo}");
    assert!(
        !echo.contains("authorization") && !echo.contains("cookie"),
        "{echo}"
    );

    // The twin: a same-origin redirect keeps them.
    let url = server.url("localhost", "/to-echo");
    assert_eq!(send(&mut h, &format!("GET {url}{credentials}")), "Ok(2)");
    let echo = done(&mut h)["body"].clone();
    assert!(echo.contains("authorization: bearer secret"), "{echo}");
    assert!(echo.contains("cookie: session=1"), "{echo}");
}

/// Finding 3: a POST redirected with 303 arrives as a GET without its body; 307 keeps both.
#[test]
fn see_other_turns_a_post_into_a_bodyless_get() {
    let server = Server::start();
    let mut h = harness_with(allow_localhost, BTreeMap::new());
    let post = |path: &str| {
        let url = server.url("localhost", path);
        format!("POST {url}|content-type=text/plain|body=hello")
    };
    assert_eq!(send(&mut h, &post("/see-other")), "Ok(1)");
    let echo = done(&mut h)["body"].clone();
    assert!(echo.starts_with("method=GET\nbody=0\n"), "{echo}");
    assert!(!echo.contains("content-type"), "{echo}");

    assert_eq!(send(&mut h, &post("/temporary")), "Ok(2)");
    let echo = done(&mut h)["body"].clone();
    assert!(echo.starts_with("method=POST\nbody=5\n"), "{echo}");
    assert!(echo.contains("content-type: text/plain"), "{echo}");
}

/// Finding 10: a body without content-length (ended by closing the connection) is capped too.
#[test]
fn http_body_cap_without_content_length() {
    let server = Server::start();
    let mut h = harness_with(allow_localhost, BTreeMap::new());
    assert_eq!(
        fetch(&mut h, &server.url("localhost", "/max-stream")),
        "Ok(1)"
    );
    let answer = done(&mut h);
    assert_eq!(
        answer.get("body-len").map(String::as_str),
        Some("4194304"),
        "{answer:?}"
    );
    assert_eq!(
        fetch(&mut h, &server.url("localhost", "/big-stream")),
        "Ok(2)"
    );
    let answer = done(&mut h);
    let error = answer.get("error").expect("an error answer");
    assert!(error.contains("4 MiB"), "{answer:?}");
}

#[test]
fn a_test_responder_answers_only_allowed_requests() {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&seen);
    let responder: Responder = Arc::new(move |method, url| {
        log.lock().unwrap().push(format!("{method} {url}"));
        Ok((200, b"faked".to_vec()))
    });
    let mut h = harness_built(
        |package| {
            package.manifest.permissions.http = vec![DomainPattern::parse("example.com").unwrap()]
        },
        BTreeMap::new(),
        |plugin| plugin.with_http_responder(responder),
    );
    assert_eq!(fetch(&mut h, "https://example.com/x"), "Ok(1)");
    let answer = done(&mut h);
    assert_eq!(
        (answer["status"].as_str(), answer["body"].as_str()),
        ("200", "faked")
    );
    let refused = fetch(&mut h, "https://example.org/x");
    assert!(
        refused.starts_with("Err") && refused.contains("not allowed"),
        "{refused}"
    );
    assert_eq!(*seen.lock().unwrap(), ["GET https://example.com/x"]);
}
