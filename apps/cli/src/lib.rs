pub mod config;
pub mod console;
pub mod help;
#[cfg(test)]
mod probe;
pub mod remember;

use std::ffi::OsString;

use clap::{Command, CommandFactory, Parser, Subcommand};

use crate::console::args::{Parsed, parse, typed, usage, verbs};
use crate::console::env::Env;
use crate::console::outcome::{Failure, Outcome};
use crate::console::settings::Settings;
use crate::help::overview;
use crate::help::page::shape;

#[derive(Debug, Parser)]
#[command(
    name = "bilbo",
    version,
    about = overview::DESCRIPTION,
    override_usage = overview::USAGE,
    after_help = overview::AFTER,
    help_template = overview::TEMPLATE,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub verb: Option<Verb>,
}

#[derive(Debug, Subcommand)]
pub enum Verb {
    Remember(remember::Args),
    Config(config::Args),
}

pub fn bilbo() -> Command {
    let root = Cli::command();
    verbs(&root)
        .into_iter()
        .fold(root, |root, name| root.mut_subcommand(name, shape))
}

/// `settings` is what `main` resolved; help, the version and a usage error never read it.
pub fn run(
    args: &[OsString],
    env: &mut Env,
    settings: Result<Settings, Failure>,
) -> Result<Outcome, Failure> {
    run_command(&bilbo(), args, env, settings)
}

pub fn run_command(
    root: &Command,
    args: &[OsString],
    env: &mut Env,
    settings: Result<Settings, Failure>,
) -> Result<Outcome, Failure> {
    match parse(root, args)? {
        Parsed::Help(topic) => help::run(root, &topic),
        Parsed::Version => Ok(Outcome::new(format!(
            "bilbo {}\n",
            env!("CARGO_PKG_VERSION")
        ))),
        Parsed::Matches(matches) => {
            let Some(verb) = typed::<Cli>(root, &matches)?.verb else {
                return Err(usage(root, "missing verb".to_owned(), None));
            };
            let settings = settings?;
            match verb {
                Verb::Remember(args) => remember::run(root, args, env, &settings),
                Verb::Config(args) => config::run(root, args, &settings),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{bilbo, run, run_command};
    use crate::console::env::Env;
    use crate::console::outcome::Failure;
    use crate::console::settings::Settings;
    use crate::console::view::View;
    use crate::probe::{self, with_verb};

    fn unread() -> Result<Settings, Failure> {
        Err(Failure::Config(
            "these tests never read a setting".to_owned(),
        ))
    }

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn stdout(root: &clap::Command, list: &[&str]) -> String {
        run_command(root, &args(list), &mut Env::quiet(), unread())
            .unwrap_or_else(|failure| panic!("{list:?}: {failure:?}"))
            .stdout(&View::PLAIN)
    }

    #[test]
    fn a_newline_in_a_help_topic_cannot_cut_the_reason() {
        let reason =
            |list: &[&str]| match run_command(&bilbo(), &args(list), &mut Env::quiet(), unread()) {
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
            let outcome = run(&args(list), &mut Env::quiet(), unread()).expect("a result");
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
        let human = run_command(
            &root,
            &args(&["bilbo", "probe", "--help"]),
            &mut Env::quiet(),
            unread(),
        )
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
