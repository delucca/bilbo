use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use bilbo_entries::follows::Follow;
use bilbo_entries::read;
use bilbo_entries::tag;
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

pub type KindKey = Key<usize>;

#[derive(Debug)]
pub struct Group {
    pub about: &'static [&'static About],
    pub resolve: fn(&Source<'_>) -> Result<Vec<Shown>, Error>,
}

/// Every key, in the order `bilbo config` lists them.
pub fn keys(kind: &[&'static KindKey], groups: &[&'static Group]) -> Vec<&'static About> {
    PATHS
        .into_iter()
        .chain(keys::ABOUT)
        .chain([
            &FOLLOW.about,
            &read::BUDGET.about,
            &read::OUTLINE_WIDTH.about,
            &read::MATCHES_LISTED.about,
            &tag::CAP.about,
        ])
        .chain(bilbo_search::keys::ABOUT)
        .chain(bilbo_search::recall::keys::ABOUT)
        .chain(bilbo_llm::keys::ABOUT)
        .chain(groups.iter().flat_map(|group| group.about.iter().copied()))
        .chain(bilbo_derive::keys::ABOUT)
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
    pub tag_cap: Resolved<usize>,
    pub search: bilbo_search::keys::Section,
    pub recall: bilbo_search::recall::keys::Section,
    pub llm: bilbo_llm::keys::Section,
    pub derive: bilbo_derive::keys::Section,
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
        shown.push(tag::CAP.shown(&self.tag_cap));
        shown.extend(self.search.shown());
        shown.extend(self.recall.shown());
        shown.extend(self.llm.shown());
        shown.extend(self.shown.iter().cloned());
        shown.extend(self.derive.shown());
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

/// The config file's path from the environment, then the file, then every key.
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
        store: keys::section(&source)?,
        follow: FOLLOW.resolve(&source)?,
        read_budget: read::BUDGET.resolve(&source)?,
        read_outline_width: read::OUTLINE_WIDTH.resolve(&source)?,
        read_matches_listed: read::MATCHES_LISTED.resolve(&source)?,
        tag_cap: tag::CAP.resolve(&source)?,
        search: bilbo_search::keys::section(&source)?,
        recall: bilbo_search::recall::keys::section(&source)?,
        llm: bilbo_llm::keys::section(&source)?,
        derive: bilbo_derive::keys::section(&source)?,
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

    use super::{CACHE, CONFIG, FOLLOW, Group, ROOT, STATE, keys, load, show_follows};

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
    fn the_registry_is_the_folders_the_store_the_follows_then_the_read_tag_search_recall_llm_derive_and_kinds_keys()
     {
        let names: Vec<&str> = keys(&kind(), &[]).iter().map(|about| about.name).collect();
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
                "llm.provider",
                "llm.response",
                "llm.timeout",
                "llm.poll",
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
                 paths.cache, store.seal_at, store.seal_after, store.compression, store.tear_budget, store.wait, store.poll, follow, read.budget, read.outline_width, read.matches_listed, tag.cap, search.chunk_tokens, search.chunk_overlap, search.workers, search.index_memory, search.merge_deleted_percent, search.entry_tries, search.label_batch, search.list_limit, recall.budget, recall.notes_budget, recall.candidates, recall.cap, recall.record_cap, recall.copy_share, recall.copy_words, recall.question_words, recall.languages, recall.bm25_k1, recall.bm25_b, recall.read_batch, recall.typo_terms, recall.typo_letters, recall.typo_max_letters, recall.typo_two_edits, recall.pair_weight, llm.provider, llm.response, llm.timeout, llm.poll, llm.anthropic.model, llm.anthropic.via, llm.anthropic.api.endpoint, llm.anthropic.api.version, llm.anthropic.api.key_env, llm.anthropic.api.key_command, llm.anthropic.api.headers, llm.anthropic.api.extra, llm.anthropic.tool.program, llm.anthropic.tool.effort, llm.anthropic.tool.arguments, llm.anthropic.tool.inherit, llm.anthropic.tool.environment, llm.anthropic.tool.project_files, llm.openai.model, llm.openai.via, llm.openai.api.endpoint, llm.openai.api.key_env, llm.openai.api.key_command, llm.openai.api.headers, llm.openai.api.extra, llm.openai.tool.program, llm.openai.tool.effort, llm.openai.tool.arguments, llm.openai.tool.inherit, llm.openai.tool.environment, llm.openai.tool.home, llm.openai.tool.system_files, derive.enabled, derive.in, derive.not_in, derive.instructions, derive.quiet, derive.window, derive.passage, derive.passage_head, derive.earlier, derive.names, derive.answer_tokens, derive.calls, derive.retries, derive.retry_wait, derive.retry_factor, derive.retry_limit, derive.tries, derive.failures, markdown.heading_width, session.unpack_floor, session.unpack_ratio, session.unpack_window, session.wide_fields"
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
            .filter(|word| word.starts_with("BILBO_") || word.starts_with("XDG_"))
            .map(str::to_owned)
            .collect()
    }

    fn expected_names() -> BTreeSet<String> {
        let variables = keys(&kind(), &crate::provider_keys())
            .into_iter()
            .filter_map(|about| about.variable);
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
                "BILBO_DERIVE_TRIES",
                "BILBO_DERIVE_WINDOW",
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
                "BILBO_STORE_COMPRESSION",
                "BILBO_STORE_POLL",
                "BILBO_STORE_SEAL_AFTER",
                "BILBO_STORE_SEAL_AT",
                "BILBO_STORE_TEAR_BUDGET",
                "BILBO_STORE_WAIT",
                "BILBO_TAG_CAP",
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
}
