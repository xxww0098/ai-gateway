//! `--health-check`: probe the local readiness endpoint and turn the answer
//! into a process exit code.
//!
//! This exists because the distroless runtime image ships no shell and no
//! curl, so the container `HEALTHCHECK` re-executes the gateway binary
//! instead.
//!
//! Deliberately synchronous std-only sockets: the probe runs in a process that
//! does nothing else, and hand-writing one GET keeps an HTTP client out of the
//! dependency graph.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// The default probe port used when `SERVER_PORT` is empty (`"8888"`).
pub const DEFAULT_PORT: u16 = 8888;
/// The probe HTTP timeout (3 seconds).
pub const TIMEOUT: Duration = Duration::from_secs(3);
/// The endpoint probed. Owned by [`crate::health::readiness`].
pub const READINESS_PATH: &str = "/api/health/ready";

/// Exit code for a ready instance.
pub const EXIT_READY: i32 = 0;
/// Exit code for anything else — unreachable, non-200, or an unusable
/// `SERVER_PORT`.
pub const EXIT_NOT_READY: i32 = 1;

/// Read `SERVER_PORT`, probe loopback, return the process exit code.
pub fn health_check_exit_code() -> i32 {
    let Some(port) = health_check_port(&std::env::var("SERVER_PORT").unwrap_or_default()) else {
        // An unusable URL makes the GET fail; same outcome.
        return EXIT_NOT_READY;
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    if probe_readiness(addr, READINESS_PATH, TIMEOUT) {
        EXIT_READY
    } else {
        EXIT_NOT_READY
    }
}

/// Resolve the port to probe from a raw `SERVER_PORT` value.
///
/// Trims `SERVER_PORT` plus the `""` fallback; `None` means the value cannot
/// address a local server.
pub fn health_check_port(raw: &str) -> Option<u16> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(DEFAULT_PORT);
    }
    raw.parse::<u16>().ok().filter(|port| *port != 0)
}

/// `true` only when the endpoint answers 200.
///
/// Any failure — refused connection, timeout, malformed response, non-200 —
/// is "not ready"; a health check must never report success on doubt.
pub fn probe_readiness(addr: SocketAddr, path: &str, timeout: Duration) -> bool {
    read_status(addr, path, timeout) == Some(200)
}

fn read_status(addr: SocketAddr, path: &str, timeout: Duration) -> Option<u16> {
    let mut stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nUser-Agent: gw-server/{version}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        version = crate::cli::APP_VERSION,
    );
    stream.write_all(request.as_bytes()).ok()?;

    parse_status_line(&mut stream)
}

/// Read just far enough to see the status line: `HTTP/1.1 200 OK`.
fn parse_status_line<R: Read>(reader: &mut R) -> Option<u16> {
    // A status line longer than this is not something we want to keep reading.
    let mut buffer = [0u8; 128];
    let mut filled = 0;

    loop {
        if filled == buffer.len() {
            return None;
        }
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => return None,
            Ok(read) => filled += read,
            Err(_) => return None,
        }
        if let Some(end) = buffer[..filled].windows(2).position(|pair| pair == b"\r\n") {
            let line = std::str::from_utf8(&buffer[..end]).ok()?;
            let (version, rest) = line.split_once(' ')?;
            if !version.starts_with("HTTP/") {
                return None;
            }
            let code = rest.split(' ').next()?;
            return code.parse().ok();
        }
    }
}

#[cfg(test)]
mod tests;
