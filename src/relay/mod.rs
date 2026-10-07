//! The relay: `bilbo relay`, the server one person runs so their devices sync through it. It keeps the sync
//! transport's tree in a data folder, admits only the owners it was started with, and serves the API under `/v1/`.

pub mod admit;
pub mod http;
pub mod mailbox;
pub mod route;
pub mod store;
pub mod walk;

use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::Failure;
use crate::identity::keys;

/// The address the relay listens on unless `--listen` says otherwise.
pub const LISTEN: &str = "127.0.0.1:8738";

/// The largest value a limit flag takes.
const LIMIT_MAX: u64 = 1_048_576;

/// How often the sweeper removes expired nameplates and the refusal count is written.
const TICK: Duration = Duration::from_secs(30);

/// The flags of `bilbo relay`.
#[derive(Debug, Clone, PartialEq)]
pub struct Flags {
    pub data: PathBuf,
    /// The admitted owners, as `keys::owner_fingerprint` spells them.
    pub owners: Vec<String>,
    pub listen: SocketAddr,
    pub max_scopes: u64,
    pub max_scope_mb: u64,
    pub max_object_mb: u64,
}

/// Seconds since the Unix epoch. Tests pass a clock they move.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// The system clock.
pub fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    })
}

/// What every connection shares.
pub struct State<'a> {
    pub flags: Flags,
    pub clock: Clock,
    /// `main`'s stderr sink.
    pub log: &'a (dyn Fn(&str) + Sync),
    pub data: store::Data,
    pub scopes: admit::Scopes,
    pub mailbox: mailbox::Mailbox,
    pub nonces: route::Nonces,
    pub refusals: route::Refusals,
}

/// Serves until the process is stopped. `log` takes every stderr line.
pub fn run(args: &[String], log: &(dyn Fn(&str) + Sync)) -> Result<(), Failure> {
    let flags = parse(args)?;
    let clock = system_clock();
    let (state, listener) = open(flags, clock, log)?;
    serve(&state, &listener, &AtomicBool::new(false));
    Ok(())
}

/// Everything before the first request, in this order: the data folder taken (so a second relay on it stops before
/// it binds), the address bound and the startup line printed, then the walk, whose lines follow the startup line.
fn open(
    flags: Flags,
    clock: Clock,
    log: &(dyn Fn(&str) + Sync),
) -> Result<(State<'_>, TcpListener), Failure> {
    let data = store::Data::open(&flags.data).map_err(Failure::Refused)?;
    let listener = TcpListener::bind(flags.listen)
        .map_err(|e| Failure::Refused(format!("cannot listen on {}: {e}", flags.listen)))?;
    let bound = listener
        .local_addr()
        .map_err(|e| Failure::Refused(format!("cannot listen on {}: {e}", flags.listen)))?;
    log(&format!("relay listening on http://{bound}"));
    if !bound.ip().is_loopback() {
        log(&format!(
            "relay serves plain HTTP on {bound}; put a TLS proxy in front of it"
        ));
    }
    let held = walk::walk(&data, &flags.owners, log).map_err(Failure::Refused)?;
    let scopes = admit::Scopes::new(&flags.owners, held);
    let mailbox = mailbox::Mailbox::open(&data, clock()).map_err(Failure::Refused)?;
    let state = State {
        flags,
        clock,
        log,
        data,
        scopes,
        mailbox,
        nonces: route::Nonces::default(),
        refusals: route::Refusals::default(),
    };
    Ok((state, listener))
}

/// Serves `listener` until `stop` is set, with the sweeper beside it.
fn serve(state: &State, listener: &TcpListener, stop: &AtomicBool) {
    std::thread::scope(|s| {
        s.spawn(|| {
            let mut last = Instant::now();
            while !stop.load(Ordering::SeqCst) {
                std::thread::park_timeout(Duration::from_millis(200));
                if last.elapsed() >= TICK {
                    last = Instant::now();
                    let now = (state.clock)();
                    state.mailbox.sweep(&state.data, now);
                    route::tick(state, now);
                }
            }
        });
        http::serve(listener, state, &http::Limits::default(), stop);
    });
}

/// A relay serving on its own thread, for tests in this and other domains. Dropping it stops the relay.
#[cfg(test)]
pub struct Running {
    pub port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(test)]
