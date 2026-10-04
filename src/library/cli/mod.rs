//! `bilbo library`: its arguments, its output and the subcommand dispatch.

mod land;
mod list;
mod plan;
mod read;
mod reference;
mod show;
mod stage;

use std::io;
use std::path::{Path, PathBuf};

use crate::Failure;
use crate::library::corpus::SourceFile;
use crate::shared::markdown::{self, Section};
use crate::shared::store;

const VALUE_OPTIONS: [&str; 9] = [
    "--depth",
    "--origin",
    "--fetched",
    "--keep",
    "--title",
    "--budget-tokens",
    "--slice-bytes",
    "--slice-lines",
    "--part",
];

pub struct Output {
    /// stderr lines (without "bilbo: "), printed before stdout.
    pub warnings: Vec<String>,
    pub lines: Vec<String>,
}

impl Output {
    fn lines(lines: Vec<String>) -> Output {
        Output {
            warnings: Vec::new(),
            lines,
        }
    }
}

fn usage(message: impl Into<String>) -> Failure {
    Failure::Usage(message.into())
}

fn refused(message: impl Into<String>) -> Failure {
    Failure::Refused(message.into())
}

fn io_failure(what: &'static str, path: &Path) -> impl Fn(io::Error) -> Failure {
    let path = path.to_path_buf();
    move |e| refused(format!("cannot {what} {}: {e}", path.display()))
}

struct Args {
    positional: Vec<String>,
    options: Vec<(String, String)>,
    replace: bool,
    force: bool,
    html: bool,
}

impl Args {
    fn parse(args: &[String]) -> Result<Args, Failure> {
        let mut parsed = Args {
            positional: Vec::new(),
            options: Vec::new(),
            replace: false,
            force: false,
            html: false,
        };
        let mut options_ended = false;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let is_option = arg.chars().nth(1).is_some_and(|c| !c.is_whitespace());
            if options_ended || !arg.starts_with('-') || !is_option {
                parsed.positional.push(arg.clone());
                continue;
            }
            if arg == "--" {
                options_ended = true;
                continue;
            }
            let (name, inline) = match arg.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (arg.as_str(), None),
            };
            if name == "--replace" {
                if inline.is_some() {
                    return Err(usage("--replace takes no value"));
                }
                parsed.replace = true;
            } else if name == "--html" {
                if inline.is_some() {
                    return Err(usage("--html takes no value"));
                }
                parsed.html = true;
            } else if name == "--force" {
                if inline.is_some() {
                    return Err(usage("--force takes no value"));
                }
                parsed.force = true;
            } else if VALUE_OPTIONS.contains(&name) {
                let value = match inline {
                    Some(value) => value,
                    None => iter
                        .next()
                        .ok_or_else(|| usage(format!("{name} needs a value")))?
                        .clone(),
                };
                if name != "--keep" && parsed.options.iter().any(|(n, _)| n == name) {
                    return Err(usage(format!("{name} given more than once")));
                }
                parsed.options.push((name.to_string(), value));
            } else {
                return Err(usage(format!("unknown option '{name}'")));
            }
        }
        Ok(parsed)
    }

    fn one(&self, name: &str) -> Option<&str> {
        self.all(name).into_iter().next()
    }

    fn all(&self, name: &str) -> Vec<&str> {
        self.options
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// Refuses every option that is not in `taken`.
    fn only(&self, taken: &[&str]) -> Result<(), Failure> {
        let given = self.options.iter().map(|(n, _)| n.as_str());
        let given = given
            .chain(self.replace.then_some("--replace"))
            .chain(self.force.then_some("--force"))
            .chain(self.html.then_some("--html"));
        match given.into_iter().find(|name| !taken.contains(name)) {
            Some(name) => Err(usage(format!("option '{name}' does not apply here"))),
            None => Ok(()),
        }
    }

    /// The operands after the subcommand word, exactly as many as `names`.
    fn operands<const N: usize>(&self, names: [&str; N]) -> Result<[&str; N], Failure> {
        let rest = &self.positional[1..];
        if let Some(extra) = rest.get(N) {
            return Err(usage(format!("unexpected argument '{extra}'")));
        }
        let mut out = [""; N];
        for (i, name) in names.iter().enumerate() {
            out[i] = rest
                .get(i)
                .ok_or_else(|| usage(format!("missing {name}")))?;
        }
        Ok(out)
    }
}

pub fn run(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    let args = Args::parse(args)?;
    match args.positional.first().map(String::as_str) {
        None => {
            args.only(&[])?;
            list::run(env)
        }
        Some("show") => {
            args.only(&["--depth"])?;
            show::run(&args, env)
        }
        Some("stage") => {
            args.only(&["--origin", "--fetched", "--html"])?;
            stage::run(&args, env)
        }
        Some("land") => {
            args.only(&["--keep", "--title", "--replace", "--force"])?;
            land::run(&args, env)
        }
        Some("plan") => {
            args.only(&["--budget-tokens", "--slice-bytes", "--slice-lines"])?;
            plan::run(&args, env)
        }
        Some("read") => {
            args.only(&["--part"])?;
            read::run(&args, env)
        }
        Some(name) => {
            args.only(&[])?;
            if let Some(extra) = args.positional.get(1) {
                return Err(usage(format!("unexpected argument '{extra}'")));
            }
            if !store::is_topic(name) {
                return Err(usage(format!(
                    "invalid corpus '{name}': use segments of a-z and 0-9 joined by single hyphens"
                )));
            }
            list::show_corpus(name, env)
        }
    }
}

fn root(env: &store::Env) -> Result<PathBuf, Failure> {
    store::root(env).map_err(Failure::Config)
}

fn sections(file: &SourceFile) -> Vec<Section> {
    markdown::outline(&markdown::lines(&file.text), file.source.body_start)
}

fn or_dash(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("-")
}

fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn today() -> String {
    jiff::Zoned::now().date().to_string()
}

fn state_failure() -> Failure {
    Failure::Config(
        "cannot find the state folder: set XDG_STATE_HOME, or HOME, to an absolute path".into(),
    )
}

/// The line of the first level-1 heading outside fences and its text.
fn capture_title(lines: &[&str]) -> Option<(usize, String)> {
    markdown::outside_fences(lines)
        .into_iter()
        .find_map(|i| match markdown::heading(lines[i]) {
            Some((1, text)) => Some((i + 1, text)),
            _ => None,
        })
}
