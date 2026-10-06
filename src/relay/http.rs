//! HTTP/1.1 framing for the relay on a std `TcpListener`: one request per connection and one thread each, at most
//! 256 at once, strict framing, deadlines, and bodies read through a reader capped at `Content-Length`. It never
//! prints.

use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// A request whose head passed the framing checks.
pub struct Request {
    pub method: String,
    /// The request target exactly as received.
    pub target: String,
    /// Header names in lowercase, values as received, in order.
    pub headers: Vec<(String, String)>,
    /// `Content-Length`, when the request carries one.
    pub length: Option<u64>,
    pub peer: IpAddr,
}

impl Request {
    /// The value of header `name`, given in lowercase.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A response. `serve` adds `Content-Length`, `Connection: close` and `Bilbo-Time`.
#[derive(Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16, body: Vec<u8>) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body,
        }
    }

    /// A refusal, with the body `{"error":"<reason>"}`.
    pub fn error(status: u16, reason: &str) -> Response {
        Response::new(status, format!("{{\"error\":\"{reason}\"}}").into_bytes())
    }

    pub fn with(mut self, name: &str, value: &str) -> Response {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

/// What the handler decides before a body is read.
pub enum Head {
    /// Answer now; the body is not read, and the connection is drained before it closes.
    Refuse(Response),
    /// Read the body (sending `100 Continue` when the request expects it), then call `answer`.
    Read,
}

/// The relay's side of a connection.
pub trait Handler: Sync {
    /// The relay's clock in Unix seconds, for `Bilbo-Time` on every response, those `serve` writes itself included.
    fn now(&self) -> u64;

    /// The checks that need no body. A `PUT`'s length is known here, so a body too large for its path is refused
    /// before it is read.
    fn head(&self, request: &Request) -> Head;

    /// Answers a request whose head was `Read`; `body` yields exactly its `Content-Length` bytes, none for a `GET`.
    fn answer(&self, request: &Request, body: &mut dyn Read) -> Response;
}

/// The connection limits.
pub struct Limits {
    pub connections: usize,
    /// From the accept to the end of the headers.
    pub head: Duration,
    /// The longest a body may send nothing.
    pub idle: Duration,
    /// From the accept to the end of the response: the body read and the response write together.
    pub body: Duration,
    /// The largest request line and headers (431 beyond).
    pub header_bytes: usize,
    /// How long a refused connection is drained before it closes.
    pub drain: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            connections: 256,
            head: Duration::from_secs(10),
            idle: Duration::from_secs(30),
            body: Duration::from_secs(300),
            header_bytes: 16 * 1024,
            drain: Duration::from_secs(2),
        }
    }
}

/// A request head as `httparse` reads it.
type Parsed<'h, 'b> = httparse::Request<'h, 'b>;

/// The most header lines a request may carry (431 beyond).
const HEADERS_MAX: usize = 64;

/// A parsed head: the request, its length in bytes, and whether it expects `100 Continue`.
struct Framed {
    request: Request,
    size: usize,
    expect: bool,
}

/// Why a connection stops before the handler sees a request.
enum Stop {
    /// Close without a word: the client went away or ran out of time.
    Close,
    Answer(Response),
}

/// Releases one slot of the connection count when its thread ends.
struct Slot<'a>(&'a AtomicUsize);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Serves `listener` with `handler` until `stop` is set and a connection wakes the accept loop.
pub fn serve(listener: &TcpListener, handler: &dyn Handler, limits: &Limits, stop: &AtomicBool) {
    let active = AtomicUsize::new(0);
    thread::scope(|scope| {
        loop {
            let (stream, _) = match listener.accept() {
                Ok(accepted) => accepted,
                Err(_) => {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
            };
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let started = Instant::now();
            if active.fetch_add(1, Ordering::SeqCst) >= limits.connections {
                active.fetch_sub(1, Ordering::SeqCst);
                busy(stream, handler);
                continue;
            }
            let slot = Slot(&active);
            // A failed spawn drops the closure with its stream and slot: the client sees a close and retries.
            let _ = thread::Builder::new().spawn_scoped(scope, move || {
                let _slot = slot;
                connection(stream, started, handler, limits);
            });
        }
    });
}

/// Answers 503 at the cap without waiting for the client.
fn busy(mut stream: TcpStream, handler: &dyn Handler) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let response = Response::error(503, "busy").with("Retry-After", "5");
    let _ = write_response(&mut stream, handler, &response);
    let _ = stream.shutdown(Shutdown::Write);
    // Take what the client already sent, so closing does not reset the connection under the answer.
    if stream.set_nonblocking(true).is_ok() {
        let _ = stream.read(&mut [0; 16 * 1024]);
    }
}

