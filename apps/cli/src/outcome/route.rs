use std::io::{self, Write};

use crate::outcome::Outcome;
use crate::outcome::failure::Failure;
use crate::terminal::View;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Emit {
    Stdout(String),
    Stderr(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub writes: Vec<Emit>,
    pub code: u8,
}

/// Each stream is rendered in its own view. When stdout gets the human view the diagnostics
/// follow the result; otherwise they come first, so a reader of both streams merged sees them
/// before a long result.
pub fn plan(result: &Result<Outcome, Failure>, out: &View, err: &View) -> Plan {
    match result {
        Ok(outcome) => {
            let result = Some(outcome.stdout(out))
                .filter(|text| !text.is_empty())
                .map(Emit::Stdout);
            let diagnostics = outcome.stderr_lines(err).into_iter().map(Emit::Stderr);
            let writes = if out.human {
                result.into_iter().chain(diagnostics).collect()
            } else {
                diagnostics.chain(result).collect()
            };
            Plan {
                writes,
                code: outcome.exit_code(),
            }
        }
        Err(failure) => failed(failure, err),
    }
}

pub fn execute(
    plan: &Plan,
    err: &View,
    write: &mut impl FnMut(&Emit) -> Result<(), Failure>,
) -> u8 {
    for emit in &plan.writes {
        if let Err(failure) = write(emit) {
            for line in &failed(&failure, err).writes {
                let _ = write(line);
            }
            return failure.exit_code();
        }
    }
    plan.code
}

fn failed(failure: &Failure, err: &View) -> Plan {
    Plan {
        writes: failure
            .stderr_lines(err)
            .into_iter()
            .map(Emit::Stderr)
            .collect(),
        code: failure.exit_code(),
    }
}

/// A reader that went away (`bilbo --help | head -1`) is not a failure; any other write error is.
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
    use std::io::{self, Write};

    use super::{Emit, Plan, execute, plan, write_result};
    use crate::outcome::failure::Failure;
    use crate::outcome::{Exit, Outcome};
    use crate::terminal::View;
    use crate::terminal::message::{Mark, Message};

    fn outcome() -> Outcome {
        Outcome::new("hits\n").with_diagnostics(vec![Message {
            mark: Mark::Warning,
            text: "searched here".to_owned(),
            details: Vec::new(),
        }])
    }

    fn stdout(text: &str) -> Emit {
        Emit::Stdout(text.to_owned())
    }

    fn stderr(text: &str) -> Emit {
        Emit::Stderr(text.to_owned())
    }

    #[test]
    fn diagnostics_follow_a_result_shown_on_a_terminal() {
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        let plan = plan(&Ok(outcome()), &unpainted, &View::PLAIN);
        assert_eq!(
            plan.writes,
            [stdout("hits\n"), stderr("bilbo: searched here")]
        );
        assert_eq!(plan.code, 0);
    }

    #[test]
    fn diagnostics_precede_a_result_that_is_piped() {
        let plan = plan(&Ok(outcome()), &View::PLAIN, &View::PLAIN);
        assert_eq!(
            plan.writes,
            [stderr("bilbo: searched here"), stdout("hits\n")]
        );
    }

    #[test]
    fn each_stream_is_rendered_in_its_own_view() {
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        let piped_out = plan(&Ok(outcome()), &View::PLAIN, &unpainted);
        assert_eq!(
            piped_out.writes,
            [stderr("▲  searched here"), stdout("hits\n")]
        );
        let piped_err = plan(&Ok(outcome()), &unpainted, &View::PLAIN);
        assert_eq!(
            piped_err.writes,
            [stdout("hits\n"), stderr("bilbo: searched here")]
        );
    }

    #[test]
    fn an_empty_result_writes_nothing_to_stdout_and_exit_1_is_kept() {
        let outcome = Outcome::new("").with_exit(Exit::NothingMatched);
        let plan = plan(&Ok(outcome), &View::PLAIN, &View::PLAIN);
        assert_eq!(plan.writes, []);
        assert_eq!(plan.code, 1);
    }

    #[test]
    fn a_failure_is_its_message_on_stderr_in_the_view_of_stderr() {
        let failure = Failure::Refused("no".to_owned());
        let plain = plan(&Err(failure.clone()), &View::HUMAN, &View::PLAIN);
        assert_eq!(plain.writes, [stderr("bilbo: no")]);
        assert_eq!(plain.code, 1);
        let human = plan(&Err(failure), &View::PLAIN, &View::HUMAN);
        assert_eq!(human.writes, [stderr("\x1b[31m■\x1b[39m  no")]);
    }

    fn stdout_that_fails(plan: &Plan, err: &View) -> (u8, Vec<Emit>) {
        let mut written = Vec::new();
        let code = execute(plan, err, &mut |emit| {
            written.push(emit.clone());
            match emit {
                Emit::Stdout(_) => Err(Failure::Output("cannot write to stdout: boom".to_owned())),
                Emit::Stderr(_) => Ok(()),
            }
        });
        (code, written)
    }

    #[test]
    fn a_failed_stdout_write_exits_1_with_its_message_and_writes_nothing_after_it() {
        let plan = Plan {
            writes: vec![stdout("hits\n"), stderr("bilbo: later")],
            code: 0,
        };
        let (code, written) = stdout_that_fails(&plan, &View::PLAIN);
        assert_eq!(code, 1);
        assert_eq!(
            written,
            [
                stdout("hits\n"),
                stderr("bilbo: cannot write to stdout: boom")
            ]
        );
    }

    #[test]
    fn a_failed_write_is_reported_in_the_view_of_stderr() {
        let plan = Plan {
            writes: vec![stdout("hits\n")],
            code: 0,
        };
        let (_, written) = stdout_that_fails(&plan, &View::HUMAN);
        assert_eq!(
            written[1],
            stderr("\x1b[31m■\x1b[39m  cannot write to stdout: boom")
        );
    }

    #[test]
    fn writes_that_succeed_end_with_the_exit_code_of_the_plan() {
        let plan = Plan {
            writes: vec![stderr("bilbo: warn"), stdout("hits\n")],
            code: 1,
        };
        let mut written = Vec::new();
        let code = execute(&plan, &View::PLAIN, &mut |emit| {
            written.push(emit.clone());
            Ok(())
        });
        assert_eq!(code, 1);
        assert_eq!(written, plan.writes);
    }

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
}
