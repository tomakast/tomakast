//! A tiny HTTP/1.1 server on 127.0.0.1 for the plugin HTTP tests: fixed routes, no external
//! network.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const MIB: usize = 1024 * 1024;

/// A tiny HTTP/1.1 server on 127.0.0.1 that counts the connections it accepted.
pub struct Server {
    pub port: u16,
    accepted: Arc<AtomicUsize>,
}

impl Server {
    pub fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&accepted);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                count.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || serve(stream, port));
            }
        });
        Server { port, accepted }
    }

    pub fn url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.port)
    }

    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

fn serve(stream: TcpStream, port: u16) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let method = line.split(' ').next().unwrap_or_default().to_string();
    let path = line.split(' ').nth(1).unwrap_or("/").to_string();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
            break;
        }
        headers.push(header.trim_end().to_ascii_lowercase());
    }
    let length: usize = headers
        .iter()
        .find_map(|h| h.strip_prefix("content-length: "))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let mut received = vec![0; length];
    let _ = reader.read_exact(&mut received);
    if path.ends_with("-stream") {
        // No content-length: the body ends when the connection closes.
        let size = if path == "/big-stream" {
            4 * MIB + 1
        } else {
            4 * MIB
        };
        let mut stream = stream;
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n");
        let _ = stream.write_all(&vec![b'x'; size]);
        return;
    }
    let (status, extra, body): (&str, String, Vec<u8>) = match path.as_str() {
        // What arrived: the method, the body length and the headers, one per line.
        "/echo" => {
            let mut echo = format!("method={method}\nbody={}\n", received.len());
            for header in &headers {
                echo.push_str(header);
                echo.push('\n');
            }
            ("200 OK", String::new(), echo.into_bytes())
        }
        "/to-echo" => ("302 Found", "location: /echo\r\n".into(), Vec::new()),
        "/to-ip" => (
            "302 Found",
            format!("location: http://127.0.0.1:{port}/echo\r\n"),
            Vec::new(),
        ),
        "/see-other" => ("303 See Other", "location: /echo\r\n".into(), Vec::new()),
        "/temporary" => (
            "307 Temporary Redirect",
            "location: /echo\r\n".into(),
            Vec::new(),
        ),
        "/hello" => ("200 OK", "x-test: yes\r\n".into(), b"hello".to_vec()),
        "/redirect-in" => ("302 Found", "location: /hello\r\n".into(), Vec::new()),
        "/redirect-out" => (
            "302 Found",
            format!("location: http://127.0.0.1:{port}/hello\r\n"),
            Vec::new(),
        ),
        "/loop" => ("302 Found", "location: /loop\r\n".into(), Vec::new()),
        "/max" => ("200 OK", String::new(), vec![b'x'; 4 * MIB]),
        "/big" => ("200 OK", String::new(), vec![b'x'; 4 * MIB + 1]),
        "/hold" => {
            std::thread::sleep(Duration::from_millis(1500));
            ("200 OK", String::new(), b"held".to_vec())
        }
        _ => ("404 Not Found", String::new(), Vec::new()),
    };
    let mut stream = stream;
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n{extra}\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}
