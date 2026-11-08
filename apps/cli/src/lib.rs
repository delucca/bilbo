//! The bilbo command line: what a run prints and with which exit code. `main.rs` is the only
//! writer of stdout and stderr; everything here returns text.

pub mod cli;
pub mod failure;
pub mod help;
pub mod message;
pub mod outcome;
#[cfg(test)]
mod probe;
pub mod route;
pub mod terminal;
pub mod theme;
pub mod view;

use std::ffi::OsString;
use std::io::{self, Write};

use clap::Command;

use crate::failure::Failure;
use crate::outcome::Outcome;
use crate::view::View;

/// Runs the arguments (`args[0]` is the program) against the verbs of this build.
pub fn run(args: &[OsString]) -> Result<Outcome, Failure> {
    run_command(&cli::command(), args)
}

/// Runs the arguments against `root`, which tests extend with a verb of their own.
pub fn run_command(root: &Command, args: &[OsString]) -> Result<Outcome, Failure> {
    if args.get(1).is_some_and(|arg| arg == "help") {
        return help::topic(root, &args[2..]).map(page);
    }
    match cli::parse(root, args)? {
        cli::Parsed::Help(text) => Ok(page(text)),
        cli::Parsed::Version => Ok(Outcome::new(format!(
            "bilbo {}\n",
            env!("CARGO_PKG_VERSION")
        ))),
        cli::Parsed::Matches(matches) => match cli::typed(root, &matches)?.verb {
            None => Err(help::usage(root, "missing verb".to_owned(), None)),
        },
    }
}

/// A help page: plain, or bold in its headings and columns where the human view paints.
fn page(text: String) -> Outcome {
    Outcome::new(text.clone()).with_human(move |view| help::styled(&text, view))
}

/// The lines stderr gets for a failure.
pub fn stderr_lines(failure: &Failure, view: &View) -> Vec<String> {
    failure.message().lines(view)
}