/// One connection: one request, one response, then close.
fn connection(stream: TcpStream, started: Instant, handler: &dyn Handler, limits: &Limits) {
    let Ok(peer) = stream.peer_addr().map(|a| a.ip()) else {
        return;
    };
    let until = started + limits.body;
    let (buf, framed) = match read_head(&stream, started, limits, peer) {
        Ok(read) => read,
        Err(Stop::Close) => return,
        Err(Stop::Answer(response)) => {
            return refuse(stream, handler, limits, until, &response);
        }
    };
    let Framed {
        request,
        size,
        expect,
    } = framed;
    match panic::catch_unwind(AssertUnwindSafe(|| handler.head(&request))) {
        Err(_) => refuse(
            stream,
            handler,
            limits,
            until,
            &Response::error(500, "internal"),
        ),
        Ok(Head::Refuse(response)) => refuse(stream, handler, limits, until, &response),
        Ok(Head::Read) => {
            let mut writer = Bounded::new(&stream, until, limits.idle);
            if expect && writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").is_err() {
                return;
            }
            let mut body = Body {
                stream: &stream,
                pending: &buf[size..],
                remaining: request.length.unwrap_or(0),
                idle: limits.idle,
                until,
            };
            let response =
                panic::catch_unwind(AssertUnwindSafe(|| handler.answer(&request, &mut body)))
                    .unwrap_or_else(|_| Response::error(500, "internal"));
            let unread = body.remaining > 0;
            if write_response(&mut writer, handler, &response).is_err() {
                return;
            }
            if unread {
                drain(&stream, limits.drain);
            }
        }
    }
}

/// Sends `response` before the body is read, stops writing and discards what the client still sends.
fn refuse(
    stream: TcpStream,
    handler: &dyn Handler,
    limits: &Limits,
    until: Instant,
    response: &Response,
) {
    let mut writer = Bounded::new(&stream, until, limits.idle);
    if write_response(&mut writer, handler, response).is_err() {
        return;
    }
    drain(&stream, limits.drain);
}

