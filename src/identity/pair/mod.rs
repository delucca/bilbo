//! `bilbo pair`: shows a one-time code on an enrolled device, or, given that code, joins this device to the scopes
//! it pairs, through a mailbox on the scopes' transport.

#[cfg(test)]
mod exchange;
mod join;
mod show;

use std::ffi::OsString;
use std::io::BufRead;
use std::time::{Duration, Instant};

use crate::Failure;
use crate::shared::store;

/// How long each side waits, and how often it polls. `main` passes the defaults; tests shorten them.
pub struct Limits {
    /// From showing the code to creating `c.msg`.
    pub window: Duration,
    /// How long the joining device waits for `a.msg` to appear.
    pub appear: Duration,
    /// How long the joining device waits for the manifests after the reply.
    pub manifests: Duration,
    /// The poll interval on a `file://` transport.
    pub poll_file: Duration,
    /// The poll interval on an `https://` transport.
    pub poll_https: Duration,
    /// The age of `a.msg` past which any run removes its mailbox.
    pub sweep: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            window: Duration::from_secs(10 * 60),
            appear: Duration::from_secs(2 * 60),
            manifests: Duration::from_secs(2 * 60),
            poll_file: Duration::from_millis(250),
            poll_https: Duration::from_secs(2),
            sweep: Duration::from_secs(30 * 60),
        }
    }
}

/// What both sides get from `run`.
struct Cx<'a> {
    env: &'a store::Env,
    /// Stdin and stderr are terminals and neither agent marker is set.
    human: bool,
    answer: &'a mut dyn BufRead,
    limits: &'a Limits,
    out: &'a mut dyn FnMut(&str),
    err: &'a mut dyn FnMut(&str),
}

/// What the arguments ask for.
#[derive(Debug, PartialEq)]
enum Form {
    /// `bilbo pair [--scope <name>]... [--via <url>]`.
    Show {
        scopes: Vec<String>,
        via: Option<String>,
    },
    /// `bilbo pair <code> --via <url> [--name <name>]`.
    Join {
        code: String,
        via: String,
        name: Option<String>,
    },
}

/// Runs the verb. `terminal` says stdin and stderr are both terminals; `answer` is where the showing device reads the
/// user's confirmation; `out` and `err` take the lines for stdout and stderr.
pub fn run(
    args: &[String],
    env: &store::Env,
    terminal: bool,
    answer: &mut dyn BufRead,
    limits: &Limits,
    out: &mut dyn FnMut(&str),
    err: &mut dyn FnMut(&str),
) -> Result<(), Failure> {
    let form = parse(args)?;
    let mut cx = Cx {
        env,
        human: terminal && !marked(&env.claudecode) && !marked(&env.codex_thread_id),
        answer,
        limits,
        out,
        err,
    };
    match form {
        Form::Show { scopes, via } => show::run(&mut cx, &scopes, via.as_deref()),
        Form::Join { code, via, name } => join::run(&mut cx, &code, &via, name.as_deref()),
    }
}

/// An agent marker counts when it is set and not empty.
fn marked(var: &Option<OsString>) -> bool {
    var.as_ref().is_some_and(|v| !v.is_empty())
}

fn parse(args: &[String]) -> Result<Form, Failure> {
    let (mut code, mut scopes, mut via, mut name) = (None, Vec::new(), None, None);
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = |option: &str| {
            rest.next()
                .filter(|v| !v.starts_with('-'))
                .cloned()
                .ok_or_else(|| Failure::Usage(format!("{option} needs a value")))
        };
        match arg.as_str() {
            "--scope" => scopes.push(value("--scope")?),
            "--via" if via.is_none() => via = Some(value("--via")?),
            "--name" if name.is_none() => name = Some(value("--name")?),
            "--via" | "--name" => return Err(Failure::Usage(format!("{arg} given twice"))),
            _ if arg.starts_with('-') => {
                return Err(Failure::Usage(format!("unknown option '{arg}'")));
            }
            _ if code.is_none() && scopes.is_empty() && via.is_none() && name.is_none() => {
                code = Some(arg.clone());
            }
            _ => return Err(Failure::Usage(format!("unexpected argument '{arg}'"))),
        }
    }
    match code {
        None if name.is_some() => Err(Failure::Usage(
            "--name names the device that joins: bilbo pair <code> --via <url> --name <name>"
                .into(),
        )),
        None => Ok(Form::Show { scopes, via }),
        Some(_) if !scopes.is_empty() => Err(Failure::Usage(
            "--scope belongs to the device that shows the code".into(),
        )),
        Some(code) => match via {
            Some(via) => Ok(Form::Join { code, via, name }),
            None => Err(Failure::Usage(
                "a device that joins needs --via <url>, the transport's URL on this device".into(),
            )),
        },
    }
}

/// The poll interval for `url`'s transport.
fn interval(url: &str, limits: &Limits) -> Duration {
    if url.starts_with("file://") {
        limits.poll_file
    } else {
        limits.poll_https
    }
}

/// How long a poll rests after a relay answers 429 `rate`.
const RATE_PAUSE: Duration = Duration::from_secs(5);

/// Whether `why` is a relay's 429 `rate`, as the relay client words it.
fn is_rate(why: &str) -> bool {
    why.ends_with(" answered 429: rate")
}

