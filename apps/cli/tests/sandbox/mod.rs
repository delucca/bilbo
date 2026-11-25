use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use bilbo_cli::console::env::{Env, Who};
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings;
use bilbo_cli::console::view::View;

pub struct Run {
    pub code: u8,
    pub stdout: String,
    pub stderr: Vec<String>,
    pub notices: Vec<String>,
}

/// Notices also go to `notices`, for a test that waits on one.
pub fn run(
    vars: HashMap<String, OsString>,
    args: &[&str],
    input: &[u8],
    notices: Option<Sender<String>>,
) -> Run {
    run_reading(
        vars,
        args,
        Box::new(io::Cursor::new(input.to_vec())),
        notices,
    )
}

pub fn run_reading(
    vars: HashMap<String, OsString>,
    args: &[&str],
    input: Box<dyn io::Read>,
    notices: Option<Sender<String>>,
) -> Run {
    run_as(vars, args, Who::Nobody, input, notices)
}

pub fn run_as(
    vars: HashMap<String, OsString>,
    args: &[&str],
    who: Who,
    input: Box<dyn io::Read>,
    notices: Option<Sender<String>>,
) -> Run {
    let said = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&said);
    let mut env = Env {
        var: std::rc::Rc::new(move |name| vars.get(name).cloned()),
        stdin: input,
        who,
        notice: Box::new(move |message| {
            let lines = message.plain_lines();
            if let Some(notices) = &notices {
                notices.send(lines.join("\n")).ok();
            }
            sink.lock().expect("notices").extend(lines);
        }),
    };
    let argv: Vec<OsString> = ["bilbo"]
        .into_iter()
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect();
    let settings = settings::load(
        &*env.var,
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
        &bilbo_cli::provider_keys(),
    )
    .map_err(Failure::from);
    let (code, stdout, stderr) = match bilbo_cli::run(&argv, &mut env, settings) {
        Ok(outcome) => (
            outcome.exit_code(),
            outcome.stdout(&View::PLAIN),
            outcome.stderr_lines(&View::PLAIN),
        ),
        Err(failure) => (
            failure.exit_code(),
            String::new(),
            failure.stderr_lines(&View::PLAIN),
        ),
    };
    let notices = said.lock().expect("notices").clone();
    Run {
        code,
        stdout,
        stderr,
        notices,
    }
}

pub fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
    fs::write(path, bytes).expect("write");
}
