use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bilbo_entries::follows::Follow;
use bilbo_entries::read;
use bilbo_entries::tag;
use bilbo_settings::duration::{self, DAY, MINUTE};
use bilbo_settings::error::Error;
use bilbo_settings::file::{File, Named};
use bilbo_settings::folder::{self, Absolute, Folder};
use bilbo_settings::key::{
    About, Default, First, Key, Origin, Resolved, Shown, Source, Value, quote,
};
use bilbo_settings::size::{self, MIB};
use bilbo_store::keys;
use bilbo_store::layout::Layout;

/// Not in `keys()`: the file cannot name itself.
pub const CONFIG: Key<PathBuf> = Key {
    about: About {
        name: "config",
        variable: Some("BILBO_CONFIG"),
        default: Default::Folder(Folder {
            xdg: "XDG_CONFIG_HOME",
            under_xdg: "bilbo/config.toml",
            under_home: ".config/bilbo/config.toml",
        }),
        allowed: "an absolute path or one starting with ~/",
        doc: "The config file.",
    },
    parse: folder::path,
    show: |path| folder::show(path),
};

pub const ROOT: Key<PathBuf> = Key {
    about: About {
        name: "paths.root",
        variable: Some("BILBO_HOME"),
        default: Default::Folder(Folder {
            xdg: "XDG_DATA_HOME",
            under_xdg: "bilbo/store",
            under_home: ".local/share/bilbo/store",
        }),
        doc: "The store: its sealed segments.",
        ..CONFIG.about
    },
    ..CONFIG
};

pub const STATE: Key<PathBuf> = Key {
    about: About {
        name: "paths.state",
        variable: None,
        default: Default::Folder(Folder {
            xdg: "XDG_STATE_HOME",
            under_xdg: "bilbo",
            under_home: ".local/state/bilbo",
        }),
        doc: "This device's state: open.seg, the store lock, its store root and the query log.",
        ..CONFIG.about
    },
    ..CONFIG
};

pub const CACHE: Key<PathBuf> = Key {
    about: About {
        name: "paths.cache",
        variable: None,
        default: Default::Folder(Folder {
            xdg: "XDG_CACHE_HOME",
            under_xdg: "bilbo",
            under_home: ".cache/bilbo",
        }),
        doc: "What bilbo rebuilds from the store: the catalog.",
        ..CONFIG.about
    },
    ..CONFIG
};

pub const TEMP: Key<PathBuf> = Key {
    about: About {
        name: "paths.temp",
        variable: None,
        default: Default::Absolute(Absolute {
            variable: "TMPDIR",
            fallback: "/tmp",
        }),
        doc: "What bilbo keeps only for a moment: the draft of bilbo remember --edit, in a private folder of its own.",
        ..CONFIG.about
    },
    ..CONFIG
};

pub const EDITOR: Key<OsString> = Key {
    about: About {
        name: "remember.editor",
        variable: Some("BILBO_REMEMBER_EDITOR"),
        default: Default::First(First {
            variables: &["VISUAL", "EDITOR"],
            toml: r#""""#,
        }),
        allowed: "a command line, or the empty text for no editor",
        doc: "The command bilbo remember --edit starts as the editor: /bin/sh reads it as a command line, with the note's file as its last argument. Empty: no editor, and --edit says so.",
    },
    parse: |value| {
        value
            .os_text()
            .filter(|text| !text.as_bytes().contains(&0))
            .map(OsStr::to_os_string)
            .ok_or_else(|| value.refuse())
    },
    show: |text| quote(&text.to_string_lossy()),
};

/// The composition root's: its default names a kind, which only the registry may.
pub const FOLLOW: Key<Vec<Follow>> = Key {
    about: About {
        name: "follow",
        variable: None,
        default: Default::Toml(
            r#"[{ path = "~/.claude/projects", kind = "session" }, { path = "~/.codex/sessions", kind = "session" }]"#,
        ),
        allowed: "a list of [[follow]] tables, each with a path and an optional kind",
        doc: "What bilbo refresh keeps up with: each folder or file, and the kind that runs there, else the one detected.",
    },
    parse: follows,
    show: |list| show_follows(list),
};

