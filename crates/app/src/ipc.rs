//! Single-instance IPC: a line-based protocol over a per-user local socket.

use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, bail};
use interprocess::local_socket::traits::{ListenerExt as _, Stream as _};
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, Name, Stream, ToFsName, ToNsName,
};
use tomakast_core::ItemId;

pub const SOCKET_NAME: &str = "tomakast.sock";

const REPLY_OK: &str = "ok";

/// Deadline for a client exchange (connect, write, reply) and for the stale-socket probe.
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(2);

/// Upper bound on the size of a request line.
const MAX_REQUEST_LEN: u64 = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Toggle,
    Run(ItemId),
    /// Open the settings window, on a page when one is given.
    Settings(Option<String>),
    Ping,
}

impl Request {
    pub fn encode(&self) -> String {
        match self {
            Request::Toggle => "toggle".to_string(),
            Request::Run(id) => format!("run {}", id.as_str()),
            Request::Settings(None) => "settings".to_string(),
            Request::Settings(Some(page)) => format!("settings {page}"),
            Request::Ping => "ping".to_string(),
        }
    }

    pub fn decode(line: &str) -> anyhow::Result<Request> {
        let line = line.trim_end_matches(['\r', '\n']);
        match line {
            "toggle" => Ok(Request::Toggle),
            "ping" => Ok(Request::Ping),
            "settings" => Ok(Request::Settings(None)),
            _ => match line.strip_prefix("run ") {
                Some(id) => ItemId::parse(id)
                    .map(Request::Run)
                    .with_context(|| format!("invalid item id {id:?}")),
                None => match line.strip_prefix("settings ") {
                    Some(page) => Ok(Request::Settings(
                        Some(page.to_string()).filter(|p| !p.is_empty()),
                    )),
                    None => bail!("unknown request {line:?}"),
                },
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("another tomakast instance is already running")]
    AlreadyRunning,
    #[error("cannot start the IPC server: {0}")]
    Io(#[from] io::Error),
}

/// Where a socket lives on this platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketLocation {
    File(PathBuf),
    Namespaced(String),
}

/// Resolves a bare socket name to the per-user platform location.
///
/// Linux: `$TOMAKAST_HOME/<name>` if set, else `$XDG_RUNTIME_DIR/<name>`.
/// macOS: `<temp dir>/<name>`. Windows: namespaced `<name>-<USERNAME>`.
pub fn resolve_socket(
    name: &str,
    env: impl Fn(&str) -> Option<OsString>,
) -> io::Result<SocketLocation> {
    #[cfg(windows)]
    {
        let user = env("USERNAME")
            .map(|u| u.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(SocketLocation::Namespaced(format!("{name}-{user}")))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = env;
        Ok(SocketLocation::File(std::env::temp_dir().join(name)))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let dir = env("TOMAKAST_HOME")
            .filter(|v| !v.is_empty())
            .or_else(|| env("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "neither TOMAKAST_HOME nor XDG_RUNTIME_DIR is set",
                )
            })?;
        Ok(SocketLocation::File(PathBuf::from(dir).join(name)))
    }
}

fn locate(name: &str) -> io::Result<SocketLocation> {
    resolve_socket(name, |k| std::env::var_os(k))
}

fn to_name(loc: &SocketLocation) -> io::Result<Name<'_>> {
    match loc {
        SocketLocation::File(path) => path.as_path().to_fs_name::<GenericFilePath>(),
        SocketLocation::Namespaced(n) => n.as_str().to_ns_name::<GenericNamespaced>(),
    }
}

/// Keeps the server alive; the socket file (if any) is removed on drop.
pub struct ServerHandle {
    location: SocketLocation,
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        if let SocketLocation::File(path) = &self.location {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn serve(handler: impl Fn(Request) + Send + 'static) -> Result<ServerHandle, ServeError> {
    serve_on(SOCKET_NAME, handler)
}

pub fn send(req: &Request) -> anyhow::Result<()> {
    send_to(SOCKET_NAME, req)
}

pub fn serve_on(
    name: &str,
    handler: impl Fn(Request) + Send + 'static,
) -> Result<ServerHandle, ServeError> {
    serve_at(locate(name)?, handler)
}

/// Like [`serve_on`] for an already resolved location.
pub fn serve_at(
    location: SocketLocation,
    handler: impl Fn(Request) + Send + 'static,
) -> Result<ServerHandle, ServeError> {
    let bind = || {
        ListenerOptions::new()
            .name(to_name(&location)?)
            .create_sync()
    };
    let listener = match bind() {
        Ok(l) => l,
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            let SocketLocation::File(path) = &location else {
                return Err(ServeError::AlreadyRunning);
            };
            // Only a definite failure to reach a server marks the socket as stale. A server
            // that accepts but does not answer in time may be alive, so we never delete its socket.
            match exchange_with_deadline(&location, &Request::Ping) {
                Exchange::Dead(_) => {}
                Exchange::Replied(_) | Exchange::TimedOut => {
                    return Err(ServeError::AlreadyRunning);
                }
            }
            tracing::info!(path = %path.display(), "removing stale IPC socket");
            std::fs::remove_file(path)?;
            bind()?
        }
        Err(e) => return Err(e.into()),
    };
    let handler = Arc::new(Mutex::new(handler));
    thread::Builder::new()
        .name("tomakast-ipc".into())
        .spawn(move || {
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        // One thread per connection: a stalled or panicking client cannot wedge
                        // the acceptor.
                        let handler = Arc::clone(&handler);
                        let spawned = thread::Builder::new()
                            .name("tomakast-ipc-conn".into())
                            .spawn(move || {
                                if let Err(e) = handle_connection(stream, &handler) {
                                    tracing::warn!("ipc connection failed: {e:#}");
                                }
                            });
                        if let Err(e) = spawned {
                            tracing::warn!("cannot spawn ipc connection thread: {e}");
                        }
                    }
                    Err(e) => tracing::warn!("ipc accept failed: {e}"),
                }
            }
        })?;
    Ok(ServerHandle { location })
}

