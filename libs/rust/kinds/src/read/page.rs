use bilbo_store::entries::RecordRange;

use crate::read::Budget;
use crate::read::refs::last;
use crate::reader::Passage;
use crate::tokens::Tokens;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub records: RecordRange,
    pub text: String,
    pub cut: Option<Cut>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut {
    pub characters: usize,
    /// A budget that prints the record whole when it is read alone under the locator a command
    /// carries.
    pub budget: Budget,
}

pub(crate) struct Locators<'l> {
    pub shown: &'l dyn Fn(RecordRange) -> String,
    /// In a command to run later: with the generation, so never shorter than `shown`.
    pub pinned: &'l dyn Fn(RecordRange) -> String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Page {
    pub blocks: Vec<Block>,
    pub more: Option<RecordRange>,
}

struct Unit<'p> {
    passage: usize,
    record: u64,
    rest: u64,
    text: &'p str,
}

/// Offsets a reader got wrong give an empty text, never a panic.
fn units(passages: &[Passage], asked: RecordRange) -> impl Iterator<Item = Unit<'_>> {
    let asked_last = last(asked);
    passages
        .iter()
        .enumerate()
        .flat_map(move |(passage, found)| {
            let passage_last = last(found.range).min(asked_last);
            found.ends.iter().enumerate().map(move |(at, &end)| {
                let start = at.checked_sub(1).map_or(0, |before| found.ends[before]);
                let at = u64::try_from(at).unwrap_or(u64::MAX);
                let record = found.range.first.saturating_add(at);
                Unit {
                    passage,
                    record,
                    rest: passage_last.saturating_sub(record).saturating_add(1),
                    text: found.text.get(start..end).unwrap_or_default(),
                }
            })
        })
        .filter(move |unit| {
            asked.count > 0 && unit.record >= asked.first && unit.record <= asked_last
        })
}

fn newline(text: &str) -> usize {
    usize::from(!text.ends_with('\n'))
}

fn body(head: usize, text: &str, tokens: &dyn Tokens) -> usize {
    head.saturating_add(tokens.count(text))
        .saturating_add(newline(text))
}