/// Calls `look` every `every` until it finds something, or `limit` has passed since `since`: `None` then. A relay's
/// 429 `rate` is a wait of `RATE_PAUSE` (the client's error carries no `Retry-After`), not a failure.
fn wait<T>(
    since: Instant,
    limit: Duration,
    every: Duration,
    look: impl FnMut() -> Result<Option<T>, String>,
) -> Result<Option<T>, String> {
    wait_pausing(since, limit, every, RATE_PAUSE, look)
}

fn wait_pausing<T>(
    since: Instant,
    limit: Duration,
    every: Duration,
    pause: Duration,
    mut look: impl FnMut() -> Result<Option<T>, String>,
) -> Result<Option<T>, String> {
    loop {
        let rested = match look() {
            Ok(Some(found)) => return Ok(Some(found)),
            Ok(None) => every,
            Err(why) if is_rate(&why) => pause,
            Err(why) => return Err(why),
        };
        let left = limit.saturating_sub(since.elapsed());
        if left.is_zero() {
            return Ok(None);
        }
        std::thread::sleep(rested.min(left));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(String::from).collect()
    }

    fn usage(text: &str) -> String {
        match parse(&args(text)) {
            Err(Failure::Usage(message)) => message,
            Err(_) => panic!("{text}: not a usage error"),
            Ok(form) => panic!("{text}: parsed as {form:?}"),
        }
    }

    #[test]
    fn a_rate_refusal_is_waited_out_and_any_other_error_ends_the_wait() {
        let mut calls = 0;
        let found = wait_pausing(
            Instant::now(),
            Duration::from_secs(5),
            Duration::from_millis(1),
            Duration::from_millis(5),
            || {
                calls += 1;
                match calls {
                    1 | 2 => Err("relay http://r answered 429: rate".to_string()),
                    _ => Ok(Some(calls)),
                }
            },
        );
        assert_eq!(found, Ok(Some(3)));
        let started = Instant::now();
        let none: Result<Option<u8>, String> = wait_pausing(
            started,
            Duration::from_millis(30),
            Duration::from_millis(1),
            Duration::from_secs(60),
            || Err("relay http://r answered 429: rate".to_string()),
        );
        assert_eq!(none, Ok(None));
        assert!(started.elapsed() < Duration::from_secs(5));
        let failed: Result<Option<u8>, String> = wait_pausing(
            Instant::now(),
            Duration::from_secs(5),
            Duration::from_millis(1),
            Duration::from_millis(1),
            || Err("relay http://r answered 429: busy".to_string()),
        );
        assert!(failed.is_err());
    }

    #[test]
    fn the_two_forms_parse() {
        assert_eq!(
            parse(&args("")).ok(),
            Some(Form::Show {
                scopes: vec![],
                via: None
            })
        );
        assert_eq!(
            parse(&args("--scope a --scope b --via file:///srv/sync")).ok(),
            Some(Form::Show {
                scopes: vec!["a".into(), "b".into()],
                via: Some("file:///srv/sync".into())
            })
        );
        assert_eq!(
            parse(&args(
                "42-orbit-tunnel-velvet --via file:///srv/sync --name bywater"
            ))
            .ok(),
            Some(Form::Join {
                code: "42-orbit-tunnel-velvet".into(),
                via: "file:///srv/sync".into(),
                name: Some("bywater".into())
            })
        );
        let spaced = vec![
            "42 ORBI tunn velvet".to_string(),
            "--via".into(),
            "file:///srv/sync".into(),
        ];
        assert!(
            matches!(parse(&spaced), Ok(Form::Join { code, .. }) if code == "42 ORBI tunn velvet")
        );
    }

    #[test]
    fn stray_and_missing_arguments_are_usage_errors() {
        assert!(usage("--scope").contains("--scope"));
        assert!(usage("--via").contains("--via"));
        assert!(usage("--scope --via file:///srv/sync").contains("--scope"));
        assert!(usage("--frob").contains("--frob"));
        assert!(usage("42-orbit-tunnel-velvet").contains("--via"));
        assert!(usage("42-orbit-tunnel-velvet --via a --via b").contains("twice"));
        assert!(usage("42-orbit-tunnel-velvet extra --via a").contains("extra"));
        assert!(usage("42-orbit-tunnel-velvet --via a --scope s").contains("--scope"));
        assert!(usage("--name bywater").contains("--name"));
        assert!(usage("--via a 42-orbit-tunnel-velvet").contains("42-orbit-tunnel-velvet"));
    }

    #[test]
    fn waiting_stops_at_the_limit_or_on_a_find() {
        let since = Instant::now();
        let none: Option<()> = wait(
            since,
            Duration::from_millis(30),
            Duration::from_millis(5),
            || Ok(None),
        )
        .unwrap();
        assert!(none.is_none() && since.elapsed() >= Duration::from_millis(30));
        let mut calls = 0;
        let found = wait(
            Instant::now(),
            Duration::from_secs(5),
            Duration::from_millis(1),
            || {
                calls += 1;
                Ok((calls == 3).then_some(calls))
            },
        );
        assert_eq!(found, Ok(Some(3)));
        let failed: Result<Option<()>, String> = wait(
            Instant::now(),
            Duration::from_secs(5),
            Duration::from_millis(1),
            || Err("unreachable".to_string()),
        );
        assert_eq!(failed, Err("unreachable".to_string()));
    }

    #[test]
    fn an_agent_marker_counts_only_when_not_empty() {
        assert!(!marked(&None));
        assert!(!marked(&Some(OsString::new())));
        assert!(marked(&Some(OsString::from("1"))));
    }
}