fn handle_connection<H: Fn(Request)>(stream: Stream, handler: &Mutex<H>) -> anyhow::Result<()> {
    let mut reader = BufReader::new((&stream).take(MAX_REQUEST_LEN));
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let reply = if !line.ends_with('\n') {
        "err request too long or truncated".to_string()
    } else {
        match Request::decode(&line) {
            Ok(req) => {
                if req != Request::Ping {
                    // A handler panic poisons the lock; later requests must still be served.
                    let handler = handler.lock().unwrap_or_else(|e| e.into_inner());
                    handler(req);
                }
                REPLY_OK.to_string()
            }
            Err(e) => format!("err {e}"),
        }
    };
    let mut stream = &stream;
    stream.write_all(reply.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

pub fn send_to(name: &str, req: &Request) -> anyhow::Result<()> {
    send_at(&locate(name)?, req)
}

/// Like [`send_to`] for an already resolved location. Fails after [`CLIENT_TIMEOUT`].
pub fn send_at(location: &SocketLocation, req: &Request) -> anyhow::Result<()> {
    match exchange_with_deadline(location, req) {
        Exchange::Replied(r) => r,
        Exchange::Dead(e) => Err(e),
        Exchange::TimedOut => bail!("tomakast did not answer within {CLIENT_TIMEOUT:?}"),
    }
}

enum Exchange {
    /// The server was reached; carries the protocol outcome.
    Replied(anyhow::Result<()>),
    /// Could not connect or talk to anything.
    Dead(anyhow::Error),
    /// Connected (or still trying) but no answer before the deadline.
    TimedOut,
}

/// Runs the exchange on a worker thread so a wedged peer cannot block the caller.
fn exchange_with_deadline(location: &SocketLocation, req: &Request) -> Exchange {
    let (tx, rx) = mpsc::channel();
    let location = location.clone();
    let req = req.clone();
    let spawned = thread::Builder::new()
        .name("tomakast-ipc-client".into())
        .spawn(move || {
            let _ = tx.send(exchange(&location, &req));
        });
    if let Err(e) = spawned {
        return Exchange::Dead(e.into());
    }
    match rx.recv_timeout(CLIENT_TIMEOUT) {
        Ok(r) => r,
        Err(_) => Exchange::TimedOut,
    }
}

fn exchange(location: &SocketLocation, req: &Request) -> Exchange {
    let stream = match to_name(location).and_then(Stream::connect) {
        Ok(s) => s,
        Err(e) => return Exchange::Dead(anyhow::Error::new(e).context("connecting to tomakast")),
    };
    Exchange::Replied(talk(stream, req))
}

fn talk(stream: Stream, req: &Request) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream);
    let writer = reader.get_mut();
    writer.write_all(req.encode().as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    let mut reply = String::new();
    reader.read_line(&mut reply)?;
    if reply.trim_end() != REPLY_OK {
        bail!("unexpected reply {:?}", reply.trim_end());
    }
    Ok(())
}