/// Shuts down writes, then reads and discards for up to `window` or until the client closes.
fn drain(stream: &TcpStream, window: Duration) {
    let _ = stream.shutdown(Shutdown::Write);
    let until = Instant::now() + window;
    let mut reader = stream;
    let mut sink = [0; 16 * 1024];
    loop {
        let Some(left) = until
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
        else {
            return;
        };
        if stream.set_read_timeout(Some(left)).is_err() {
            return;
        }
        match reader.read(&mut sink) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

/// Reads until the head is complete: the bytes read (the head, then the start of the body) and the head parsed.
fn read_head(
    stream: &TcpStream,
    started: Instant,
    limits: &Limits,
    peer: IpAddr,
) -> Result<(Vec<u8>, Framed), Stop> {
    let mut reader = stream;
    let mut buf = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        if let Some(framed) = parse(&buf, limits, peer).map_err(Stop::Answer)? {
            return Ok((buf, framed));
        }
        let left = limits
            .head
            .checked_sub(started.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or(Stop::Close)?;
        stream
            .set_read_timeout(Some(left))
            .map_err(|_| Stop::Close)?;
        match reader.read(&mut chunk) {
            Ok(0) => return Err(Stop::Close),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(Stop::Close),
        }
    }
}

fn bad_request() -> Response {
    Response::error(400, "bad-request")
}

fn too_large() -> Response {
    Response::error(431, "bad-request")
}

/// Parses `buf` as a request head: `None` while it is incomplete, a refusal when it is not acceptable.
fn parse(buf: &[u8], limits: &Limits, peer: IpAddr) -> Result<Option<Framed>, Response> {
    if matches!(buf.first(), Some(b'\r' | b'\n')) {
        return Err(bad_request());
    }
    let mut slots = [httparse::EMPTY_HEADER; HEADERS_MAX];
    let mut parsed = Parsed::new(&mut slots);
    let size = match parsed.parse(buf) {
        Ok(httparse::Status::Complete(size)) => size,
        Ok(httparse::Status::Partial) => {
            return if buf.len() >= limits.header_bytes {
                Err(too_large())
            } else {
                Ok(None)
            };
        }
        Err(httparse::Error::TooManyHeaders) => return Err(too_large()),
        Err(_) => return Err(bad_request()),
    };
    if size > limits.header_bytes {
        return Err(too_large());
    }
    if !crlf_only(&buf[..size]) || parsed.version != Some(1) {
        return Err(bad_request());
    }
    let (Some(method), Some(target)) = (parsed.method, parsed.path) else {
        return Err(bad_request());
    };
    if !target.starts_with('/') || target.contains('#') {
        return Err(bad_request());
    }
    let mut headers = Vec::with_capacity(parsed.headers.len());
    let mut length = None;
    let mut hosts = 0;
    let mut expect = false;
    for header in parsed.headers.iter() {
        let name = header.name.to_ascii_lowercase();
        let value = std::str::from_utf8(header.value)
            .map_err(|_| bad_request())?
            .trim_matches([' ', '\t']);
        match name.as_str() {
            "transfer-encoding" => return Err(bad_request()),
            "content-length" => {
                let plain = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
                let plain = plain && (value == "0" || !value.starts_with('0'));
                match value.parse::<u64>() {
                    Ok(n) if plain && length.is_none() => length = Some(n),
                    _ => return Err(bad_request()),
                }
            }
            "host" => hosts += 1,
            n if n.starts_with("bilbo-") && headers.iter().any(|(h, _)| h == n) => {
                return Err(bad_request());
            }
            "expect" => {
                if expect || !value.eq_ignore_ascii_case("100-continue") {
                    return Err(bad_request());
                }
                expect = true;
            }
            _ => {}
        }
        headers.push((name, value.to_string()));
    }
    if hosts != 1 || (method == "GET" && length.is_some_and(|n| n > 0)) {
        return Err(bad_request());
    }
    if method == "PUT" && length.is_none() {
        return Err(Response::error(411, "bad-request"));
    }
    Ok(Some(Framed {
        request: Request {
            method: method.to_string(),
            target: target.to_string(),
            headers,
            length,
            peer,
        },
        size,
        expect,
    }))
}

/// Whether every line of `head` ends in CRLF, with no bare CR or LF anywhere.
fn crlf_only(head: &[u8]) -> bool {
    head.iter().enumerate().all(|(i, &b)| match b {
        b'\n' => i > 0 && head[i - 1] == b'\r',
        b'\r' => head.get(i + 1) == Some(&b'\n'),
        _ => true,
    })
}

/// The body of a request: what arrived with the head, then the socket, up to `Content-Length` bytes.
struct Body<'a> {
    stream: &'a TcpStream,
    pending: &'a [u8],
    remaining: u64,
    idle: Duration,
    /// When the whole request must be over, whatever the client keeps sending.
    until: Instant,
}

/// The time left before `until`, at most `idle`; a `TimedOut` error once none is left.
fn allowance(until: Instant, idle: Duration) -> io::Result<Duration> {
    until
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .map(|left| left.min(idle))
        .ok_or_else(|| io::ErrorKind::TimedOut.into())
}

/// A socket writer whose every write waits at most `idle`, and none past `until`.
struct Bounded<'a> {
    stream: &'a TcpStream,
    until: Instant,
    idle: Duration,
}

