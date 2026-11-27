//! Minimal HTTP/1.1 server: one thread per connection, keep-alive, Content-Length bodies.
//!
//! Each connection is read by its own blocking thread, so every request is read and answered in
//! order. Only what the game and our tools send is supported: no chunked request bodies.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// Idle connections are closed after this long (the game opens new ones as needed).
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

pub struct Request {
    pub method: String,
    /// Path and query, as sent (e.g. "/cdn/host/file.png?v=1").
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(body: Vec<u8>) -> Self {
        Response { status: 200, content_type: None, body }
    }
    pub fn with_type(mut self, content_type: &str) -> Self {
        self.content_type = Some(content_type.to_string());
        self
    }
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
}

/// Accepts connections forever, calling `handler` for every request.
pub fn serve<F>(listener: TcpListener, handler: F)
where
    F: Fn(&Request) -> Response + Send + Sync + 'static,
{
    let handler = std::sync::Arc::new(handler);
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let handler = handler.clone();
        let _ = std::thread::Builder::new()
            .name("superstar-http".into())
            .spawn(move || {
                let _ = connection(stream, &*handler);
            });
    }
}

fn connection<F: Fn(&Request) -> Response>(stream: TcpStream, handler: &F) -> std::io::Result<()> {
    stream.set_read_timeout(Some(IDLE_TIMEOUT))?;
    let _ = stream.set_nodelay(true);
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    loop {
        let Some(req) = read_request(&mut reader)? else { return Ok(()) };
        let close = req.header("Connection").is_some_and(|v| v.eq_ignore_ascii_case("close"));
        let res = handler(&req);
        write_response(&mut writer, &req, &res, close)?;
        if close {
            return Ok(());
        }
    }
}

/// Reads one request; Ok(None) when the client closed the connection between requests.
fn read_request(reader: &mut BufReader<TcpStream>) -> std::io::Result<Option<Request>> {
    let bad = |msg: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string());
    let mut line = String::new();
    // Tolerate stray empty lines between requests.
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(url)) = (parts.next(), parts.next()) else { return Err(bad("bad request line")) };
    let (method, url) = (method.to_string(), url.to_string());

    let mut headers = Vec::new();
    let mut total = line.len();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        total += n;
        if n == 0 || total > MAX_HEADER_BYTES {
            return Err(bad("truncated or oversized headers"));
        }
        let l = line.trim_end_matches(['\r', '\n']);
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    let mut req = Request { method, url, headers, body: Vec::new() };
    let len: usize = req.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if len > MAX_BODY_BYTES {
        return Err(bad("body too large"));
    }
    if req.header("Expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        reader.get_mut().write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
    }
    req.body = vec![0; len];
    reader.read_exact(&mut req.body)?;
    Ok(Some(req))
}

fn write_response(w: &mut TcpStream, req: &Request, res: &Response, close: bool) -> std::io::Result<()> {
    let reason = match res.status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "",
    };
    let mut head = format!("HTTP/1.1 {} {reason}\r\nContent-Length: {}\r\n", res.status, res.body.len());
    if let Some(t) = &res.content_type {
        head.push_str(&format!("Content-Type: {t}\r\n"));
    }
    head.push_str(if close { "Connection: close\r\n\r\n" } else { "Connection: keep-alive\r\n\r\n" });
    w.write_all(head.as_bytes())?;
    if req.method != "HEAD" {
        w.write_all(&res.body)?;
    }
    w.flush()
}
