//! HTTP/1.1 framing for the relay on a std `TcpListener`: one request per connection and one thread each, at most
//! 256 at once, strict framing, deadlines, and bodies read through a reader capped at `Content-Length`. It never
//! prints.

use std::io::Read;
use std::net::{IpAddr, TcpListener};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

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
            header_bytes: 16 * 1024,
            drain: Duration::from_secs(2),
        }
    }
}

/// A request head as `httparse` reads it.
type Parsed<'h, 'b> = httparse::Request<'h, 'b>;

/// Serves `listener` with `handler` until `stop` is set and a connection wakes the accept loop.
pub fn serve(listener: &TcpListener, handler: &dyn Handler, limits: &Limits, stop: &AtomicBool) {
    let _ = (listener, handler, limits, stop);
    unimplemented!("stream H")
}