impl Running {
    /// `http://127.0.0.1:<port>`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

#[cfg(test)]
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Starts a relay for `flags` on a thread, as `run` would serve it, with `clock` and `log` injected.
#[cfg(test)]
pub fn start(
    flags: Flags,
    clock: Clock,
    log: Arc<dyn Fn(&str) + Send + Sync>,
) -> Result<Running, String> {
    let (ready, started) = std::sync::mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let thread = std::thread::spawn(move || match open(flags, clock, &*log) {
        Ok((state, listener)) => {
            let port = listener.local_addr().map(|a| a.port());
            let _ = ready.send(port.map_err(|e| e.to_string()));
            serve(&state, &listener, &stopped);
        }
        Err(
            Failure::Usage(why)
            | Failure::Config(why)
            | Failure::Refused(why)
            | Failure::Unmatched { message: why, .. },
        ) => {
            let _ = ready.send(Err(why));
        }
    });
    let port = started
        .recv()
        .map_err(|_| "the relay thread stopped".to_string())??;
    Ok(Running {
        port,
        stop,
        thread: Some(thread),
    })
}

fn parse(args: &[String]) -> Result<Flags, Failure> {
    let (mut data, mut owners, mut listen) = (None, Vec::new(), None);
    let (mut max_scopes, mut max_scope_mb, mut max_object_mb) = (None, None, None);
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = |option: &str| {
            rest.next()
                .filter(|v| !v.starts_with('-'))
                .cloned()
                .ok_or_else(|| Failure::Usage(format!("{option} needs a value")))
        };
        let twice = |set: bool| {
            if set {
                Err(Failure::Usage(format!("{arg} given twice")))
            } else {
                Ok(())
            }
        };
        match arg.as_str() {
            "--data" => {
                twice(data.is_some())?;
                data = Some(PathBuf::from(value("--data")?));
            }
            "--owner" => {
                let typed = value("--owner")?;
                let owner = keys::parse_fingerprint(&typed).ok_or_else(|| {
                    Failure::Usage(format!("'{typed}' is not an owner fingerprint"))
                })?;
                if !owners.contains(&owner) {
                    owners.push(owner);
                }
            }
            "--listen" => {
                twice(listen.is_some())?;
                let typed = value("--listen")?;
                listen = Some(typed.parse::<SocketAddr>().map_err(|_| {
                    Failure::Usage(format!("--listen takes <address:port>, got '{typed}'"))
                })?);
            }
            "--max-scopes" => {
                twice(max_scopes.is_some())?;
                max_scopes = Some(limit("--max-scopes", &value("--max-scopes")?)?);
            }
            "--max-scope-mb" => {
                twice(max_scope_mb.is_some())?;
                max_scope_mb = Some(limit("--max-scope-mb", &value("--max-scope-mb")?)?);
            }
            "--max-object-mb" => {
                twice(max_object_mb.is_some())?;
                max_object_mb = Some(limit("--max-object-mb", &value("--max-object-mb")?)?);
            }
            _ if arg.starts_with('-') => {
                return Err(Failure::Usage(format!("unknown option '{arg}'")));
            }
            _ => return Err(Failure::Usage(format!("unexpected argument '{arg}'"))),
        }
    }
    let data = data.ok_or_else(|| Failure::Usage("missing --data <dir>".into()))?;
    if owners.is_empty() {
        return Err(Failure::Usage("missing --owner <fingerprint>".into()));
    }
    Ok(Flags {
        data,
        owners,
        listen: match listen {
            Some(listen) => listen,
            None => LISTEN.parse().expect("the default address parses"),
        },
        max_scopes: max_scopes.unwrap_or(16),
        max_scope_mb: max_scope_mb.unwrap_or(1024),
        max_object_mb: max_object_mb.unwrap_or(16),
    })
}

/// A limit flag's value: a whole number from 1 to `LIMIT_MAX`.
fn limit(option: &str, typed: &str) -> Result<u64, Failure> {
    typed
        .parse::<u64>()
        .ok()
        .filter(|n| (1..=LIMIT_MAX).contains(n) && n.to_string() == typed)
        .ok_or_else(|| {
            Failure::Usage(format!(
                "{option} takes a whole number from 1 to {LIMIT_MAX}, got '{typed}'"
            ))
        })
}

/// `bilbo relay --help`; its Usage block is also the synopsis a usage error shows.
pub const HELP: &str = r#"bilbo relay: serve the sync transport to your devices over HTTP, until it is
stopped.

Usage:
  bilbo relay --data <dir> --owner <fingerprint> [--owner <fingerprint>]...
              [--listen <address:port>] [--max-scopes <n>] [--max-scope-mb <n>]
              [--max-object-mb <n>]