pub const DIGEST_MARKDOWN_MOST: Key<usize> = Key {
    about: About {
        name: "digest.markdown.most",
        variable: Some("BILBO_DIGEST_MARKDOWN_MOST"),
        default: Default::Toml("6"),
        allowed: "a whole number from 0 to 100",
        doc: "The most Markdown files bilbo digest lists for a prompt; 0 turns that list off.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| (0..=100).contains(count))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const DIGEST_MARKDOWN_FLOOR: Key<f64> = Key {
    about: About {
        name: "digest.markdown.floor",
        variable: Some("BILBO_DIGEST_MARKDOWN_FLOOR"),
        default: Default::Toml("8"),
        allowed: "a number from 0 to 1000",
        doc: "The lowest score a Markdown file's best chunk may have for bilbo digest to list the file; 0 lists every file it took.",
    },
    parse: |value| {
        value
            .decimal()
            .filter(|floor| (0.0..=1000.0).contains(floor))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const DIGEST_SESSION_MOST: Key<usize> = Key {
    about: About {
        name: "digest.session.most",
        variable: Some("BILBO_DIGEST_SESSION_MOST"),
        default: Default::Toml("6"),
        allowed: "a whole number from 0 to 100",
        doc: "The most sessions bilbo digest lists for a prompt; 0 turns that list off.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| (0..=100).contains(count))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const DIGEST_SESSION_FLOOR: Key<f64> = Key {
    about: About {
        name: "digest.session.floor",
        variable: Some("BILBO_DIGEST_SESSION_FLOOR"),
        default: Default::Toml("12"),
        allowed: "a number from 0 to 1000",
        doc: "The lowest score a session's best passage may have for bilbo digest to list the session; 0 lists every session it took.",
    },
    parse: |value| {
        value
            .decimal()
            .filter(|floor| (0.0..=1000.0).contains(floor))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const DIGEST_SESSION_OLD_AFTER: Key<Duration> = Key {
    about: About {
        name: "digest.session.old_after",
        variable: Some("BILBO_DIGEST_SESSION_OLD_AFTER"),
        default: Default::Toml(r#""30d""#),
        allowed: "between 1d and 36500d",
        doc: "The age of a session, counted from when it began, after which bilbo digest no longer lists it.",
    },
    parse: |value| {
        duration::read(value)
            .filter(|age| (DAY..=36500 * DAY).contains(age))
            .ok_or_else(|| value.refuse())
    },
    show: |age| duration::show(*age),
};

pub const DIGEST_SESSION_SUBAGENTS: Key<bool> = Key {
    about: About {
        name: "digest.session.subagents",
        variable: Some("BILBO_DIGEST_SESSION_SUBAGENTS"),
        default: Default::Toml("false"),
        allowed: "true or false",
        doc: "Whether bilbo digest lists a subagent's file among the sessions.",
    },
    parse: |value| value.boolean().ok_or_else(|| value.refuse()),
    show: ToString::to_string,
};

pub const DIGEST_SESSION_PARTS: Key<bool> = Key {
    about: About {
        name: "digest.session.parts",
        variable: Some("BILBO_DIGEST_SESSION_PARTS"),
        default: Default::Toml("false"),
        allowed: "true or false",
        doc: "Whether bilbo digest lists a session at a passage that is a part of a record too long for one chunk, such as a long tool output.",
    },
    parse: |value| value.boolean().ok_or_else(|| value.refuse()),
    show: ToString::to_string,
};

pub const DIGEST_NOTE_MOST: Key<usize> = Key {
    about: About {
        name: "digest.note.most",
        variable: Some("BILBO_DIGEST_NOTE_MOST"),
        default: Default::Toml("6"),
        allowed: "a whole number from 0 to 100",
        doc: "The most notes bilbo digest lists for a prompt; 0 turns that list off.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| (0..=100).contains(count))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const DIGEST_NOTE_FLOOR: Key<f64> = Key {
    about: About {
        name: "digest.note.floor",
        variable: Some("BILBO_DIGEST_NOTE_FLOOR"),
        default: Default::Toml("8"),
        allowed: "a number from 0 to 1000",
        doc: "The lowest score a note may have for bilbo digest to list it; 0 lists every note it took.",
    },
    parse: |value| {
        value
            .decimal()
            .filter(|floor| (0.0..=1000.0).contains(floor))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const DIGEST_NOTE_WIDTH: Key<usize> = Key {
    about: About {
        name: "digest.note.width",
        variable: Some("BILBO_DIGEST_NOTE_WIDTH"),
        default: Default::Toml("300"),
        allowed: "a whole number from 20 to 1000",
        doc: "The most characters of a note bilbo digest shows in a row, an ellipsis included: a longer note is cut and ends in one.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|width| usize::try_from(width).ok())
            .filter(|width| (20..=1000).contains(width))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const DIGEST_SUMMARY_ENABLED: Key<bool> = Key {
    about: About {
        name: "digest.summary.enabled",
        variable: Some("BILBO_DIGEST_SUMMARY_ENABLED"),
        default: Default::Toml("true"),
        allowed: "true or false",
        doc: "Whether bilbo digest shows on a row the summary bilbo derive wrote of the row's entry.",
    },
    parse: |value| value.boolean().ok_or_else(|| value.refuse()),
    show: ToString::to_string,
};

pub const DIGEST_WAIT: Key<Duration> = Key {
    about: About {
        name: "digest.wait",
        variable: Some("BILBO_DIGEST_WAIT"),
        default: Default::Toml(r#""1s""#),
        allowed: "between 0s and 1m",
        doc: "How long bilbo digest waits, at a session's first prompt, for a bilbo that is writing the store before it gives up and lists nothing.",
    },
    parse: |value| {
        duration::read(value)
            .filter(|wait| *wait <= MINUTE)
            .ok_or_else(|| value.refuse())
    },
    show: |wait| duration::show(*wait),
};

pub const DIGEST_INPUT: Key<u64> = Key {
    about: About {
        name: "digest.input",
        variable: Some("BILBO_DIGEST_INPUT"),
        default: Default::Toml(r#""4 MiB""#),
        allowed: "between 1 KiB and 64 MiB",
        doc: "The longest input, in bytes, that bilbo digest takes from a hook on standard input: a longer one is no prompt.",
    },
    parse: |value| {
        size::read(value)
            .filter(|bytes| (1024..=64 * MIB).contains(bytes))
            .ok_or_else(|| value.refuse())
    },
    show: |bytes| size::show(*bytes),
};

pub const DIGEST_AGY_TAIL: Key<u64> = Key {
    about: About {
        name: "digest.agy.tail",
        variable: Some("BILBO_DIGEST_AGY_TAIL"),
        default: Default::Toml(r#""1 MiB""#),
        allowed: "between 1 KiB and 64 MiB",
        doc: "The most bytes of the end of an AGY transcript that bilbo digest --agy reads for the prompt: a last step longer than that gives no prompt.",
    },
    parse: |value| {
        size::read(value)
            .filter(|bytes| (1024..=64 * MIB).contains(bytes))
            .ok_or_else(|| value.refuse())
    },
    show: |bytes| size::show(*bytes),
};

pub const DIGEST_LOG_ENABLED: Key<bool> = Key {
    about: About {
        name: "digest.log.enabled",
        variable: Some("BILBO_DIGEST_LOG_ENABLED"),
        default: Default::Toml("true"),
        allowed: "true or false",
        doc: "Whether bilbo digest logs each run in digest.log in the state folder.",
    },
    parse: |value| value.boolean().ok_or_else(|| value.refuse()),
    show: ToString::to_string,
};

const LISTS: [&About; 11] = [
    &DIGEST_MARKDOWN_MOST.about,
    &DIGEST_MARKDOWN_FLOOR.about,
    &DIGEST_SESSION_MOST.about,
    &DIGEST_SESSION_FLOOR.about,
    &DIGEST_SESSION_OLD_AFTER.about,
    &DIGEST_SESSION_SUBAGENTS.about,
    &DIGEST_SESSION_PARTS.about,
    &DIGEST_NOTE_MOST.about,
    &DIGEST_NOTE_FLOOR.about,
    &DIGEST_NOTE_WIDTH.about,
    &DIGEST_SUMMARY_ENABLED.about,
];

const PATHS: [&About; 4] = [&ROOT.about, &STATE.about, &CACHE.about, &TEMP.about];

pub type KindKey = Key<usize>;

#[derive(Debug)]
pub struct Group {
    pub about: &'static [&'static About],
    pub resolve: fn(&Source<'_>) -> Result<Vec<Shown>, Error>,
}

pub fn keys(kind: &[&'static KindKey], groups: &[&'static Group]) -> Vec<&'static About> {
    PATHS
        .into_iter()
        .chain(keys::ABOUT)
        .chain([
            &FOLLOW.about,
            &EDITOR.about,
            &read::BUDGET.about,
            &read::OUTLINE_WIDTH.about,
            &read::MATCHES_LISTED.about,
            &tag::CAP.about,
        ])
        .chain(bilbo_search::keys::ABOUT)
        .chain(bilbo_search::recall::keys::ABOUT)
        .chain(LISTS)
        .chain(bilbo_search::digest::keys::ABOUT)
        .chain([
            &DIGEST_WAIT.about,
            &DIGEST_INPUT.about,
            &DIGEST_AGY_TAIL.about,
            &DIGEST_LOG_ENABLED.about,
        ])
        .chain(bilbo_llm::keys::ABOUT)
        .chain(groups.iter().flat_map(|group| group.about.iter().copied()))
        .chain(bilbo_derive::keys::ABOUT)
        .chain(bilbo_derive::tags::keys::ABOUT)
        .chain(bilbo_derive::summary::keys::ABOUT)
        .chain([
            &SETUP_PLUGIN.about,
            &SETUP_NOTES.about,
            &SETUP_CLAUDE_ENABLED.about,
            &SETUP_CLAUDE_PROGRAM.about,
            &SETUP_CODEX_ENABLED.about,
            &SETUP_CODEX_PROGRAM.about,
            &SETUP_AGY_ENABLED.about,
            &SETUP_AGY_PROGRAM.about,
            &SETUP_TIMER_LOGS.about,
        ])
        .chain(bilbo_setup::keys::ABOUT)
        .chain(kind.iter().map(|key| &key.about))
        .collect()
}

fn follows(value: &Value<'_>) -> Result<Vec<Follow>, Error> {
    let items = value.items().ok_or_else(|| value.refuse())?;
    items
        .iter()
        .map(|item| {
            let fields = item.fields().ok_or_else(|| item.refuse())?;
            let mut path = None;
            let mut kind = None;
            for (name, field) in fields {
                match name {
                    "path" => path = Some(folder::path(&field)?),
                    "kind" => {
                        let text = field.text().filter(|text| !text.is_empty());
                        kind = Some(text.ok_or_else(|| field.refuse())?.to_owned());
                    }
                    _ => return Err(field.refuse()),
                }
            }
            Ok(Follow {
                path: path.ok_or_else(|| item.refuse())?,
                kind,
            })
        })
        .collect()
}

pub fn inline_follow(follow: &Follow) -> String {
    match &follow.kind {
        Some(kind) => format!(
            "{{ path = {}, kind = {} }}",
            folder::show(&follow.path),
            quote(kind)
        ),
        None => format!("{{ path = {} }}", folder::show(&follow.path)),
    }
}

pub fn show_follows(list: &[Follow]) -> String {
    let tables: Vec<String> = list.iter().map(inline_follow).collect();
    format!("[{}]", tables.join(", "))
}

#[derive(Debug)]
pub struct Digest {
    pub markdown_most: Resolved<usize>,
    pub markdown_floor: Resolved<f64>,
    pub session_most: Resolved<usize>,
    pub session_floor: Resolved<f64>,
    pub session_old_after: Resolved<Duration>,
    pub session_subagents: Resolved<bool>,
    pub session_parts: Resolved<bool>,
    pub note_most: Resolved<usize>,
    pub note_floor: Resolved<f64>,
    pub note_width: Resolved<usize>,
    pub summary_enabled: Resolved<bool>,
    pub wait: Resolved<Duration>,
    pub input: Resolved<u64>,
    pub agy_tail: Resolved<u64>,
    pub log_enabled: Resolved<bool>,
}

fn optional_path(value: &Value<'_>) -> Result<Option<PathBuf>, Error> {
    match value.os_text() {
        Some(text) if text.is_empty() => Ok(None),
        _ => folder::path(value).map(Some),
    }
}

fn shown_path(path: &Option<PathBuf>) -> String {
    path.as_deref().map_or_else(|| quote(""), folder::show)
}

fn program(value: &Value<'_>) -> Result<PathBuf, Error> {
    match value.os_text() {
        Some(text) if text.is_empty() => Err(value.refuse()),
        Some(text) if !text.as_bytes().contains(&b'/') && !text.as_bytes().contains(&0) => {
            Ok(PathBuf::from(text))
        }
        _ => folder::path(value),
    }
}

pub const SETUP_PLUGIN: Key<Option<PathBuf>> = Key {
    about: About {
        name: "setup.plugin",
        variable: Some("BILBO_SETUP_PLUGIN"),
        default: Default::Toml(r#""""#),
        allowed: "an absolute path or one starting with ~/, or the empty text for the package's own",
        doc: "The folder that holds the plugin bilbo setup installs: the two marketplace files and plugins/bilbo. Empty: the folder share/bilbo of the package this bilbo runs from.",
    },
    parse: optional_path,
    show: shown_path,
};

pub const SETUP_NOTES: Key<Option<PathBuf>> = Key {
    about: About {
        name: "setup.notes",
        variable: Some("BILBO_SETUP_NOTES"),
        default: Default::Folder(Folder {
            xdg: "XDG_DATA_HOME",
            under_xdg: "bilbo/notes",
            under_home: ".local/share/bilbo/notes",
        }),
        allowed: "an absolute path or one starting with ~/, or the empty text for none",
        doc: "The notes folder bilbo setup makes, when it is missing, and follows as Markdown. Its default is used only while the config file follows no Markdown folder that is there.",
    },
    parse: optional_path,
    show: shown_path,
};

pub const SETUP_CLAUDE_ENABLED: Key<bool> = Key {
    about: About {
        name: "setup.claude.enabled",
        variable: Some("BILBO_SETUP_CLAUDE_ENABLED"),
        default: Default::Toml("true"),
        allowed: "true or false",
        doc: "Whether bilbo setup installs the plugin in Claude Code, and bilbo setup --remove takes it out of it.",
    },
    parse: |value| value.boolean().ok_or_else(|| value.refuse()),
    show: ToString::to_string,
};

pub const SETUP_CLAUDE_PROGRAM: Key<PathBuf> = Key {
    about: About {
        name: "setup.claude.program",
        variable: Some("BILBO_SETUP_CLAUDE_PROGRAM"),
        default: Default::Toml(r#""claude""#),
        allowed: "a program's name, or its absolute path or one starting with ~/",
        doc: "The program bilbo setup starts to install the plugin in Claude Code.",
    },
    parse: program,
    show: |path| folder::show(path),
};

pub const SETUP_CODEX_ENABLED: Key<bool> = Key {
    about: About {
        name: "setup.codex.enabled",
        variable: Some("BILBO_SETUP_CODEX_ENABLED"),
        default: Default::Toml("true"),
        allowed: "true or false",
        doc: "Whether bilbo setup installs the plugin in Codex and trusts its hooks there, and bilbo setup --remove takes both out.",
    },
    parse: |value| value.boolean().ok_or_else(|| value.refuse()),
    show: ToString::to_string,
};

pub const SETUP_CODEX_PROGRAM: Key<PathBuf> = Key {
    about: About {
        name: "setup.codex.program",
        variable: Some("BILBO_SETUP_CODEX_PROGRAM"),
        default: Default::Toml(r#""codex""#),
        allowed: "a program's name, or its absolute path or one starting with ~/",
        doc: "The program bilbo setup starts to install the plugin in Codex and to trust its hooks there.",
    },
    parse: program,
    show: |path| folder::show(path),
};

pub const SETUP_AGY_ENABLED: Key<bool> = Key {
    about: About {
        name: "setup.agy.enabled",
        variable: Some("BILBO_SETUP_AGY_ENABLED"),
        default: Default::Toml("true"),
        allowed: "true or false",
        doc: "Whether bilbo setup installs the plugin in AGY, and bilbo setup --remove takes it out of it.",
    },
    parse: |value| value.boolean().ok_or_else(|| value.refuse()),
    show: ToString::to_string,
};

pub const SETUP_AGY_PROGRAM: Key<PathBuf> = Key {
    about: About {
        name: "setup.agy.program",
        variable: Some("BILBO_SETUP_AGY_PROGRAM"),
        default: Default::Toml(r#""agy""#),
        allowed: "a program's name, or its absolute path or one starting with ~/",
        doc: "The program bilbo setup starts to install the plugin in AGY.",
    },
    parse: program,
    show: |path| folder::show(path),
};

pub const SETUP_TIMER_LOGS: Key<Option<PathBuf>> = Key {
    about: About {
        name: "setup.timer.logs",
        variable: Some("BILBO_SETUP_TIMER_LOGS"),
        default: Default::Toml(r#""""#),
        allowed: "an absolute path or one starting with ~/, or the empty text for the state folder's own",
        doc: "The folder the timers' runs append what they print to, as refresh.log and derive.log. Empty: the folder timer in the state folder.",
    },
    parse: optional_path,
    show: shown_path,
};

#[derive(Debug)]
pub struct Setup {
    pub plugin: Resolved<Option<PathBuf>>,
    pub notes: Resolved<Option<PathBuf>>,
    pub claude_enabled: Resolved<bool>,
    pub claude_program: Resolved<PathBuf>,
    pub codex_enabled: Resolved<bool>,
    pub codex_program: Resolved<PathBuf>,
    pub agy_enabled: Resolved<bool>,
    pub agy_program: Resolved<PathBuf>,
    pub timer_logs: Resolved<Option<PathBuf>>,
}

pub fn folder_variables() -> Vec<&'static str> {
    let mut names = vec!["HOME"];
    for key in [&CONFIG, &ROOT, &STATE, &CACHE] {
        let own = key.about.variable.into_iter();
        let read = match &key.about.default {
            Default::Folder(folder) => Some(folder.xdg),
            _ => None,
        };
        for name in own.chain(read) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}
#[derive(Debug)]
pub struct Settings {
    pub config: Resolved<PathBuf>,
    pub root: Resolved<PathBuf>,
    pub state: Resolved<PathBuf>,
    pub cache: Resolved<PathBuf>,
    pub temp: Resolved<PathBuf>,
    pub store: keys::Section,
    pub follow: Resolved<Vec<Follow>>,
    pub remember_editor: Resolved<OsString>,
    pub read_budget: Resolved<read::Budget>,
    pub read_outline_width: Resolved<usize>,
    pub read_matches_listed: Resolved<usize>,
    pub tag_cap: Resolved<usize>,
    pub search: bilbo_search::keys::Section,
    pub recall: bilbo_search::recall::keys::Section,
    pub digest: Digest,
    pub digest_search: bilbo_search::digest::keys::Section,
    pub llm: bilbo_llm::keys::Section,
    pub derive: bilbo_derive::keys::Section,
    pub tags: bilbo_derive::tags::keys::Section,
    pub summary: bilbo_derive::summary::keys::Section,
    pub setup: Setup,
    pub setup_timer: bilbo_setup::keys::Section,
    pub kind: Vec<(&'static KindKey, Resolved<usize>)>,
    pub group: Vec<&'static Group>,
    shown: Vec<Shown>,
    file: File,
}

impl Settings {
    pub fn layout(&self) -> Layout {
        Layout {
            root: self.root.value.clone(),
            state: self.state.value.clone(),
            cache: self.cache.value.clone(),
        }
    }

    pub fn kind_value(&self, key: &KindKey) -> Option<usize> {
        self.kind
            .iter()
            .find(|(declared, _)| declared.about.name == key.about.name)
            .map(|(_, resolved)| resolved.value)
    }

    pub fn source<'a>(&'a self, var: &'a dyn Fn(&str) -> Option<OsString>) -> Source<'a> {
        Source {
            var,
            file: &self.file,
        }
    }

    pub fn shown(&self) -> Vec<Shown> {
        let mut shown = vec![
            ROOT.shown(&self.root),
            STATE.shown(&self.state),
            CACHE.shown(&self.cache),
            TEMP.shown(&self.temp),
        ];
        shown.extend(self.store.shown());
        shown.push(FOLLOW.shown(&self.follow));
        shown.push(EDITOR.shown(&self.remember_editor));
        shown.push(read::BUDGET.shown(&self.read_budget));
        shown.push(read::OUTLINE_WIDTH.shown(&self.read_outline_width));
        shown.push(read::MATCHES_LISTED.shown(&self.read_matches_listed));
        shown.push(tag::CAP.shown(&self.tag_cap));
        shown.extend(self.search.shown());
        shown.extend(self.recall.shown());
        shown.extend([
            DIGEST_MARKDOWN_MOST.shown(&self.digest.markdown_most),
            DIGEST_MARKDOWN_FLOOR.shown(&self.digest.markdown_floor),
            DIGEST_SESSION_MOST.shown(&self.digest.session_most),
            DIGEST_SESSION_FLOOR.shown(&self.digest.session_floor),
            DIGEST_SESSION_OLD_AFTER.shown(&self.digest.session_old_after),
            DIGEST_SESSION_SUBAGENTS.shown(&self.digest.session_subagents),
            DIGEST_SESSION_PARTS.shown(&self.digest.session_parts),
            DIGEST_NOTE_MOST.shown(&self.digest.note_most),
            DIGEST_NOTE_FLOOR.shown(&self.digest.note_floor),
            DIGEST_NOTE_WIDTH.shown(&self.digest.note_width),
            DIGEST_SUMMARY_ENABLED.shown(&self.digest.summary_enabled),
        ]);
        shown.extend(self.digest_search.shown());
        shown.push(DIGEST_WAIT.shown(&self.digest.wait));
        shown.push(DIGEST_INPUT.shown(&self.digest.input));
        shown.push(DIGEST_AGY_TAIL.shown(&self.digest.agy_tail));
        shown.push(DIGEST_LOG_ENABLED.shown(&self.digest.log_enabled));
        shown.extend(self.llm.shown());
        shown.extend(self.shown.iter().cloned());
        shown.extend(self.derive.shown());
        shown.extend(self.tags.shown());
        shown.extend(self.summary.shown());
        shown.extend([
            SETUP_PLUGIN.shown(&self.setup.plugin),
            SETUP_NOTES.shown(&self.setup.notes),
            SETUP_CLAUDE_ENABLED.shown(&self.setup.claude_enabled),
            SETUP_CLAUDE_PROGRAM.shown(&self.setup.claude_program),
            SETUP_CODEX_ENABLED.shown(&self.setup.codex_enabled),
            SETUP_CODEX_PROGRAM.shown(&self.setup.codex_program),
            SETUP_AGY_ENABLED.shown(&self.setup.agy_enabled),
            SETUP_AGY_PROGRAM.shown(&self.setup.agy_program),
            SETUP_TIMER_LOGS.shown(&self.setup.timer_logs),
        ]);
        shown.extend(self.setup_timer.shown());
        shown.extend(self.kind.iter().map(|(key, resolved)| key.shown(resolved)));
        shown
    }
}

pub fn config_path(var: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    let none = File::default();
    Ok(CONFIG.resolve(&Source { var, file: &none })?.value)
}

pub fn allowed_value(
    name: &str,
    text: &str,
    home: Option<&OsString>,
    kind: &[&'static KindKey],
    groups: &[&'static Group],
) -> Result<(), String> {
    const PATH: &str = "/allowed.toml";
    let var = |asked: &str| match asked {
        "HOME" => home.cloned(),
        "BILBO_CONFIG" => Some(OsString::from(PATH)),
        _ => None,
    };
    let held = format!("{name} = {text}\n").into_bytes();
    load(
        &var,
        &|path| {
            if path == Path::new(PATH) {
                Ok(held.clone())
            } else {
                Err(io::ErrorKind::NotFound.into())
            }
        },
        kind,
        groups,
    )
    .map(|_| ())
    .map_err(|error| {
        let said = error.to_string();
        said.strip_prefix(PATH)
            .and_then(|rest| rest.strip_prefix(':'))
            .map(|rest| rest.trim_start_matches(|c: char| c.is_ascii_digit()))
            .and_then(|rest| rest.strip_prefix(": "))
            .map_or(said.clone(), str::to_owned)
    })
}

pub fn load(
    var: &dyn Fn(&str) -> Option<OsString>,
    read: &dyn Fn(&Path) -> io::Result<Vec<u8>>,
    kind: &[&'static KindKey],
    groups: &[&'static Group],
) -> Result<Settings, Error> {
    let none = File::default();
    let config = CONFIG.resolve(&Source { var, file: &none })?;
    let named = match config.origin {
        Origin::Variable(name) => Named::ByVariable(name),
        _ => Named::ByDefault,
    };
    let file = File::load(&config.value, named, read(&config.value))?;
    let names: Vec<&'static str> = keys(kind, groups).iter().map(|about| about.name).collect();
    file.check(&names)?;
    let source = Source { var, file: &file };
    Ok(Settings {
        root: ROOT.resolve(&source)?,
        state: STATE.resolve(&source)?,
        cache: CACHE.resolve(&source)?,
        temp: TEMP.resolve(&source)?,
        store: keys::section(&source)?,
        follow: FOLLOW.resolve(&source)?,
        remember_editor: EDITOR.resolve(&source)?,
        read_budget: read::BUDGET.resolve(&source)?,
        read_outline_width: read::OUTLINE_WIDTH.resolve(&source)?,
        read_matches_listed: read::MATCHES_LISTED.resolve(&source)?,
        tag_cap: tag::CAP.resolve(&source)?,
        search: bilbo_search::keys::section(&source)?,
        recall: bilbo_search::recall::keys::section(&source)?,
        digest: Digest {
            markdown_most: DIGEST_MARKDOWN_MOST.resolve(&source)?,
            markdown_floor: DIGEST_MARKDOWN_FLOOR.resolve(&source)?,
            session_most: DIGEST_SESSION_MOST.resolve(&source)?,
            session_floor: DIGEST_SESSION_FLOOR.resolve(&source)?,
            session_old_after: DIGEST_SESSION_OLD_AFTER.resolve(&source)?,
            session_subagents: DIGEST_SESSION_SUBAGENTS.resolve(&source)?,
            session_parts: DIGEST_SESSION_PARTS.resolve(&source)?,
            note_most: DIGEST_NOTE_MOST.resolve(&source)?,
            note_floor: DIGEST_NOTE_FLOOR.resolve(&source)?,
            note_width: DIGEST_NOTE_WIDTH.resolve(&source)?,
            summary_enabled: DIGEST_SUMMARY_ENABLED.resolve(&source)?,
            wait: DIGEST_WAIT.resolve(&source)?,
            input: DIGEST_INPUT.resolve(&source)?,
            agy_tail: DIGEST_AGY_TAIL.resolve(&source)?,
            log_enabled: DIGEST_LOG_ENABLED.resolve(&source)?,
        },
        digest_search: bilbo_search::digest::keys::section(&source)?,
        llm: bilbo_llm::keys::section(&source)?,
        derive: bilbo_derive::keys::section(&source)?,
        tags: bilbo_derive::tags::keys::section(&source)?,
        summary: bilbo_derive::summary::keys::section(&source)?,
        setup: Setup {
            plugin: SETUP_PLUGIN.resolve(&source)?,
            notes: SETUP_NOTES.resolve(&source)?,
            claude_enabled: SETUP_CLAUDE_ENABLED.resolve(&source)?,
            claude_program: SETUP_CLAUDE_PROGRAM.resolve(&source)?,
            codex_enabled: SETUP_CODEX_ENABLED.resolve(&source)?,
            codex_program: SETUP_CODEX_PROGRAM.resolve(&source)?,
            agy_enabled: SETUP_AGY_ENABLED.resolve(&source)?,
            agy_program: SETUP_AGY_PROGRAM.resolve(&source)?,
            timer_logs: SETUP_TIMER_LOGS.resolve(&source)?,
        },
        setup_timer: bilbo_setup::keys::section(&source)?,
        kind: kind
            .iter()
            .map(|key| Ok((*key, key.resolve(&source)?)))
            .collect::<Result<_, Error>>()?,
        group: groups.to_vec(),
        shown: groups
            .iter()
            .map(|group| (group.resolve)(&source))
            .collect::<Result<Vec<_>, Error>>()?
            .into_iter()
            .flatten()
            .collect(),
        config,
        file,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};

    use std::os::unix::ffi::OsStringExt;

    use bilbo_settings::error::{At, Error};
    use bilbo_settings::key::{About, Default, Key, Origin, Shown, Source};

    use bilbo_entries::follows::Follow;
    use bilbo_search::analyze::Language;

    use super::{CONFIG, FOLLOW, Group, ROOT, STATE, keys, load, show_follows};

    const WORD: Key<String> = Key {
        about: About {
            name: "test.word",
            variable: Some("BILBO_TEST_WORD"),
            default: Default::Toml(r#""""#),
            allowed: "a word",
            doc: "a text key",
        },
        parse: |value| {
            value
                .text()
                .map(str::to_owned)
                .ok_or_else(|| value.refuse())
        },
        show: |text| bilbo_settings::key::quote(text),
    };

    const TABLE: Key<Vec<(String, String)>> = Key {
        about: About {
            name: "test.table",
            variable: None,
            default: Default::Toml("{}"),
            allowed: "a table of words",
            doc: "a table key",
        },
        parse: |value| {
            let fields = value.fields().ok_or_else(|| value.refuse())?;
            fields
                .into_iter()
                .map(|(name, field)| {
                    let text = field.text().ok_or_else(|| field.refuse())?;
                    Ok((name.to_owned(), text.to_owned()))
                })
                .collect()
        },
        show: |pairs| format!("{pairs:?}"),
    };

    static GROUP: Group = Group {
        about: &[&WORD.about, &TABLE.about],
        resolve: |source| {
            Ok(vec![
                WORD.shown(&WORD.resolve(source)?),
                TABLE.shown(&TABLE.resolve(source)?),
            ])
        },
    };

    fn grouped(
        env: &[(&str, &str)],
        text: &str,
        groups: &[&'static Group],
    ) -> Result<super::Settings, Error> {
        let env: HashMap<String, OsString> = env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
            .collect();
        let read = |_: &Path| Ok(text.as_bytes().to_vec());
        load(
            &|name| {
                env.get(name)
                    .cloned()
                    .or_else(|| (name == "HOME").then(|| OsString::from("/h")))
            },
            &read,
            &kind(),
            groups,
        )
    }

    fn kind() -> Vec<&'static super::KindKey> {
        crate::kind_keys()
    }

    const PAGE: &str = include_str!("../../../../docs/reference/configuration.md");

    fn settings(env: &[(&str, &str)], files: &[(&str, &str)]) -> Result<super::Settings, String> {
        let env: HashMap<String, OsString> = env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
            .collect();
        let files: HashMap<PathBuf, Vec<u8>> = files
            .iter()
            .map(|(path, text)| (PathBuf::from(path), text.as_bytes().to_vec()))
            .collect();
        let read = |path: &Path| {
            files
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        };
        load(
            &|name| env.get(name).cloned(),
            &read,
            &kind(),
            &crate::provider_keys(),
        )
        .map_err(|error| error.to_string())
    }

    #[test]
    fn the_shown_keys_are_the_keys_in_their_order() {
        let shown: Vec<&str> = settings(&[("HOME", "/h")], &[])
            .expect("loads")
            .shown()
            .iter()
            .map(|row| row.name)
            .collect();
        let registered: Vec<&str> = keys(&kind(), &crate::provider_keys())
            .iter()
            .map(|about| about.name)
            .collect();
        assert_eq!(shown, registered);
    }

    #[test]
    fn the_registry_is_the_folders_the_store_the_follows_then_the_read_tag_search_recall_digest_llm_derive_and_kinds_keys()
     {
        let names: Vec<&str> = keys(&kind(), &crate::provider_keys())
            .iter()
            .map(|about| about.name)
            .collect();
        assert_eq!(
            names,
            [
                "paths.root",
                "paths.state",
                "paths.cache",
                "paths.temp",
                "store.seal_at",
                "store.seal_after",
                "store.compression",
                "store.tear_budget",
                "store.wait",
                "store.poll",
                "follow",
                "remember.editor",
                "read.budget",
                "read.outline_width",
                "read.matches_listed",
                "tag.cap",
                "search.chunk_tokens",
                "search.chunk_overlap",
                "search.workers",
                "search.index_memory",
                "search.merge_deleted_percent",
                "search.entry_tries",
                "search.label_batch",
                "search.list_limit",
                "recall.budget",
                "recall.notes_budget",
                "recall.candidates",
                "recall.cap",
                "recall.record_cap",
                "recall.copy_share",
                "recall.copy_words",
                "recall.question_words",
                "recall.languages",
                "recall.bm25_k1",
                "recall.bm25_b",
                "recall.read_batch",
                "recall.typo_terms",
                "recall.typo_letters",
                "recall.typo_max_letters",
                "recall.typo_two_edits",
                "recall.pair_weight",
                "digest.markdown.most",
                "digest.markdown.floor",
                "digest.session.most",
                "digest.session.floor",
                "digest.session.old_after",
                "digest.session.subagents",
                "digest.session.parts",
                "digest.note.most",
                "digest.note.floor",
                "digest.note.width",
                "digest.summary.enabled",
                "digest.candidates",
                "digest.query",
                "digest.wait",
                "digest.input",
                "digest.agy.tail",
                "digest.log.enabled",
                "llm.provider",
                "llm.response",
                "llm.timeout",
                "llm.poll",
                "llm.anthropic.model",
                "llm.anthropic.via",
                "llm.anthropic.api.endpoint",
                "llm.anthropic.api.version",
                "llm.anthropic.api.key_env",
                "llm.anthropic.api.key_command",
                "llm.anthropic.api.headers",
                "llm.anthropic.api.extra",
                "llm.anthropic.tool.program",
                "llm.anthropic.tool.effort",
                "llm.anthropic.tool.arguments",
                "llm.anthropic.tool.inherit",
                "llm.anthropic.tool.environment",
                "llm.anthropic.tool.project_files",
                "llm.openai.model",
                "llm.openai.via",
                "llm.openai.api.endpoint",
                "llm.openai.api.key_env",
                "llm.openai.api.key_command",
                "llm.openai.api.headers",
                "llm.openai.api.extra",
                "llm.openai.tool.program",
                "llm.openai.tool.effort",
                "llm.openai.tool.arguments",
                "llm.openai.tool.inherit",
                "llm.openai.tool.environment",
                "llm.openai.tool.home",
                "llm.openai.tool.system_files",
                "derive.enabled",
                "derive.in",
                "derive.not_in",
                "derive.instructions",
                "derive.quiet",
                "derive.window",
                "derive.passage",
                "derive.passage_head",
                "derive.earlier",
                "derive.names",
                "derive.answer_tokens",
                "derive.calls",
                "derive.retries",
                "derive.retry_wait",
                "derive.retry_factor",
                "derive.retry_limit",
                "derive.tries",
                "derive.failures",
                "derive.tags.enabled",
                "derive.tags.pick",
                "derive.tags.instruction",
                "derive.tags.model",
                "derive.tags.most",
                "derive.tags.notes",
                "derive.summary.enabled",
                "derive.summary.instruction",
                "derive.summary.most",
                "derive.summary.window",
                "derive.summary.least",
                "setup.plugin",
                "setup.notes",
                "setup.claude.enabled",
                "setup.claude.program",
                "setup.codex.enabled",
                "setup.codex.program",
                "setup.agy.enabled",
                "setup.agy.program",
                "setup.timer.logs",
                "setup.timer.path",
                "setup.refresh.every",
                "setup.derive.every",
                "setup.wait",
                "setup.poll",
                "markdown.heading_width",
                "session.unpack_floor",
                "session.unpack_ratio",
                "session.unpack_window",
                "session.wide_fields"
            ]
        );
    }

    #[test]
    fn the_config_file_is_bilbo_config_else_under_xdg_else_under_home() {
        let file = "[store]\ncompression = 3\n";
        let level = |env: &[(&str, &str)], path: &str| {
            settings(env, &[(path, file)]).map(|s| s.store.compression.value.level())
        };
        assert_eq!(
            level(
                &[("HOME", "/h"), ("BILBO_CONFIG", "/c/b.toml")],
                "/c/b.toml"
            ),
            Ok(3)
        );
        assert_eq!(
            level(
                &[("HOME", "/h"), ("XDG_CONFIG_HOME", "/x")],
                "/x/bilbo/config.toml"
            ),
            Ok(3)
        );
        assert_eq!(
            level(&[("HOME", "/h")], "/h/.config/bilbo/config.toml"),
            Ok(3)
        );
        assert_eq!(
            level(&[("HOME", "/h")], "/h/.config/bilbo/config"),
            Ok(bilbo_store::keys::defaults().compression.value.level()),
            "0.20's file name is never read"
        );
    }

    #[test]
    fn a_missing_file_is_the_defaults_unless_bilbo_config_names_it() {
        let defaults = settings(&[("HOME", "/h")], &[]).expect("defaults");
        assert_eq!(
            defaults.root.value,
            PathBuf::from("/h/.local/share/bilbo/store")
        );
        assert_eq!(defaults.root.origin, Origin::Default);
        assert_eq!(
            settings(&[("HOME", "/h"), ("BILBO_CONFIG", "/c/none.toml")], &[]).err(),
            Some("BILBO_CONFIG names a file that does not exist: /c/none.toml".to_owned())
        );
        assert_eq!(
            settings(&[("HOME", "/h"), ("BILBO_CONFIG", "c.toml")], &[]).err(),
            Some("BILBO_CONFIG is not an absolute path: c.toml".to_owned())
        );
    }

    #[test]
    fn an_unknown_key_is_refused_before_any_value_is_read() {
        assert_eq!(
            settings(
                &[("HOME", "/h"), ("BILBO_CONFIG", "/c.toml")],
                &[("/c.toml", "[store]\nseal_at = \"0\"\nsealat = 1\n")]
            )
            .err(),
            Some(
                "/c.toml:3: unknown key store.sealat; the keys are paths.root, paths.state, \
                 paths.cache, paths.temp, store.seal_at, store.seal_after, store.compression, store.tear_budget, store.wait, store.poll, follow, remember.editor, read.budget, read.outline_width, read.matches_listed, tag.cap, search.chunk_tokens, search.chunk_overlap, search.workers, search.index_memory, search.merge_deleted_percent, search.entry_tries, search.label_batch, search.list_limit, recall.budget, recall.notes_budget, recall.candidates, recall.cap, recall.record_cap, recall.copy_share, recall.copy_words, recall.question_words, recall.languages, recall.bm25_k1, recall.bm25_b, recall.read_batch, recall.typo_terms, recall.typo_letters, recall.typo_max_letters, recall.typo_two_edits, recall.pair_weight, digest.markdown.most, digest.markdown.floor, digest.session.most, digest.session.floor, digest.session.old_after, digest.session.subagents, digest.session.parts, digest.note.most, digest.note.floor, digest.note.width, digest.summary.enabled, digest.candidates, digest.query, digest.wait, digest.input, digest.agy.tail, digest.log.enabled, llm.provider, llm.response, llm.timeout, llm.poll, llm.anthropic.model, llm.anthropic.via, llm.anthropic.api.endpoint, llm.anthropic.api.version, llm.anthropic.api.key_env, llm.anthropic.api.key_command, llm.anthropic.api.headers, llm.anthropic.api.extra, llm.anthropic.tool.program, llm.anthropic.tool.effort, llm.anthropic.tool.arguments, llm.anthropic.tool.inherit, llm.anthropic.tool.environment, llm.anthropic.tool.project_files, llm.openai.model, llm.openai.via, llm.openai.api.endpoint, llm.openai.api.key_env, llm.openai.api.key_command, llm.openai.api.headers, llm.openai.api.extra, llm.openai.tool.program, llm.openai.tool.effort, llm.openai.tool.arguments, llm.openai.tool.inherit, llm.openai.tool.environment, llm.openai.tool.home, llm.openai.tool.system_files, derive.enabled, derive.in, derive.not_in, derive.instructions, derive.quiet, derive.window, derive.passage, derive.passage_head, derive.earlier, derive.names, derive.answer_tokens, derive.calls, derive.retries, derive.retry_wait, derive.retry_factor, derive.retry_limit, derive.tries, derive.failures, derive.tags.enabled, derive.tags.pick, derive.tags.instruction, derive.tags.model, derive.tags.most, derive.tags.notes, derive.summary.enabled, derive.summary.instruction, derive.summary.most, derive.summary.window, derive.summary.least, setup.plugin, setup.notes, setup.claude.enabled, setup.claude.program, setup.codex.enabled, setup.codex.program, setup.agy.enabled, setup.agy.program, setup.timer.logs, setup.timer.path, setup.refresh.every, setup.derive.every, setup.wait, setup.poll, markdown.heading_width, session.unpack_floor, session.unpack_ratio, session.unpack_window, session.wide_fields"
                    .to_owned()
            )
        );
    }

    #[test]
    fn a_key_of_the_old_names_is_an_unknown_key() {
        for (file, line, name) in [
            (
                "[derive.claude]\nprogram = \"x\"\n",
                2,
                "derive.claude.program",
            ),
            ("[derive.claude]\n", 1, "derive.claude"),
            (
                "derive.anthropic.endpoint = \"https://e.test\"\n",
                1,
                "derive.anthropic.endpoint",
            ),
            (
                "[derive.anthropic.headers]\nX-A = \"1\"\n",
                2,
                "derive.anthropic.headers.X-A",
            ),
            (
                "[derive.openai]\nkey_env = \"K\"\n",
                2,
                "derive.openai.key_env",
            ),
            ("derive.provider = \"anthropic\"\n", 1, "derive.provider"),
            ("derive.timeout = \"1m\"\n", 1, "derive.timeout"),
            ("derive.response = \"2 KiB\"\n", 1, "derive.response"),
            ("derive.poll = \"10ms\"\n", 1, "derive.poll"),
            (
                "[derive.anthropic]\nmodel = \"m\"\n",
                2,
                "derive.anthropic.model",
            ),
            ("[derive.openai]\nmodel = \"m\"\n", 2, "derive.openai.model"),
        ] {
            let said = settings(
                &[("HOME", "/h"), ("BILBO_CONFIG", "/c.toml")],
                &[("/c.toml", file)],
            )
            .expect_err("refused");
            let start = format!("/c.toml:{line}: unknown key {name}; the keys are paths.root, ");
            assert!(said.starts_with(&start), "{said}");
        }
    }

    #[test]
    fn no_old_variable_of_a_moved_key_is_read() {
        let alone = settings(&[("HOME", "/h")], &[]).expect("loads").shown();
        let old = [
            ("BILBO_DERIVE_PROVIDER", "anthropic"),
            ("BILBO_DERIVE_TIMEOUT", "1s"),
            ("BILBO_DERIVE_RESPONSE", "2 KiB"),
            ("BILBO_DERIVE_POLL", "10ms"),
            ("BILBO_DERIVE_ANTHROPIC_MODEL", "x"),
            ("BILBO_DERIVE_ANTHROPIC_VIA", "x"),
            ("BILBO_DERIVE_ANTHROPIC_API_ENDPOINT", "x"),
            ("BILBO_DERIVE_ANTHROPIC_API_VERSION", "x"),
            ("BILBO_DERIVE_ANTHROPIC_API_KEY_ENV", "x"),
            ("BILBO_DERIVE_ANTHROPIC_TOOL_PROGRAM", "x"),
            ("BILBO_DERIVE_ANTHROPIC_TOOL_EFFORT", "x"),
            ("BILBO_DERIVE_OPENAI_MODEL", "x"),
            ("BILBO_DERIVE_OPENAI_VIA", "x"),
            ("BILBO_DERIVE_OPENAI_API_ENDPOINT", "x"),
            ("BILBO_DERIVE_OPENAI_API_KEY_ENV", "x"),
            ("BILBO_DERIVE_OPENAI_TOOL_PROGRAM", "x"),
            ("BILBO_DERIVE_OPENAI_TOOL_EFFORT", "x"),
            ("BILBO_DERIVE_OPENAI_TOOL_HOME", "x"),
        ];
        for (name, value) in old {
            let found = settings(&[("HOME", "/h"), (name, value)], &[])
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(found.shown(), alone, "{name}");
        }
    }

    #[test]
    fn the_folders_come_from_bilbo_home_the_file_and_xdg_in_that_order() {
        let at = |line| Origin::File {
            path: PathBuf::from("/c.toml"),
            line,
        };
        let xdg = [
            ("HOME", "/h"),
            ("BILBO_CONFIG", "/c.toml"),
            ("XDG_DATA_HOME", "/xd"),
            ("XDG_STATE_HOME", "/xs"),
            ("XDG_CACHE_HOME", "/xc"),
        ];

        let file = "[paths]\nroot = \"~/r\"\nstate = \"/s\"\ncache = \"/c\"\n";
        let found = settings(&xdg, &[("/c.toml", file)]).expect("valid");
        assert_eq!(found.root.value, PathBuf::from("/h/r"));
        assert_eq!(found.root.origin, at(2));
        assert_eq!(found.state.value, PathBuf::from("/s"));
        assert_eq!(found.state.origin, at(3));
        assert_eq!(found.cache.value, PathBuf::from("/c"));
        assert_eq!(found.cache.origin, at(4));
        assert_eq!(found.layout().cache, PathBuf::from("/c"));

        let silent = settings(&xdg, &[("/c.toml", "")]).expect("valid");
        assert_eq!(silent.root.value, PathBuf::from("/xd/bilbo/store"));
        assert_eq!(silent.state.value, PathBuf::from("/xs/bilbo"));
        assert_eq!(silent.cache.value, PathBuf::from("/xc/bilbo"));
        for folder in [&silent.root, &silent.state, &silent.cache] {
            assert_eq!(folder.origin, Origin::Default);
        }

        let mut env = xdg.to_vec();
        env.push(("BILBO_HOME", "/b"));
        let found = settings(&env, &[("/c.toml", file)]).expect("valid");
        assert_eq!(found.root.value, PathBuf::from("/b"));
        assert_eq!(found.root.origin, Origin::Variable("BILBO_HOME"));
        assert_eq!(found.state.origin, at(3));
        assert_eq!(found.cache.origin, at(4));
    }

    fn keys_table() -> Vec<Vec<String>> {
        let section = PAGE
            .split("\n## Keys\n")
            .nth(1)
            .and_then(|rest| rest.split("\n## ").next())
            .unwrap_or_default();
        section
            .lines()
            .filter(|line| line.starts_with("| `"))
            .map(|line| {
                line.trim_matches('|')
                    .split(" | ")
                    .take(4)
                    .map(|cell| cell.trim().to_owned())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_reference_page_lists_every_key_with_its_variable_and_default() {
        let expected: Vec<Vec<String>> = keys(&kind(), &crate::provider_keys())
            .iter()
            .map(|about| {
                vec![
                    format!("`{}`", about.name),
                    about
                        .variable
                        .map_or_else(|| "none".to_owned(), |name| format!("`{name}`")),
                    format!("`{}`", about.default.describe()),
                ]
            })
            .collect();
        let found: Vec<Vec<String>> = keys_table()
            .into_iter()
            .map(|row| row.into_iter().take(3).collect())
            .collect();
        assert_eq!(found, expected);
        let names: Vec<&str> = keys(&kind(), &crate::provider_keys())
            .iter()
            .map(|about| about.name)
            .collect();
        let example = format!(
            "bilbo: /home/me/.config/bilbo/config.toml:9: unknown key store.sealat; the keys are {}",
            names.join(", ")
        );
        assert!(
            PAGE.lines().any(|line| line == example),
            "the page's unknown key example"
        );
    }

    #[test]
    fn the_reference_page_means_what_each_key_declares() {
        let meanings: Vec<String> = keys_table()
            .into_iter()
            .map(|row| row[3].replace('`', ""))
            .collect();
        assert_eq!(meanings.len(), keys(&kind(), &crate::provider_keys()).len());
        for (about, meaning) in keys(&kind(), &crate::provider_keys()).iter().zip(&meanings) {
            assert!(
                meaning.starts_with(about.doc),
                "{}: the Meaning cell must start with {:?}, got {meaning:?}",
                about.name,
                about.doc
            );
            assert!(
                meaning.contains(about.allowed),
                "{}: the Meaning cell must contain {:?}, got {meaning:?}",
                about.name,
                about.allowed
            );
        }
    }

    #[test]
    fn the_reference_page_holds_the_sentence_about_when_the_scope_is_read_exactly_twice() {
        let sentence = "`derive.in` and `derive.not_in` are read when a `derive` run plans. A tag you add or remove while a run is under way counts from the next run: to keep an entry from a provider, tag it before the run that would send it.";
        assert_eq!(PAGE.matches(sentence).count(), 2);
    }

    #[test]
    fn the_reference_page_lists_every_language() {
        let page: Vec<&str> = PAGE.lines().collect();
        let mut headings = (0..page.len()).filter(|&at| page[at] == "## Languages");
        let heading = headings
            .next()
            .expect("docs/reference/configuration.md has no \"## Languages\" section");
        assert!(
            headings.next().is_none(),
            "docs/reference/configuration.md holds more than one \"## Languages\" section"
        );
        let lines: Vec<&str> = page[heading + 1..]
            .iter()
            .copied()
            .take_while(|line| !line.starts_with("## "))
            .collect();
        let headers: Vec<usize> = (0..lines.len())
            .filter(|&at| lines[at].trim() == "| Code | Language |")
            .collect();
        assert_eq!(
            headers.len(),
            1,
            "the Languages section of docs/reference/configuration.md must hold exactly one table headed \"| Code | Language |\""
        );
        assert_eq!(
            lines.get(headers[0] + 1).map(|line| line.trim()),
            Some("| --- | --- |"),
            "the header of the Languages table must be followed by \"| --- | --- |\""
        );
        let found: Vec<Vec<String>> = lines[headers[0] + 2..]
            .iter()
            .take_while(|line| line.starts_with('|'))
            .map(|line| {
                let row = line.trim();
                let row = row.strip_prefix('|').unwrap_or(row);
                row.strip_suffix('|')
                    .unwrap_or(row)
                    .split('|')
                    .map(|cell| cell.trim().to_owned())
                    .collect()
            })
            .collect();
        let expected: Vec<Vec<String>> = Language::all()
            .map(|language| vec![format!("`{}`", language.code()), language.name().to_owned()])
            .collect();
        assert_eq!(
            found, expected,
            "the Languages table must list every code recall.languages takes, in code order"
        );
    }

    fn names_in(text: &str) -> BTreeSet<String> {
        text.split(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .filter(|word| {
                (word.starts_with("BILBO_") || word.starts_with("XDG_"))
                    && !matches!(*word, "BILBO_" | "XDG_")
            })
            .map(str::to_owned)
            .collect()
    }

    fn read_names() -> BTreeSet<String> {
        keys(&kind(), &crate::provider_keys())
            .iter()
            .flat_map(|about| about.default.reads())
            .chain(CONFIG.about.default.reads())
            .map(str::to_owned)
            .collect()
    }

    fn expected_names() -> BTreeSet<String> {
        let all = keys(&kind(), &crate::provider_keys());
        all.iter()
            .filter_map(|about| about.variable)
            .chain(CONFIG.about.variable)
            .map(str::to_owned)
            .chain(read_names())
            .collect()
    }

    fn is_prefixed(name: &str) -> bool {
        name.starts_with("BILBO_") || name.starts_with("XDG_")
    }

    fn missing_and_foreign(page: &str) -> (Vec<String>, Vec<String>) {
        let registry: BTreeSet<String> = expected_names()
            .into_iter()
            .filter(|name| is_prefixed(name))
            .collect();
        let found = names_in(page);
        (
            registry.difference(&found).cloned().collect(),
            found.difference(&registry).cloned().collect(),
        )
    }

    fn without_row(page: &str, first_cell: &str) -> String {
        let table = settings_table(page);
        let row = table
            .lines()
            .find(|line| line.starts_with(first_cell))
            .expect("a row of the settings table");
        page.replace(&format!("{row}\n"), "")
    }

    fn settings_table(page: &str) -> &str {
        page.split("\n### Settings\n")
            .nth(1)
            .and_then(|rest| rest.split("\n### ").next())
            .unwrap_or_default()
    }

    fn code_names(cell: &str) -> Vec<String> {
        cell.split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }

    fn read_names_lacking_in_the_first_cells(page: &str) -> Vec<String> {
        let rows: Vec<Vec<String>> = settings_table(page)
            .lines()
            .filter(|line| line.starts_with("| `"))
            .map(|line| {
                code_names(
                    line.trim_matches('|')
                        .split(" | ")
                        .next()
                        .unwrap_or_default(),
                )
            })
            .collect();
        read_names()
            .into_iter()
            .filter(|name| !rows.iter().any(|row| row.contains(name)))
            .collect()
    }

    #[test]
    fn the_reference_page_names_exactly_the_variables_bilbo_reads() {
        let expected = expected_names();
        let all = keys(&kind(), &crate::provider_keys());
        let table = settings_table(PAGE);
        let code = code_names;
        let mut set = Vec::new();
        for line in table.lines().filter(|line| line.starts_with("| `")) {
            let cells: Vec<&str> = line.trim_matches('|').split(" | ").collect();
            let effect = cells[1].trim();
            if !(effect.starts_with("Set ") || effect.starts_with("Sets ")) {
                continue;
            }
            let variables = code(cells[0]);
            let effect = if variables
                .iter()
                .any(|name| name.starts_with("BILBO_RECALL_"))
            {
                effect
                    .strip_suffix(
                        " `recall.question_words` and `recall.languages` have no variable.",
                    )
                    .unwrap_or(effect)
            } else {
                effect
            };
            let keys_named = code(effect);
            assert_eq!(variables.len(), keys_named.len(), "{line}");
            for (variable, key) in variables.iter().zip(&keys_named) {
                let about = all
                    .iter()
                    .find(|about| about.name == key)
                    .unwrap_or_else(|| panic!("{key} is no key"));
                assert_eq!(about.variable, Some(variable.as_str()), "{key}");
                set.push(key.clone());
            }
        }
        for about in &all {
            if about.variable.is_some() {
                assert!(set.iter().any(|key| key == about.name), "{}", about.name);
            }
        }

        assert_eq!(
            expected.iter().map(String::as_str).collect::<Vec<_>>(),
            [
                "BILBO_CONFIG",
                "BILBO_DERIVE_ANSWER_TOKENS",
                "BILBO_DERIVE_CALLS",
                "BILBO_DERIVE_EARLIER",
                "BILBO_DERIVE_ENABLED",
                "BILBO_DERIVE_FAILURES",
                "BILBO_DERIVE_NAMES",
                "BILBO_DERIVE_PASSAGE",
                "BILBO_DERIVE_PASSAGE_HEAD",
                "BILBO_DERIVE_QUIET",
                "BILBO_DERIVE_RETRIES",
                "BILBO_DERIVE_RETRY_FACTOR",
                "BILBO_DERIVE_RETRY_LIMIT",
                "BILBO_DERIVE_RETRY_WAIT",
                "BILBO_DERIVE_SUMMARY_ENABLED",
                "BILBO_DERIVE_SUMMARY_INSTRUCTION",
                "BILBO_DERIVE_SUMMARY_LEAST",
                "BILBO_DERIVE_SUMMARY_MOST",
                "BILBO_DERIVE_SUMMARY_WINDOW",
                "BILBO_DERIVE_TAGS_ENABLED",
                "BILBO_DERIVE_TAGS_INSTRUCTION",
                "BILBO_DERIVE_TAGS_MODEL",
                "BILBO_DERIVE_TAGS_MOST",
                "BILBO_DERIVE_TAGS_NOTES",
                "BILBO_DERIVE_TRIES",
                "BILBO_DERIVE_WINDOW",
                "BILBO_DIGEST_AGY_TAIL",
                "BILBO_DIGEST_CANDIDATES",
                "BILBO_DIGEST_INPUT",
                "BILBO_DIGEST_LOG_ENABLED",
                "BILBO_DIGEST_MARKDOWN_FLOOR",
                "BILBO_DIGEST_MARKDOWN_MOST",
                "BILBO_DIGEST_NOTE_FLOOR",
                "BILBO_DIGEST_NOTE_MOST",
                "BILBO_DIGEST_NOTE_WIDTH",
                "BILBO_DIGEST_QUERY",
                "BILBO_DIGEST_SESSION_FLOOR",
                "BILBO_DIGEST_SESSION_MOST",
                "BILBO_DIGEST_SESSION_OLD_AFTER",
                "BILBO_DIGEST_SESSION_PARTS",
                "BILBO_DIGEST_SESSION_SUBAGENTS",
                "BILBO_DIGEST_SUMMARY_ENABLED",
                "BILBO_DIGEST_WAIT",
                "BILBO_HOME",
                "BILBO_LLM_ANTHROPIC_API_ENDPOINT",
                "BILBO_LLM_ANTHROPIC_API_KEY_ENV",
                "BILBO_LLM_ANTHROPIC_API_VERSION",
                "BILBO_LLM_ANTHROPIC_MODEL",
                "BILBO_LLM_ANTHROPIC_TOOL_EFFORT",
                "BILBO_LLM_ANTHROPIC_TOOL_PROGRAM",
                "BILBO_LLM_ANTHROPIC_VIA",
                "BILBO_LLM_OPENAI_API_ENDPOINT",
                "BILBO_LLM_OPENAI_API_KEY_ENV",
                "BILBO_LLM_OPENAI_MODEL",
                "BILBO_LLM_OPENAI_TOOL_EFFORT",
                "BILBO_LLM_OPENAI_TOOL_HOME",
                "BILBO_LLM_OPENAI_TOOL_PROGRAM",
                "BILBO_LLM_OPENAI_VIA",
                "BILBO_LLM_POLL",
                "BILBO_LLM_PROVIDER",
                "BILBO_LLM_RESPONSE",
                "BILBO_LLM_TIMEOUT",
                "BILBO_MARKDOWN_HEADING_WIDTH",
                "BILBO_READ_BUDGET",
                "BILBO_READ_MATCHES_LISTED",
                "BILBO_READ_OUTLINE_WIDTH",
                "BILBO_RECALL_BM25_B",
                "BILBO_RECALL_BM25_K1",
                "BILBO_RECALL_BUDGET",
                "BILBO_RECALL_CANDIDATES",
                "BILBO_RECALL_CAP",
                "BILBO_RECALL_COPY_SHARE",
                "BILBO_RECALL_COPY_WORDS",
                "BILBO_RECALL_NOTES_BUDGET",
                "BILBO_RECALL_PAIR_WEIGHT",
                "BILBO_RECALL_READ_BATCH",
                "BILBO_RECALL_RECORD_CAP",
                "BILBO_RECALL_TYPO_LETTERS",
                "BILBO_RECALL_TYPO_MAX_LETTERS",
                "BILBO_RECALL_TYPO_TERMS",
                "BILBO_RECALL_TYPO_TWO_EDITS",
                "BILBO_REMEMBER_EDITOR",
                "BILBO_SEARCH_CHUNK_OVERLAP",
                "BILBO_SEARCH_CHUNK_TOKENS",
                "BILBO_SEARCH_ENTRY_TRIES",
                "BILBO_SEARCH_INDEX_MEMORY",
                "BILBO_SEARCH_LABEL_BATCH",
                "BILBO_SEARCH_LIST_LIMIT",
                "BILBO_SEARCH_MERGE_DELETED_PERCENT",
                "BILBO_SEARCH_WORKERS",
                "BILBO_SESSION_UNPACK_FLOOR",
                "BILBO_SESSION_UNPACK_RATIO",
                "BILBO_SESSION_UNPACK_WINDOW",
                "BILBO_SESSION_WIDE_FIELDS",
                "BILBO_SETUP_AGY_ENABLED",
                "BILBO_SETUP_AGY_PROGRAM",
                "BILBO_SETUP_CLAUDE_ENABLED",
                "BILBO_SETUP_CLAUDE_PROGRAM",
                "BILBO_SETUP_CODEX_ENABLED",
                "BILBO_SETUP_CODEX_PROGRAM",
                "BILBO_SETUP_DERIVE_EVERY",
                "BILBO_SETUP_NOTES",
                "BILBO_SETUP_PLUGIN",
                "BILBO_SETUP_POLL",
                "BILBO_SETUP_REFRESH_EVERY",
                "BILBO_SETUP_TIMER_LOGS",
                "BILBO_SETUP_TIMER_PATH",
                "BILBO_SETUP_WAIT",
                "BILBO_STORE_COMPRESSION",
                "BILBO_STORE_POLL",
                "BILBO_STORE_SEAL_AFTER",
                "BILBO_STORE_SEAL_AT",
                "BILBO_STORE_TEAR_BUDGET",
                "BILBO_STORE_WAIT",
                "BILBO_TAG_CAP",
                "EDITOR",
                "HOME",
                "TMPDIR",
                "VISUAL",
                "XDG_CACHE_HOME",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_STATE_HOME",
            ]
        );
        assert!(read_names_lacking_in_the_first_cells(PAGE).is_empty());
        assert_eq!(missing_and_foreign(PAGE), (Vec::new(), Vec::new()));
    }

    #[test]
    fn a_missing_foreign_or_lookalike_variable_fails_the_page_check() {
        let none = || Vec::<String>::new();
        assert_eq!(missing_and_foreign(PAGE), (none(), none()));
        let without = PAGE.replace("BILBO_CONFIG", "the config variable");
        assert_eq!(
            missing_and_foreign(&without),
            (vec!["BILBO_CONFIG".to_owned()], none())
        );
        for foreign in ["BILBO_ROOT", "BILBO_HOME2", "XDG_RUNTIME_DIR"] {
            let with = format!("{PAGE}\n`{foreign}`\n");
            assert_eq!(
                missing_and_foreign(&with),
                (none(), vec![foreign.to_owned()]),
                "{foreign}"
            );
        }
        for (first_cell, lacking) in [
            ("| `TMPDIR`", vec!["TMPDIR"]),
            (
                "| `XDG_DATA_HOME`",
                vec![
                    "XDG_CACHE_HOME",
                    "XDG_CONFIG_HOME",
                    "XDG_DATA_HOME",
                    "XDG_STATE_HOME",
                ],
            ),
            ("| `HOME`", vec!["HOME"]),
        ] {
            assert_eq!(
                read_names_lacking_in_the_first_cells(&without_row(PAGE, first_cell)),
                lacking,
                "{first_cell}"
            );
        }
    }

    fn editor_with(env: &[(&str, &str)], file: Option<&str>) -> Result<super::Settings, String> {
        let mut all = vec![("HOME", "/h"), ("BILBO_CONFIG", "/c.toml")];
        all.extend_from_slice(env);
        settings(&all, &[("/c.toml", file.unwrap_or(""))])
    }

    fn taken_and_refused(
        key: &str,
        variable: &str,
        allowed: &str,
        taken: &[&str],
        refused: &[&str],
    ) {
        for value in taken {
            editor_with(&[(variable, value.trim_matches('"'))], None)
                .unwrap_or_else(|error| panic!("{variable}={value}: {error}"));
            editor_with(&[], Some(&format!("{key} = {value}\n")))
                .unwrap_or_else(|error| panic!("{key}={value}: {error}"));
        }
        for value in refused {
            let named = editor_with(&[(variable, value.trim_matches('"'))], None).expect_err(value);
            assert!(
                named.starts_with(&format!("{variable} must be {allowed}, got ")),
                "{named}"
            );
            let held = editor_with(&[], Some(&format!("{key} = {value}\n"))).expect_err(value);
            assert!(
                held.starts_with(&format!("/c.toml:1: {key} must be {allowed}, got ")),
                "{held}"
            );
        }
    }

    #[test]
    fn the_roots_keys_of_the_digest_take_their_defaults_and_refuse_what_is_outside() {
        let settings = editor_with(&[], None).expect("the defaults");
        assert_eq!(
            settings.digest.wait.value,
            std::time::Duration::from_secs(1)
        );
        assert_eq!(settings.digest.input.value, 4 * 1024 * 1024);
        let set = editor_with(
            &[
                ("BILBO_DIGEST_WAIT", "250ms"),
                ("BILBO_DIGEST_INPUT", "2 KiB"),
            ],
            None,
        )
        .expect("set");
        assert_eq!(set.digest.wait.value, std::time::Duration::from_millis(250));
        assert_eq!(set.digest.input.value, 2048);
        taken_and_refused(
            "digest.wait",
            "BILBO_DIGEST_WAIT",
            "between 0s and 1m",
            &["\"0s\"", "\"1m\""],
            &["\"61s\"", "\"soon\"", "\"2m\""],
        );
        taken_and_refused(
            "digest.input",
            "BILBO_DIGEST_INPUT",
            "between 1 KiB and 64 MiB",
            &["\"1 KiB\"", "\"64 MiB\""],
            &["\"1023\"", "\"65 MiB\"", "\"big\""],
        );
        assert_eq!(settings.digest.agy_tail.value, 1024 * 1024);
        taken_and_refused(
            "digest.agy.tail",
            "BILBO_DIGEST_AGY_TAIL",
            "between 1 KiB and 64 MiB",
            &["\"1 KiB\"", "\"64 MiB\""],
            &["\"1023\"", "\"65 MiB\"", "\"big\""],
        );
    }

    #[test]
    fn digest_log_is_true_or_false() {
        assert!(
            editor_with(&[], None)
                .expect("the default")
                .digest
                .log_enabled
                .value
        );
        for (env, file) in [
            (&[("BILBO_DIGEST_LOG_ENABLED", "false")][..], ""),
            (&[][..], "digest.log.enabled = false\n"),
        ] {
            assert!(
                !editor_with(env, Some(file))
                    .expect("false")
                    .digest
                    .log_enabled
                    .value
            );
        }
        assert!(
            editor_with(
                &[("BILBO_DIGEST_LOG_ENABLED", "false")],
                Some("digest.log.enabled = true\n")
            )
            .is_ok_and(|settings| !settings.digest.log_enabled.value)
        );
        taken_and_refused(
            "digest.log.enabled",
            "BILBO_DIGEST_LOG_ENABLED",
            "true or false",
            &["true", "false"],
            &["1", "\"yes\"", "0"],
        );
    }

    #[test]
    fn the_digests_summary_switch_is_true_or_false() {
        assert!(
            editor_with(&[], None)
                .expect("the default")
                .digest
                .summary_enabled
                .value
        );
        for (env, file) in [
            (&[("BILBO_DIGEST_SUMMARY_ENABLED", "false")][..], ""),
            (&[][..], "digest.summary.enabled = false\n"),
        ] {
            assert!(
                !editor_with(env, Some(file))
                    .expect("false")
                    .digest
                    .summary_enabled
                    .value
            );
        }
        assert!(
            editor_with(
                &[("BILBO_DIGEST_SUMMARY_ENABLED", "false")],
                Some("digest.summary.enabled = true\n")
            )
            .is_ok_and(|settings| !settings.digest.summary_enabled.value)
        );
        for (value, got) in [("\"yes\"", "\"yes\""), ("1", "1"), ("0", "0")] {
            assert_eq!(
                editor_with(&[], Some(&format!("digest.summary.enabled = {value}\n")))
                    .err()
                    .as_deref(),
                Some(
                    format!("/c.toml:1: digest.summary.enabled must be true or false, got {got}")
                        .as_str()
                )
            );
        }
        assert_eq!(
            editor_with(&[("BILBO_DIGEST_SUMMARY_ENABLED", "maybe")], None)
                .err()
                .as_deref(),
            Some("BILBO_DIGEST_SUMMARY_ENABLED must be true or false, got \"maybe\"")
        );
    }

    #[test]
    fn each_lists_keys_take_their_defaults_and_refuse_what_is_outside() {
        let d = editor_with(&[], None).expect("the defaults").digest;
        assert_eq!(d.markdown_most.value, 6);
        assert!((d.markdown_floor.value - 8.0).abs() < f64::EPSILON);
        assert_eq!(d.session_most.value, 6);
        assert!((d.session_floor.value - 12.0).abs() < f64::EPSILON);
        assert_eq!(
            d.session_old_after.value,
            30 * std::time::Duration::from_secs(86_400)
        );
        assert!(!d.session_subagents.value && !d.session_parts.value);
        assert_eq!(d.note_most.value, 6);
        assert!((d.note_floor.value - 8.0).abs() < f64::EPSILON);
        assert_eq!(d.note_width.value, 300);
        let counts = "a whole number from 0 to 100";
        let floors = "a number from 0 to 1000";
        for (key, variable) in [
            ("digest.markdown.most", "BILBO_DIGEST_MARKDOWN_MOST"),
            ("digest.session.most", "BILBO_DIGEST_SESSION_MOST"),
            ("digest.note.most", "BILBO_DIGEST_NOTE_MOST"),
        ] {
            taken_and_refused(
                key,
                variable,
                counts,
                &["0", "100"],
                &["-1", "101", "\"x\""],
            );
        }
        for (key, variable) in [
            ("digest.markdown.floor", "BILBO_DIGEST_MARKDOWN_FLOOR"),
            ("digest.session.floor", "BILBO_DIGEST_SESSION_FLOOR"),
            ("digest.note.floor", "BILBO_DIGEST_NOTE_FLOOR"),
        ] {
            taken_and_refused(
                key,
                variable,
                floors,
                &["0", "1000", "8.5"],
                &["-1", "1001", "\"x\""],
            );
        }
        taken_and_refused(
            "digest.session.old_after",
            "BILBO_DIGEST_SESSION_OLD_AFTER",
            "between 1d and 36500d",
            &["\"1d\"", "\"36500d\"", "\"24h\"", "\"36h\""],
            &["\"23h\"", "\"36501d\"", "30", "\"x\""],
        );
        for (key, variable) in [
            ("digest.session.subagents", "BILBO_DIGEST_SESSION_SUBAGENTS"),
            ("digest.session.parts", "BILBO_DIGEST_SESSION_PARTS"),
        ] {
            taken_and_refused(
                key,
                variable,
                "true or false",
                &["true", "false"],
                &["1", "0", "\"x\""],
            );
        }
        for (first, second) in [("true", "false"), ("false", "true")] {
            let set = editor_with(
                &[
                    ("BILBO_DIGEST_SESSION_SUBAGENTS", first),
                    ("BILBO_DIGEST_SESSION_PARTS", second),
                ],
                None,
            )
            .expect("set")
            .digest;
            assert_eq!(
                (set.session_subagents.value, set.session_parts.value),
                (first == "true", second == "true")
            );
        }
        taken_and_refused(
            "digest.note.width",
            "BILBO_DIGEST_NOTE_WIDTH",
            "a whole number from 20 to 1000",
            &["20", "1000"],
            &["19", "1001", "\"x\""],
        );
        let set = editor_with(
            &[
                ("BILBO_DIGEST_MARKDOWN_MOST", "1"),
                ("BILBO_DIGEST_MARKDOWN_FLOOR", "2.5"),
                ("BILBO_DIGEST_SESSION_MOST", "3"),
                ("BILBO_DIGEST_SESSION_FLOOR", "4"),
                ("BILBO_DIGEST_SESSION_OLD_AFTER", "120h"),
                ("BILBO_DIGEST_NOTE_MOST", "7"),
                ("BILBO_DIGEST_NOTE_FLOOR", "9"),
                ("BILBO_DIGEST_NOTE_WIDTH", "50"),
            ],
            None,
        )
        .expect("set")
        .digest;
        assert_eq!(
            (
                set.markdown_most.value,
                set.session_most.value,
                set.session_old_after.value,
                set.note_most.value,
                set.note_width.value
            ),
            (1, 3, 5 * std::time::Duration::from_secs(86_400), 7, 50)
        );
        assert!((set.markdown_floor.value - 2.5).abs() < f64::EPSILON);
        assert!((set.session_floor.value - 4.0).abs() < f64::EPSILON);
        assert!((set.note_floor.value - 9.0).abs() < f64::EPSILON);
    }

    fn at_line_one() -> Origin {
        Origin::File {
            path: PathBuf::from("/c.toml"),
            line: 1,
        }
    }

    #[test]
    fn the_editor_is_its_variable_then_the_file_then_visual_then_editor() {
        let value = |settings: Result<super::Settings, String>| {
            let settings = settings.expect("settings");
            (
                settings
                    .remember_editor
                    .value
                    .to_string_lossy()
                    .into_owned(),
                settings.remember_editor.origin,
            )
        };
        let file = Some("remember.editor = \"code --wait\"\n");
        let both = [("VISUAL", "vim"), ("EDITOR", "vi")];
        let with = |extra: &[(&'static str, &'static str)]| {
            let mut all = both.to_vec();
            all.extend_from_slice(extra);
            all
        };
        assert_eq!(
            value(editor_with(
                &with(&[("BILBO_REMEMBER_EDITOR", "ed -s")]),
                file
            )),
            (
                "ed -s".to_owned(),
                Origin::Variable("BILBO_REMEMBER_EDITOR")
            )
        );
        assert_eq!(
            value(editor_with(&with(&[("BILBO_REMEMBER_EDITOR", "")]), file)),
            ("code --wait".to_owned(), at_line_one())
        );
        assert_eq!(
            value(editor_with(&both, None)),
            ("vim".to_owned(), Origin::Default)
        );
        assert_eq!(
            value(editor_with(&[("VISUAL", ""), ("EDITOR", "vi")], None)),
            ("vi".to_owned(), Origin::Default)
        );
        assert_eq!(
            value(editor_with(&[("EDITOR", "vi")], None)),
            ("vi".to_owned(), Origin::Default)
        );
        assert_eq!(
            value(editor_with(&[], None)),
            (String::new(), Origin::Default)
        );
        assert_eq!(
            value(editor_with(&both, Some("remember.editor = \"\"\n"))),
            (String::new(), at_line_one())
        );
        assert_eq!(
            editor_with(&both, Some("remember.editor = 3\n")).expect_err("a number"),
            "/c.toml:1: remember.editor must be a command line, or the empty text for no editor, got 3"
        );
        let shown = editor_with(&[("EDITOR", "vi")], None)
            .expect("settings")
            .shown();
        let line = shown
            .iter()
            .find(|shown| shown.name == "remember.editor")
            .expect("the key is shown");
        assert_eq!(
            (line.value.as_str(), &line.origin),
            ("\"vi\"", &Origin::Default)
        );
    }

    #[test]
    fn the_temp_folder_is_the_file_then_tmpdir_then_tmp() {
        let value = |env: &[(&str, &str)], file: &str| {
            let settings = editor_with(env, Some(file)).expect("settings");
            (settings.temp.value, settings.temp.origin)
        };
        assert_eq!(
            value(&[("TMPDIR", "/scratch")], "paths.temp = \"/var/drafts\"\n"),
            (PathBuf::from("/var/drafts"), at_line_one())
        );
        assert_eq!(
            value(&[], "paths.temp = \"~/drafts\"\n"),
            (PathBuf::from("/h/drafts"), at_line_one())
        );
        assert_eq!(
            value(&[("TMPDIR", "/scratch")], ""),
            (PathBuf::from("/scratch"), Origin::Default)
        );
        for env in [&[("TMPDIR", "scratch")][..], &[("TMPDIR", "")], &[]] {
            assert_eq!(
                value(env, ""),
                (PathBuf::from("/tmp"), Origin::Default),
                "{env:?}"
            );
        }
        assert_eq!(
            editor_with(&[("TMPDIR", "/a/../b")], None).expect_err("a dotted value"),
            "TMPDIR has a . or .. component: /a/../b"
        );
        assert_eq!(
            editor_with(&[], Some("paths.temp = \"drafts\"\n")).expect_err("a relative path"),
            "/c.toml:1: paths.temp is not an absolute path: drafts"
        );
        let shown = editor_with(&[], None).expect("settings").shown();
        let line = shown
            .iter()
            .find(|shown| shown.name == "paths.temp")
            .expect("the key is shown");
        assert_eq!(
            (line.value.as_str(), &line.origin),
            ("\"/tmp\"", &Origin::Default)
        );
    }

    fn follow(path: &str, kind: Option<&str>) -> Follow {
        Follow {
            path: PathBuf::from(path),
            kind: kind.map(str::to_owned),
        }
    }

    #[test]
    fn the_follows_are_the_two_defaults_until_the_file_declares_one() {
        let env = [("HOME", "/h"), ("BILBO_CONFIG", "/c.toml")];
        let defaults = settings(&env, &[("/c.toml", "")]).expect("defaults");
        assert_eq!(
            defaults.follow.value,
            [
                follow("/h/.claude/projects", Some("session")),
                follow("/h/.codex/sessions", Some("session")),
            ]
        );
        assert_eq!(defaults.follow.origin, Origin::Default);
        let file = "[store]\ncompression = 3\n\n[[follow]]\npath = \"~/notes\"\n\n[[follow]]\npath = \"/x\"\nkind = \"markdown\"\n";
        let declared = settings(&env, &[("/c.toml", file)]).expect("declared");
        assert_eq!(
            declared.follow.value,
            [follow("/h/notes", None), follow("/x", Some("markdown"))]
        );
        assert_eq!(
            declared.follow.origin,
            Origin::File {
                path: PathBuf::from("/c.toml"),
                line: 4
            }
        );
        let none = settings(&env, &[("/c.toml", "follow = []\n")]).expect("none");
        assert!(none.follow.value.is_empty());
    }

    #[test]
    fn a_follow_the_file_cannot_mean_is_a_config_error_naming_its_line() {
        let env = [("HOME", "/h"), ("BILBO_CONFIG", "/c.toml")];
        let error = |text: &str| settings(&env, &[("/c.toml", text)]).err();
        let allowed = "must be a list of [[follow]] tables, each with a path and an optional kind";
        assert_eq!(
            error("[[follow]]\nkind = \"session\"\n"),
            Some(format!("/c.toml:1: follow {allowed}, got a table"))
        );
        assert_eq!(
            error("[[follow]]\npath = \"/x\"\nkinds = \"session\"\n"),
            Some(format!(
                "/c.toml:3: follow.kinds {allowed}, got \"session\""
            ))
        );
        assert_eq!(
            error("[[follow]]\npath = \"/x\"\nkind = \"\"\n"),
            Some(format!("/c.toml:3: follow.kind {allowed}, got \"\""))
        );
        assert_eq!(
            error("follow = \"/x\"\n"),
            Some(format!("/c.toml:1: follow {allowed}, got \"/x\""))
        );
        assert_eq!(
            error("[[follow]]\npath = \"notes\"\n"),
            Some("/c.toml:2: follow.path is not an absolute path: notes".to_owned())
        );
        assert_eq!(
            settings(
                &[
                    ("BILBO_CONFIG", "/c.toml"),
                    ("XDG_DATA_HOME", "/xd"),
                    ("XDG_STATE_HOME", "/xs"),
                    ("XDG_CACHE_HOME", "/xc")
                ],
                &[("/c.toml", "")]
            )
            .err(),
            Some("the default of follow needs HOME, which is not set".to_owned())
        );
    }

    #[test]
    fn the_follows_show_as_inline_tables_in_the_order_given() {
        assert_eq!(show_follows(&[]), "[]");
        assert_eq!(
            show_follows(&[follow("/h/a", Some("session")), follow("/h/b \"q\"", None)]),
            "[{ path = \"/h/a\", kind = \"session\" }, { path = \"/h/b \\\"q\\\"\" }]"
        );
        assert_eq!(FOLLOW.about.variable, None);
    }

    #[test]
    fn a_group_is_listed_after_the_librarys_own_keys_in_its_own_order() {
        let names = |groups: &[&'static Group]| -> Vec<&str> {
            keys(&kind(), groups)
                .iter()
                .map(|about| about.name)
                .collect()
        };
        let without = names(&[]);
        let at = without
            .iter()
            .position(|name| *name == "derive.enabled")
            .expect("derive.enabled");
        let mut with = without.clone();
        with.splice(at..at, ["test.word", "test.table"]);
        assert_eq!(names(&[&GROUP]), with);
    }

    #[test]
    fn a_groups_keys_are_known_to_the_file_and_shown_with_their_origin() {
        let text = "[test]\nword = \"a\"\n\n[test.table]\nx = \"1\"\n";
        let settings = grouped(&[("BILBO_CONFIG", "/c.toml")], text, &[&GROUP]).expect("loads");
        let config = PathBuf::from("/c.toml");
        let shown = settings.shown();
        let at = shown
            .iter()
            .position(|item| item.name == "derive.enabled")
            .expect("derive.enabled");
        assert_eq!(
            shown[at - 2..at],
            [
                Shown {
                    name: "test.word",
                    value: "\"a\"".to_owned(),
                    origin: Origin::File {
                        path: config.clone(),
                        line: 2
                    },
                },
                Shown {
                    name: "test.table",
                    value: "[(\"x\", \"1\")]".to_owned(),
                    origin: Origin::File {
                        path: config,
                        line: 4
                    },
                },
            ]
        );
        match grouped(&[("BILBO_CONFIG", "/c.toml")], text, &[]) {
            Err(Error::Unknown { line, key, .. }) => {
                assert_eq!((line, key.as_str()), (2, "test.word"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_value_a_groups_key_refuses_is_a_config_error_at_load() {
        let refused = |env: &[(&str, &str)], text: &str| {
            grouped(env, text, &[&GROUP])
                .err()
                .map(|error| error.to_string())
        };
        assert_eq!(
            refused(&[("BILBO_CONFIG", "/c.toml")], "[test]\nword = 5\n"),
            Some("/c.toml:2: test.word must be a word, got 5".to_owned())
        );
        assert_eq!(
            refused(&[("BILBO_CONFIG", "/c.toml")], "[test.table]\nx = 5\n"),
            Some("/c.toml:2: test.table.x must be a table of words, got 5".to_owned())
        );
        let var = |name: &str| match name {
            "BILBO_CONFIG" => Some(OsString::from("/c.toml")),
            "HOME" => Some(OsString::from("/h")),
            "BILBO_TEST_WORD" => Some(OsString::from_vec(vec![0xFF])),
            _ => None,
        };
        let read = |_: &Path| Ok(b"[test]\nword = \"a\"\n".to_vec());
        let error = load(&var, &read, &kind(), &[&GROUP]).expect_err("a refusal");
        match &error {
            Error::Invalid { at, allowed, got } => {
                assert_eq!(*at, At::Variable("BILBO_TEST_WORD"));
                assert_eq!(*allowed, "a word");
                assert_eq!(got, "\"\u{FFFD}\"");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            error.to_string(),
            "BILBO_TEST_WORD must be a word, got \"\u{FFFD}\""
        );
    }

    #[test]
    fn the_settings_keep_the_file_so_a_group_resolves_again_as_load_did() {
        let both = |env: &[(&str, &str)], text: &str| -> (Shown, Shown) {
            let settings = grouped(env, text, &[&GROUP]).expect("loads");
            let loaded = settings
                .shown()
                .into_iter()
                .find(|shown| shown.name == "test.word")
                .expect("the row load kept");
            let env: HashMap<String, OsString> = env
                .iter()
                .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
                .collect();
            let var = |name: &str| env.get(name).cloned();
            let source: Source<'_> = settings.source(&var);
            let again = WORD.shown(&WORD.resolve(&source).expect("resolves"));
            (loaded, again)
        };
        let config = [("BILBO_CONFIG", "/c.toml")];
        let word = |value: &str, origin: Origin| Shown {
            name: "test.word",
            value: value.to_owned(),
            origin,
        };
        let cases = [
            (
                both(&config, "[test]\nword = \"a\"\n"),
                word(
                    "\"a\"",
                    Origin::File {
                        path: PathBuf::from("/c.toml"),
                        line: 2,
                    },
                ),
            ),
            (
                both(
                    &[("BILBO_CONFIG", "/c.toml"), ("BILBO_TEST_WORD", "b")],
                    "[test]\nword = \"a\"\n",
                ),
                word("\"b\"", Origin::Variable("BILBO_TEST_WORD")),
            ),
            (both(&config, ""), word("\"\"", Origin::Default)),
        ];
        for ((loaded, again), expected) in cases {
            assert_eq!(loaded, again);
            assert_eq!(loaded, expected);
        }
    }

    fn repeated(listed: &[&'static About]) -> Vec<&'static str> {
        let mut found: Vec<&'static str> = Vec::new();
        for (at, about) in listed.iter().enumerate() {
            let again = listed[at + 1..]
                .iter()
                .any(|later| later.name == about.name);
            if again && !found.contains(&about.name) {
                found.push(about.name);
            }
        }
        found
    }

    static CORE_AGAIN: Group = Group {
        about: &[&ROOT.about],
        resolve: |_| Ok(Vec::new()),
    };

    const KIND_ABOUT: About = About {
        name: "markdown.heading_width",
        variable: None,
        default: Default::Toml("100"),
        allowed: "a number",
        doc: "a kind's key, named again",
    };

    static KIND_AGAIN: Group = Group {
        about: &[&KIND_ABOUT],
        resolve: |_| Ok(Vec::new()),
    };

    fn resolved_names(group: &'static Group) -> Vec<&'static str> {
        let none = bilbo_settings::file::File::default();
        let source = Source {
            var: &|_| None,
            file: &none,
        };
        (group.resolve)(&source)
            .expect("a group resolves over no file and no variable")
            .iter()
            .map(|shown| shown.name)
            .collect()
    }

    fn declared_names(group: &'static Group) -> Vec<&'static str> {
        group.about.iter().map(|about| about.name).collect()
    }

    #[test]
    fn every_group_resolves_the_keys_it_declares_in_their_order() {
        for group in crate::provider_keys() {
            assert_eq!(resolved_names(group), declared_names(group));
        }
        assert_eq!(resolved_names(&GROUP), declared_names(&GROUP));
        assert_eq!(declared_names(&GROUP), ["test.word", "test.table"]);
        assert_eq!(declared_names(&CORE_AGAIN), ["paths.root"]);
        assert!(resolved_names(&CORE_AGAIN).is_empty());
    }

    #[test]
    fn every_key_of_the_registry_has_a_name_of_its_own() {
        assert!(repeated(&keys(&kind(), &crate::provider_keys())).is_empty());
        assert_eq!(repeated(&keys(&kind(), &[&CORE_AGAIN])), ["paths.root"]);
        assert_eq!(
            repeated(&keys(&kind(), &[&KIND_AGAIN])),
            ["markdown.heading_width"]
        );
        let listed = [&ROOT.about, &STATE.about, &STATE.about, &ROOT.about];
        assert_eq!(repeated(&listed), ["paths.root", "paths.state"]);
    }

    #[test]
    fn a_value_refused_on_a_later_line_names_no_file() {
        let said = super::allowed_value(
            "derive.in",
            "[\n  \"a\",\n  \"b\",\n  \"!!\",\n]",
            Some(&OsString::from("/h")),
            &crate::kind_keys(),
            &crate::provider_keys(),
        )
        .expect_err("a refused item");
        assert!(said.starts_with("derive.in must be "), "{said}");
        assert!(!said.contains("/allowed.toml"), "{said}");
    }

    #[test]
    fn each_setup_key_of_the_root_takes_its_default_and_refuses_what_is_outside() {
        let defaults = editor_with(&[], None).expect("defaults").setup;
        assert_eq!(defaults.plugin.value, None);
        assert_eq!(defaults.timer_logs.value, None);
        for (switch, expected) in [
            (&defaults.claude_enabled, true),
            (&defaults.codex_enabled, true),
            (&defaults.agy_enabled, true),
        ] {
            assert_eq!(switch.value, expected);
        }
        assert_eq!(defaults.claude_program.value, PathBuf::from("claude"));
        assert_eq!(defaults.codex_program.value, PathBuf::from("codex"));
        assert_eq!(defaults.agy_program.value, PathBuf::from("agy"));
        for key in [
            "setup.claude.enabled",
            "setup.codex.enabled",
            "setup.agy.enabled",
        ] {
            for value in ["\"yes\"", "1", "0", "\"\""] {
                let said = editor_with(&[], Some(&format!("{key} = {value}\n"))).expect_err(value);
                assert!(
                    said.contains(&format!("{key} must be true or false")),
                    "{said}"
                );
            }
        }
        for key in [
            "setup.claude.program",
            "setup.codex.program",
            "setup.agy.program",
        ] {
            for value in ["\"tool\"", "\"/usr/bin/tool\"", "\"~/bin/tool\""] {
                editor_with(&[], Some(&format!("{key} = {value}\n")))
                    .unwrap_or_else(|error| panic!("{key}={value}: {error}"));
            }
            for value in ["\"\"", "\"bin/tool\"", "\"./tool\"", "\"/a/../tool\""] {
                let said = editor_with(&[], Some(&format!("{key} = {value}\n"))).expect_err(value);
                assert!(said.contains(key), "{said}");
            }
        }
        for key in ["setup.plugin", "setup.notes", "setup.timer.logs"] {
            for value in ["\"\"", "\"/p\"", "\"~/p\""] {
                editor_with(&[], Some(&format!("{key} = {value}\n")))
                    .unwrap_or_else(|error| panic!("{key}={value}: {error}"));
            }
            for value in ["\"p\"", "\"./p\"", "3"] {
                assert!(
                    editor_with(&[], Some(&format!("{key} = {value}\n"))).is_err(),
                    "{key}={value}"
                );
            }
        }
    }

    #[test]
    fn a_tool_is_kept_out_from_the_file_and_from_its_variable() {
        for (env, file) in [
            (&[][..], "setup.codex.enabled = false\n"),
            (&[("BILBO_SETUP_CODEX_ENABLED", "false")][..], ""),
        ] {
            let setup = editor_with(env, Some(file)).expect("false").setup;
            assert!(!setup.codex_enabled.value);
            assert!(setup.claude_enabled.value && setup.agy_enabled.value);
        }
        let setup = editor_with(&[("BILBO_SETUP_CODEX_ENABLED", "")], None)
            .expect("an empty variable is unset")
            .setup;
        assert!(setup.codex_enabled.value);
    }

    #[test]
    fn the_folder_variables_are_home_and_what_the_four_folder_keys_read() {
        assert_eq!(
            super::folder_variables(),
            [
                "HOME",
                "BILBO_CONFIG",
                "XDG_CONFIG_HOME",
                "BILBO_HOME",
                "XDG_DATA_HOME",
                "XDG_STATE_HOME",
                "XDG_CACHE_HOME",
            ]
        );
    }
}
