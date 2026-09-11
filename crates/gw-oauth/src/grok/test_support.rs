//! Loopback HTTP/1.1 mock for Grok OAuth tests.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) struct Recorded {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

pub(crate) struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: String,
}

impl Reply {
    pub(crate) fn json(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into().into_bytes(),
            content_type: "application/json".to_owned(),
        }
    }

    pub(crate) fn bytes(status: u16, body: Vec<u8>, content_type: &str) -> Self {
        Self {
            status,
            body,
            content_type: content_type.to_owned(),
        }
    }
}

pub(crate) struct Server {
    pub base: String,
    pub seen: Arc<Mutex<Vec<Recorded>>>,
}

pub(crate) fn spawn(
    n: usize,
    handler: impl Fn(&Recorded) -> Reply + Send + Sync + 'static,
) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    listener.set_nonblocking(true).expect("nonblocking accept");
    let addr = listener.local_addr().expect("local addr");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_thread = Arc::clone(&seen);
    let handler = Arc::new(handler);
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut left = n;
        while left > 0 && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    // macOS inherits the listener's nonblocking flag. The reader
                    // below uses blocking I/O with a timeout, not readiness polling.
                    stream.set_nonblocking(false).expect("blocking mock stream");
                    let _ = stream.set_nodelay(true);
                    let Some(recorded) = read_request(&mut stream) else {
                        continue;
                    };
                    let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        handler(&recorded)
                    }))
                    .unwrap_or_else(|_| Reply::json(500, r#"{"error":"handler panic"}"#));
                    seen_thread
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(recorded);
                    let _ = write_reply(&mut stream, &reply);
                    left = left.saturating_sub(1);
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    Server {
        base: format!("http://{addr}"),
        seen,
    }
}

pub(crate) fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .build()
        .expect("reqwest client")
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Recorded> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    loop {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(header_end) = find_double_crlf(&buf) {
            let content_length = content_length(&buf[..header_end]).unwrap_or(0);
            while buf.len() < header_end + content_length {
                let n = stream.read(&mut tmp).ok()?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let head = std::str::from_utf8(&buf[..header_end]).ok()?;
            let mut lines = head.split("\r\n");
            let request_line = lines.next().unwrap_or("");
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or("").to_owned();
            let path = parts.next().unwrap_or("").to_owned();
            let body = buf
                .get(header_end..header_end + content_length)
                .unwrap_or(&[])
                .to_vec();
            return Some(Recorded { method, path, body });
        }
        if buf.len() > 1_000_000 {
            return None;
        }
    }
    None
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn content_length(headers: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(headers).ok()?;
    for line in text.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

fn write_reply(stream: &mut std::net::TcpStream, reply: &Reply) -> std::io::Result<()> {
    let reason = match reply.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status,
        reason,
        reply.content_type,
        reply.body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&reply.body)?;
    stream.flush()
}
