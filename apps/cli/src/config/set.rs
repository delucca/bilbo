use std::ffi::OsString;
use std::path::Path;

use crate::console::settings::KindKey;
use bilbo_settings::edit::{self, Failed, Misplaced, Value};
use bilbo_settings::key::Default;

use super::{Set, unknown_key};
use crate::console::message::{Message, sanitize};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::settings::{self, CACHE, CONFIG, Group, ROOT, STATE};
use clap::Command;

const BY_HAND: &str = "set it by hand:";

enum Why<'a> {
    Refused(&'a str),
    Misplaced(Misplaced),
    Changed,
    Unreadable(&'a str),
}

struct Place<'a> {
    key: &'a str,
    file: &'a str,
    value: &'a str,
}

impl Place<'_> {
    fn said(&self, why: &Why<'_>) -> (String, String) {
        let Place { key, file, value } = self;
        let reason = match why {
            Why::Refused(reason) => format!("{key} in {file}: {reason}"),
            Why::Misplaced(found) => format!("{key} in {file}: {}", Failed::Misplaced(*found)),
            Why::Changed => format!("{key} in {file}: {}", Failed::Changed),
            Why::Unreadable(error) => format!("{key}: {error}"),
        };
        (
            format!("cannot set {reason}; {BY_HAND}"),
            format!("{key} = {value}\n"),
        )
    }
}

fn names_a_folder(name: &str) -> bool {
    name == "HOME"
        || [&CONFIG, &ROOT, &STATE, &CACHE].iter().any(|key| {
            key.about.variable == Some(name)
                || matches!(&key.about.default, Default::Folder(folder) if folder.xdg == name)
        })
}

