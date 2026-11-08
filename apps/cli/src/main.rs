//! The `bilbo` binary: the one writer of stdout and stderr.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::process::ExitCode;

use bilbo_cli::failure::Failure;
use bilbo_cli::route::{self, Emit};
use bilbo_cli::terminal::{self, Stream};

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let out = terminal::open(Stream::Stdout);
    let err = terminal::open(Stream::Stderr);
    let plan = route::plan(&bilbo_cli::run(&args), &out, &err);
    ExitCode::from(route::execute(&plan, &err, &mut perform))
}

fn perform(write: &Emit) -> Result<(), Failure> {
    match write {
        Emit::Stdout(text) => bilbo_cli::write_result(&mut stdout()?, text),
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
