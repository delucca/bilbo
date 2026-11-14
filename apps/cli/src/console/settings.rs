use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use bilbo_kinds::follows::Follow;
use bilbo_kinds::read;
use bilbo_settings::error::Error;
use bilbo_settings::file::{File, Named};
use bilbo_settings::folder::{self, Folder};
use bilbo_settings::key::{About, Default, Key, Origin, Resolved, Shown, Source, Value, quote};
use bilbo_store::keys;
use bilbo_store::layout::Layout;

/// Not a key: the file cannot name itself.
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
            under_xdg: "bilbo",
            under_home: ".local/share/bilbo",
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

/// The composition root's: its default names a kind, which only the registry may.
pub const FOLLOW: Key<Vec<Follow>> = Key {
    about: About {
        name: "follow",
        variable: None,
        default: Default::Toml(
            r#"[{ path = "~/.claude/projects", kind = "session" }, { path = "~/.codex/sessions", kind = "session" }]"#,
        ),
        allowed: "a list of [[follow]] tables, each with a path and an optional kind",
        doc: "What a bare bilbo remember keeps up with: each folder or file, and the kind that runs there, else the one detected.",
    },
    parse: follows,
    show: |list| show_follows(list),
};

const PATHS: [&About; 3] = [&ROOT.about, &STATE.about, &CACHE.about];

/// A key a kind crate declares; `lib.rs`, the one file that names a kind crate, hands them in.
pub type KindKey = Key<usize>;

