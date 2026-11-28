pub mod set;
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
  bilbo config set <key> <value>
The config file is $BILBO_CONFIG, else $XDG_CONFIG_HOME/bilbo/config.toml,
else ~/.config/bilbo/config.toml.";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE, after_help = PAGE, args_conflicts_with_subcommands = true)]
pub struct Args {
    #[arg(value_name = "key", value_parser = NonEmptyStringValueParser::new())]
    pub key: Option<String>,
    #[command(subcommand)]
    pub set: Option<Sub>,
}

#[derive(Debug, clap::Subcommand)]
pub enum Sub {
    #[command(name = "set", about = "Write one key into the config file")]
    Set(Set),
}

#[derive(Debug, clap::Args)]
pub struct Set {
    #[arg(value_name = "key", value_parser = NonEmptyStringValueParser::new())]
    pub key: String,
    #[arg(value_name = "value", allow_hyphen_values = true)]
    pub value: String,
}

const PAGE: &str = "\
Output:
  <key> = <value> (<origin>)
One line per key, the value as TOML; the origin is default, <file>:<line>, a
variable or a flag. With <key>, its value alone, a string without its quotes.
  set <key> in <file>
After set: the key is written in the config file; every other byte stays.

Exit:
  0  success
  2  usage or config error: a key, value or file bilbo refuses

Examples:
  bilbo config
  bilbo config paths.root
  bilbo config set derive.enabled true

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
            Err(unknown_key(root, &name, &names))
        },
        |one| Ok(Outcome::new(format!("{}\n", sanitize(&one.to_bare())))),
    )
}

pub fn unknown_key(root: &Command, name: &str, names: &[&str]) -> Failure {
    usage(
        root,
        format!(
            "unknown key '{}'; the keys are {}",
            sanitize(name),
            names.join(", ")
        ),
        Some("config"),
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
            &crate::provider_keys(),
        )
        .expect("the defaults")
    }

    fn key(name: &str) -> Args {
        Args {
            key: Some(name.to_owned()),
            set: None,
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
        assert_eq!(stdout("paths.root"), "/h/.local/share/bilbo/store\n");
    }

    #[test]
    fn an_unknown_key_is_a_usage_error_that_lists_the_keys() {
        match run(&bilbo(), key("store.sealat"), &defaults()) {
            Err(Failure::Usage { reason, page, .. }) => {
                assert_eq!(
                    reason,
                    "unknown key 'store.sealat'; the keys are paths.root, paths.state, \
                     paths.cache, paths.temp, store.seal_at, store.seal_after, store.compression, store.tear_budget, store.wait, store.poll, follow, remember.editor, read.budget, read.outline_width, read.matches_listed, tag.cap, search.chunk_tokens, search.chunk_overlap, search.workers, search.index_memory, search.merge_deleted_percent, search.entry_tries, search.label_batch, search.list_limit, recall.budget, recall.notes_budget, recall.candidates, recall.cap, recall.record_cap, recall.copy_share, recall.copy_words, recall.question_words, recall.languages, recall.bm25_k1, recall.bm25_b, recall.read_batch, recall.typo_terms, recall.typo_letters, recall.typo_max_letters, recall.typo_two_edits, recall.pair_weight, digest.markdown.most, digest.markdown.floor, digest.session.most, digest.session.floor, digest.session.old_after, digest.session.subagents, digest.session.parts, digest.note.most, digest.note.floor, digest.note.width, digest.summary.enabled, digest.candidates, digest.query, digest.wait, digest.input, digest.agy.tail, digest.log.enabled, llm.provider, llm.response, llm.timeout, llm.poll, llm.anthropic.model, llm.anthropic.via, llm.anthropic.api.endpoint, llm.anthropic.api.version, llm.anthropic.api.key_env, llm.anthropic.api.key_command, llm.anthropic.api.headers, llm.anthropic.api.extra, llm.anthropic.tool.program, llm.anthropic.tool.effort, llm.anthropic.tool.arguments, llm.anthropic.tool.inherit, llm.anthropic.tool.environment, llm.anthropic.tool.project_files, llm.openai.model, llm.openai.via, llm.openai.api.endpoint, llm.openai.api.key_env, llm.openai.api.key_command, llm.openai.api.headers, llm.openai.api.extra, llm.openai.tool.program, llm.openai.tool.effort, llm.openai.tool.arguments, llm.openai.tool.inherit, llm.openai.tool.environment, llm.openai.tool.home, llm.openai.tool.system_files, derive.enabled, derive.in, derive.not_in, derive.instructions, derive.quiet, derive.window, derive.passage, derive.passage_head, derive.earlier, derive.names, derive.answer_tokens, derive.calls, derive.retries, derive.retry_wait, derive.retry_factor, derive.retry_limit, derive.tries, derive.failures, derive.tags.enabled, derive.tags.pick, derive.tags.instruction, derive.tags.model, derive.tags.most, derive.tags.notes, derive.summary.enabled, derive.summary.instruction, derive.summary.most, derive.summary.window, derive.summary.least, setup.plugin, setup.notes, setup.claude.enabled, setup.claude.program, setup.codex.enabled, setup.codex.program, setup.agy.enabled, setup.agy.program, setup.timer.logs, setup.timer.path, setup.refresh.every, setup.derive.every, setup.wait, setup.poll, markdown.heading_width, session.unpack_floor, session.unpack_ratio, session.unpack_window, session.wide_fields"
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
            .splitn(3, '\n')
            .nth(2)
            .map(|note| note.replace('\n', " "));
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
            &crate::provider_keys(),
        )
        .expect("a path with control bytes");
        let stdout = run(&bilbo(), key("paths.root"), &hostile)
            .expect("a value")
            .stdout(&View::PLAIN);
        assert_eq!(stdout, "/x/\\u{1b}]0;t\\u{7}\\nbilbo: forged\n");
    }

    #[test]
    fn set_is_a_form_of_config_with_two_values() {
        let parse = |words: &[&str]| {
            let mut argv = vec!["bilbo", "config"];
            argv.extend_from_slice(words);
            bilbo().try_get_matches_from(argv).is_ok()
        };
        assert!(parse(&["set", "derive.enabled", "true"]));
        assert!(!parse(&["set", "derive.enabled"]));
        assert!(!parse(&["set"]));
        assert!(parse(&["derive.enabled"]));
    }
}
