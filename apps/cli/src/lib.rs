pub mod command;
pub mod outcome;
pub mod terminal;

use std::ffi::OsString;

use clap::{Command, CommandFactory, Parser, Subcommand};

use crate::command::help;
use crate::command::usage::usage;
use crate::command::{Parsed, parse, shape, typed};
use crate::outcome::Outcome;
use crate::outcome::failure::Failure;

#[derive(Debug, Parser)]
#[command(
    name = "bilbo",
    version,
    about = help::DESCRIPTION,
    override_usage = help::USAGE,
    after_help = help::AFTER,
    help_template = help::TEMPLATE,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub verb: Option<Verb>,
}

#[derive(Debug, Subcommand)]
pub enum Verb {}

pub fn bilbo() -> Command {
    let root = Cli::command();
    let names = command::verbs(&root);
    names
        .into_iter()
        .fold(root, |root, name| root.mut_subcommand(name, shape))
}

pub fn run(args: &[OsString]) -> Result<Outcome, Failure> {
    run_command(&bilbo(), args)
}

pub fn run_command(root: &Command, args: &[OsString]) -> Result<Outcome, Failure> {
    match parse(root, args)? {
        Parsed::Help(text) => Ok(help::outcome(text)),
        Parsed::Version => Ok(Outcome::new(format!(
            "bilbo {}\n",
            env!("CARGO_PKG_VERSION")
        ))),
        Parsed::Matches(matches) => match typed::<Cli>(root, &matches)?.verb {
            None => Err(usage(root, "missing verb".to_owned(), None)),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{bilbo, run, run_command};
    use crate::command::probe::{self, with_verb};
    use crate::outcome::failure::Failure;
    use crate::terminal::View;

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
        let reason = |list: &[&str]| match run_command(&bilbo(), &args(list)) {
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
        let root = with_verb(bilbo(), probe::verb());
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
        let root = with_verb(bilbo(), probe::verb());
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
        let root = with_verb(bilbo(), probe::verb());
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
