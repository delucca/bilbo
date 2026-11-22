pub mod view;

use std::collections::BTreeSet;

use bilbo_entries::read;
use bilbo_entries::read::refs::{Locator, Ref};
use bilbo_entries::tag::{self, ChangeError, Request};
use bilbo_store::labels::{Labels, Tag};
use bilbo_store::segments::Store;
use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::env::Env;
use crate::console::message::{Message, notice_message, sanitize};
use crate::console::outcome::{Failure, Outcome};
use crate::console::settings::Settings;

const SUMMARY: &str = "List the tags, or change the tags of entries";
const ABOUT: &str = "bilbo tag: list the tags, list the entries of one, or change entries' tags.";
const USAGE: &str = "bilbo tag
  bilbo tag tag:<name>
  bilbo tag <ref>... [--add <tag>]... [--remove <tag>]...";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE, after_help = PAGE)]
pub struct Args {
    #[arg(value_name = "ref", value_parser = NonEmptyStringValueParser::new())]
    pub refs: Vec<String>,
    #[arg(
        long,
        value_name = "tag",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Declare this tag for each entry named"
    )]
    pub add: Vec<String>,
    #[arg(
        long,
        value_name = "tag",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Take this tag off each entry named"
    )]
    pub remove: Vec<String>,
}

const PAGE: &str = "\
Output:
  tag:<name>  <n> entries  <kind> <n>, ...     one row a tag
  <id>  <kind>  <capture label>  person|model  one row an entry of a tag
  changed <n>, unchanged <n>                   after --add or --remove

Exit:
  0  success, also when nothing changed and when no entry carries a tag
  1  a ref that names no entry, a tag no entry carries, an entry that would
     pass tag.cap, a store failure
  2  usage or config error, or a ref that names several entries

Examples:
  bilbo tag
  bilbo tag tag:rust
  bilbo tag 01M6ZGE4KTBYYH07AS2KVMSCAF --add garden --remove shed

Docs: https://github.com/delucca/bilbo/wiki/Commands#tag";

enum Form {
    Tags,
    Entries(Tag),
    Change(Request),
}

pub fn run(
    root: &Command,
    args: &Args,
    env: &mut Env,
    settings: &Settings,
) -> Result<Outcome, Failure> {
    let misuse = |reason: String| usage(root, sanitize(&reason), Some("tag"));
    match form(args, settings.read_matches_listed.value).map_err(misuse)? {
        Form::Tags => {
            let store = open_read_only(env, settings)?;
            let found = store.tags().map_err(|error| Failure::from(&error))?;
            Ok(listing(view::tags(&found), "no entry carries a tag"))
        }
        Form::Entries(tag) => {
            let store = open_read_only(env, settings)?;
            let found =
                tag::carriers(&tag, &store, &store).map_err(|error| Failure::from(&error))?;
            if found.is_empty() {
                return Err(Failure::Refused(format!("{tag} names no entry")));
            }
            Ok(Outcome::new(view::entries(&found)))
        }
        Form::Change(request) => {
            let layout = settings.layout();
            Store::check(&layout)?;
            let notice = &mut env.notice;
            let mut store = Store::open(&layout, settings.store.options(), &mut |said| {
                notice(&notice_message(&said));
            })?;
            let cap = settings.tag_cap.value;
            let output = tag::change(&request, cap, &mut store)
                .map_err(|error| change_failure(root, &error, cap))?;
            drop(store);
            Ok(Outcome::new(view::changed(&output)).with_diagnostics(view::new(&output.new)))
        }
    }
}

fn listing(text: String, none: &str) -> Outcome {
    if text.is_empty() {
        Outcome::new(text).with_diagnostics(vec![Message::info(none)])
    } else {
        Outcome::new(text)
    }
}

fn open_read_only(env: &mut Env, settings: &Settings) -> Result<Store, Failure> {
    let notice = &mut env.notice;
    Ok(Store::open_read_only(
        &settings.layout(),
        settings.store.options(),
        &mut |said| notice(&notice_message(&said)),
    )?)
}

fn not_a_tag(subject: &str) -> String {
    format!(
        "{subject} is not a tag: parts of a to z and digits joined by single hyphens, at most {} bytes",
        Tag::MAX_BYTES
    )
}

fn names(option: &str, texts: &[String]) -> Result<BTreeSet<Tag>, String> {
    texts
        .iter()
        .map(|text| Tag::new(text).ok_or_else(|| not_a_tag(&format!("{option} '{text}'"))))
        .collect()
}

