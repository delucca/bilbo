pub mod config;
pub mod console;
pub mod help;
#[cfg(test)]
mod probe;
pub mod read;
pub mod remember;

use std::ffi::OsString;

use bilbo_kinds::kind::Kind;
use bilbo_kinds::reader::Reader;
use clap::{Command, CommandFactory, Parser, Subcommand};

use crate::console::args::{Parsed, parse, typed, usage, verbs, with_verbs};
use crate::console::env::Env;
use crate::console::outcome::{Failure, Outcome};
use crate::console::settings::{KindKey, Settings};
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
    Read(read::Args),
    Config(config::Args),
}

pub fn kinds() -> Vec<Box<dyn Kind>> {
    vec![
        Box::new(bilbo_markdown::Markdown::default()),
        Box::new(bilbo_session::Session),
    ]
}

pub fn kind_keys() -> Vec<&'static KindKey> {
    vec![&bilbo_markdown::HEADING_WIDTH]
}

pub fn readers(settings: &Settings) -> Result<Vec<Box<dyn Reader>>, Failure> {
    let width = settings
        .kind_value(&bilbo_markdown::HEADING_WIDTH)
        .ok_or_else(|| {
            Failure::Config(format!(
                "{} was not registered with the settings",
                bilbo_markdown::HEADING_WIDTH.about.name
            ))
        })?;
    Ok(vec![
        Box::new(bilbo_markdown::Markdown::new(width)),
        Box::new(bilbo_session::Session),
    ])
}

pub fn bilbo() -> Command {
    let root = Cli::command();
    verbs(&root).into_iter().fold(root, |root, name| {
        let kinds = (name == "remember").then(|| remember::page(&kinds()));
        root.mut_subcommand(name, |verb| {
            shape(match kinds {
                Some(page) => verb.after_help(page),
                None => verb,
            })
        })
    })
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
                return Err(usage(root, with_verbs(root, "missing verb"), None));
            };
            match verb {
                Verb::Remember(args) if args.hook => {
                    Ok(remember::hook::run(env, settings, &kinds()))
                }
                Verb::Remember(args) => remember::run(root, args, env, &settings?, &kinds()),
                Verb::Read(args) => {
                    let settings = settings?;
                    let readers = readers(&settings)?;
                    read::run(root, &args, env, &settings, &readers)
                }
                Verb::Config(args) => config::run(root, args, &settings?),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{bilbo, kinds, readers, run, run_command};
    use crate::console::env::Env;
    use crate::console::outcome::Failure;
    use crate::console::settings::Settings;
    use crate::console::view::View;
    use crate::probe::{self, with_verb};

    fn defaults() -> Settings {
        crate::console::settings::load(
            &|name| (name == "HOME").then(|| OsString::from("/h")),
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &super::kind_keys(),
        )
        .expect("the defaults")
    }

    #[test]
    fn a_reader_whose_key_was_not_registered_is_a_config_error_not_a_default() {
        let settings = crate::console::settings::load(
            &|name| match name {
                "HOME" => Some(OsString::from("/h")),
                "BILBO_MARKDOWN_HEADING_WIDTH" => Some(OsString::from("20")),
                _ => None,
            },
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &[],
        )
        .expect("settings with no kind key");
        match readers(&settings) {
            Err(Failure::Config(text)) => {
                assert_eq!(
                    text,
                    "markdown.heading_width was not registered with the settings"
                );
            }
            other => panic!("{:?}", other.map(|list| list.len())),
        }
    }

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
    fn every_kind_has_its_own_lowercase_name_and_its_reader() {
        let names: Vec<&str> = kinds().iter().map(|kind| kind.name()).collect();
        assert!(!names.is_empty());
        for name in &names {
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase()),
                "{name:?}"
            );
        }
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        let read: Vec<&str> = readers(&defaults())
            .expect("the readers")
            .iter()
            .map(|reader| reader.kind())
            .collect();
        assert_eq!(
            read, names,
            "a kind that is kept and cannot be read, or the reverse"
        );
    }

    #[test]
    fn the_remember_page_lists_every_kind() {
        let page = stdout(&bilbo(), &["bilbo", "remember", "--help"]);
        let at = page.find("\nKinds:\n").expect("a Kinds section");
        for kind in kinds() {
            let row = format!("  {:<8}  {}\n", kind.name(), kind.summary());
            assert!(page[at..].contains(&row), "{row:?} missing in\n{page}");
        }
    }

    #[test]
    fn a_newline_in_a_help_topic_cannot_cut_the_reason() {
        let reason =
            |list: &[&str]| match run_command(&bilbo(), &args(list), &mut Env::quiet(), unread()) {
                Err(Failure::Usage { reason, .. }) => reason,
                other => panic!("{list:?}: {other:?}"),
            };
        assert_eq!(
            reason(&["bilbo", "help", "a\nb"]),
            "unknown verb 'a\\nb'; the verbs are remember, read, config"
        );
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
    fn a_top_level_usage_error_is_two_lines() {
        for list in [
            &["bilbo"][..],
            &["bilbo", "frob"],
            &["bilbo", "--frobnicate"],
            &["bilbo", "help", "frob"],
            &["bilbo", "help", "frob", "x"],
        ] {
            let failure = run(&args(list), &mut Env::quiet(), unread()).expect_err("a usage error");
            assert_eq!(failure.exit_code(), 2, "{list:?}");
            for view in [View::PLAIN, View::HUMAN] {
                let lines = failure.stderr_lines(&view);
                assert_eq!(lines.len(), 2, "{list:?}: {lines:?}");
            }
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
        assert!(at("\nOutput:\n") < at("\nExit:\n  0  "));
        assert!(at("\nExit:\n  0  ") < at("\nExamples:\n"));
        assert!(at("\nExamples:\n") < at("\nDocs: "));
        assert!(!page.contains("--help"), "{page}");
        assert!(!page.contains("Only the long help says this."), "{page}");
        for line in page.lines() {
            assert!(line.chars().count() <= 80, "{line:?}");
        }
    }

    #[test]
    fn a_verb_page_on_a_terminal_is_styled() {
        let root = with_verb(bilbo(), probe::verb());
        let human = run_command(
            &root,
            &args(&["bilbo", "probe", "--help"]),
            &mut Env::quiet(),
            unread(),
        )
        .expect("a page")
        .stdout(&View::HUMAN);
        for styled in [
            "\x1b[1mUsage:\x1b[22m\n  \x1b[36mbilbo probe",
            "\x1b[36mbilbo probe [options] \x1b[39m\x1b[2m<word>\x1b[22m",
            "\x1b[36mbilbo probe --all \x1b[39m\x1b[2m<word>\x1b[22m",
            "\x1b[2mOne line per word.\x1b[22m",
            "\x1b[1mOptions:\x1b[22m\n",
            "\x1b[1m-k, --kind \x1b[22m\x1b[2m<kind>\x1b[22m",
            "\x1b[1m--all\x1b[22m",
            "\x1b[1m--times \x1b[22m\x1b[2m<n>\x1b[22m",
            "\x1b[1mOutput:\x1b[22m\n",
            "\n  \x1b[2m<word>\x1b[22m\n",
            "\x1b[1mExit:\x1b[22m\n  \x1b[1m0\x1b[22m  success\n",
            "\x1b[1mExamples:\x1b[22m\n",
            "\x1b[1mDocs:\x1b[22m \x1b[2mhttps://",
        ] {
            assert!(human.contains(styled), "{styled:?} missing in\n{human}");
        }
    }
}
