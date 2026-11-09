pub mod view;

use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::message::sanitize;
use crate::console::outcome::{Failure, Outcome};
use crate::console::settings::Settings;

const ABOUT: &str = "bilbo config: show each setting, its value and where the value came from.";
const USAGE: &str = "bilbo config [<key>]";

#[derive(Debug, clap::Args)]
#[command(about = ABOUT, override_usage = USAGE, after_help = PAGE)]
pub struct Args {
    #[arg(value_name = "key", value_parser = NonEmptyStringValueParser::new())]
    pub key: Option<String>,
}

const PAGE: &str = "\
A key takes the first of these that sets it: a flag, its variable, the config
file, its default. The config file is $BILBO_CONFIG, else
$XDG_CONFIG_HOME/bilbo/config.toml, else ~/.config/bilbo/config.toml; when it
is missing every key takes its variable or its default, unless BILBO_CONFIG
names it. reference/configuration.md says what each key means.

Output:
  <key> = <value> (<origin>)
One line per key, the value as TOML; the origin is default, <file>:<line>,
the variable or the flag. With <key>, its value alone, a string without its
quotes.

Exit: 0 success; 2 usage or config error: an unknown key, or a config file or
variable bilbo refuses.

Examples:
  bilbo config
  bilbo config paths.root

Docs: reference/commands.md#config";

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

    use super::{Args, run};
    use crate::bilbo;
    use crate::console::outcome::Failure;
    use crate::console::settings::{self, Settings};
    use crate::console::view::View;

    fn defaults() -> Settings {
        settings::load(
            &|name| (name == "HOME").then(|| OsString::from("/h")),
            &|_| Err(std::io::ErrorKind::NotFound.into()),
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
                     paths.cache, store.seal_at, store.compression, store.tear_budget"
                );
                assert_eq!(page, "bilbo config --help");
            }
            other => panic!("{other:?}"),
        }
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
        )
        .expect("a path with control bytes");
        let stdout = run(&bilbo(), key("paths.root"), &hostile)
            .expect("a value")
            .stdout(&View::PLAIN);
        assert_eq!(stdout, "/x/\\u{1b}]0;t\\u{7}\\nbilbo: forged\n");
    }
}
