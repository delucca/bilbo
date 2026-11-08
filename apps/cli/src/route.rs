//! Which text goes to which stream, and in what order. Pure: `main.rs` only performs the plan.

use crate::failure::Failure;
use crate::outcome::Outcome;
use crate::view::View;

/// One write: the result as a whole to stdout, or one line to stderr.
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

/// Performs the writes of `plan` through `write` and returns the exit code. A failed write ends
/// the run: its message goes to stderr in `err`'s view and its exit code replaces the plan's.
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

/// The plan for a failure: its message on stderr and its exit code.
pub fn failed(failure: &Failure, err: &View) -> Plan {
    Plan {
        writes: failure
            .message()
            .lines(err)
            .into_iter()
            .map(Emit::Stderr)
            .collect(),
        code: failure.exit_code(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Emit, Plan, execute, plan};
    use crate::failure::Failure;
    use crate::message::{Mark, Message};
    use crate::outcome::{Exit, Outcome};
    use crate::view::View;

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
}
