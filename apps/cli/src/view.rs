//! Which view a stream gets: the plain view for pipes, files and agents, the
//! human view for a person's terminal.

use std::ffi::OsString;

/// Not `CLAUDECODE`: it marks an IDE terminal a person types in.
pub const AGENT_MARKERS: [&str; 5] = [
    "AI_AGENT",
    "CLAUDE_CODE_CHILD_SESSION",
    "CODEX_THREAD_ID",
    "CODEX_CI",
    "ANTIGRAVITY_AGENT",
];

/// How one stream prints, decided once per stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    /// A terminal, and no agent runs bilbo: print the human view.
    pub human: bool,
    /// The human view may use escape codes.
    pub paint: bool,
    /// Marks in Unicode, else their ASCII forms.
    pub unicode: bool,
}

impl View {
    /// The view of a pipe, a file or an agent.
    pub const PLAIN: View = View {
        human: false,
        paint: false,
        unicode: false,
    };

    /// The view of a person's terminal that may paint.
    pub const HUMAN: View = View {
        human: true,
        paint: true,
        unicode: true,
    };
}

/// `CLICOLOR_FORCE` and `FORCE_COLOR` are never read: they leak into agent environments.
pub fn decide(terminal: bool, macos: bool, var: impl Fn(&str) -> Option<OsString>) -> View {
    let set = |name: &str| var(name).is_some_and(|value| !value.is_empty());
    let human = terminal && !AGENT_MARKERS.into_iter().any(set);
    let term_ok = var("TERM").is_some_and(|term| !term.is_empty() && term != "dumb");
    let colour = !set("NO_COLOR") && var("CLICOLOR").is_none_or(|v| v != "0") && term_ok;
    View {
        human,
        paint: human && colour,
        unicode: unicode(macos, var),
    }
}

/// macOS terminals always draw Unicode; elsewhere the locale's codeset decides.
pub fn unicode(macos: bool, var: impl Fn(&str) -> Option<OsString>) -> bool {
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .find_map(|name| var(name).filter(|value| !value.is_empty()));
    macos
        || locale.is_some_and(|locale| {
            let locale = locale.to_string_lossy().to_ascii_lowercase();
            let codeset = locale.split('@').next().and_then(|l| l.rsplit_once('.'));
            codeset.is_some_and(|(_, codeset)| matches!(codeset, "utf-8" | "utf8"))
        })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{decide, unicode};

    const XTERM: (&str, &str) = ("TERM", "xterm-256color");

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    fn view(terminal: bool, env: &[(&str, &str)]) -> (bool, bool) {
        let view = decide(terminal, true, vars(env));
        (view.human, view.paint)
    }

    #[test]
    fn the_human_view_needs_a_terminal_and_no_agent_marker() {
        assert_eq!(view(true, &[XTERM]), (true, true));
        assert_eq!(view(false, &[XTERM]), (false, false));
        for marker in [
            "AI_AGENT",
            "CLAUDE_CODE_CHILD_SESSION",
            "CODEX_THREAD_ID",
            "CODEX_CI",
            "ANTIGRAVITY_AGENT",
        ] {
            assert_eq!(
                view(true, &[XTERM, (marker, "1")]),
                (false, false),
                "{marker}"
            );
            assert_eq!(
                view(true, &[XTERM, (marker, "")]),
                (true, true),
                "empty {marker}"
            );
        }
        assert_eq!(view(true, &[XTERM, ("CLAUDECODE", "1")]), (true, true));
    }

    #[test]
    fn escapes_need_no_color_unset_clicolor_not_0_and_a_term() {
        assert_eq!(view(true, &[XTERM, ("NO_COLOR", "1")]), (true, false));
        assert_eq!(view(true, &[XTERM, ("NO_COLOR", "")]), (true, true));
        assert_eq!(view(true, &[XTERM, ("CLICOLOR", "0")]), (true, false));
        assert_eq!(view(true, &[XTERM, ("CLICOLOR", "1")]), (true, true));
        assert_eq!(view(true, &[("TERM", "dumb")]), (true, false));
        assert_eq!(view(true, &[("TERM", "")]), (true, false));
        assert_eq!(view(true, &[]), (true, false));
    }

    #[test]
    fn forced_colour_never_turns_the_plain_view_human() {
        let forced = [XTERM, ("CLICOLOR_FORCE", "1"), ("FORCE_COLOR", "1")];
        assert_eq!(view(false, &forced), (false, false));
        let agent = [
            XTERM,
            ("CLICOLOR_FORCE", "1"),
            ("FORCE_COLOR", "1"),
            ("AI_AGENT", "x"),
        ];
        assert_eq!(view(true, &agent), (false, false));
    }

    struct Locale {
        macos: bool,
        env: &'static [(&'static str, &'static str)],
        unicode: bool,
    }

    #[test]
    fn marks_are_unicode_on_macos_or_in_a_utf8_locale() {
        let cases = [
            Locale {
                macos: false,
                env: &[("LANG", "en_US.UTF-8")],
                unicode: true,
            },
            Locale {
                macos: false,
                env: &[("LANG", "C.utf8")],
                unicode: true,
            },
            Locale {
                macos: false,
                env: &[("LANG", "de_DE.UTF-8@euro")],
                unicode: true,
            },
            Locale {
                macos: false,
                env: &[("LANG", "C")],
                unicode: false,
            },
            Locale {
                macos: false,
                env: &[("LANG", "en_US.ISO-8859-1")],
                unicode: false,
            },
            Locale {
                macos: false,
                env: &[],
                unicode: false,
            },
            Locale {
                macos: false,
                env: &[("LC_ALL", "C"), ("LANG", "en_US.UTF-8")],
                unicode: false,
            },
            Locale {
                macos: false,
                env: &[("LC_CTYPE", "en_US.UTF-8"), ("LANG", "C")],
                unicode: true,
            },
            Locale {
                macos: false,
                env: &[("LC_ALL", ""), ("LANG", "en_US.UTF-8")],
                unicode: true,
            },
            Locale {
                macos: true,
                env: &[("LANG", "C")],
                unicode: true,
            },
            Locale {
                macos: true,
                env: &[],
                unicode: true,
            },
        ];
        for case in cases {
            let shown = unicode(case.macos, vars(case.env));
            assert_eq!(shown, case.unicode, "macos {} {:?}", case.macos, case.env);
        }
    }
}