/// Every key, in the order `bilbo config` lists them.
pub fn keys(kind: &[&'static KindKey]) -> Vec<&'static About> {
    PATHS
        .into_iter()
        .chain(keys::ABOUT)
        .chain([
            &FOLLOW.about,
            &read::BUDGET.about,
            &read::OUTLINE_WIDTH.about,
            &read::MATCHES_LISTED.about,
        ])
        .chain(bilbo_search::keys::ABOUT)
        .chain(bilbo_search::recall::keys::ABOUT)
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
pub struct Settings {
    pub config: Resolved<PathBuf>,
    pub root: Resolved<PathBuf>,
    pub state: Resolved<PathBuf>,
    pub cache: Resolved<PathBuf>,
    pub store: keys::Section,
    pub follow: Resolved<Vec<Follow>>,
    pub read_budget: Resolved<read::Budget>,
    pub read_outline_width: Resolved<usize>,
    pub read_matches_listed: Resolved<usize>,
    pub search: bilbo_search::keys::Section,
    pub recall: bilbo_search::recall::keys::Section,
    pub kind: Vec<(&'static KindKey, Resolved<usize>)>,
}

impl Settings {
    pub fn layout(&self) -> Layout {
        Layout {
            root: self.root.value.clone(),
            state: self.state.value.clone(),
            cache: self.cache.value.clone(),
        }
    }

    /// The value a kind crate's key resolved to, if `load` was given that key.
    pub fn kind_value(&self, key: &KindKey) -> Option<usize> {
        self.kind
            .iter()
            .find(|(declared, _)| declared.about.name == key.about.name)
            .map(|(_, resolved)| resolved.value)
    }

    /// Every key with its value and origin, in the order of `keys`.
    pub fn shown(&self) -> Vec<Shown> {
        let mut shown = vec![
            ROOT.shown(&self.root),
            STATE.shown(&self.state),
            CACHE.shown(&self.cache),
        ];
        shown.extend(self.store.shown());
        shown.push(FOLLOW.shown(&self.follow));
        shown.push(read::BUDGET.shown(&self.read_budget));
        shown.push(read::OUTLINE_WIDTH.shown(&self.read_outline_width));
        shown.push(read::MATCHES_LISTED.shown(&self.read_matches_listed));
        shown.extend(self.search.shown());
        shown.extend(self.recall.shown());
        shown.extend(self.kind.iter().map(|(key, resolved)| key.shown(resolved)));
        shown
    }
}

/// The config file's path from the environment, then the file, then every key.
pub fn load(
    var: &dyn Fn(&str) -> Option<OsString>,
    read: &dyn Fn(&Path) -> io::Result<Vec<u8>>,
    kind: &[&'static KindKey],
) -> Result<Settings, Error> {
    let none = File::default();
    let config = CONFIG.resolve(&Source { var, file: &none })?;
    let named = match config.origin {
        Origin::Variable(name) => Named::ByVariable(name),
        _ => Named::ByDefault,
    };
    let file = File::load(&config.value, named, read(&config.value))?;
    let names: Vec<&'static str> = keys(kind).iter().map(|about| about.name).collect();
    file.check(&names)?;
    let source = Source { var, file: &file };
    Ok(Settings {
        root: ROOT.resolve(&source)?,
        state: STATE.resolve(&source)?,
        cache: CACHE.resolve(&source)?,
        store: keys::section(&source)?,
        follow: FOLLOW.resolve(&source)?,
        read_budget: read::BUDGET.resolve(&source)?,
        read_outline_width: read::OUTLINE_WIDTH.resolve(&source)?,
        read_matches_listed: read::MATCHES_LISTED.resolve(&source)?,
        search: bilbo_search::keys::section(&source)?,
        recall: bilbo_search::recall::keys::section(&source)?,
        kind: kind
            .iter()
            .map(|key| Ok((*key, key.resolve(&source)?)))
            .collect::<Result<_, Error>>()?,
        config,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};

    use bilbo_settings::key::{Default, Origin};

    use bilbo_kinds::follows::Follow;

    use super::{CACHE, CONFIG, FOLLOW, ROOT, STATE, keys, load, show_follows};

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
        load(&|name| env.get(name).cloned(), &read, &kind()).map_err(|error| error.to_string())
    }

    #[test]
    fn the_registry_is_the_three_folders_the_six_store_keys_the_follows_then_the_read_search_recall_and_markdown_keys()
     {
        let names: Vec<&str> = keys(&kind()).iter().map(|about| about.name).collect();
        assert_eq!(
            names,
            [
                "paths.root",
                "paths.state",
                "paths.cache",
                "store.seal_at",
                "store.seal_after",
                "store.compression",
                "store.tear_budget",
                "store.wait",
                "store.poll",
                "follow",
                "read.budget",
                "read.outline_width",
                "read.matches_listed",
                "search.chunk_tokens",
                "search.chunk_overlap",
                "search.workers",
                "search.index_memory",
                "search.merge_deleted_percent",
                "search.entry_tries",
                "search.label_batch",
                "recall.budget",
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
                "markdown.heading_width",
                "session.unpack_floor",
                "session.unpack_ratio"
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
        assert_eq!(defaults.root.value, PathBuf::from("/h/.local/share/bilbo"));
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
                 paths.cache, store.seal_at, store.seal_after, store.compression, store.tear_budget, store.wait, store.poll, follow, read.budget, read.outline_width, read.matches_listed, search.chunk_tokens, search.chunk_overlap, search.workers, search.index_memory, search.merge_deleted_percent, search.entry_tries, search.label_batch, recall.budget, recall.candidates, recall.cap, recall.record_cap, recall.copy_share, recall.copy_words, recall.question_words, recall.languages, recall.bm25_k1, recall.bm25_b, recall.read_batch, markdown.heading_width, session.unpack_floor, session.unpack_ratio"
                    .to_owned()
            )
        );
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
        assert_eq!(silent.root.value, PathBuf::from("/xd/bilbo"));
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
        let expected: Vec<Vec<String>> = keys(&kind())
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
    }

    #[test]
    fn the_reference_page_means_what_each_key_declares() {
        let meanings: Vec<String> = keys_table()
            .into_iter()
            .map(|row| row[3].replace('`', ""))
            .collect();
        assert_eq!(meanings.len(), keys(&kind()).len());
        for (about, meaning) in keys(&kind()).iter().zip(&meanings) {
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

    fn names_in(text: &str) -> BTreeSet<String> {
        text.split(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .filter(|word| word.starts_with("BILBO_") || word.starts_with("XDG_"))
            .map(str::to_owned)
            .collect()
    }

    fn expected_names() -> BTreeSet<String> {
        let variables = keys(&kind()).into_iter().filter_map(|about| about.variable);
        let folders = [&ROOT, &STATE, &CACHE, &CONFIG].map(|key| match key.about.default {
            Default::Folder(folder) => folder.xdg,
            Default::Toml(_) => unreachable!("a folder key"),
        });
        variables
            .chain(CONFIG.about.variable)
            .chain(folders)
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_reference_page_names_exactly_the_variables_bilbo_reads() {
        let expected = expected_names();
        assert_eq!(
            expected.iter().map(String::as_str).collect::<Vec<_>>(),
            [
                "BILBO_CONFIG",
                "BILBO_HOME",
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
                "BILBO_RECALL_READ_BATCH",
                "BILBO_RECALL_RECORD_CAP",
                "BILBO_SEARCH_CHUNK_OVERLAP",
                "BILBO_SEARCH_CHUNK_TOKENS",
                "BILBO_SEARCH_ENTRY_TRIES",
                "BILBO_SEARCH_INDEX_MEMORY",
                "BILBO_SEARCH_LABEL_BATCH",
                "BILBO_SEARCH_MERGE_DELETED_PERCENT",
                "BILBO_SEARCH_WORKERS",
                "BILBO_SESSION_UNPACK_FLOOR",
                "BILBO_SESSION_UNPACK_RATIO",
                "BILBO_STORE_COMPRESSION",
                "BILBO_STORE_POLL",
                "BILBO_STORE_SEAL_AFTER",
                "BILBO_STORE_SEAL_AT",
                "BILBO_STORE_TEAR_BUDGET",
                "BILBO_STORE_WAIT",
                "XDG_CACHE_HOME",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_STATE_HOME",
            ]
        );
        assert_eq!(names_in(PAGE), expected);
    }

    #[test]
    fn a_missing_foreign_or_lookalike_variable_fails_the_page_check() {
        let expected = expected_names();
        let without = PAGE.replace("BILBO_CONFIG", "the config variable");
        assert_ne!(names_in(&without), expected, "a missing name must fail");
        for foreign in ["BILBO_ROOT", "BILBO_HOME2", "XDG_RUNTIME_DIR"] {
            let with = format!("{PAGE}\n`{foreign}`\n");
            assert_ne!(names_in(&with), expected, "{foreign} must fail");
        }
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
}
