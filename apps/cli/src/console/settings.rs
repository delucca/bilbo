use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use bilbo_settings::error::Error;
use bilbo_settings::file::{File, Named};
use bilbo_settings::folder::{self, Folder};
use bilbo_settings::key::{About, Default, Key, Origin, Resolved, Shown, Source};
use bilbo_store::keys;
use bilbo_store::layout::Layout;
use rustix::fs::{FileType, Mode, OFlags};

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
        doc: "This device's state: open.seg, the store lock and its store root.",
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

const PATHS: [&About; 3] = [&ROOT.about, &STATE.about, &CACHE.about];

/// Every key, in the order `bilbo config` lists them.
pub fn keys() -> Vec<&'static About> {
    PATHS.into_iter().chain(keys::ABOUT).collect()
}

#[derive(Debug)]
pub struct Settings {
    pub root: Resolved<PathBuf>,
    pub state: Resolved<PathBuf>,
    pub cache: Resolved<PathBuf>,
    pub store: keys::Section,
}

impl Settings {
    pub fn layout(&self) -> Layout {
        Layout {
            root: self.root.value.clone(),
            state: self.state.value.clone(),
            cache: self.cache.value.clone(),
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
        shown
    }
}

/// A file's bytes, read only from a regular file: a named pipe or a device would block the read,
/// and the file is read before any verb parses its arguments.
pub fn read_regular(path: &Path) -> io::Result<Vec<u8>> {
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let stat = rustix::fs::fstat(&fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    fs::File::from(fd).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The config file's path from the environment, then the file, then every key.
pub fn load(
    var: &dyn Fn(&str) -> Option<OsString>,
    read: &dyn Fn(&Path) -> io::Result<Vec<u8>>,
) -> Result<Settings, Error> {
    let none = File::default();
    let config = CONFIG.resolve(&Source { var, file: &none })?;
    let named = match config.origin {
        Origin::Variable(name) => Named::ByVariable(name),
        _ => Named::ByDefault,
    };
    let file = File::load(&config.value, named, read(&config.value))?;
    let names: Vec<&'static str> = keys().iter().map(|about| about.name).collect();
    file.check(&names)?;
    let source = Source { var, file: &file };
    Ok(Settings {
        root: ROOT.resolve(&source)?,
        state: STATE.resolve(&source)?,
        cache: CACHE.resolve(&source)?,
        store: keys::section(&source)?,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};

    use bilbo_settings::key::{Default, Origin};

    use super::{CACHE, CONFIG, ROOT, STATE, keys, load};

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
        load(&|name| env.get(name).cloned(), &read).map_err(|error| error.to_string())
    }

    #[test]
    fn the_registry_is_the_three_folders_then_the_three_store_keys() {
        let names: Vec<&str> = keys().iter().map(|about| about.name).collect();
        assert_eq!(
            names,
            [
                "paths.root",
                "paths.state",
                "paths.cache",
                "store.seal_at",
                "store.compression",
                "store.tear_budget"
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
                 paths.cache, store.seal_at, store.compression, store.tear_budget"
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
        let expected: Vec<Vec<String>> = keys()
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
        assert_eq!(meanings.len(), keys().len());
        for (about, meaning) in keys().iter().zip(&meanings) {
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
        let variables = keys().into_iter().filter_map(|about| about.variable);
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
                "BILBO_STORE_COMPRESSION",
                "BILBO_STORE_SEAL_AT",
                "BILBO_STORE_TEAR_BUDGET",
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
}
