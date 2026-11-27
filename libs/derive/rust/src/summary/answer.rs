use crate::check::dropped_for_names;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Empty,
    Paragraphs,
    Opaque,
    Names,
}

impl Why {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Why::Empty => "empty",
            Why::Paragraphs => "paragraphs",
            Why::Opaque => "opaque",
            Why::Names => "names",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    Kept(String),
    None(Why),
}

#[must_use]
pub fn read(answer: &str, grounds: &str, share: f64) -> Read {
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return Read::None(Why::Empty);
    }
    if trimmed.lines().any(|line| line.trim().is_empty()) {
        return Read::None(Why::Paragraphs);
    }
    let text = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    if bilbo_store::opaque::placeholder(&text).is_some() {
        return Read::None(Why::Opaque);
    }
    if dropped_for_names(&text, grounds, share) {
        return Read::None(Why::Names);
    }
    Read::Kept(text)
}

#[cfg(test)]
mod tests {
    use super::{Read, Why, read};

    fn kept(text: &str) -> Read {
        Read::Kept(text.to_owned())
    }

    #[test]
    fn an_answer_is_kept_as_one_line_with_single_spaces() {
        assert_eq!(
            read("  The carrots\nwere   thinned.\t Done. \n", "", 0.0),
            kept("The carrots were thinned. Done.")
        );
    }

    #[test]
    fn an_answer_with_nothing_or_with_two_paragraphs_is_no_summary() {
        for text in ["", " \n\t"] {
            assert_eq!(read(text, "", 0.0), Read::None(Why::Empty));
        }
        for text in ["One.\n\nTwo.", "One.\n \t\nTwo."] {
            assert_eq!(read(text, "", 0.0), Read::None(Why::Paragraphs));
        }
        assert_eq!(read("One.\nTwo.", "", 0.0), kept("One. Two."));
        assert_eq!(Why::Empty.word(), "empty");
        assert_eq!(Why::Paragraphs.word(), "paragraphs");
    }

    #[test]
    fn an_answer_that_names_what_was_not_sent_is_no_summary() {
        let grounds = "The user edited cli.rs and ran cargo test.\n";
        assert_eq!(
            read("The user edited cli.rs.", grounds, 0.5),
            kept("The user edited cli.rs.")
        );
        assert_eq!(
            read("The user edited main.rs and lib.rs.", grounds, 0.5),
            Read::None(Why::Names)
        );
        assert_eq!(
            read("cli.rs and main.rs", grounds, 0.5),
            kept("cli.rs and main.rs")
        );
        assert_eq!(
            read("cli.rs and main.rs", grounds, 0.75),
            Read::None(Why::Names)
        );
        for answer in [
            "The user edited cli.rs.",
            "The user edited main.rs and lib.rs.",
            "cli.rs and main.rs",
        ] {
            assert_eq!(read(answer, grounds, 0.0), kept(answer));
        }
        assert_eq!(Why::Names.word(), "names");
    }

    #[test]
    fn an_answer_the_store_would_replace_is_no_summary() {
        assert_eq!(read(&"Z".repeat(256), "", 0.0), Read::None(Why::Opaque));
        assert_eq!(read(&"Z".repeat(255), "", 0.0), kept(&"Z".repeat(255)));
        assert_eq!(read(&"a".repeat(300), "", 0.0), kept(&"a".repeat(300)));
        assert_eq!(Why::Opaque.word(), "opaque");
    }
}
