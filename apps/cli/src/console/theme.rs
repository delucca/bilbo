use crate::console::view::View;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Bold,
    Dim,
    Red,
    Green,
    Yellow,
    Blue,
    Cyan,
}

impl Tone {
    fn codes(self) -> (&'static str, &'static str) {
        match self {
            Tone::Bold => ("\x1b[1m", "\x1b[22m"),
            Tone::Dim => ("\x1b[2m", "\x1b[22m"),
            Tone::Red => ("\x1b[31m", "\x1b[39m"),
            Tone::Green => ("\x1b[32m", "\x1b[39m"),
            Tone::Yellow => ("\x1b[33m", "\x1b[39m"),
            Tone::Blue => ("\x1b[34m", "\x1b[39m"),
            Tone::Cyan => ("\x1b[36m", "\x1b[39m"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Error,
    Warning,
    Done,
    Info,
}

impl Mark {
    pub fn glyph(self, unicode: bool) -> &'static str {
        let (unicode_glyph, ascii) = match self {
            Self::Error => ("■", "x"),
            Self::Warning => ("▲", "!"),
            Self::Done => ("◆", "*"),
            Self::Info => ("●", "i"),
        };
        if unicode { unicode_glyph } else { ascii }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Heading,
    Name,
    Command,
    Note,
    Slot,
    Mark(Mark),
}

impl Style {
    fn tone(self) -> Tone {
        match self {
            Self::Heading | Self::Name => Tone::Bold,
            Self::Command => Tone::Cyan,
            Self::Note | Self::Slot => Tone::Dim,
            Self::Mark(Mark::Error) => Tone::Red,
            Self::Mark(Mark::Warning) => Tone::Yellow,
            Self::Mark(Mark::Done) => Tone::Green,
            Self::Mark(Mark::Info) => Tone::Blue,
        }
    }
}

pub fn paint(view: &View, style: Style, text: &str) -> String {
    if !view.paint || text.is_empty() {
        return text.to_owned();
    }
    let (open, close) = style.tone().codes();
    format!("{open}{text}{close}")
}

#[cfg(test)]
mod tests {
    use crate::console::view::View;

    use super::{Mark, Style, Tone, paint};

    #[test]
    fn a_style_paints_only_when_the_view_paints() {
        assert_eq!(
            paint(&View::HUMAN, Style::Heading, "Usage:"),
            "\x1b[1mUsage:\x1b[22m"
        );
        assert_eq!(
            paint(&View::HUMAN, Style::Mark(Mark::Error), "■"),
            "\x1b[31m■\x1b[39m"
        );
        assert_eq!(paint(&View::HUMAN, Style::Note, ""), "");
        assert_eq!(paint(&View::PLAIN, Style::Heading, "Usage:"), "Usage:");
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        assert_eq!(paint(&unpainted, Style::Command, "x"), "x");
    }

    #[test]
    fn each_style_has_its_tone() {
        for (style, open, close) in [
            (Style::Heading, "\x1b[1m", "\x1b[22m"),
            (Style::Name, "\x1b[1m", "\x1b[22m"),
            (Style::Command, "\x1b[36m", "\x1b[39m"),
            (Style::Note, "\x1b[2m", "\x1b[22m"),
            (Style::Slot, "\x1b[2m", "\x1b[22m"),
            (Style::Mark(Mark::Error), "\x1b[31m", "\x1b[39m"),
            (Style::Mark(Mark::Warning), "\x1b[33m", "\x1b[39m"),
            (Style::Mark(Mark::Done), "\x1b[32m", "\x1b[39m"),
            (Style::Mark(Mark::Info), "\x1b[34m", "\x1b[39m"),
        ] {
            assert_eq!(
                paint(&View::HUMAN, style, "x"),
                format!("{open}x{close}"),
                "{style:?}"
            );
        }
    }

    #[test]
    fn every_tone_is_bold_dim_or_a_basic_colour() {
        for tone in [
            Tone::Bold,
            Tone::Dim,
            Tone::Red,
            Tone::Green,
            Tone::Yellow,
            Tone::Blue,
            Tone::Cyan,
        ] {
            let (open, close) = tone.codes();
            for code in [open, close] {
                let param: u8 = code
                    .strip_prefix("\x1b[")
                    .and_then(|c| c.strip_suffix('m'))
                    .and_then(|c| c.parse().ok())
                    .unwrap();
                assert!(
                    matches!(param, 1 | 2 | 22 | 31..=36 | 39),
                    "{tone:?} {code:?}"
                );
            }
        }
    }
}
