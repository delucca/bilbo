#![expect(
    clippy::disallowed_methods,
    reason = "main.rs is the only writer of stdout and stderr"
)]

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, IsTerminal, Write};
use std::os::fd::AsFd;
use std::process::ExitCode;

use bilbo_cli::outcome::failure::Failure;
use bilbo_cli::outcome::route::{self, Emit};
use bilbo_cli::terminal;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let macos = cfg!(target_os = "macos");
    let out = terminal::decide(io::stdout().is_terminal(), macos, |name| {
        std::env::var_os(name)
    });
    let err = terminal::decide(io::stderr().is_terminal(), macos, |name| {
        std::env::var_os(name)
    });
    let plan = route::plan(&bilbo_cli::run(&args), &out, &err);
    ExitCode::from(route::execute(&plan, &err, &mut perform))
}

fn perform(write: &Emit) -> Result<(), Failure> {
    match write {
        Emit::Stdout(text) => route::write_result(&mut stdout()?, text),
        Emit::Stderr(line) => {
            let _ = writeln!(io::stderr().lock(), "{line}");
            Ok(())
        }
    }
}

/// A file on a duplicate of stdout's descriptor: `std::io::stdout()` reports a write to a
/// descriptor that cannot be written (`EBADF`) as success.
fn stdout() -> Result<File, Failure> {
    io::stdout()
        .as_fd()
        .try_clone_to_owned()
        .map(File::from)
        .map_err(|error| Failure::Output(format!("cannot write to stdout: {error}")))
}
