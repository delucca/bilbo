#![expect(
    clippy::disallowed_methods,
    reason = "main.rs is the only writer of stdout and stderr and the only reader of the environment"
)]

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, IsTerminal, Write};
use std::os::fd::AsFd;
use std::process::ExitCode;

use bilbo_cli::console::env::Env;
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings;
use bilbo_cli::console::streams::{self, Emit};
use bilbo_cli::console::view;
use bilbo_settings::edit;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let macos = cfg!(target_os = "macos");
    let out = view::decide(io::stdout().is_terminal(), macos, |name| {
        std::env::var_os(name)
    });
    let err = view::decide(io::stderr().is_terminal(), macos, |name| {
        std::env::var_os(name)
    });
    let mut env = Env {
        var: Box::new(|name| std::env::var_os(name)),
        stdin: Box::new(io::stdin()),
        notice: Box::new(move |message| {
            for line in message.lines(&err) {
                let _ = writeln!(io::stderr().lock(), "{line}");
            }
        }),
    };
    let settings = settings::load(&*env.var, &edit::read_regular, &bilbo_cli::kind_keys())
        .map_err(Failure::from);
    let plan = streams::plan(&bilbo_cli::run(&args, &mut env, settings), &out, &err);
    ExitCode::from(streams::execute(&plan, &err, &mut perform))
}

fn perform(write: &Emit) -> Result<(), Failure> {
    match write {
        Emit::Stdout(text) => streams::write_result(&mut stdout()?, text),
        Emit::Stderr(line) => {
            let _ = writeln!(io::stderr().lock(), "{line}");
            Ok(())
        }
    }
}

/// `std::io::stdout()` reports a write to an unwritable descriptor (`EBADF`) as success.
fn stdout() -> Result<File, Failure> {
    io::stdout()
        .as_fd()
        .try_clone_to_owned()
        .map(File::from)
        .map_err(|error| Failure::Output(format!("cannot write to stdout: {error}")))
}