fn form(args: &Args, listed: usize) -> Result<Form, String> {
    let options = !args.add.is_empty() || !args.remove.is_empty();
    if args.refs.is_empty() && !options {
        return Ok(Form::Tags);
    }
    if let Some(written) = args
        .refs
        .iter()
        .find_map(|text| Tag::written(text).map(|tag| (text, tag)))
    {
        let (text, tag) = written;
        let tag = tag.map_err(|_| not_a_tag(&format!("'{text}'")))?;
        if args.refs.len() > 1 || options {
            return Err(format!(
                "{text} lists the entries of a tag and takes no other argument"
            ));
        }
        return Ok(Form::Entries(tag));
    }
    let Some(first) = args.refs.first() else {
        return Err("--add and --remove need an entry: give its ref".to_owned());
    };
    if !options {
        return Err(format!(
            "give --add or --remove to change the tags of '{first}'"
        ));
    }
    let add = names("--add", &args.add)?;
    let remove = names("--remove", &args.remove)?;
    if let Some(both) = add.intersection(&remove).next() {
        return Err(format!("{both} is both added and removed"));
    }
    let mut targets = Vec::new();
    for text in &args.refs {
        let parsed = Ref::parse(text).map_err(|malformed| malformed.to_string())?;
        if parsed.locator != Locator::default() {
            return Err(format!("'{text}' names records; a tag is on an entry"));
        }
        targets.push(parsed.target);
    }
    Ok(Form::Change(Request {
        targets,
        add,
        remove,
        listed,
    }))
}

fn change_failure(root: &Command, error: &ChangeError, cap: usize) -> Failure {
    match error {
        ChangeError::Ref(ambiguous @ read::Error::Ambiguous { .. }) => {
            usage(root, sanitize(&ambiguous.to_string()), Some("tag"))
        }
        ChangeError::Ref(read::Error::Store(error))
        | ChangeError::Tag(tag::Error::Store(error)) => Failure::from(error),
        ChangeError::Ref(error) => Failure::Refused(sanitize(&error.to_string())),
        ChangeError::Tag(tag::Error::Full(id)) => Failure::Refused(format!(
            "{id} would carry more than tag.cap tags ({cap}); nothing was changed"
        )),
        ChangeError::Tag(tag::Error::Gone(id)) => Failure::Refused(format!("no entry {id}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, Form, form};

    fn args(refs: &[&str], add: &[&str], remove: &[&str]) -> Args {
        let owned = |texts: &[&str]| texts.iter().map(|text| (*text).to_owned()).collect();
        Args {
            refs: owned(refs),
            add: owned(add),
            remove: owned(remove),
        }
    }

    fn reason(given: &Args) -> String {
        match form(given, 5) {
            Err(reason) => reason,
            Ok(Form::Tags | Form::Entries(_) | Form::Change(_)) => panic!("{given:?} is no misuse"),
        }
    }

    const GRAMMAR: &str = "parts of a to z and digits joined by single hyphens, at most 64 bytes";

    #[test]
    fn each_misuse_of_the_verb_has_its_sentence() {
        let id = "01JC5Q3V7XKM4P8R2T6W9YB0DB";
        assert_eq!(
            reason(&args(&[id, "x"], &[], &[])),
            format!("give --add or --remove to change the tags of '{id}'")
        );
        assert_eq!(
            reason(&args(&[], &["a"], &[])),
            "--add and --remove need an entry: give its ref"
        );
        assert_eq!(
            reason(&args(&[id], &["Rust"], &[])),
            format!("--add 'Rust' is not a tag: {GRAMMAR}")
        );
        assert_eq!(
            reason(&args(&[id], &[], &["a b"])),
            format!("--remove 'a b' is not a tag: {GRAMMAR}")
        );
        assert_eq!(
            reason(&args(&[id], &["a"], &["a"])),
            "tag:a is both added and removed"
        );
        assert_eq!(
            reason(&args(&[&format!("{id}#r1")], &["a"], &[])),
            format!("'{id}#r1' names records; a tag is on an entry")
        );
        assert_eq!(
            reason(&args(&["tag:Rust"], &[], &[])),
            format!("'tag:Rust' is not a tag: {GRAMMAR}")
        );
        assert_eq!(
            reason(&args(&["tag:rust", id], &[], &[])),
            "tag:rust lists the entries of a tag and takes no other argument"
        );
        assert_eq!(
            reason(&args(&["tag:rust"], &["a"], &[])),
            "tag:rust lists the entries of a tag and takes no other argument"
        );
        assert_eq!(
            reason(&args(&["not an id"], &["a"], &[])),
            "'not an id' is not an entry id, its last 10 characters, a session id or 8 or more characters of one"
        );
    }
}
