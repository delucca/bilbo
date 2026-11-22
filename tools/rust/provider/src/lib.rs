use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PACE: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Answer {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        pause: Option<Duration>,
    },
    Stall,
    Held {
        body: Vec<u8>,
        release: Release,
    },
}

#[derive(Debug, Clone)]
pub struct Release(Arc<AtomicBool>);

impl PartialEq for Release {
    fn eq(&self, other: &Release) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Release {}

impl Release {
    pub fn release(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Reply {
    pub fn held(body: &str) -> (Reply, Release) {
        let release = Release(Arc::new(AtomicBool::new(false)));
        let reply = Reply::Held {
            body: body.as_bytes().to_vec(),
            release: release.clone(),
        };
        (reply, release)
    }

    pub fn answer(status: u16, body: &str) -> Reply {
        Reply::Answer {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            pause: None,
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Reply {
        if let Reply::Answer { headers, .. } = &mut self {
            headers.push((name.to_owned(), value.to_owned()));
        }
        self
    }

    pub fn trickle(body: &str, pause: Duration) -> Reply {
        Reply::Answer {
            status: 200,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            pause: Some(pause),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub at: Instant,
}

impl Request {
    pub fn header(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }
}

#[derive(Debug)]
pub struct Provider {
    address: SocketAddr,
    seen: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Provider {
    pub fn start(script: Vec<Reply>) -> Provider {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port of this machine");
        listener
            .set_nonblocking(true)
            .expect("a listener that does not block");
        let address = listener.local_addr().expect("the listener's address");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let seen = Arc::clone(&seen);
            let stop = Arc::clone(&stop);
            move || serve(&listener, script, &seen, &stop)
        });
        Provider {
            address,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.seen.lock().expect("the request list").clone()
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    listener: &TcpListener,
    script: Vec<Reply>,
    seen: &Mutex<Vec<Request>>,
    stop: &AtomicBool,
) {
    let mut script = script.into_iter();
    while !stop.load(Ordering::SeqCst) {
        let Ok((stream, _)) = listener.accept() else {
            thread::sleep(PACE);
            continue;
        };
        let Some(request) = read_request(&stream, stop) else {
            continue;
        };
        seen.lock().expect("the request list").push(request);
        let reply = script
            .next()
            .unwrap_or_else(|| Reply::answer(500, "no reply scripted"));
        let _ = answer(stream, &reply, stop);
    }
}

fn read_request(stream: &TcpStream, stop: &AtomicBool) -> Option<Request> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(PACE)).ok()?;
    stream.set_write_timeout(Some(PACE)).ok()?;
    let mut raw = Vec::new();
    let mut byte = [0u8; 1];
    let mut input = stream;
    while !raw.ends_with(b"\r\n\r\n") {
        match input.read(&mut byte) {
            Ok(0) => return None,
            Ok(_) => raw.push(byte[0]),
            Err(error) if retry(&error) && !stop.load(Ordering::SeqCst) => {}
            Err(_) => return None,
        }
    }
    let head = String::from_utf8_lossy(&raw).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let path = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_owned(), value.trim().to_owned()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    let mut filled = 0;
    while filled < length {
        match input.read(&mut body[filled..]) {
            Ok(0) => return None,
            Ok(count) => filled += count,
            Err(error) if retry(&error) && !stop.load(Ordering::SeqCst) => {}
            Err(_) => return None,
        }
    }
    Some(Request {
        method,
        path,
        headers,
        body,
        at: Instant::now(),
    })
}

fn retry(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

fn answer(mut stream: TcpStream, reply: &Reply, stop: &AtomicBool) -> io::Result<()> {
    match reply {
        Reply::Stall => {
            while !stop.load(Ordering::SeqCst) {
                thread::sleep(PACE);
            }
            Ok(())
        }
        Reply::Held { body, release } => {
            while !release.0.load(Ordering::SeqCst) {
                if stop.load(Ordering::SeqCst) {
                    return Ok(());
                }
                thread::sleep(PACE);
            }
            put(&mut stream, &head(200, &[], body.len()), stop)?;
            put(&mut stream, body, stop)
        }
        Reply::Answer {
            status,
            headers,
            body,
            pause,
        } => {
            put(&mut stream, &head(*status, headers, body.len()), stop)?;
            let Some(pause) = pause else {
                return put(&mut stream, body, stop);
            };
            for byte in body {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                put(&mut stream, &[*byte], stop)?;
                rest(*pause, stop);
            }
            Ok(())
        }
    }
}

fn put(stream: &mut TcpStream, bytes: &[u8], stop: &AtomicBool) -> io::Result<()> {
    let mut at = 0;
    while at < bytes.len() {
        match stream.write(&bytes[at..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => at += count,
            Err(error) if retry(&error) && !stop.load(Ordering::SeqCst) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn rest(total: Duration, stop: &AtomicBool) {
    let mut left = total;
    while !left.is_zero() && !stop.load(Ordering::SeqCst) {
        let step = left.min(PACE);
        thread::sleep(step);
        left -= step;
    }
}

fn head(status: u16, headers: &[(String, String)], length: usize) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        302 => "Found",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    };
    let mut text = format!("HTTP/1.1 {status} {reason}\r\n");
    for (name, value) in headers {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str(&format!(
        "content-length: {length}\r\nconnection: close\r\n\r\n"
    ));
    text.into_bytes()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    use super::{Provider, Reply};

    fn connect(provider: &Provider) -> TcpStream {
        TcpStream::connect(provider.url().trim_start_matches("http://")).unwrap()
    }

    fn send(provider: &Provider, request: &str) -> String {
        let mut stream = connect(provider);
        stream.write_all(request.as_bytes()).unwrap();
        let mut text = String::new();
        stream.read_to_string(&mut text).unwrap();
        text
    }

    #[test]
    fn a_request_is_kept_as_it_was_sent() {
        let provider = Provider::start(vec![Reply::answer(200, "ok")]);
        let reply = send(
            &provider,
            "POST /v1/x?a=1 HTTP/1.1\r\nHost: h\r\nX-B: 2\r\nx-a: 1\r\nContent-Length: 5\r\n\r\nhello",
        );
        assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
        assert!(reply.ends_with("\r\n\r\nok"), "{reply}");
        let seen = provider.requests();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].path, "/v1/x?a=1");
        assert_eq!(
            seen[0].headers,
            [
                ("Host".to_owned(), "h".to_owned()),
                ("X-B".to_owned(), "2".to_owned()),
                ("x-a".to_owned(), "1".to_owned()),
                ("Content-Length".to_owned(), "5".to_owned()),
            ]
        );
        assert_eq!(seen[0].body, b"hello");
    }

    #[test]
    fn replies_come_in_the_order_scripted_and_then_a_500() {
        let provider = Provider::start(vec![
            Reply::answer(404, "first"),
            Reply::answer(200, "second").with_header("Retry-After", "7"),
        ]);
        let request = "GET / HTTP/1.1\r\nhost: h\r\n\r\n";
        let first = send(&provider, request);
        assert!(first.starts_with("HTTP/1.1 404 Not Found\r\n"), "{first}");
        assert!(first.ends_with("\r\n\r\nfirst"), "{first}");
        let second = send(&provider, request);
        assert!(second.contains("Retry-After: 7\r\n"), "{second}");
        assert!(second.ends_with("\r\n\r\nsecond"), "{second}");
        let third = send(&provider, request);
        assert!(
            third.starts_with("HTTP/1.1 500 Internal Server Error\r\n"),
            "{third}"
        );
        assert!(third.ends_with("\r\n\r\nno reply scripted"), "{third}");
        assert_eq!(provider.requests().len(), 3);
    }

    #[test]
    fn a_held_reply_is_sent_when_the_test_releases_it_and_not_before() {
        let (reply, release) = Reply::held("late");
        let provider = Provider::start(vec![reply]);
        let mut stream = connect(&provider);
        stream
            .write_all(b"GET / HTTP/1.1\r\nhost: h\r\n\r\n")
            .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let mut byte = [0u8; 1];
        assert!(stream.read(&mut byte).is_err());
        release.release();
        stream.set_read_timeout(None).unwrap();
        let mut text = String::new();
        stream.read_to_string(&mut text).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
        assert!(text.ends_with("\r\n\r\nlate"), "{text}");
    }

    #[test]
    fn a_stall_never_answers_and_dropping_the_provider_ends_it() {
        let provider = Provider::start(vec![Reply::Stall]);
        let mut stream = connect(&provider);
        stream
            .write_all(b"GET / HTTP/1.1\r\nhost: h\r\n\r\n")
            .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let mut byte = [0u8; 1];
        let error = stream.read(&mut byte).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
            "{error:?}"
        );
        let start = Instant::now();
        drop(provider);
        assert!(start.elapsed() < Duration::from_secs(2));
        stream.set_read_timeout(None).unwrap();
        assert_eq!(stream.read(&mut byte).unwrap_or(0), 0);
    }

    #[test]
    fn dropping_the_provider_ends_a_reply_no_client_reads() {
        let body = "x".repeat(32 << 20);
        let provider = Provider::start(vec![Reply::answer(200, &body)]);
        let mut client = connect(&provider);
        client
            .write_all(b"GET / HTTP/1.1\r\nhost: h\r\n\r\n")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while provider.requests().is_empty() {
            assert!(Instant::now() < deadline, "the request was never seen");
            std::thread::sleep(Duration::from_millis(5));
        }
        let (done, ended) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(provider);
            let _ = done.send(());
        });
        assert!(
            ended.recv_timeout(Duration::from_secs(10)).is_ok(),
            "the drop did not end"
        );
        drop(client);
    }
}
