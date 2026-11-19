use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use bilbo_cli::console::env::Env;
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
pub fn run(vars: HashMap<String, OsString>, args: &[&str], notices: Option<Sender<String>>) -> Run {
    let said = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&said);
    let mut env = Env {
        var: std::rc::Rc::new(move |name| vars.get(name).cloned()),
        stdin: Box::new(io::empty()),
        notice: Box::new(move |message| {
            let lines = message.plain_lines();
            if let Some(notices) = &notices {
                notices.send(lines.join("\n")).ok();
            }
            sink.lock().expect("notices").extend(lines);
        }),
    };
    let argv: Vec<OsString> = ["bilbo", "remember"]
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

pub fn text(path: &Path) -> &str {
    path.to_str().expect("UTF-8 path")
}

pub fn counts(new: u64, appended: u64, generation: u64, unchanged: u64, refused: u64) -> String {
    format!(
        "new {new}, appended {appended}, new generation {generation}, unchanged {unchanged}, refused {refused}"
    )
}
