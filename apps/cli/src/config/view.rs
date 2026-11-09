use bilbo_settings::key::Shown;

use crate::console::message::sanitize;
use crate::console::theme::{Tone, paint};
use crate::console::view::View;

pub fn plain(shown: &[Shown]) -> String {
    shown
        .iter()
        .map(|one| {
            format!(
                "{} = {} ({})\n",
                one.name,
                sanitize(&one.value),
                origin(one)
            )
        })
        .collect()
}

fn origin(one: &Shown) -> String {
    sanitize(&one.origin.to_string())
}

pub fn human(shown: &[Shown], view: &View) -> String {
    let width = shown
        .iter()
        .map(|one| one.name.len())
        .max()
        .unwrap_or_default();
    shown
        .iter()
        .map(|one| {
            format!(
                "{:<width$} = {}  {}\n",
                one.name,
                sanitize(&one.value),
                paint(view, Tone::Dim, &format!("({})", origin(one)))
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bilbo_settings::key::{Origin, Shown};

    use super::{human, plain};
    use crate::console::view::View;

    fn shown() -> Vec<Shown> {
        vec![
            Shown {
                name: "paths.root",
                value: "\"/r\"".to_owned(),
                origin: Origin::Variable("BILBO_HOME"),
            },
            Shown {
                name: "store.seal_at",
                value: "\"16 MiB\"".to_owned(),
                origin: Origin::File {
                    path: PathBuf::from("/c/config.toml"),
                    line: 5,
                },
            },
            Shown {
                name: "recall.budget",
                value: "900".to_owned(),
                origin: Origin::Flag("--budget"),
            },
        ]
    }

    #[test]
    fn the_plain_view_is_one_line_per_key_with_its_value_and_origin() {
        assert_eq!(
            plain(&shown()),
            "paths.root = \"/r\" (BILBO_HOME)\n\
             store.seal_at = \"16 MiB\" (/c/config.toml:5)\n\
             recall.budget = 900 (--budget)\n"
        );
    }

    #[test]
    fn a_file_path_with_a_newline_or_an_escape_byte_stays_one_plain_line() {
        let shown = vec![Shown {
            name: "store.seal_at",
            value: "\"16 MiB\"".to_owned(),
            origin: Origin::File {
                path: PathBuf::from("/c/a\nb\u{1b}[31m.toml"),
                line: 5,
            },
        }];
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        for text in [plain(&shown), human(&shown, &unpainted)] {
            assert_eq!(text.lines().count(), 1, "{text:?}");
            assert!(text.ends_with('\n'));
            assert!(text.contains("/c/a\\nb\\u{1b}[31m.toml:5"), "{text:?}");
            assert!(!text.contains('\u{1b}'), "{text:?}");
        }
        let painted = human(&shown, &View::HUMAN);
        assert_eq!(painted.lines().count(), 1, "{painted:?}");
        assert_eq!(painted.matches('\u{1b}').count(), 2, "only the dim codes");
    }

    #[test]
    fn the_human_view_lines_up_the_values_and_dims_the_origins() {
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        assert_eq!(
            human(&shown(), &unpainted),
            "paths.root    = \"/r\"  (BILBO_HOME)\n\
             store.seal_at = \"16 MiB\"  (/c/config.toml:5)\n\
             recall.budget = 900  (--budget)\n"
        );
        assert!(human(&shown(), &View::HUMAN).contains("\u{1b}[2m(BILBO_HOME)\u{1b}[22m"));
    }
}