/// Writes the result to stdout. A reader that went away (`bilbo --help | head -1`) is not a
/// failure; any other write error is.
pub fn write_result(out: &mut impl Write, text: &str) -> Result<(), Failure> {
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            Err(Failure::Output(format!("cannot write to stdout: {error}")))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io::{self, Write};

    use super::{run, run_command, write_result};
    use crate::cli;
    use crate::failure::Failure;
    use crate::probe;
    use crate::view::View;

    #[derive(Clone, Copy)]
    enum Break {
        Write(io::ErrorKind),
        Flush(io::ErrorKind),
    }

    struct Broken(Break);

    impl Write for Broken {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            match self.0 {
                Break::Write(kind) => Err(io::Error::new(kind, "broken")),
                Break::Flush(_) => Ok(buf.len()),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            match self.0 {
                Break::Flush(kind) => Err(io::Error::new(kind, "broken")),
                Break::Write(_) => Ok(()),
            }
        }
    }

    #[test]
    fn a_result_is_written_whole_and_flushed() {
        let mut out = Vec::new();
        assert_eq!(write_result(&mut out, "hits\n"), Ok(()));
        assert_eq!(out, b"hits\n");
    }

    #[test]
    fn a_write_error_is_an_output_failure() {
        let failure = write_result(&mut Broken(Break::Write(io::ErrorKind::Other)), "x");
        assert_eq!(
            failure,
            Err(Failure::Output("cannot write to stdout: broken".to_owned()))
        );
    }

    #[test]
    fn a_flush_error_is_an_output_failure() {
        let failure = write_result(&mut Broken(Break::Flush(io::ErrorKind::Other)), "x");
        assert_eq!(
            failure,
            Err(Failure::Output("cannot write to stdout: broken".to_owned()))
        );
    }

    #[test]
    fn a_reader_that_went_away_is_success() {
        for kind in [
            Break::Write(io::ErrorKind::BrokenPipe),
            Break::Flush(io::ErrorKind::BrokenPipe),
        ] {
            assert_eq!(write_result(&mut Broken(kind), "x"), Ok(()));
        }
    }

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn stdout(root: &clap::Command, list: &[&str]) -> String {
        run_command(root, &args(list))
            .unwrap_or_else(|failure| panic!("{list:?}: {failure:?}"))
            .stdout(&View::PLAIN)
    }

    #[test]
    fn a_newline_in_a_help_topic_cannot_cut_the_reason() {
        let reason = |list: &[&str]| match run_command(&cli::command(), &args(list)) {
            Err(Failure::Usage { reason, .. }) => reason,
            other => panic!("{list:?}: {other:?}"),
        };
        assert_eq!(reason(&["bilbo", "help", "a\nb"]), "unknown verb 'a\\nb'");
        assert_eq!(
            reason(&["bilbo", "help", "--a\nb"]),
            "unknown option '--a\\nb'"
        );
        assert_eq!(
            reason(&["bilbo", "help", "x", "y\nz"]),
            "unexpected argument 'y\\nz'"
        );
    }

    #[test]
    fn help_and_version_are_results_that_exit_0() {
        for list in [&["bilbo", "--help"][..], &["bilbo", "--version"]] {
            let outcome = run(&args(list)).expect("a result");
            assert_eq!(outcome.exit_code(), 0);
            assert!(outcome.stderr_lines(&View::PLAIN).is_empty());
        }
    }

    #[test]
    fn a_verb_page_is_the_same_from_every_help_form() {
        let root = cli::with_verb(cli::command(), probe::verb());
        let page = stdout(&root, &["bilbo", "probe", "--help"]);
        for list in [
            &["bilbo", "probe", "-h"][..],
            &["bilbo", "help", "probe"],
            &["bilbo", "help", "probe", "--help"],
        ] {
            assert_eq!(stdout(&root, list), page, "{list:?}");
        }
    }

    #[test]
    fn a_verb_page_has_its_sections_in_order_within_80_columns() {
        let root = cli::with_verb(cli::command(), probe::verb());
        let page = stdout(&root, &["bilbo", "probe", "--help"]);
        let at = |needle: &str| {
            page.find(needle)
                .unwrap_or_else(|| panic!("{needle:?} missing in\n{page}"))
        };
        assert!(page.starts_with("bilbo probe: shows the shape of a verb page.\n\nUsage:\n  "));
        assert!(
            at("Usage:\n  bilbo probe [options] <word>\n  bilbo probe --all <word>\n")
                < at("\nOptions:\n")
        );
        assert!(at("\nOptions:\n") < at("\nOutput:\n"));
        assert!(at("\nOutput:\n") < at("\nExit: "));
        assert!(at("\nExit: ") < at("\nExamples:\n"));
        assert!(at("\nExamples:\n") < at("\nDocs: "));
        assert!(!page.contains("--help"), "{page}");
        assert!(!page.contains("Only the long help says this."), "{page}");
        for line in page.lines() {
            assert!(line.chars().count() <= 80, "{line:?}");
        }
    }

    #[test]
    fn a_verb_page_is_bold_in_its_headings_and_option_columns() {
        let root = cli::with_verb(cli::command(), probe::verb());
        let human = run_command(&root, &args(&["bilbo", "probe", "--help"]))
            .expect("a page")
            .stdout(&View::HUMAN);
        for bold in [
            "\x1b[1mUsage:\x1b[22m\n  bilbo probe",
            "\x1b[1mOptions:\x1b[22m\n",
            "\x1b[1m-k, --kind <kind>\x1b[22m",
            "\x1b[1m--all\x1b[22m",
            "\x1b[1m--times <n>\x1b[22m",
            "\x1b[1mOutput:\x1b[22m\n",
            "\x1b[1mExamples:\x1b[22m\n",
            "\x1b[1mDocs:\x1b[22m reference",
        ] {
            assert!(human.contains(bold), "{bold:?} missing in\n{human}");
        }
    }
}
