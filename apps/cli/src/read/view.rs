use std::fmt::Write;
use std::time::SystemTime;

use bilbo_kinds::read::outline::Line;
use bilbo_kinds::read::page::Block;
use bilbo_kinds::read::refs::SCHEME;
use bilbo_kinds::read::{About, Body, Budget, Output};
use bilbo_store::entries::RecordRange;

use crate::console::message::{group, sanitize};
use crate::console::theme::{Style, paint};
use crate::console::view::View;

struct Show<'v> {
    view: &'v View,
}

impl Show<'_> {
    fn painted(&self, style: Style, text: &str) -> String {
        paint(self.view, style, text)
    }

    fn number(&self, number: u64) -> String {
        if self.view.human {
            group(number)
        } else {
            number.to_string()
        }
    }

    /// A terminal gets a record's control characters escaped, whoever reads it; a pipe gets
    /// its bytes.
    fn body(&self, text: &str) -> String {
        if !self.view.terminal {
            return text.to_owned();
        }
        text.split('\n')
            .map(|line| {
                line.split('\t')
                    .map(sanitize)
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// What a pipe gets: the contract bytes.
pub fn plain(output: &Output, budget: Option<Budget>) -> String {
    shown(output, budget, &View::PLAIN)
}

/// What `plain` prints around the lines of an outline at its longest: the metadata of `frame`
/// and a `more:` line of the two last records asked, which no other trailer is longer than.
pub fn beside(frame: &Output, budget: Option<Budget>) -> String {
    let asked = frame.asked;
    let widest = (asked.count > 1).then(|| RecordRange {
        first: asked.first.saturating_add(asked.count - 2),
        count: 2,
    });
    let longest = Output {
        more: widest.or(frame.more),
        ..frame.clone()
    };
    plain(&longest, budget)
}

/// What the stream of `view` gets: a terminal never a raw control character, a person's also
/// grouped numbers and paint.
pub fn shown(output: &Output, budget: Option<Budget>, view: &View) -> String {
    render(output, budget, &Show { view })
}

/// `budget` is the one a flag gave: the next command carries it.
fn render(output: &Output, budget: Option<Budget>, show: &Show<'_>) -> String {
    let mut text = String::new();
    let outline = match &output.body {
        Body::Outline { about, lines } => {
            if let Some(about) = about {
                text += &about_lines(output, about, show);
            }
            for Line {
                records,
                text: line,
            } in lines
            {
                let locator = show.painted(Style::Name, &output.locator(*records).to_string());
                let _ = writeln!(text, "{locator} {}", show.body(line));
            }
            true
        }
        Body::Page(blocks) => {
            for block in blocks {
                text += &block_lines(output, block, show);
            }
            false
        }
    };
    text + &trailer(output, outline, budget, show)
}

fn about_lines(output: &Output, about: &About, show: &Show<'_>) -> String {
    let mut rows = vec![("kind", sanitize(&about.kind))];
    if let Some(origin) = &about.origin {
        rows.push(("origin", sanitize(origin.as_str())));
    }
    rows.push(("captured", utc(about.captured)));
    if let Some(started) = about.started {
        rows.push(("started", utc(started)));
    }
    if let Some(parent) = about.parent {
        rows.push(("parent", format!("{SCHEME}{parent}")));
    }
    rows.push(("records", show.number(output.total)));
    rows.push((
        "generation",
        if output.older {
            format!("{} of {}", output.generation.0, about.current.0)
        } else {
            output.generation.0.to_string()
        },
    ));
    let mut text = show.painted(Style::Name, &format!("{SCHEME}{}", output.id)) + "\n";
    for (name, value) in rows {
        let _ = writeln!(
            text,
            "{} {value}",
            show.painted(Style::Note, &format!("{name}:"))
        );
    }
    text + &show.painted(Style::Heading, "outline:") + "\n"
}

fn block_lines(output: &Output, block: &Block, show: &Show<'_>) -> String {
    let locator = output.locator(block.records).to_string();
    let mut text = format!(
        "{} {}",
        show.painted(Style::Name, &locator),
        show.body(&block.text)
    );
    // A cut always gets its own newline: what stands before the last line is then the record's
    // text, to the byte, also when the cut falls after a line end.
    if block.cut.is_some() || !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// What a page that ends in a cut says first on its last line: how much of the record is left
/// and the command that prints it whole.
fn cut_part(output: &Output, block: &Block, show: &Show<'_>) -> Option<String> {
    let cut = block.cut.as_ref()?;
    let command = format!(
        "bilbo read {}{} --budget {}",
        output.id,
        output.pinned(block.records),
        cut.budget
    );
    Some(format!(
        "{} {} more characters; {}; ",
        show.painted(Style::Note, "cut:"),
        show.number(u64::try_from(cut.characters).unwrap_or(u64::MAX)),
        show.painted(Style::Command, &command),
    ))
}

/// The one line of an output that is bilbo's own: a record's text can look like anything above
/// it, so a cut is said here too, never on a line of its own.
fn trailer(output: &Output, outline: bool, budget: Option<Budget>, show: &Show<'_>) -> String {
    let cut = match &output.body {
        Body::Page(blocks) => blocks
            .iter()
            .find_map(|block| cut_part(output, block, show)),
        Body::Outline { .. } => None,
    };
    cut.unwrap_or_default() + &last_part(output, outline, budget, show)
}

fn last_part(output: &Output, outline: bool, budget: Option<Budget>, show: &Show<'_>) -> String {
    let Some(more) = output.more else {
        let asked = if output.asked.count == 0 {
            String::new()
        } else {
            format!("{} of ", output.locator(output.asked))
        };
        let noun = if output.total == 1 {
            "record"
        } else {
            "records"
        };
        return format!(
            "{} {asked}{} {noun}\n",
            show.painted(Style::Note, "end:"),
            show.number(output.total)
        );
    };
    let mut command = format!("bilbo read {}{}", output.id, output.pinned(more));
    if outline {
        command += " --outline";
    }
    if let Some(budget) = budget {
        let _ = write!(command, " --budget {budget}");
    }
    format!(
        "{} {}\n",
        show.painted(Style::Note, "more:"),
        show.painted(Style::Command, &command)
    )
}

/// `2026-11-07 16:39 UTC`; a time before 1970 is 1970.
fn utc(time: SystemTime) -> String {
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // Days to a civil date, counted in 400-year eras from 0000-03-01.
    let shifted = days + 719_468;
    let (era, day_of_era) = (shifted / 146_097, shifted % 146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * march_month + 2) / 5 + 1;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    use bilbo_kinds::read::outline::Line;
    use bilbo_kinds::read::page::{Block, Cut};
    use bilbo_kinds::read::{About, Body, Budget, Output};
    use bilbo_store::entries::{Generation, Origin, RecordRange};
    use bilbo_store::id::Id;

    use super::{beside, plain, shown, utc};
    use crate::console::view::View;

    const ID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";

    fn range(first: u64, last: u64) -> RecordRange {
        RecordRange {
            first,
            count: last - first + 1,
        }
    }

    fn budget(tokens: usize) -> Budget {
        Budget::try_from(tokens).expect("a budget the key accepts")
    }

    fn output(body: Body, asked: RecordRange, more: Option<RecordRange>) -> Output {
        Output {
            id: Id::parse(ID).expect("id"),
            generation: Generation(2),
            older: false,
            total: 4970,
            asked,
            body,
            more,
        }
    }

    fn block(first: u64, last: u64, text: &str) -> Block {
        Block {
            records: range(first, last),
            text: text.to_owned(),
            cut: None,
        }
    }

    fn page() -> Output {
        output(
            Body::Page(vec![
                block(120, 120, "user: plant the beans"),
                block(121, 121, "assistant: done\ntool_use Bash:\ncommand: ls"),
                block(125, 127, "## Heading\nline\n"),
            ]),
            range(120, 260),
            Some(range(130, 260)),
        )
    }

    fn about() -> About {
        About {
            kind: "session".to_owned(),
            origin: Origin::file(Path::new("/h/.claude/projects/p/5c3a9e1f.jsonl")),
            captured: SystemTime::UNIX_EPOCH + Duration::from_secs(1_794_193_920),
            parent: Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DF"),
            started: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_794_069_552)),
            current: Generation(2),
        }
    }

    fn outline(about: Option<About>, more: Option<RecordRange>) -> Output {
        let line = |record: u64, text: &str| Line {
            records: range(record, record),
            text: text.to_owned(),
        };
        output(
            Body::Outline {
                about,
                lines: vec![line(3, "user: first prompt"), line(57, "user: second")],
            },
            range(1, 4970),
            more,
        )
    }

    #[test]
    fn a_page_is_each_block_after_its_locator_then_the_next_command() {
        assert_eq!(
            plain(&page(), None),
            "#r120 user: plant the beans\n\
             #r121 assistant: done\ntool_use Bash:\ncommand: ls\n\
             #r125-127 ## Heading\nline\n\
             more: bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g2:r130-260\n"
        );
    }

    #[test]
    fn the_last_page_ends_with_what_was_asked_and_how_many_records_there_are() {
        let last = Output {
            more: None,
            ..page()
        };
        assert!(plain(&last, None).ends_with("line\nend: #r120-260 of 4970 records\n"));
        let one = Output {
            total: 1,
            asked: range(1, 1),
            more: None,
            ..output(Body::Page(vec![block(1, 1, "x")]), range(1, 1), None)
        };
        assert_eq!(plain(&one, None), "#r1 x\nend: #r1 of 1 record\n");
        let nothing = output(Body::Page(Vec::new()), range(6, 8), None);
        assert_eq!(plain(&nothing, None), "end: #r6-8 of 4970 records\n");
        let empty = Output {
            total: 0,
            ..output(
                Body::Outline {
                    about: None,
                    lines: Vec::new(),
                },
                RecordRange { first: 1, count: 0 },
                None,
            )
        };
        assert_eq!(plain(&empty, None), "end: 0 records\n");
    }

    #[test]
    fn a_cut_record_says_how_much_is_left_and_the_command_that_prints_it_whole() {
        let cut = output(
            Body::Page(vec![Block {
                cut: Some(Cut {
                    characters: 41_200,
                    budget: budget(18_000),
                }),
                ..block(130, 130, "tool_result Bash:\nhalf a li")
            }]),
            range(120, 260),
            Some(range(131, 260)),
        );
        assert_eq!(
            plain(&cut, None),
            "#r130 tool_result Bash:\nhalf a li\n\
             cut: 41200 more characters; bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g2:r130 --budget 18000; \
             more: bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g2:r131-260\n"
        );
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        assert!(
            shown(&cut, None, &unpainted).ends_with(
                "\ncut: 41,200 more characters; bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g2:r130 \
                 --budget 18000; more: bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g2:r131-260\n"
            ),
            "the human view groups the number"
        );
    }

    #[test]
    fn a_cut_after_a_line_end_keeps_that_line_end_before_its_own() {
        let cut = output(
            Body::Page(vec![Block {
                cut: Some(Cut {
                    characters: 7,
                    budget: budget(1000),
                }),
                ..block(4, 4, "first line\n")
            }]),
            range(4, 4),
            None,
        );
        assert!(
            plain(&cut, None).starts_with("#r4 first line\n\ncut: 7 more characters; "),
            "{}",
            plain(&cut, None)
        );
        assert!(
            plain(&cut, None).ends_with(" --budget 1000; end: #r4 of 4970 records\n"),
            "nothing is left after the cut record: {}",
            plain(&cut, None)
        );
    }

    #[test]
    fn a_budget_given_by_flag_is_carried_by_the_next_command() {
        assert!(
            plain(&page(), Some(budget(900))).ends_with(
                "more: bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g2:r130-260 --budget 900\n"
            )
        );
        assert!(
            plain(&outline(None, Some(range(58, 4970))), Some(budget(900))).ends_with(
                "more: bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g2:r58-4970 --outline --budget 900\n"
            )
        );
    }

    #[test]
    fn an_older_generation_is_in_every_locator_and_command() {
        let older = Output {
            generation: Generation(1),
            older: true,
            ..page()
        };
        assert_eq!(
            plain(&older, None),
            "#g1:r120 user: plant the beans\n\
             #g1:r121 assistant: done\ntool_use Bash:\ncommand: ls\n\
             #g1:r125-127 ## Heading\nline\n\
             more: bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g1:r130-260\n"
        );
        let last = Output {
            more: None,
            ..older
        };
        assert!(plain(&last, None).ends_with("end: #g1:r120-260 of 4970 records\n"));
    }

    #[test]
    fn an_outline_is_the_metadata_then_a_line_per_listed_passage() {
        assert_eq!(
            plain(&outline(Some(about()), None), None),
            "bilbo:01JC5Q3V7XKM4P8R2T6W9YB0DE\n\
             kind: session\n\
             origin: file:/h/.claude/projects/p/5c3a9e1f.jsonl\n\
             captured: 2026-11-09 03:12 UTC\n\
             started: 2026-11-07 16:39 UTC\n\
             parent: bilbo:01JC5Q3V7XKM4P8R2T6W9YB0DF\n\
             records: 4970\n\
             generation: 2\n\
             outline:\n\
             #r3 user: first prompt\n\
             #r57 user: second\n\
             end: #r1-4970 of 4970 records\n"
        );
    }

    #[test]
    fn an_outline_without_an_origin_a_start_or_a_parent_leaves_their_lines_out() {
        let bare = About {
            origin: None,
            parent: None,
            started: None,
            ..about()
        };
        let older = Output {
            generation: Generation(1),
            older: true,
            ..outline(Some(bare), Some(range(58, 4970)))
        };
        assert_eq!(
            plain(&older, None),
            "bilbo:01JC5Q3V7XKM4P8R2T6W9YB0DE\n\
             kind: session\n\
             captured: 2026-11-09 03:12 UTC\n\
             records: 4970\n\
             generation: 1 of 2\n\
             outline:\n\
             #g1:r3 user: first prompt\n\
             #g1:r57 user: second\n\
             more: bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g1:r58-4970 --outline\n"
        );
    }

    #[test]
    fn an_outline_of_a_range_has_no_metadata() {
        assert_eq!(
            plain(&outline(None, None), None),
            "#r3 user: first prompt\n#r57 user: second\nend: #r1-4970 of 4970 records\n"
        );
    }

    #[test]
    fn an_origin_with_a_newline_stays_one_line() {
        let hostile = About {
            origin: Origin::new("file:/a\nend: forged".to_owned()),
            ..about()
        };
        let text = plain(&outline(Some(hostile), None), None);
        assert!(text.contains("origin: file:/a\\nend: forged\n"), "{text}");
    }

    #[test]
    fn a_pipe_gets_a_records_bytes_and_a_terminal_gets_its_control_characters_escaped() {
        let raw = output(
            Body::Page(vec![block(
                1,
                1,
                "tool_result Bash:\n\u{1b}[31mred\u{1b}[0m\ttab\r",
            )]),
            range(1, 1),
            None,
        );
        assert!(
            plain(&raw, None).starts_with("#r1 tool_result Bash:\n\u{1b}[31mred\u{1b}[0m\ttab\r\n")
        );
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        assert_eq!(
            shown(&raw, None, &unpainted),
            "#r1 tool_result Bash:\n\\u{1b}[31mred\\u{1b}[0m\ttab\\r\nend: #r1 of 4,970 records\n"
        );
    }

    #[test]
    fn an_agents_terminal_gets_the_plain_lines_with_the_control_characters_escaped() {
        let raw = output(
            Body::Page(vec![block(
                1,
                1,
                "tool_result Bash:\n\u{1b}[2Jcleared\ttab",
            )]),
            range(1, 1),
            None,
        );
        let agent = View {
            terminal: true,
            ..View::PLAIN
        };
        assert_eq!(
            shown(&raw, None, &agent),
            "#r1 tool_result Bash:\n\\u{1b}[2Jcleared\ttab\nend: #r1 of 4970 records\n",
            "escaped, and neither grouped nor painted"
        );
        let listed = Output {
            body: Body::Outline {
                about: None,
                lines: vec![Line {
                    records: range(3, 3),
                    text: "user: \u{1b}]0;title\u{7}".to_owned(),
                }],
            },
            ..raw
        };
        assert!(!shown(&listed, None, &agent).contains('\u{1b}'));
        assert!(
            plain(&listed, None).contains('\u{1b}'),
            "a pipe gets the bytes"
        );
    }

    #[test]
    fn a_command_always_names_the_generation_it_read() {
        let newest = plain(&page(), None);
        assert!(
            newest.starts_with("#r120 user: "),
            "a locator shown is short: {newest}"
        );
        assert!(newest.ends_with("more: bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g2:r130-260\n"));
    }

    #[test]
    fn the_human_view_groups_numbers_and_is_the_plain_text_without_its_paint() {
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        let text = shown(&outline(Some(about()), None), None, &unpainted);
        assert!(text.contains("records: 4,970\n"), "{text}");
        assert!(text.ends_with("end: #r1-4970 of 4,970 records\n"), "{text}");
        assert_eq!(shown(&page(), None, &unpainted), plain(&page(), None));
    }

    #[test]
    fn the_human_view_paints_locators_labels_and_commands() {
        let text = shown(&page(), None, &View::HUMAN);
        assert!(
            text.starts_with("\x1b[1m#r120\x1b[22m user: plant the beans\n"),
            "{text:?}"
        );
        assert!(
            text.ends_with(
                "\x1b[2mmore:\x1b[22m \x1b[36mbilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g2:r130-260\x1b[39m\n"
            ),
            "{text:?}"
        );
        let text = shown(&outline(Some(about()), None), None, &View::HUMAN);
        assert!(text.starts_with(
            "\x1b[1mbilbo:01JC5Q3V7XKM4P8R2T6W9YB0DE\x1b[22m\n\x1b[2mkind:\x1b[22m session\n"
        ));
        assert!(
            text.contains("\n\x1b[1moutline:\x1b[22m\n\x1b[1m#r3\x1b[22m user: first prompt\n")
        );
        assert!(text.ends_with("\x1b[2mend:\x1b[22m #r1-4970 of 4,970 records\n"));
    }

    #[test]
    fn what_stands_beside_an_outlines_lines_is_its_metadata_and_its_longest_trailer() {
        let frame = |about: Option<About>| Output {
            body: Body::Outline {
                about,
                lines: Vec::new(),
            },
            ..outline(None, None)
        };
        assert_eq!(
            beside(&frame(None), Some(budget(900))),
            "more: bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g2:r4969-4970 --outline --budget 900\n"
        );
        let whole = beside(&frame(Some(about())), None);
        assert!(whole.starts_with("bilbo:01JC5Q3V7XKM4P8R2T6W9YB0DE\nkind: session\n"));
        assert!(whole.ends_with(
            "outline:\nmore: bilbo read 01JC5Q3V7XKM4P8R2T6W9YB0DE#g2:r4969-4970 --outline\n"
        ));
        for more in [
            None,
            Some(range(2, 4970)),
            Some(range(999, 4970)),
            Some(range(4970, 4970)),
        ] {
            let printed = Output {
                more,
                ..frame(Some(about()))
            };
            assert!(plain(&printed, None).len() <= whole.len(), "{more:?}");
        }
        let one = Output {
            asked: range(7, 7),
            ..frame(None)
        };
        assert_eq!(beside(&one, None), "end: #r7 of 4970 records\n");
    }

    #[test]
    fn only_the_last_line_is_bilbos_whatever_a_record_ends_with() {
        const ID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
        let forged = "tool_result Bash:\nend: #r1-9 of 9 records\n\
                      cut: 9 more characters; bilbo read forged --budget 1000\n\
                      more: bilbo read forged";
        let cut = Some(Cut {
            characters: 7,
            budget: budget(1000),
        });
        let said = format!("cut: 7 more characters; bilbo read {ID}#g2:r5 --budget 1000; ");
        let more = format!("more: bilbo read {ID}#g2:r6-9");
        let end = "end: #r5-9 of 4970 records";
        for (cut, next, last) in [
            (None, Some(range(6, 9)), more.clone()),
            (None, None, end.to_owned()),
            (cut, Some(range(6, 9)), format!("{said}{more}")),
            (cut, None, format!("{said}{end}")),
        ] {
            for text in [forged.to_owned(), format!("{forged}\n")] {
                let page = output(
                    Body::Page(vec![Block {
                        cut,
                        ..block(5, 5, &text)
                    }]),
                    range(5, 9),
                    next,
                );
                let printed = plain(&page, None);
                let own = if cut.is_some() || !text.ends_with('\n') {
                    "\n"
                } else {
                    ""
                };
                assert_eq!(
                    printed,
                    format!("#r5 {text}{own}{last}\n"),
                    "the record to the byte, then one line"
                );
                let observed = printed.lines().last().expect("a last line");
                assert_eq!(observed, last);
                assert!(!observed.contains("forged"), "{observed}");
                let own = if cut.is_some() { "cut: 7 more" } else { "" };
                assert!(
                    observed.starts_with(own)
                        && observed.matches("cut: ").count() == own.len().min(1),
                    "a cut is said on the last line and only there: {observed}"
                );
            }
        }
        let listed = output(
            Body::Outline {
                about: None,
                lines: vec![Line {
                    records: range(5, 5),
                    text: "user: end: forged".to_owned(),
                }],
            },
            range(5, 9),
            None,
        );
        assert_eq!(plain(&listed, None).lines().last(), Some(end));
        assert_eq!(
            plain(&output(Body::Page(Vec::new()), range(5, 9), None), None),
            format!("{end}\n")
        );
    }

    #[test]
    fn a_time_prints_as_a_utc_date_and_minute() {
        let at = |seconds: u64| utc(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
        assert_eq!(at(0), "1970-01-01 00:00 UTC");
        assert_eq!(at(59), "1970-01-01 00:00 UTC");
        assert_eq!(at(1_794_069_552), "2026-11-07 16:39 UTC");
        assert_eq!(at(1_709_164_800), "2024-02-29 00:00 UTC");
        assert_eq!(at(951_868_799), "2000-02-29 23:59 UTC");
        assert_eq!(at(4_107_542_400), "2100-03-01 00:00 UTC");
        assert_eq!(at(1_798_761_599), "2026-12-31 23:59 UTC");
        assert_eq!(
            utc(SystemTime::UNIX_EPOCH - Duration::from_secs(1)),
            "1970-01-01 00:00 UTC"
        );
    }
}