pub fn run(
    root: &Command,
    set: &Set,
    var: &dyn Fn(&str) -> Option<OsString>,
    kind: &[&'static KindKey],
    groups: &[&'static Group],
) -> Result<Outcome, Failure> {
    let abouts = settings::keys(kind, groups);
    let names: Vec<&str> = abouts.iter().map(|about| about.name).collect();
    let key = set.key.as_str();
    if !names.contains(&key) {
        return Err(unknown_key(root, key, &names));
    }
    let home = var("HOME");
    let allowed =
        |value: &Value| settings::allowed_value(key, value.as_str(), home.as_ref(), kind, groups);
    let value = match Value::literal(&set.value) {
        Some(value) => match allowed(&value) {
            Ok(()) => value,
            Err(first) => {
                let text = Value::text(&set.value);
                allowed(&text).map_err(|_| Failure::Config(first))?;
                text
            }
        },
        None => {
            let text = Value::text(&set.value);
            allowed(&text).map_err(Failure::Config)?;
            text
        }
    };
    let config = settings::config_path(var)?;
    let shown = config.display().to_string();
    let place = Place {
        key,
        file: &shown,
        value: value.as_str(),
    };
    let stop = |why: Why<'_>| {
        let (text, line) = place.said(&why);
        Ok(Outcome::new(line)
            .with_diagnostics(vec![Message::error(sanitize(&text))])
            .with_exit(Exit::Config))
    };
    if let Err(reason) = edit::check(&config) {
        return stop(Why::Refused(&reason));
    }
    let lock = match edit::lock_file(&config) {
        Ok(lock) => lock,
        Err(error) => return stop(Why::Refused(&error.to_string())),
    };
    if let Err(error) = lock.lock() {
        return stop(Why::Refused(&error.to_string()));
    }
    let before = match edit::read(&config) {
        Ok(before) => before,
        Err(error) => return stop(Why::Refused(&error)),
    };
    let after = match edit::set(&config, &before, key, &value) {
        Ok(after) => after,
        Err(Failed::Misplaced(found)) => return stop(Why::Misplaced(found)),
        Err(Failed::Changed) => return stop(Why::Changed),
        Err(Failed::Unreadable(error)) => return stop(Why::Unreadable(&error.to_string())),
    };
    let unsynced = match edit::replace(&config, &after) {
        Ok(unsynced) => unsynced,
        Err(error) => return stop(Why::Refused(&error.to_string())),
    };
    let mut diagnostics = vec![Message::info(&format!("set {key} in {shown}"))];
    if let Some(failed) = unsynced {
        diagnostics.push(Message::warning(&format!(
            "{shown}'s folder could not be synced: {failed}"
        )));
    }
    let folders = |name: &str| {
        if names_a_folder(name) {
            var(name)
        } else {
            None
        }
    };
    let loaded = settings::load(
        &folders,
        &|path: &Path| {
            if path == config {
                Ok(after.clone())
            } else {
                edit::read_regular(path)
            }
        },
        kind,
        groups,
    );
    if let Err(failed) = loaded {
        diagnostics.push(Message::warning(&format!(
            "{shown} still holds a value bilbo refuses: {failed}"
        )));
    }
    if let Some(variable) = abouts
        .iter()
        .find(|about| about.name == key)
        .and_then(|about| about.variable)
        && var(variable).is_some()
    {
        diagnostics.push(Message::warning(&format!(
            "{variable} is set and wins over {key} in {shown}"
        )));
    }
    Ok(Outcome::new(String::new()).with_diagnostics(diagnostics))
}

#[cfg(test)]
mod tests {
    use bilbo_settings::edit::{How, Misplaced};

    use super::{BY_HAND, Place, Why};

    fn said(why: &Why<'_>, key: &str) -> (String, String) {
        Place {
            key,
            file: "/c/config.toml",
            value: "true",
        }
        .said(why)
    }

    #[test]
    fn each_reason_has_its_clause_and_the_line_holds_the_whole_path() {
        let rows = [
            (
                Why::Refused("it is a symbolic link"),
                "cannot set derive.enabled in /c/config.toml: it is a symbolic link; {BY_HAND}",
            ),
            (
                Why::Refused("Permission denied (os error 13)"),
                "cannot set derive.enabled in /c/config.toml: Permission denied (os error 13); {BY_HAND}",
            ),
            (
                Why::Misplaced(Misplaced {
                    how: How::Dotted,
                    line: 1,
                    holds: true,
                }),
                "cannot set derive.enabled in /c/config.toml: line 1 sets it with a dotted key; {BY_HAND}",
            ),
            (
                Why::Misplaced(Misplaced {
                    how: How::Dotted,
                    line: 1,
                    holds: false,
                }),
                "cannot set derive.enabled in /c/config.toml: line 1 sets its table with a dotted key; {BY_HAND}",
            ),
            (
                Why::Misplaced(Misplaced {
                    how: How::Inline,
                    line: 2,
                    holds: true,
                }),
                "cannot set derive.enabled in /c/config.toml: line 2 sets it inside braces; {BY_HAND}",
            ),
            (
                Why::Misplaced(Misplaced {
                    how: How::Inline,
                    line: 2,
                    holds: false,
                }),
                "cannot set derive.enabled in /c/config.toml: line 2 sets its table inside braces; {BY_HAND}",
            ),
            (
                Why::Misplaced(Misplaced {
                    how: How::OwnTable,
                    line: 4,
                    holds: true,
                }),
                "cannot set derive.enabled in /c/config.toml: line 4 sets it as a table of its own; {BY_HAND}",
            ),
            (
                Why::Changed,
                "cannot set derive.enabled in /c/config.toml: bilbo could not place it; {BY_HAND}",
            ),
            (
                Why::Unreadable("/c/config.toml:1: bad"),
                "cannot set derive.enabled: /c/config.toml:1: bad; {BY_HAND}",
            ),
        ];
        for (why, text) in rows {
            let (said, line) = said(&why, "derive.enabled");
            assert_eq!(said, text.replace("{BY_HAND}", BY_HAND));
            assert_eq!(line, "derive.enabled = true\n");
        }
    }
}
