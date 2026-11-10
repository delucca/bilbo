use bilbo_store::entries::RecordRange;

use crate::read::refs::last;
use crate::reader::Passage;
use crate::tokens::Tokens;

/// The characters of a passage's line an outline shows.
const WIDTH: usize = 100;
const ELLIPSIS: char = '…';

/// A listed passage: all its records, and the line its reader lists it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub records: RecordRange,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Listing {
    pub lines: Vec<Line>,
    /// From the next listed passage to the end of what was asked.
    pub more: Option<RecordRange>,
}

/// One line of at most `WIDTH` characters: the first of `text` that is not blank.
fn shown(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    match line.char_indices().nth(WIDTH) {
        Some((end, _)) => {
            let kept = line[..end]
                .char_indices()
                .next_back()
                .map_or(0, |(at, _)| at);
            format!("{}{ELLIPSIS}", &line[..kept])
        }
        None => line.to_owned(),
    }
}

/// The passages a reader marked, among those that start inside `asked`, while their lines fit
/// `budget`; a page lists at least one.
pub(crate) fn outline(
    passages: &[Passage],
    asked: RecordRange,
    locator: &dyn Fn(RecordRange) -> String,
    budget: usize,
    tokens: &dyn Tokens,
) -> Listing {
    let asked_last = last(asked);
    let mut listing = Listing::default();
    let mut spent = 0usize;
    let listed = passages.iter().filter_map(|passage| {
        let inside = asked.count > 0
            && passage.range.first >= asked.first
            && passage.range.first <= asked_last;
        passage
            .outline
            .as_deref()
            .filter(|_| inside)
            .map(|line| (passage, line))
    });
    for (passage, line) in listed {
        let text = shown(line);
        let cost = tokens
            .count(&locator(passage.range))
            .saturating_add(tokens.count(&text))
            .saturating_add(2);
        if !listing.lines.is_empty() && spent.saturating_add(cost) > budget {
            listing.more = Some(RecordRange {
                first: passage.range.first,
                count: asked_last - passage.range.first + 1,
            });
            break;
        }
        spent = spent.saturating_add(cost);
        listing.lines.push(Line {
            records: passage.range,
            text,
        });
    }
    listing
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::RecordRange;

    use super::{Line, Listing, WIDTH, outline, shown};
    use crate::fake::{Chars, section, single};
    use crate::read::refs::Locator;
    use crate::reader::Passage;

    fn range(first: u64, last: u64) -> RecordRange {
        RecordRange {
            first,
            count: last - first + 1,
        }
    }

    fn locator(records: RecordRange) -> String {
        Locator {
            generation: None,
            records: Some(records),
        }
        .to_string()
    }

    fn listed(passages: &[Passage], asked: RecordRange, budget: usize) -> Listing {
        outline(passages, asked, &locator, budget, &Chars)
    }

    fn prompt(record: u64, text: &str) -> Passage {
        Passage {
            outline: text.lines().next().map(str::to_owned),
            ..single(record, text)
        }
    }

    fn session() -> Vec<Passage> {
        vec![
            prompt(2, "user: first prompt\nits second line"),
            single(3, "assistant: an answer"),
            prompt(7, "user: second prompt"),
            single(8, "tool_use Bash:\nls"),
            prompt(12, "compact summary: what happened"),
        ]
    }

    fn line(first: u64, last: u64, text: &str) -> Line {
        Line {
            records: range(first, last),
            text: text.to_owned(),
        }
    }

    #[test]
    fn an_outline_lists_the_passages_their_reader_gave_a_line() {
        let listing = listed(&session(), range(1, 12), 1000);
        assert_eq!(
            listing.lines,
            [
                line(2, 2, "user: first prompt"),
                line(7, 7, "user: second prompt"),
                line(12, 12, "compact summary: what happened"),
            ]
        );
        assert_eq!(listing.more, None);
    }

    #[test]
    fn a_section_is_listed_with_all_its_records() {
        let mut one = section(1, &["# One\n", "first\n"]);
        one.outline = Some("One".to_owned());
        let mut two = section(3, &["\n", "## Two\n", "second\n"]);
        two.outline = Some("One > Two".to_owned());
        assert_eq!(
            listed(&[one, two], range(1, 5), 1000).lines,
            [line(1, 2, "One"), line(3, 5, "One > Two")]
        );
    }

    #[test]
    fn an_outline_is_paged_by_the_budget_and_more_starts_at_the_next_listed_passage() {
        // "#r2 user: first prompt\n" is 23 characters, "#r7 user: second prompt\n" 24.
        let listing = listed(&session(), range(1, 12), 23 + 24);
        assert_eq!(listing.lines.len(), 2);
        assert_eq!(listing.more, Some(range(12, 12)));
        let listing = listed(&session(), range(1, 12), 23 + 23);
        assert_eq!(listing.lines.len(), 1);
        assert_eq!(listing.more, Some(range(7, 12)));
        let rest = listed(&session(), range(7, 12), 1000);
        assert_eq!(rest.lines.len(), 2);
        assert_eq!(rest.more, None);
    }

    #[test]
    fn a_page_lists_at_least_one_passage_whatever_the_budget() {
        let mut next = Some(range(1, 12));
        let mut seen = Vec::new();
        while let Some(asked) = next {
            let listing = listed(&session(), asked, 1);
            assert_eq!(listing.lines.len(), 1);
            seen.push(listing.lines[0].records.first);
            next = listing.more;
        }
        assert_eq!(seen, [2, 7, 12]);
    }

    #[test]
    fn only_passages_that_start_in_the_range_are_listed() {
        assert_eq!(listed(&session(), range(3, 11), 1000).lines.len(), 1);
        assert_eq!(listed(&session(), range(3, 6), 1000), Listing::default());
        assert_eq!(listed(&[], range(1, 9), 1000), Listing::default());
    }

    #[test]
    fn a_long_line_is_cut_to_the_width_and_only_one_line_is_ever_shown() {
        let exact = "x".repeat(WIDTH);
        assert_eq!(shown(&exact), exact);
        let long = format!("{}é tail", "x".repeat(WIDTH - 1));
        let cut = shown(&long);
        assert_eq!(cut.chars().count(), WIDTH);
        assert_eq!(cut, format!("{}…", "x".repeat(WIDTH - 1)));
        assert_eq!(shown("\n  \nthird\nfourth"), "third");
        assert_eq!(shown("\n\n"), "");
        assert_eq!(shown(""), "");
    }
}