Options:
  --data <dir>             The folder it keeps objects in, created with mode
                           0700 when missing
  --owner <fingerprint>    An owner whose devices it serves; repeat for
                           several. Case and hyphens do not matter
  --listen <address:port>  Where to listen (default 127.0.0.1:8738)
  --max-scopes <n>         Scopes per owner (default 16)
  --max-scope-mb <n>       MiB per scope (default 1024)
  --max-object-mb <n>      MiB per segment (default 16)
Each option takes its value as the next argument. Each limit is a whole
number from 1 to 1048576.

It speaks plain HTTP under /v1/; put a TLS proxy in front of any address but
loopback. Nothing goes to stdout: stderr says where it listens, and logs each
object it stores and each request it refuses.

Exit: it runs until stopped; 1 when it cannot start (the address is taken,
or another relay serves the folder); 2 usage error.

Examples:
  bilbo relay --data /srv/bilbo-relay --owner yb4b-5aju-v6zb-x2nm-nc5x-ompf

Docs: https://github.com/delucca/bilbo/wiki/Commands#relay
"#;

#[cfg(test)]
mod tests {
    use super::*;

    const PRINT: &str = "yb4b-5aju-v6zb-x2nm-nc5x-ompf";

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(String::from).collect()
    }

    fn flags(text: &str) -> Flags {
        match parse(&args(text)) {
            Ok(flags) => flags,
            Err(_) => panic!("{text}: refused"),
        }
    }

    fn usage(text: &str) -> String {
        match parse(&args(text)) {
            Err(Failure::Usage(why)) => why,
            Err(_) => panic!("{text}: not a usage error"),
            Ok(flags) => panic!("{text}: parsed as {flags:?}"),
        }
    }

    #[test]
    fn the_defaults_fill_what_the_flags_leave_out() {
        let flags = flags(&format!("--data /srv/relay --owner {PRINT}"));
        assert_eq!(
            flags,
            Flags {
                data: PathBuf::from("/srv/relay"),
                owners: vec![PRINT.into()],
                listen: "127.0.0.1:8738".parse().unwrap(),
                max_scopes: 16,
                max_scope_mb: 1024,
                max_object_mb: 16,
            }
        );
    }

    #[test]
    fn every_flag_is_read() {
        let other = "abcd-efgh-ijkl-mnop-qrst-uvwx";
        let flags = flags(&format!(
            "--owner {} --data d --owner {} --listen 0.0.0.0:0 --max-scopes 2 --max-scope-mb 3 --max-object-mb 1048576",
            PRINT.to_uppercase().replace('-', ""),
            other
        ));
        assert_eq!(flags.owners, vec![PRINT.to_string(), other.to_string()]);
        assert_eq!(flags.listen, "0.0.0.0:0".parse().unwrap());
        assert_eq!(
            (flags.max_scopes, flags.max_scope_mb, flags.max_object_mb),
            (2, 3, 1_048_576)
        );
    }

    #[test]
    fn an_owner_given_twice_counts_once() {
        let flags = flags(&format!(
            "--data d --owner {PRINT} --owner {}",
            PRINT.replace('-', "")
        ));
        assert_eq!(flags.owners, vec![PRINT.to_string()]);
    }

    #[test]
    fn what_is_missing_or_wrong_is_a_usage_error() {
        assert_eq!(usage("--data d"), "missing --owner <fingerprint>");
        assert_eq!(usage(&format!("--owner {PRINT}")), "missing --data <dir>");
        assert_eq!(
            usage("--data d --owner nope"),
            "'nope' is not an owner fingerprint"
        );
        assert_eq!(
            usage(&format!("--data d --owner {PRINT} --max-scope-mb 0")),
            "--max-scope-mb takes a whole number from 1 to 1048576, got '0'"
        );
        for bad in ["1048577", "+1", "01", "x", "-1"] {
            let why = usage(&format!("--data d --owner {PRINT} --max-scopes {bad}"));
            assert!(why.starts_with("--max-scopes"), "{bad}: {why}");
        }
        assert_eq!(
            usage(&format!("--data d --owner {PRINT} --listen localhost")),
            "--listen takes <address:port>, got 'localhost'"
        );
        assert_eq!(
            usage(&format!("--data d --data e --owner {PRINT}")),
            "--data given twice"
        );
        assert_eq!(
            usage(&format!("--data d --owner {PRINT} --tls")),
            "unknown option '--tls'"
        );
        assert_eq!(
            usage(&format!("--data d --owner {PRINT} extra")),
            "unexpected argument 'extra'"
        );
        assert_eq!(usage("--data"), "--data needs a value");
    }
}
