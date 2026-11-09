pub mod overview;
pub mod page;

use std::ffi::OsString;

use clap::Command;

use crate::console::args::{usage, with_verbs};
use crate::console::message::sanitize;
use crate::console::outcome::{Failure, Outcome};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Request {
    Overview,
    Page(String),
    NoPage(String),
    Extra(String),
}

fn request(root: &Command, words: &[OsString]) -> Request {
    let words: Vec<&OsString> = words
        .iter()
        .filter(|word| *word != "-h" && *word != "--help")
        .collect();
    match words.as_slice() {
        [] => Request::Overview,
        [word] if *word == "help" => Request::Overview,
        [word] => {
            let name = word.to_string_lossy().into_owned();
            if root.find_subcommand(&name).is_some() {
                Request::Page(name)
            } else {
                Request::NoPage(name)
            }
        }
        [_, extra, ..] => Request::Extra(extra.to_string_lossy().into_owned()),
    }
}

pub fn run(root: &Command, words: &[OsString]) -> Result<Outcome, Failure> {
    let text = match request(root, words) {
        Request::Overview => page::overview(root),
        Request::Page(verb) => page::page(root, &verb).ok_or_else(|| no_page(root, &verb))?,
        Request::NoPage(word) => return Err(no_page(root, &word)),
        Request::Extra(word) => {
            return Err(usage(
                root,
                format!("unexpected argument '{}'", sanitize(&word)),
                None,
            ));
        }
    };
    Ok(page::outcome(text))
}

fn no_page(root: &Command, word: &str) -> Failure {
    let word = sanitize(word);
    let reason = if word.starts_with('-') {
        format!("unknown option '{word}'")
    } else {
        with_verbs(root, &format!("unknown verb '{word}'"))
    };
    usage(root, reason, None)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{Request, request, run};
    use crate::bilbo;
    use crate::console::outcome::Failure;
    use crate::console::view::View;
    use crate::help::page;
    use crate::probe::{self, with_verb};

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_request_is_the_overview_a_page_no_page_or_an_extra_word() {
        let root = with_verb(bilbo(), probe::verb());
        for list in [&[][..], &["help"], &["--help"], &["help", "-h"], &["-h"]] {
            assert_eq!(request(&root, &args(list)), Request::Overview, "{list:?}");
        }
        assert_eq!(
            request(&root, &args(&["probe"])),
            Request::Page("probe".to_owned())
        );
        assert_eq!(
            request(&root, &args(&["probe", "--help"])),
            Request::Page("probe".to_owned())
        );
        assert_eq!(
            request(&root, &args(&["frob"])),
            Request::NoPage("frob".to_owned())
        );
        assert_eq!(
            request(&root, &args(&["--verbose"])),
            Request::NoPage("--verbose".to_owned())
        );
        assert_eq!(
            request(&root, &args(&["frob", "x"])),
            Request::Extra("x".to_owned())
        );
    }

    #[test]
    fn help_topics_are_the_overview_a_page_or_a_usage_error() {
        let root = with_verb(bilbo(), probe::verb());
        let stdout = |list: &[&str]| {
            run(&root, &args(list))
                .unwrap_or_else(|failure| panic!("{list:?}: {failure:?}"))
                .stdout(&View::PLAIN)
        };
        assert_eq!(stdout(&[]), page::overview(&root));
        assert_eq!(stdout(&["help"]), page::overview(&root));
        assert_eq!(stdout(&["probe"]), page::page(&root, "probe").unwrap());
        let reason = |list: &[&str]| match run(&root, &args(list)) {
            Err(Failure::Usage { reason, page, .. }) => format!("{reason}; {page}"),
            other => panic!("{list:?}: {other:?}"),
        };
        assert_eq!(
            reason(&["frob"]),
            "unknown verb 'frob'; the verbs are remember, config, probe; bilbo --help"
        );
        assert_eq!(
            reason(&["--verbose"]),
            "unknown option '--verbose'; bilbo --help"
        );
        assert_eq!(
            reason(&["frob", "x"]),
            "unexpected argument 'x'; bilbo --help"
        );
        assert_eq!(
            reason(&["a\nb"]),
            "unknown verb 'a\\nb'; the verbs are remember, config, probe; bilbo --help"
        );
    }
}
