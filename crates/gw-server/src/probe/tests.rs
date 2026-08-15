use std::io::Read as _;
use std::net::TcpListener;
use std::thread;

use super::*;

/// Serve exactly one request with `response`, then close. Returns the address
/// to probe.
fn serve_once(response: &'static str) -> SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");

    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut scratch = [0u8; 1024];
        let _ = stream.read(&mut scratch);
        let _ = stream.write_all(response.as_bytes());
    });

    addr
}

/// An address nothing is listening on.
fn closed_port() -> SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr
}

#[test]
fn a_200_response_means_ready() {
    let addr = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    assert!(probe_readiness(addr, READINESS_PATH, TIMEOUT));
}

#[test]
fn a_503_response_means_not_ready() {
    // What /api/health/ready actually returns when Postgres or Redis is gone.
    let addr = serve_once(
        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\n\r\n{\"status\":\"not_ready\"}",
    );
    assert!(!probe_readiness(addr, READINESS_PATH, TIMEOUT));
}

#[test]
fn any_non_200_status_means_not_ready() {
    for response in [
        "HTTP/1.1 204 No Content\r\n\r\n",
        "HTTP/1.1 301 Moved Permanently\r\nLocation: /\r\n\r\n",
        "HTTP/1.1 404 Not Found\r\n\r\n",
        "HTTP/1.1 500 Internal Server Error\r\n\r\n",
    ] {
        let addr = serve_once(response);
        assert!(
            !probe_readiness(addr, READINESS_PATH, TIMEOUT),
            "{response}"
        );
    }
}

#[test]
fn a_refused_connection_means_not_ready() {
    assert!(!probe_readiness(closed_port(), READINESS_PATH, TIMEOUT));
}

#[test]
fn a_non_http_response_means_not_ready() {
    // Something else bound the port. Reporting "ready" here would keep a dead
    // container in the load balancer.
    for response in ["garbage\r\n\r\n", "\r\n", "HTTP/1.1\r\n\r\n"] {
        let addr = serve_once(response);
        assert!(
            !probe_readiness(addr, READINESS_PATH, TIMEOUT),
            "{response}"
        );
    }
}

#[test]
fn a_silent_peer_means_not_ready() {
    // Connection accepted, nothing written, socket closed.
    let addr = serve_once("");
    assert!(!probe_readiness(addr, READINESS_PATH, TIMEOUT));
}

#[test]
fn the_probe_asks_for_the_readiness_endpoint() {
    // The request line matters: probing /api/health (liveness) would report a
    // process with no database as ready.
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut scratch = [0u8; 1024];
        let read = stream.read(&mut scratch).unwrap_or(0);
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
        String::from_utf8_lossy(&scratch[..read]).into_owned()
    });

    assert!(probe_readiness(addr, READINESS_PATH, TIMEOUT));
    let request = handle.join().expect("server thread");
    assert!(
        request.starts_with("GET /api/health/ready HTTP/1.1\r\n"),
        "{request}"
    );
    assert!(request.contains("Connection: close\r\n"), "{request}");
}

#[test]
fn the_probed_port_follows_server_port() {
    // Unset / blank falls back to the shipped default, so a container that
    // never sets SERVER_PORT still gets a working HEALTHCHECK.
    assert_eq!(health_check_port(""), Some(DEFAULT_PORT));
    assert_eq!(health_check_port("   "), Some(DEFAULT_PORT));
    assert_eq!(health_check_port(" 9001 "), Some(9001));

    // Values that cannot address a listener are a failed health check, never a
    // silent probe of the default port.
    for raw in ["0", "http", "70000", "-1", "80.5"] {
        assert_eq!(health_check_port(raw), None, "{raw:?}");
    }
}

#[test]
fn status_lines_are_parsed_from_the_first_line_only() {
    let mut body = "HTTP/1.1 200 OK\r\nX-Trap: 503\r\n\r\n".as_bytes();
    assert_eq!(parse_status_line(&mut body), Some(200));

    let mut chunked = "HTTP/1.0 503 Service Unavailable\r\n\r\n".as_bytes();
    assert_eq!(parse_status_line(&mut chunked), Some(503));
}

#[test]
fn an_overlong_status_line_is_rejected() {
    // A peer that never sends CRLF must not make the probe read forever.
    let flood = "HTTP/1.1 200 ".to_owned() + &"O".repeat(4096);
    let mut bytes = flood.as_bytes();
    assert_eq!(parse_status_line(&mut bytes), None);
}