pub(crate) fn page(
    passages: &[Passage],
    asked: RecordRange,
    locators: &Locators<'_>,
    budget: usize,
    tokens: &dyn Tokens,
) -> Page {
    let locator = locators.shown;
    let asked_last = last(asked);
    let from = |record: u64| RecordRange {
        first: record,
        count: asked_last - record + 1,
    };
    let mut page = Page::default();
    let mut open = None;
    let mut spent = 0usize;
    let mut units = units(passages, asked).peekable();
    while let Some(unit) = units.next() {
        let continues = open == Some(unit.passage);
        // The longest locator the block can end with, so the count never falls short.
        let head = if continues {
            0
        } else {
            let whole = RecordRange {
                first: unit.record,
                count: unit.rest,
            };
            tokens.count(&locator(whole)).saturating_add(1)
        };
        let cost = body(head, unit.text, tokens);
        if spent.saturating_add(cost) <= budget {
            spent = spent.saturating_add(cost);
            match page.blocks.last_mut().filter(|_| continues) {
                Some(block) => {
                    block.text.push_str(unit.text);
                    block.records.count += 1;
                }
                None => page.blocks.push(Block {
                    records: RecordRange {
                        first: unit.record,
                        count: 1,
                    },
                    text: unit.text.to_owned(),
                    cut: None,
                }),
            }
            open = Some(unit.passage);
            continue;
        }
        if !page.blocks.is_empty() {
            page.more = Some(from(unit.record));
            break;
        }
        let alone = RecordRange {
            first: unit.record,
            count: 1,
        };
        let head = tokens.count(&locator(alone)).saturating_add(1);
        let whole = body(head, unit.text, tokens);
        // Under the shorter locator of the record alone it may fit after all.
        if whole <= budget {
            spent = whole;
            page.blocks.push(Block {
                records: alone,
                text: unit.text.to_owned(),
                cut: None,
            });
            continue;
        }
        // One more for the newline a cut always gets.
        let room = budget.saturating_sub(head.saturating_add(1));
        let kept = tokens.prefix(unit.text, room).min(unit.text.len());
        let (kept, left) = unit.text.split_at_checked(kept).unwrap_or((unit.text, ""));
        page.blocks.push(Block {
            records: alone,
            text: kept.to_owned(),
            cut: (!left.is_empty()).then(|| {
                let pinned = tokens.count(&(locators.pinned)(alone)).saturating_add(1);
                Cut {
                    characters: left.chars().count(),
                    budget: Budget::holding(body(pinned, unit.text, tokens)),
                }
            }),
        });
        page.more = units.peek().map(|next| from(next.record));
        break;
    }
    page
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::{Generation, RecordRange};

    use super::{Block, Cut, Locators, Page, page};
    use crate::fake::{Chars, section, single};
    use crate::read::Budget;
    use crate::read::refs::{Locator, last};
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

    fn pinned(records: RecordRange) -> String {
        Locator {
            generation: Some(Generation(1)),
            records: Some(records),
        }
        .to_string()
    }

    const NEWEST: Locators<'static> = Locators {
        shown: &locator,
        pinned: &pinned,
    };
    const OLDER: Locators<'static> = Locators {
        shown: &pinned,
        pinned: &pinned,
    };

    fn paged(passages: &[Passage], asked: RecordRange, budget: usize) -> Page {
        page(passages, asked, &NEWEST, budget, &Chars)
    }

    fn block(first: u64, last: u64, text: &str) -> Block {
        Block {
            records: range(first, last),
            text: text.to_owned(),
            cut: None,
        }
    }

    fn session() -> Vec<Passage> {
        vec![
            single(2, "user: one"),
            single(3, "assistant: two\nlines"),
            single(5, "tool_use Bash:\nls"),
            single(9, "user: last"),
        ]
    }

    fn chain(passages: &[Passage], asked: RecordRange, budget: usize) -> Vec<Page> {
        let mut pages = Vec::new();
        let mut next = Some(asked);
        while let Some(asked) = next {
            let page = paged(passages, asked, budget);
            next = page.more;
            pages.push(page);
            assert!(pages.len() <= 100, "the chain does not end");
        }
        pages
    }

    fn printed(pages: &[Page]) -> Vec<u64> {
        pages
            .iter()
            .flat_map(|page| &page.blocks)
            .flat_map(|block| block.records.first..=last(block.records))
            .collect()
    }

    #[test]
    fn a_page_holds_each_rendered_record_of_the_range_after_its_locator() {
        let page = paged(&session(), range(1, 9), 1000);
        assert_eq!(
            page.blocks,
            [
                block(2, 2, "user: one"),
                block(3, 3, "assistant: two\nlines"),
                block(5, 5, "tool_use Bash:\nls"),
                block(9, 9, "user: last"),
            ]
        );
        assert_eq!(page.more, None);
    }

    #[test]
    fn a_range_prints_only_its_records_and_one_with_no_rendered_record_prints_nothing() {
        assert_eq!(
            paged(&session(), range(3, 5), 1000).blocks,
            [
                block(3, 3, "assistant: two\nlines"),
                block(5, 5, "tool_use Bash:\nls")
            ]
        );
        assert_eq!(paged(&session(), range(6, 8), 1000), Page::default());
        assert_eq!(paged(&session(), range(4, 4), 1000), Page::default());
        assert_eq!(paged(&[], range(1, 9), 1000), Page::default());
    }

    #[test]
    fn a_page_stops_before_the_record_that_does_not_fit_and_more_starts_there() {
        // "#r2 user: one\n" is 14 characters, "#r3 assistant: two\nlines\n" 25.
        let page = paged(&session(), range(1, 9), 14 + 25);
        assert_eq!(printed(std::slice::from_ref(&page)), [2, 3]);
        assert_eq!(page.more, Some(range(5, 9)));
        let page = paged(&session(), range(1, 9), 14 + 24);
        assert_eq!(printed(std::slice::from_ref(&page)), [2]);
        assert_eq!(page.more, Some(range(3, 9)));
    }

    #[test]
    fn more_names_the_next_record_that_renders_and_stops_at_the_end_of_the_range() {
        let page = paged(&session(), range(1, 6), 14);
        assert_eq!(page.more, Some(range(3, 6)));
        let page = paged(&session(), range(3, 6), 25);
        assert_eq!(page.more, Some(range(5, 6)));
        let page = paged(&session(), range(5, 6), 100);
        assert_eq!(page.more, None, "records 6 renders nothing");
    }

    #[test]
    fn following_more_prints_each_rendered_record_exactly_once_at_any_budget() {
        for budget in [1, 5, 14, 20, 26, 40, 64, 1000] {
            for asked in [range(1, 9), range(2, 5), range(3, 3), range(4, 9)] {
                let pages = chain(&session(), asked, budget);
                let expected: Vec<u64> = [2, 3, 5, 9]
                    .into_iter()
                    .filter(|record| (asked.first..=last(asked)).contains(record))
                    .collect();
                assert_eq!(printed(&pages), expected, "budget {budget} {asked:?}");
            }
        }
    }

    #[test]
    fn a_record_over_the_budget_alone_is_cut_on_a_char_boundary_and_more_goes_on_after_it() {
        let passages = [single(1, "user: ação e reação"), single(2, "user: next")];
        // "#r1 " is 4 and the newline 1, so 6 characters of the text fit a budget of 11.
        let page = paged(&passages, range(1, 2), 11);
        assert_eq!(
            page.blocks,
            [Block {
                records: range(1, 1),
                text: "user: ".to_owned(),
                cut: Some(Cut {
                    characters: 13,
                    budget: Budget::holding(1000),
                }),
            }]
        );
        assert_eq!(page.more, Some(range(2, 2)));
        let page = paged(&passages, range(1, 2), 13);
        assert_eq!(page.blocks[0].text, "user: aç");
        assert_eq!(page.blocks[0].cut.map(|cut| cut.characters), Some(11));
    }

    #[test]
    fn the_budget_a_cut_suggests_prints_the_record_whole_when_it_is_read_alone() {
        // "#g1:r7 " is 7 and the newline 1, so 992 characters cost 1000 under it.
        for length in [987, 991, 992, 993, 1991, 1992, 1993, 2500] {
            let passages = [single(7, &"x".repeat(length))];
            let cut = paged(&passages, range(7, 7), 50).blocks[0]
                .cut
                .expect("a cut");
            assert_eq!(cut.characters, length - 45, "{length}");
            let budget = cut.budget.tokens();
            assert_eq!(budget % 1000, 0);
            for locators in [&NEWEST, &OLDER] {
                let whole = page(&passages, range(7, 7), locators, budget, &Chars);
                assert_eq!(whole.blocks, [block(7, 7, &"x".repeat(length))], "{length}");
            }
            let short = page(&passages, range(7, 7), &OLDER, budget - 1000, &Chars);
            assert!(
                short.blocks[0].cut.is_some(),
                "{length}: the budget is the least round one"
            );
        }
    }

    #[test]
    fn the_budget_a_cut_suggests_is_counted_under_the_locator_its_command_carries() {
        // "#r7 ", 995 characters and the newline cost 1000; "#g1:r7 " costs 3 more, which the
        // command's page shows once a newer generation exists.
        let passages = [single(7, &"x".repeat(995))];
        assert!(paged(&passages, range(7, 7), 1000).blocks[0].cut.is_none());
        let cut = paged(&passages, range(7, 7), 50).blocks[0]
            .cut
            .expect("a cut");
        assert_eq!(cut.budget.tokens(), 2000);
        let later = page(&passages, range(7, 7), &OLDER, cut.budget.tokens(), &Chars);
        assert_eq!(later.blocks, [block(7, 7, &"x".repeat(995))]);
    }

    #[test]
    fn a_record_that_fits_under_its_own_locator_is_not_cut_for_the_wider_one() {
        let long = format!("{}\n", "x".repeat(40));
        let note = [section(1, &[&long, "y\n"])];
        // "#r1-2 " and the line are 47 characters, "#r1 " and the line 45.
        let page = paged(&note, range(1, 2), 45);
        assert_eq!(page.blocks, [block(1, 1, &long)]);
        assert_eq!(page.more, Some(range(2, 2)));
        let page = paged(&note, range(1, 2), 44);
        assert!(
            page.blocks[0].cut.is_some(),
            "one less and it is over the budget alone"
        );
        let pages = chain(&note, range(1, 2), 45);
        assert_eq!(printed(&pages), [1, 2]);
        let roomy = paged(&note, range(1, 2), 49);
        assert_eq!(roomy.blocks, [block(1, 2, &format!("{long}y\n"))]);
    }

    #[test]
    fn the_budget_a_cut_suggests_is_never_more_than_the_key_accepts() {
        struct Heavy;
        impl crate::tokens::Tokens for Heavy {
            fn count(&self, text: &str) -> usize {
                text.len().saturating_mul(usize::MAX / 8)
            }
            fn prefix(&self, _text: &str, _budget: usize) -> usize {
                1
            }
        }
        let page = page(
            &[single(1, "user: text")],
            range(1, 1),
            &NEWEST,
            256,
            &Heavy,
        );
        assert_eq!(
            page.blocks[0].cut.map(|cut| cut.budget.tokens()),
            Some(crate::read::MOST)
        );
    }

    #[test]
    fn a_cut_record_counts_as_printed_once() {
        let passages = [
            single(1, "user: a"),
            single(2, &"y".repeat(500)),
            single(3, "user: b"),
        ];
        let pages = chain(&passages, range(1, 3), 30);
        assert_eq!(printed(&pages), [1, 2, 3]);
        assert_eq!(pages.len(), 3);
        assert!(pages[1].blocks[0].cut.is_some());
        assert_eq!(pages[1].more, Some(range(3, 3)));
    }

    #[test]
    fn a_budget_smaller_than_the_locator_still_moves_on() {
        let pages = chain(&session(), range(1, 9), 1);
        assert_eq!(printed(&pages), [2, 3, 5, 9]);
        assert!(pages.iter().all(|page| page.blocks[0].text.is_empty()));
    }

    fn note() -> Vec<Passage> {
        vec![
            section(1, &["# One\n", "first\n", "\n"]),
            section(4, &["## Two\n", "second\n", "third"]),
        ]
    }

    #[test]
    fn a_section_is_one_block_under_the_locator_of_its_records() {
        assert_eq!(
            paged(&note(), range(1, 6), 1000).blocks,
            [
                block(1, 3, "# One\nfirst\n\n"),
                block(4, 6, "## Two\nsecond\nthird"),
            ]
        );
    }

    #[test]
    fn a_range_that_starts_or_ends_inside_a_section_prints_the_records_it_holds() {
        assert_eq!(
            paged(&note(), range(2, 5), 1000).blocks,
            [block(2, 3, "first\n\n"), block(4, 5, "## Two\nsecond\n")]
        );
        assert_eq!(
            paged(&note(), range(5, 5), 1000).blocks,
            [block(5, 5, "second\n")]
        );
    }

    #[test]
    fn a_section_over_the_budget_goes_on_at_a_record_inside_it_and_is_not_cut() {
        // "#r1-3 # One\n" is 12 characters, "first\n" 6.
        let page = paged(&note(), range(1, 6), 18);
        assert_eq!(page.blocks, [block(1, 2, "# One\nfirst\n")]);
        assert_eq!(page.more, Some(range(3, 6)));
        for budget in [8, 12, 18, 19, 30, 1000] {
            let pages = chain(&note(), range(1, 6), budget);
            assert_eq!(printed(&pages), [1, 2, 3, 4, 5, 6], "budget {budget}");
            let cuts = pages
                .iter()
                .flat_map(|page| &page.blocks)
                .filter(|block| block.cut.is_some())
                .count();
            assert_eq!(
                cuts > 0,
                budget == 8,
                "only a line over the budget alone is cut: {budget}"
            );
        }
    }

    #[test]
    fn offsets_a_reader_got_wrong_print_nothing_and_do_not_panic() {
        let mut broken = single(1, "é");
        broken.ends = vec![1];
        let mut past = single(2, "ab");
        past.ends = vec![9];
        let mut falling = section(3, &["ab", "cd"]);
        falling.ends = vec![3, 1];
        let page = paged(&[broken, past, falling], range(1, 9), 1000);
        assert_eq!(printed(std::slice::from_ref(&page)), [1, 2, 3, 4]);
        assert!(
            page.blocks
                .iter()
                .all(|block| block.text == "abc" || block.text.is_empty())
        );
    }
}
