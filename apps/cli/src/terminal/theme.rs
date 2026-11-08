use crate::terminal::View;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Bold,
    Dim,
    Red,
    Green,
    Yellow,
    Blue,
    Cyan,
}

impl Tone {
    /// Each tone closes with its own off-code, so tones nest.
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

pub fn paint(view: &View, tone: Tone, text: &str) -> String {
    if !view.paint || text.is_empty() {
        return text.to_owned();
    }
    let (open, close) = tone.codes();
    format!("{open}{text}{close}")
}

#[cfg(test)]
mod tests {
    use crate::terminal::View;

    use super::{Tone, paint};

    #[test]
    fn a_tone_wraps_the_text_only_when_the_view_paints() {
        assert_eq!(
            paint(&View::HUMAN, Tone::Bold, "Usage:"),
            "\x1b[1mUsage:\x1b[22m"
        );
        assert_eq!(paint(&View::HUMAN, Tone::Red, "■"), "\x1b[31m■\x1b[39m");
        assert_eq!(paint(&View::HUMAN, Tone::Dim, ""), "");
        assert_eq!(paint(&View::PLAIN, Tone::Bold, "Usage:"), "Usage:");
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        assert_eq!(paint(&unpainted, Tone::Cyan, "x"), "x");
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
