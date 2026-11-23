use crate::dispatcher::QueuedEvent;
use crate::model::{Config, Event, Result};
use std::{
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, TrySendError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
const HTTP_WORKERS: usize = 4;
pub struct Server {
    stop: Arc<AtomicBool>,
    handles: Vec<thread::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}
fn response(s: &mut TcpStream, status: &str, body: &str) {
    let _=write!(s,"HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
}
fn reply(s: &mut TcpStream, status: &str, body: &str) {
    response(s, status, body);
    let _ = s.shutdown(Shutdown::Write);
    let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
    let deadline = Instant::now() + Duration::from_millis(400);
    let mut sink = [0u8; 1024];
    while Instant::now() < deadline {
        match s.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}
fn fields(header: &str) -> impl Iterator<Item = (&str, &str)> {
    header
        .split("\r\n")
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim(), v.trim()))
}
fn supported_target(header: &str) -> bool {
    let first = header.split("\r\n").next().unwrap_or("");
    first == "POST /v1/events HTTP/1.1" || first == "POST /v1/events HTTP/1.0"
}
fn authorized(header: &str, token: &str) -> bool {
    !token.is_empty()
        && fields(header).any(|(k, v)| {
            if !k.eq_ignore_ascii_case("authorization") {
                return false;
            }
            let mut parts = v.splitn(2, char::is_whitespace);
            parts.next().unwrap_or("").eq_ignore_ascii_case("bearer")
                && parts.next().unwrap_or("").trim() == token
        })
}
pub fn parse_request(header: &str, body: &[u8], token: &str) -> Result<Event> {
    if !supported_target(header) {
        return Err("仅支持 POST /v1/events".into());
    }
    if !authorized(header, token) {
        return Err("本地接口密钥不正确".into());
    }
    let event: Event = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    event.validate()?;
    Ok(event)
}
fn handle_connection(
    mut stream: TcpStream,
    tx: &SyncSender<QueuedEvent>,
    token: &str,
    stop: &AtomicBool,
) {
    // Windows accepted sockets inherit the listener's nonblocking mode, which
    // silently disables the read/write timeouts below; restore blocking I/O so
    // in-flight request bytes can be waited on instead of reset at close.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(600)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(600)));
    let mut bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut header_end = None;
    let mut chunk = [0u8; 2048];
    while bytes.len() < 16384 && Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
        }
        if let Some(pos) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            header_end = Some(pos + 4);
            break;
        }
    }
    let Some(end) = header_end else {
        reply(
            &mut stream,
            "400 Bad Request",
            "{\"error\":\"invalid headers\"}",
        );
        return;
    };
    let header = String::from_utf8_lossy(&bytes[..end]).to_string();
    if !supported_target(&header) {
        reply(
            &mut stream,
            "400 Bad Request",
            &serde_json::json!({"error":"仅支持 POST /v1/events"}).to_string(),
        );
        return;
    }
    if !authorized(&header, token) {
        reply(
            &mut stream,
            "400 Bad Request",
            &serde_json::json!({"error":"本地接口密钥不正确"}).to_string(),
        );
        return;
    }
    let mut lengths = Vec::new();
    let mut encoded = false;
    for (k, v) in fields(&header) {
        if k.eq_ignore_ascii_case("content-length") {
            lengths.push(v);
        } else if k.eq_ignore_ascii_case("transfer-encoding") {
            encoded = true;
        }
    }
    if encoded {
        reply(
            &mut stream,
            "400 Bad Request",
            "{\"error\":\"transfer-encoding not supported\"}",
        );
        return;
    }
    if lengths.len() > 1 {
        reply(
            &mut stream,
            "400 Bad Request",
            "{\"error\":\"conflicting content-length\"}",
        );
        return;
    }
    let length = match lengths.first().map(|v| v.parse::<usize>()) {
        None => {
            reply(
                &mut stream,
                "411 Length Required",
                "{\"error\":\"content-length required\"}",
            );
            return;
        }
        Some(Err(_)) => {
            reply(
                &mut stream,
                "400 Bad Request",
                "{\"error\":\"invalid content-length\"}",
            );
            return;
        }
        Some(Ok(n)) if n <= 12288 => n,
        Some(Ok(_)) => {
            reply(
                &mut stream,
                "413 Payload Too Large",
                "{\"error\":\"content-length exceeds 12288\"}",
            );
            return;
        }
    };
    while bytes.len() < end + length
        && Instant::now() < deadline
        && !stop.load(Ordering::Relaxed)
    {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
        }
    }
    if bytes.len() < end + length {
        reply(
            &mut stream,
            "400 Bad Request",
            "{\"error\":\"incomplete body\"}",
        );
        return;
    }
    match parse_request(&header, &bytes[end..end + length], token) {
        Ok(event) => match tx.try_send(QueuedEvent::new(event)) {
            Ok(()) => reply(&mut stream, "202 Accepted", "{\"queued\":true}"),
            Err(TrySendError::Full(_)) => reply(
                &mut stream,
                "429 Too Many Requests",
                "{\"error\":\"queue full\"}",
            ),
            Err(TrySendError::Disconnected(_)) => reply(
                &mut stream,
                "503 Service Unavailable",
                "{\"error\":\"notification worker stopped\"}",
            ),
        },
        Err(error) => reply(
            &mut stream,
            "400 Bad Request",
            &serde_json::json!({"error":error}).to_string(),
        ),
    }
}
pub fn start(c: &Config, tx: SyncSender<QueuedEvent>) -> Result<Server> {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], c.api_port)))
        .map_err(|e| format!("本地接口启动失败：{e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let listener = Arc::new(listener);
    let token: Arc<str> = Arc::from(c.api_token.as_str());
    let stop = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::with_capacity(HTTP_WORKERS);
    // Each fixed worker accepts only when free. Excess connections remain in the
    // OS listen backlog rather than becoming unbounded threads or application sockets.
    for index in 0..HTTP_WORKERS {
        let listener = Arc::clone(&listener);
        let token = Arc::clone(&token);
        let flag = Arc::clone(&stop);
        let tx = tx.clone();
        let handle = thread::Builder::new().name(format!("http-{index}")).spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => handle_connection(stream, &tx, &token, &flag),
                    Err(_) => thread::sleep(Duration::from_millis(30)),
                }
            }
        });
        match handle {
            Ok(handle) => handles.push(handle),
            Err(error) => {
                stop.store(true, Ordering::Relaxed);
                for handle in handles { let _ = handle.join(); }
                return Err(format!("本地接口工作线程启动失败：{error}"));
            }
        }
    }
    Ok(Server { stop, handles })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::Shutdown, sync::mpsc::sync_channel};
    fn post(body: &str) -> Vec<u8> {
        format!(
            "POST /v1/events HTTP/1.1\r\nAuthorization: Bearer local-secret\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .into_bytes()
    }
    fn round_trip(request: &[u8], tx: &SyncSender<QueuedEvent>) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client.write_all(request).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let (stream, _) = listener.accept().unwrap();
        handle_connection(stream, tx, "local-secret", &AtomicBool::new(false));
        let mut out = String::new();
        client.read_to_string(&mut out).unwrap();
        out
    }
    #[test]
    fn authenticated_bounded_event_contract() {
        let body = br#"{"host":"chatgpt","event":"done"}"#;
        let h = "POST /v1/events HTTP/1.1\r\nAuthorization: Bearer abc\r\n\r\n";
        assert!(parse_request(h, body, "abc").is_ok());
        assert!(parse_request(h, body, "other").is_err());
        assert!(parse_request(h, br#"{"host":"../bad","event":"done"}"#, "abc").is_err());
        let lower = "POST /v1/events HTTP/1.1\r\nAuthorization: bearer  abc\r\n\r\n";
        assert!(parse_request(lower, body, "abc").is_ok());
        let bare = "POST /v1/events HTTP/1.1\r\nAuthorization: Bearer\r\n\r\n";
        assert!(parse_request(bare, body, "").is_err());
        let get = "GET /v1/events HTTP/1.1\r\nAuthorization: Bearer abc\r\n\r\n";
        assert!(parse_request(get, body, "abc").is_err());
        let unknown = br#"{"host":"chatgpt","event":"bogus"}"#;
        assert!(parse_request(h, unknown, "abc").is_err());
        let long = format!(
            r#"{{"host":"chatgpt","event":"done","text":"{}"}}"#,
            "x".repeat(8193)
        );
        assert!(parse_request(h, long.as_bytes(), "abc").is_err());
        let identified = br#"{"host":"codex","event":"done","event_id":"turn-1","session_id":"session-1"}"#;
        let e = parse_request(h, identified, "abc").unwrap();
        assert_eq!((e.event_id.as_str(), e.session_id.as_str()), ("turn-1", "session-1"));
        for field in ["event_id", "session_id"] {
            let mut body = serde_json::json!({"host":"codex","event":"done"});
            body[field] = serde_json::Value::String("x".repeat(513));
            assert!(parse_request(h, body.to_string().as_bytes(), "abc").is_err());
        }
    }
    #[test]
    fn server_enqueues_valid_event() {
        let (tx, rx) = sync_channel(16);
        let out = round_trip(&post(r#"{"host":"codex","event":"ask","text":"确认","event_id":"permission-1","session_id":"session-1"}"#), &tx);
        assert!(out.starts_with("HTTP/1.1 202"), "{out}");
        assert!(out.contains("{\"queued\":true}"), "{out}");
        let e = rx.recv_timeout(Duration::from_secs(1)).unwrap().event;
        assert_eq!(
            (e.host.as_str(), e.event.as_str(), e.text.as_str()),
            ("codex", "ask", "确认")
        );
        assert_eq!((e.event_id.as_str(), e.session_id.as_str()), ("permission-1", "session-1"));
    }
    #[test]
    fn server_replies_when_rejected_client_keeps_write_side_open() {
        // The production listener is nonblocking and accepted sockets inherit
        // that mode on Windows; this client also never closes its write side,
        // so the body can still be in flight when the request is rejected.
        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let (tx, _rx) = sync_channel(16);
        let config = Config {
            api_port: port,
            api_token: "local-secret".into(),
            ..Config::default()
        };
        let _server = start(&config, tx).unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let body = r#"{"host":"chatgpt","event":"done"}"#;
        let header = format!(
            "POST /v1/events HTTP/1.1\r\nAuthorization: Bearer wrong\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        client.write_all(header.as_bytes()).unwrap();
        thread::sleep(Duration::from_millis(80));
        client.write_all(body.as_bytes()).unwrap();
        let mut out = String::new();
        client.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
        assert!(out.contains("本地接口密钥不正确"), "{out}");
    }
    #[test]
    fn server_returns_429_when_queue_is_full() {
        let (tx, _rx) = sync_channel(1);
        assert!(tx.try_send(QueuedEvent::new(Event {
            host: "chatgpt".into(),
            event: "done".into(),
            text: String::new(),
            ..Event::default()
        })).is_ok());
        let out = round_trip(&post(r#"{"host":"chatgpt","event":"done"}"#), &tx);
        assert!(out.starts_with("HTTP/1.1 429"), "{out}");
    }
    #[test]
    fn server_returns_503_if_notification_worker_has_stopped() {
        let (tx, rx) = sync_channel(1);
        drop(rx);
        let out = round_trip(&post(r#"{"host":"codex","event":"done"}"#), &tx);
        assert!(out.starts_with("HTTP/1.1 503"), "{out}");
    }

    #[test]
    fn slow_connections_do_not_block_a_complete_request() {
        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let (tx, rx) = sync_channel(16);
        let config = Config { api_port: port, api_token: "local-secret".into(), ..Config::default() };
        let _server = start(&config, tx).unwrap();
        let mut slow = Vec::new();
        for _ in 1..HTTP_WORKERS {
            let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
            client.write_all(b"POST /v1/events HTTP/1.1\r\nAuthorization: Bearer local-secret\r\nContent-Length: 100\r\n\r\n{").unwrap();
            slow.push(client);
        }
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        client.write_all(&post(r#"{"host":"codex","event":"done","event_id":"fast"}"#)).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 202"), "{response}");
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap().event.event_id, "fast");
    }

    #[test]
    fn server_rejects_bad_framing_before_body() {
        let (tx, _rx) = sync_channel(16);
        let out = round_trip(b"GET /v1/events HTTP/1.1\r\n\r\n", &tx);
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
        let out = round_trip(b"POST /v1/events HTTP/1.1\r\n\r\n", &tx);
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
        let out = round_trip(
            b"POST /v1/events HTTP/1.1\r\nAuthorization: Bearer local-secret\r\n\r\n",
            &tx,
        );
        assert!(out.starts_with("HTTP/1.1 411"), "{out}");
        let out = round_trip(
            b"POST /v1/events HTTP/1.1\r\nAuthorization: Bearer local-secret\r\nContent-Length: 12289\r\n\r\n",
            &tx,
        );
        assert!(out.starts_with("HTTP/1.1 413"), "{out}");
        let out = round_trip(
            b"POST /v1/events HTTP/1.1\r\nAuthorization: Bearer local-secret\r\nContent-Length: abc\r\n\r\n",
            &tx,
        );
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
        let out = round_trip(
            b"POST /v1/events HTTP/1.1\r\nAuthorization: Bearer local-secret\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\n",
            &tx,
        );
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
        let out = round_trip(
            b"POST /v1/events HTTP/1.1\r\nAuthorization: Bearer local-secret\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n",
            &tx,
        );
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
        let out = round_trip(
            b"POST /v1/events HTTP/1.1\r\nAuthorization: Bearer local-secret\r\nContent-Length: 30\r\n\r\n{}",
            &tx,
        );
        assert!(
            out.starts_with("HTTP/1.1 400") && out.contains("incomplete body"),
            "{out}"
        );
        let out = round_trip(&post(r#"{"host":"chatgpt"}"#), &tx);
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
        let mut garbage =
            b"POST /v1/events HTTP/1.1\r\nAuthorization: Bearer local-secret\r\nX-Pad: ".to_vec();
        garbage.extend(std::iter::repeat(b'x').take(17000));
        let out = round_trip(&garbage, &tx);
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
    }
}
