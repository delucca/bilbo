use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use bilbo_cli::console::env::{Env, Var, Who};
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings;
use bilbo_cli::console::view::View;
use bilbo_entries::tokens::Tokens;
use bilbo_search::tokenizer::Tokenizer;

pub fn tokenizer() -> &'static Tokenizer {
    static LOADED: OnceLock<&'static Tokenizer> = OnceLock::new();
    LOADED.get_or_init(|| Box::leak(Box::new(bilbo_cli::tokenizer().expect("the tokenizer"))))
}

pub struct Shared;

impl Tokens for Shared {
    fn ends(&self, text: &str) -> Vec<usize> {
        tokenizer().tokens.ends(text)
    }

    fn count(&self, text: &str) -> usize {
        tokenizer().tokens.count(text)
    }

    fn prefix(&self, text: &str, budget: usize) -> usize {
        tokenizer().tokens.prefix(text, budget)
    }
}

pub fn shared_tokenizer() -> Result<Tokenizer, Failure> {
    Ok(Tokenizer {
        tokens: Box::new(Shared),
        id: tokenizer().id,
    })
}

pub struct Run {
    pub code: u8,
    pub stdout: String,
    pub stderr: Vec<String>,
}

pub fn bilbo_with(var: Var, args: &[&str], tune: &bilbo_cli::Tune) -> Run {
    let said = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&said);
    let mut env = Env {
        var,
        stdin: Box::new(io::empty()),
        who: Who::Nobody,
        program: None,
        notice: Box::new(move |message| {
            sink.lock().expect("notices").extend(message.plain_lines());
        }),
    };
    let argv: Vec<OsString> = std::iter::once("bilbo")
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
    let (code, stdout, lines) =
        match bilbo_cli::run_tuned(&bilbo_cli::bilbo(), &argv, &mut env, settings, tune) {
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
    let mut stderr = said.lock().expect("notices").clone();
    stderr.extend(lines);
    Run {
        code,
        stdout,
        stderr,
    }
}

pub struct Sandbox {
    pub dir: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