impl<'a> Bounded<'a> {
    fn new(stream: &'a TcpStream, until: Instant, idle: Duration) -> Bounded<'a> {
        Bounded {
            stream,
            until,
            idle,
        }
    }
}

impl Write for Bounded<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.stream
            .set_write_timeout(Some(allowance(self.until, self.idle)?))?;
        (&*self.stream).write(bytes).map_err(|e| match e.kind() {
            io::ErrorKind::WouldBlock => io::ErrorKind::TimedOut.into(),
            _ => e,
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for Body<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let want = out
            .len()
            .min(usize::try_from(self.remaining).unwrap_or(usize::MAX));
        if want == 0 {
            return Ok(0);
        }
        let n = if self.pending.is_empty() {
            self.stream
                .set_read_timeout(Some(allowance(self.until, self.idle)?))?;
            let n = (&*self.stream)
                .read(&mut out[..want])
                .map_err(|e| match e.kind() {
                    io::ErrorKind::WouldBlock => io::ErrorKind::TimedOut.into(),
                    _ => e,
                })?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            n
        } else {
            let n = want.min(self.pending.len());
            out[..n].copy_from_slice(&self.pending[..n]);
            self.pending = &self.pending[n..];
            n
        };
        self.remaining -= n as u64;
        Ok(n)
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        507 => "Insufficient Storage",
        _ => "Status",
    }
}

/// Writes `response` with `Content-Length`, `Connection: close` and `Bilbo-Time`; the handler's own
/// copies of those headers are dropped, and a header that could split the response becomes a 500.
fn write_response(
    out: &mut impl Write,
    handler: &dyn Handler,
    response: &Response,
) -> io::Result<()> {
    let splits = |s: &str| s.contains(['\r', '\n']);
    let internal;
    let response = if response.headers.iter().any(|(n, v)| splits(n) || splits(v)) {
        internal = Response::error(500, "internal");
        &internal
    } else {
        response
    };
    let mut bytes = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        reason(response.status)
    )
    .into_bytes();
    for (name, value) in &response.headers {
        let own = ["content-length", "connection", "bilbo-time"];
        if !own.iter().any(|o| name.eq_ignore_ascii_case(o)) {
            bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
    }
    bytes.extend_from_slice(
        format!(
            "Content-Length: {}\r\nConnection: close\r\nBilbo-Time: {}\r\n\r\n",
            response.body.len(),
            handler.now()
        )
        .as_bytes(),
    );
    bytes.extend_from_slice(&response.body);
    out.write_all(&bytes)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::Mutex;

    type Seen = (String, String, Vec<(String, String)>, Option<u64>, IpAddr);

    /// Echoes the body of `PUT /echo`, refuses `/refuse`, and records what it saw.
    #[derive(Default)]
    struct Probe {
        heads: AtomicUsize,
        answers: AtomicUsize,
        read_errors: AtomicUsize,
        seen: Mutex<Vec<Seen>>,
    }

    impl Handler for Probe {
        fn now(&self) -> u64 {
            1234
        }

        fn head(&self, request: &Request) -> Head {
            self.heads.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push((
                request.method.clone(),
                request.target.clone(),
                request.headers.clone(),
                request.length,
                request.peer,
            ));
            match request.target.as_str() {
                "/refuse" => Head::Refuse(Response::error(413, "too-large")),
                "/panic-head" => panic!("head"),
                _ => Head::Read,
            }
        }

        fn answer(&self, request: &Request, body: &mut dyn Read) -> Response {
            self.answers.fetch_add(1, Ordering::SeqCst);
            match request.target.as_str() {
                "/panic" => panic!("answer"),
                "/headers" => {
                    return Response::new(200, b"h".to_vec())
                        .with("Content-Length", "999")
                        .with("connection", "keep-alive")
                        .with("X-A", "1");
                }
                "/split" => return Response::new(200, Vec::new()).with("X-A", "1\r\nX-B: 2"),
                _ => {}
            }
            let mut got = Vec::new();
            match body.read_to_end(&mut got) {
                Ok(_) => Response::new(200, got),
                Err(_) => {
                    self.read_errors.fetch_add(1, Ordering::SeqCst);
                    Response::error(400, "bad-request")
                }
            }
        }
    }

    fn quick() -> Limits {
        Limits {
            connections: 256,
            head: Duration::from_secs(10),
            idle: Duration::from_secs(10),
            body: Duration::from_secs(60),
            header_bytes: 1024,
            drain: Duration::from_secs(5),
        }
    }

    /// Runs `serve` on 127.0.0.1:0 while `f` talks to it, then stops it through the accept loop.
    fn with_server<T>(limits: Limits, f: impl FnOnce(SocketAddr, &Probe) -> T) -> T {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = AtomicBool::new(false);
        let probe = Probe::default();
        thread::scope(|scope| {
            scope.spawn(|| serve(&listener, &probe, &limits, &stop));
            let out = panic::catch_unwind(AssertUnwindSafe(|| f(addr, &probe)));
            stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(addr);
            match out {
                Ok(out) => out,
                Err(e) => panic::resume_unwind(e),
            }
        })
    }

    fn connect(addr: SocketAddr) -> TcpStream {
        let stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        stream
    }

    /// Sends `raw` and reads until the server closes.
    fn exchange(addr: SocketAddr, raw: &[u8]) -> String {
        let mut stream = connect(addr);
        stream.write_all(raw).unwrap();
        read_all(&mut stream)
    }

    fn read_all(stream: &mut TcpStream) -> String {
        let mut out = Vec::new();
        let _ = stream.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    fn status(response: &str) -> u16 {
        response
            .strip_prefix("HTTP/1.1 ")
            .and_then(|r| r.get(..3))
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("no status in {response:?}"))
    }

    fn body(response: &str) -> &str {
        response.split_once("\r\n\r\n").map_or("", |(_, b)| b)
    }

    fn head_of(response: &str) -> String {
        response
            .split_once("\r\n\r\n")
            .map_or("", |(h, _)| h)
            .to_ascii_lowercase()
    }

    fn get(path: &str, extra: &str) -> Vec<u8> {
        format!("GET {path} HTTP/1.1\r\nHost: relay\r\n{extra}\r\n").into_bytes()
    }

    fn put(path: &str, extra: &str, payload: &str) -> Vec<u8> {
        format!("PUT {path} HTTP/1.1\r\nHost: relay\r\n{extra}\r\n{payload}").into_bytes()
    }

    #[test]
    fn a_get_is_answered_with_the_framing_headers() {
        with_server(quick(), |addr, probe| {
            let response = exchange(addr, &get("/v1/", ""));
            assert_eq!(status(&response), 200);
            let head = head_of(&response);
            assert!(head.contains("\r\ncontent-length: 0"), "{head}");
            assert!(head.contains("\r\nconnection: close"), "{head}");
            assert!(head.contains("\r\nbilbo-time: 1234"), "{head}");
            let seen = probe.seen.lock().unwrap();
            let (method, target, headers, length, peer) = &seen[0];
            assert_eq!((method.as_str(), target.as_str()), ("GET", "/v1/"));
            assert_eq!(headers, &[("host".to_string(), "relay".to_string())]);
            assert_eq!(*length, None);
            assert!(peer.is_loopback());
        });
    }

    #[test]
    fn a_put_body_reaches_the_handler_whole() {
        with_server(quick(), |addr, probe| {
            let response = exchange(addr, &put("/echo", "Content-Length: 5\r\n", "hello"));
            assert_eq!(status(&response), 200);
            assert_eq!(body(&response), "hello");
            assert_eq!(probe.seen.lock().unwrap()[0].3, Some(5));
        });
    }

    #[test]
    fn a_body_in_pieces_is_read_whole() {
        with_server(quick(), |addr, _| {
            let mut stream = connect(addr);
            stream
                .write_all(&put("/echo", "Content-Length: 6\r\n", "abc"))
                .unwrap();
            stream.write_all(b"def").unwrap();
            let response = read_all(&mut stream);
            assert_eq!(body(&response), "abcdef");
        });
    }

    #[test]
    fn the_reader_stops_at_the_length_and_one_request_is_served() {
        with_server(quick(), |addr, probe| {
            let mut raw = put("/echo", "Content-Length: 3\r\n", "abc");
            raw.extend(get("/v1/", ""));
            let response = exchange(addr, &raw);
            assert_eq!(body(&response), "abc");
            assert_eq!(response.matches("HTTP/1.1 ").count(), 1);
            assert_eq!(probe.heads.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn a_refused_request_never_reaches_the_handler() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            (
                "smuggling shape",
                put(
                    "/echo",
                    "Transfer-Encoding: chunked\r\nContent-Length: 10\r\n",
                    "",
                ),
            ),
            (
                "chunked alone",
                put("/echo", "Transfer-Encoding: chunked\r\n", "0\r\n\r\n"),
            ),
            (
                "identity coding",
                get("/v1/", "Transfer-Encoding: identity\r\n"),
            ),
            (
                "two lengths",
                put("/echo", "Content-Length: 10\r\nContent-Length: 10\r\n", ""),
            ),
            (
                "two different lengths",
                put("/echo", "Content-Length: 3\r\ncontent-length: 4\r\n", "abc"),
            ),
            ("leading zero", put("/echo", "Content-Length: 010\r\n", "")),
            ("double zero", get("/v1/", "Content-Length: 00\r\n")),
            (
                "a repeated signature header",
                get("/v1/", "Bilbo-Time: 1\r\nbilbo-time: 1\r\n"),
            ),
            ("plus sign", put("/echo", "Content-Length: +10\r\n", "")),
            ("minus sign", put("/echo", "Content-Length: -1\r\n", "")),
            ("list", put("/echo", "Content-Length: 10, 10\r\n", "")),
            ("hex", put("/echo", "Content-Length: 0x10\r\n", "")),
            ("empty", put("/echo", "Content-Length:\r\n", "")),
            (
                "overflow",
                put("/echo", "Content-Length: 99999999999999999999999\r\n", ""),
            ),
            ("no host", b"GET /v1/ HTTP/1.1\r\n\r\n".to_vec()),
            (
                "two hosts",
                b"GET /v1/ HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n".to_vec(),
            ),
            ("expect something else", get("/v1/", "Expect: 200-ok\r\n")),
            (
                "expect twice",
                put(
                    "/echo",
                    "Expect: 100-continue\r\nExpect: 100-continue\r\nContent-Length: 0\r\n",
                    "",
                ),
            ),
            (
                "absolute form",
                b"GET http://relay/v1/ HTTP/1.1\r\nHost: relay\r\n\r\n".to_vec(),
            ),
            (
                "asterisk form",
                b"OPTIONS * HTTP/1.1\r\nHost: relay\r\n\r\n".to_vec(),
            ),
            (
                "authority form",
                b"CONNECT relay:80 HTTP/1.1\r\nHost: relay\r\n\r\n".to_vec(),
            ),
            ("a fragment", get("/v1/#x", "")),
            ("a get with a body", get("/v1/", "Content-Length: 5\r\n")),
            (
                "http 1.0",
                b"GET /v1/ HTTP/1.0\r\nHost: relay\r\n\r\n".to_vec(),
            ),
            (
                "http 2 preface",
                b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec(),
            ),
            ("obsolete folding", get("/v1/", "X-A: one\r\n two\r\n")),
            (
                "space before the colon",
                get("/v1/", "Content-Length : 0\r\n"),
            ),
            (
                "bare line feed",
                b"GET /v1/ HTTP/1.1\nHost: relay\n\n".to_vec(),
            ),
            (
                "bare line feed in a header",
                b"GET /v1/ HTTP/1.1\r\nHost: relay\nX-A: b\r\n\r\n".to_vec(),
            ),
            (
                "bare carriage return",
                b"GET /v1/ HTTP/1.1\r\nHost: relay\r\nX-A: b\rc\r\n\r\n".to_vec(),
            ),
            (
                "a value that is not utf-8",
                [
                    &b"GET /v1/ HTTP/1.1\r\nHost: relay\r\nX-A: "[..],
                    &[0xff, 0xfe],
                    &b"\r\n\r\n"[..],
                ]
                .concat(),
            ),
            ("a nul in a value", get("/v1/", "X-A: a\0b\r\n")),
            (
                "a tls hello",
                vec![
                    0x16, 0x03, 0x01, 0x02, 0x00, 0x01, 0x00, 0x01, 0xfc, 0x03, 0x03,
                ],
            ),
            ("plain words", b"hello there, relay\r\n\r\n".to_vec()),
            ("a lone line", b"\r\n\r\n".to_vec()),
        ];
        with_server(quick(), |addr, probe| {
            for (name, raw) in &cases {
                let response = exchange(addr, raw);
                assert!(
                    response.starts_with("HTTP/1.1 400 "),
                    "{name}: {response:?}"
                );
                assert_eq!(body(&response), r#"{"error":"bad-request"}"#, "{name}");
                assert!(
                    head_of(&response).contains("\r\nbilbo-time: 1234"),
                    "{name}"
                );
            }
            assert_eq!(probe.heads.load(Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn a_put_without_a_length_is_411() {
        with_server(quick(), |addr, probe| {
            let response = exchange(addr, &put("/echo", "", ""));
            assert_eq!(status(&response), 411);
            assert!(head_of(&response).contains("\r\nbilbo-time: 1234"));
            assert_eq!(probe.heads.load(Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn a_get_with_a_zero_length_is_served() {
        with_server(quick(), |addr, probe| {
            let response = exchange(addr, &get("/v1/", "Content-Length: 0\r\n"));
            assert_eq!(status(&response), 200);
            assert_eq!(probe.seen.lock().unwrap()[0].3, Some(0));
        });
    }

    #[test]
    fn headers_come_with_lowercase_names_and_trimmed_values() {
        with_server(quick(), |addr, probe| {
            exchange(addr, &get("/v1/", "X-Mixed-Case:  padded \t\r\n"));
            let seen = probe.seen.lock().unwrap();
            assert!(
                seen[0]
                    .2
                    .contains(&("x-mixed-case".to_string(), "padded".to_string()))
            );
        });
    }

    #[test]
    fn a_head_past_the_cap_is_431() {
        with_server(quick(), |addr, probe| {
            let big = format!("X-A: {}\r\n", "a".repeat(2000));
            let response = exchange(addr, &get("/v1/", &big));
            assert_eq!(status(&response), 431);
            assert!(head_of(&response).contains("\r\nbilbo-time: 1234"));

            let mut stream = connect(addr);
            stream.write_all(b"GET /v1/ HTTP/1.1\r\nX-A: ").unwrap();
            let _ = stream.write_all(&[b'a'; 8000]);
            let response = read_all(&mut stream);
            assert_eq!(status(&response), 431);

            let many: String = (0..80).map(|i| format!("X-{i}: v\r\n")).collect();
            let response = exchange(addr, &get("/v1/", &many));
            assert_eq!(status(&response), 431);
            assert_eq!(probe.heads.load(Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn a_head_exactly_at_the_cap_is_served() {
        let limits = quick();
        with_server(limits, |addr, _| {
            let base = get("/v1/", "X-A: \r\n").len();
            let fill = "a".repeat(1024 - base);
            let response = exchange(addr, &get("/v1/", &format!("X-A: {fill}\r\n")));
            assert_eq!(status(&response), 200);
            let response = exchange(addr, &get("/v1/", &format!("X-A: {fill}a\r\n")));
            assert_eq!(status(&response), 431);
        });
    }

    #[test]
    fn a_head_that_stalls_is_closed_at_the_deadline() {
        let limits = Limits {
            head: Duration::from_millis(300),
            ..quick()
        };
        with_server(limits, |addr, probe| {
            let mut stream = connect(addr);
            stream.write_all(b"GET /v1/ HTTP/1.1\r\nHo").unwrap();
            assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
            let mut stream = connect(addr);
            assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
            assert_eq!(probe.heads.load(Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn a_dripping_head_does_not_extend_the_deadline() {
        let limits = Limits {
            head: Duration::from_millis(400),
            ..quick()
        };
        with_server(limits, |addr, _| {
            let mut stream = connect(addr);
            let start = Instant::now();
            for byte in b"GET /v1/ HTTP/1.1\r\nHost: relay\r\nX-Slow: a"
                .iter()
                .cycle()
            {
                if stream.write_all(&[*byte]).is_err() || start.elapsed() > Duration::from_secs(5) {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            assert_eq!(read_all(&mut stream), "");
        });
    }

    #[test]
    fn a_body_that_idles_ends_the_read_with_an_error() {
        let limits = Limits {
            idle: Duration::from_millis(300),
            ..quick()
        };
        with_server(limits, |addr, probe| {
            let mut stream = connect(addr);
            stream
                .write_all(&put("/echo", "Content-Length: 10\r\n", "abc"))
                .unwrap();
            let response = read_all(&mut stream);
            assert_eq!(status(&response), 400);
            assert_eq!(probe.read_errors.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn a_body_that_drips_ends_at_the_total_deadline() {
        let limits = Limits {
            body: Duration::from_millis(400),
            ..quick()
        };
        with_server(limits, |addr, probe| {
            let mut stream = connect(addr);
            stream
                .write_all(&put("/echo", "Content-Length: 1000\r\n", ""))
                .unwrap();
            let mut sender = stream.try_clone().unwrap();
            let start = Instant::now();
            let drip = thread::spawn(move || {
                while start.elapsed() < Duration::from_secs(5) {
                    if sender.write_all(b"a").is_err() {
                        return;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            });
            assert_eq!(read_all(&mut stream), "");
            assert!(start.elapsed() < Duration::from_secs(5));
            assert_eq!(probe.read_errors.load(Ordering::SeqCst), 1);
            drop(stream);
            drip.join().unwrap();
        });
    }

    #[test]
    fn a_client_that_leaves_mid_body_ends_the_read_with_an_error() {
        with_server(quick(), |addr, probe| {
            let mut stream = connect(addr);
            stream
                .write_all(&put("/echo", "Content-Length: 10\r\n", "abc"))
                .unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
            let _ = read_all(&mut stream);
            assert_eq!(probe.read_errors.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn a_body_is_not_read_for_a_refusal_and_100_is_not_sent() {
        with_server(quick(), |addr, probe| {
            let raw = put(
                "/refuse",
                "Expect: 100-continue\r\nContent-Length: 5\r\n",
                "hello",
            );
            let response = exchange(addr, &raw);
            assert_eq!(status(&response), 413);
            assert_eq!(body(&response), r#"{"error":"too-large"}"#);
            assert_eq!(probe.answers.load(Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn continue_is_sent_before_the_body_is_read() {
        with_server(quick(), |addr, _| {
            let mut stream = connect(addr);
            stream
                .write_all(&put(
                    "/echo",
                    "Expect: 100-continue\r\nContent-Length: 5\r\n",
                    "",
                ))
                .unwrap();
            let mut interim = [0; 25];
            stream.read_exact(&mut interim).unwrap();
            assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
            stream.write_all(b"hello").unwrap();
            let response = read_all(&mut stream);
            assert_eq!(status(&response), 200);
            assert_eq!(body(&response), "hello");
            assert!(!response.contains("100 Continue"));
        });
    }

    #[test]
    fn a_client_still_sending_reads_the_413() {
        const SIZE: usize = 17 * 1024 * 1024;
        with_server(quick(), |addr, probe| {
            let mut stream = connect(addr);
            let head = put("/refuse", &format!("Content-Length: {SIZE}\r\n"), "");
            stream.write_all(&head).unwrap();
            let mut sender = stream.try_clone().unwrap();
            let writer = thread::spawn(move || {
                let chunk = [0; 64 * 1024];
                (0..SIZE / chunk.len()).try_for_each(|_| sender.write_all(&chunk))
            });
            let response = read_all(&mut stream);
            assert_eq!(status(&response), 413);
            assert_eq!(body(&response), r#"{"error":"too-large"}"#);
            writer.join().unwrap().expect("the whole body was taken");
            assert_eq!(probe.answers.load(Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn an_unread_body_is_drained_after_the_answer() {
        with_server(quick(), |addr, _| {
            let mut stream = connect(addr);
            let head = put("/headers", "Content-Length: 4194304\r\n", "");
            stream.write_all(&head).unwrap();
            let mut sender = stream.try_clone().unwrap();
            let writer = thread::spawn(move || {
                let chunk = [0; 64 * 1024];
                (0..64).try_for_each(|_| sender.write_all(&chunk))
            });
            let response = read_all(&mut stream);
            assert_eq!(status(&response), 200);
            writer.join().unwrap().expect("the whole body was taken");
        });
    }

    #[test]
    fn sixty_four_stalled_clients_do_not_block_a_request() {
        with_server(Limits::default(), |addr, _| {
            let stalled: Vec<TcpStream> = (0..64)
                .map(|_| {
                    let mut s = connect(addr);
                    s.write_all(b"GET /v1/ HTTP/1.1\r\nHo").unwrap();
                    s
                })
                .collect();
            let response = exchange(addr, &get("/v1/", ""));
            assert_eq!(status(&response), 200);
            drop(stalled);
        });
    }

    #[test]
    fn the_cap_answers_503_at_once_and_a_freed_slot_serves_again() {
        let limits = Limits {
            connections: 2,
            ..quick()
        };
        with_server(limits, |addr, _| {
            let held: Vec<TcpStream> = (0..2)
                .map(|_| {
                    let mut s = connect(addr);
                    s.write_all(b"GET /v1/ HTTP/1.1\r\nHo").unwrap();
                    s
                })
                .collect();
            let response = exchange(addr, &get("/v1/", ""));
            assert_eq!(status(&response), 503);
            let head = head_of(&response);
            assert!(head.contains("\r\nretry-after: 5"), "{head}");
            assert!(head.contains("\r\nbilbo-time: 1234"), "{head}");
            assert!(head.contains("\r\nconnection: close"), "{head}");
            assert_eq!(body(&response), r#"{"error":"busy"}"#);

            drop(held);
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let response = exchange(addr, &get("/v1/", ""));
                if status(&response) == 200 {
                    break;
                }
                assert!(Instant::now() < deadline, "the slots were never freed");
                thread::yield_now();
            }
        });
    }

    #[test]
    fn the_handler_cannot_replace_the_framing_headers_or_split_a_response() {
        with_server(quick(), |addr, _| {
            let response = exchange(addr, &get("/headers", ""));
            let head = head_of(&response);
            assert_eq!(head.matches("content-length").count(), 1, "{head}");
            assert!(head.contains("content-length: 1\r\n"), "{head}");
            assert!(head.contains("connection: close"), "{head}");
            assert!(!head.contains("keep-alive"), "{head}");
            assert!(head.contains("\r\nx-a: 1"), "{head}");

            let response = exchange(addr, &get("/split", ""));
            assert_eq!(status(&response), 500);
            assert!(!response.contains("X-B"));
        });
    }

    #[test]
    fn a_panicking_handler_is_a_500_and_the_relay_keeps_serving() {
        with_server(quick(), |addr, _| {
            assert_eq!(status(&exchange(addr, &get("/panic", ""))), 500);
            assert_eq!(status(&exchange(addr, &get("/panic-head", ""))), 500);
            assert_eq!(status(&exchange(addr, &get("/v1/", ""))), 200);
        });
    }

    #[test]
    fn serve_returns_once_stopped_and_woken() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = AtomicBool::new(false);
        let probe = Probe::default();
        thread::scope(|scope| {
            let server = scope.spawn(|| serve(&listener, &probe, &quick(), &stop));
            assert_eq!(status(&exchange(addr, &get("/v1/", ""))), 200);
            assert!(!server.is_finished());
            stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(addr);
            server.join().unwrap();
        });
    }
}
