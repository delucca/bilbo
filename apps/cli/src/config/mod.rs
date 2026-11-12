pub mod view;

use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::message::sanitize;
use crate::console::outcome::{Failure, Outcome};
use crate::console::settings::Settings;

const SUMMARY: &str = "Show each setting, its value and where it came from";
const ABOUT: &str = "bilbo config: show each setting, its value and where the value came from.";
const USAGE: &str = "\
bilbo config [<key>]
The config file is $BILBO_CONFIG, else $XDG_CONFIG_HOME/bilbo/config.toml,
else ~/.config/bilbo/config.toml.";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE, after_help = PAGE)]
pub struct Args {
    #[arg(value_name = "key", value_parser = NonEmptyStringValueParser::new())]
    pub key: Option<String>,
}

const PAGE: &str = "\
Output:
  <key> = <value> (<origin>)
One line per key, the value as TOML; the origin is default, <file>:<line>, a
variable or a flag. With <key>, its value alone, a string without its quotes.

Exit:
  0  success
  2  usage or config error: an unknown key, or a file or variable bilbo refuses

Examples:
  bilbo config
  bilbo config paths.root

Docs: https://github.com/delucca/bilbo/wiki/Configuration";

pub fn run(root: &Command, args: Args, settings: &Settings) -> Result<Outcome, Failure> {
    let shown = settings.shown();
    let Some(name) = args.key else {
        return Ok(
            Outcome::new(view::plain(&shown)).with_human(move |out| view::human(&shown, out))
        );
    };
    shown.iter().find(|one| one.name == name).map_or_else(
        || {
            let names: Vec<&str> = shown.iter().map(|one| one.name).collect();
            Err(usage(
                root,
                format!(
                    "unknown key '{}'; the keys are {}",
                    sanitize(&name),
                    names.join(", ")
                ),
                Some("config"),
            ))
        },
        |one| Ok(Outcome::new(format!("{}\n", sanitize(&one.to_bare())))),
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use bilbo_settings::key::Default;

    use super::{Args, USAGE, run};
    use crate::bilbo;
    use crate::console::outcome::Failure;
    use crate::console::settings::{self, CONFIG, Settings};
    use crate::console::view::View;

    fn defaults() -> Settings {
        settings::load(
            &|name| (name == "HOME").then(|| OsString::from("/h")),
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &crate::kind_keys(),
        )
        .expect("the defaults")
    }

    fn key(name: &str) -> Args {
        Args {
            key: Some(name.to_owned()),
        }
    }

    #[test]
    fn a_named_key_prints_its_bare_value() {
        let stdout = |name: &str| {
            run(&bilbo(), key(name), &defaults())
                .expect("a value")
                .stdout(&View::PLAIN)
        };
        assert_eq!(stdout("store.seal_at"), "8 MiB\n");
        assert_eq!(stdout("store.tear_budget"), "4\n");
        assert_eq!(stdout("paths.root"), "/h/.local/share/bilbo\n");
    }

    #[test]
    fn an_unknown_key_is_a_usage_error_that_lists_the_keys() {
        match run(&bilbo(), key("store.sealat"), &defaults()) {
            Err(Failure::Usage { reason, page, .. }) => {
                assert_eq!(
                    reason,
                    "unknown key 'store.sealat'; the keys are paths.root, paths.state, \
                     paths.cache, store.seal_at, store.seal_after, store.compression, store.tear_budget, store.wait, store.poll, follow, read.budget, read.outline_width, read.matches_listed, search.chunk_tokens, search.chunk_overlap, search.workers, search.index_memory, search.merge_deleted_percent, search.entry_tries, markdown.heading_width"
                );
                assert_eq!(page, "bilbo config --help");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_config_file_note_holds_what_its_key_declares() {
        let Default::Folder(folder) = CONFIG.about.default else {
            unreachable!("a folder key");
        };
        let declared = format!(
            "The config file is ${}, else {}.",
            CONFIG.about.variable.unwrap_or_default(),
            folder.describe()
        );
        let note = USAGE
            .split_once('\n')
            .map(|(_, note)| note.replace('\n', " "));
        assert_eq!(note, Some(declared));
    }

    #[test]
    fn a_bare_value_with_control_bytes_is_one_escaped_line() {
        let hostile = settings::load(
            &|name| match name {
                "HOME" => Some(OsString::from("/h")),
                "BILBO_HOME" => Some(OsString::from("/x/\u{1b}]0;t\u{7}\nbilbo: forged")),
                _ => None,
            },
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &crate::kind_keys(),
        )
        .expect("a path with control bytes");
        let stdout = run(&bilbo(), key("paths.root"), &hostile)
            .expect("a value")
            .stdout(&View::PLAIN);
        assert_eq!(stdout, "/x/\\u{1b}]0;t\\u{7}\\nbilbo: forged\n");
    }
}
