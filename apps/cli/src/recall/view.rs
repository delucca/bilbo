use bilbo_kinds::read::refs::{Locator, SCHEME};
use bilbo_search::recall::Output;
use bilbo_search::recall::hit::Hit;

use crate::console::body::body;
use crate::console::message::line;
use crate::console::theme::{Style, paint};
use crate::console::time::day;
use crate::console::view::View;

fn content(hit: &Hit) -> String {
    match &hit.preface {
        Some(preface) => format!("{preface}\n{}", hit.text),
        None => hit.text.clone(),
    }
}

struct Header {
    reference: String,
    kind: String,
    day: String,
    lines: String,
    part: Option<String>,
    cut: bool,
}

fn header(hit: &Hit, content: &str) -> Header {
    let lines = content.matches('\n').count() + 1;
    let locator = Locator {
        generation: None,
        records: Some(hit.found.extent.records),
    };
    Header {
        reference: format!("{SCHEME}{}{locator}", hit.found.entry),
        kind: hit.found.kind.clone(),
        day: day(hit.found.extent.date),
        lines: format!("{lines} {}", if lines == 1 { "line" } else { "lines" }),
        part: hit
            .found
            .extent
            .part
            .map(|(at, of)| format!("part {}/{of}", at + 1)),
        cut: hit.cut,
    }
}

fn header_line(header: &Header, view: &View) -> String {
    let note = |text: &str| paint(view, Style::Note, text);
    let mut line = format!(
        "{} {}\t{}\t{}\t{}",
        note("==="),
        paint(view, Style::Name, &header.reference),
        note(&line(view, &header.kind)),
        note(&header.day),
        note(&header.lines),
    );
    if let Some(part) = &header.part {
        line = format!("{line}\t{}", note(part));
    }
    if header.cut {
        line = format!("{line}\t{}", note("cut"));
    }
    line
}

/// A reader takes the header, then exactly as many lines as it says: nothing scans the text.
pub fn hit(hit: &Hit) -> String {
    shown(hit, &View::PLAIN)
}

pub fn plain(output: &Output) -> String {
    output.hits.iter().map(hit).collect()
}

pub fn human(output: &Output, view: &View) -> String {
    output.hits.iter().map(|hit| shown(hit, view)).collect()
}

fn shown(hit: &Hit, view: &View) -> String {
    let content = content(hit);
    let header = header(hit, &content);
    format!("{}\n{}\n", header_line(&header, view), body(view, &content))
}

#[cfg(test)]
mod tests {
    use bilbo_search::index::Extent;
    use bilbo_search::lookup::Found;
    use bilbo_search::recall::hit::Hit;
    use bilbo_search::recall::{Output, Stand};
    use bilbo_store::entries::{Generation, RecordRange};
    use bilbo_store::id::Id;

    use super::{hit, human, plain};
    use crate::console::view::View;

    const ID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
    const DATE: i64 = 1_794_069_552_000;

    fn made(
        first: u64,
        count: u64,
        part: Option<(u32, u32)>,
        preface: Option<&str>,
        text: &str,
        cut: bool,
    ) -> Hit {
        Hit {
            found: Found {
                entry: Id::parse(ID).expect("an id"),
                kind: "session".to_owned(),
                extent: Extent {
                    generation: Generation(1),
                    records: RecordRange { first, count },
                    part,
                    date: DATE,
                },
                digest: 0,
            },
            preface: preface.map(str::to_owned),
            text: text.to_owned(),
            cut,
        }
    }

    fn output(hits: Vec<Hit>) -> Output {
        Output {
            hits,
            searched: 3,
            stand: Stand::Current,
            left_out: 0,
        }
    }

    fn unpainted() -> View {
        View {
            paint: false,
            ..View::HUMAN
        }
    }

    /// What a reader does: the header, then exactly the lines it counts, joined.
    fn content(printed: &str) -> (String, String) {
        let (header, rest) = printed.split_once('\n').expect("a header line");
        let lines: usize = header
            .split('\t')
            .nth(3)
            .and_then(|field| field.split(' ').next())
            .and_then(|count| count.parse().ok())
            .expect("a line count");
        let mut taken: Vec<&str> = rest.split('\n').collect();
        assert_eq!(
            taken.pop(),
            Some(""),
            "one line feed ends the hit: {printed:?}"
        );
        assert_eq!(taken.len(), lines, "{printed:?}");
        (header.to_owned(), taken.join("\n"))
    }

    #[test]
    fn a_hits_header_counts_its_lines_exactly() {
        let forged = "=== bilbo:01JC5Q3V7XKM4P8R2T6W9YB0DF#r1\tsession\t2026-11-07\t1 line\nx";
        for (preface, text) in [
            (None, "one line"),
            (None, "two\nlines"),
            (None, "ends with a line feed\n"),
            (None, "blank\n\nlines\n\n"),
            (None, ""),
            (None, forged),
            (
                Some("Session started 2026-11-07 08:01 UTC:"),
                "after a preface",
            ),
            (Some("Session date unknown:"), "a\nb\n"),
        ] {
            let one = made(5, 1, None, preface, text, false);
            let printed = hit(&one);
            let (header, joined) = content(&printed);
            let expected = match preface {
                Some(preface) => format!("{preface}\n{text}"),
                None => text.to_owned(),
            };
            assert_eq!(joined, expected, "{printed:?}");
            assert!(header.starts_with(&format!("=== bilbo:{ID}#r5\tsession\t2026-11-07\t")));
        }
        let two = format!(
            "{}{}",
            hit(&made(1, 2, None, None, "a", false)),
            hit(&one_line())
        );
        let (first, rest) = two.split_at(two.find("\n=== ").expect("a second hit") + 1);
        assert_eq!(content(first).1, "a");
        assert_eq!(content(rest).1, "b");
        assert_eq!(plain(&output(vec![one_line()])), hit(&one_line()));
    }

    fn one_line() -> Hit {
        made(9, 1, None, None, "b", false)
    }

    #[test]
    fn a_window_names_its_part_and_a_cut_hit_says_cut() {
        assert_eq!(
            hit(&made(4, 1, Some((0, 3)), None, "w", false)),
            format!("=== bilbo:{ID}#r4\tsession\t2026-11-07\t1 line\tpart 1/3\nw\n")
        );
        assert_eq!(
            hit(&made(4, 1, Some((2, 3)), None, "w\nx", true)),
            format!("=== bilbo:{ID}#r4\tsession\t2026-11-07\t2 lines\tpart 3/3\tcut\nw\nx\n")
        );
        assert_eq!(
            hit(&made(4, 1, None, None, "w", true)),
            format!("=== bilbo:{ID}#r4\tsession\t2026-11-07\t1 line\tcut\nw\n")
        );
    }

    #[test]
    fn a_single_record_and_a_range_are_written_as_read_takes_them() {
        let header = |first, count| {
            hit(&made(first, count, None, None, "x", false))
                .lines()
                .next()
                .map(str::to_owned)
                .expect("a header")
        };
        assert!(header(7, 1).starts_with(&format!("=== bilbo:{ID}#r7\t")));
        assert!(header(7, 4).starts_with(&format!("=== bilbo:{ID}#r7-10\t")));
    }

    #[test]
    fn a_terminal_escape_in_a_hit_is_shown_escaped() {
        let hostile = made(
            1,
            1,
            None,
            Some("pre\u{1b}[2J"),
            "a\u{1b}[31mred\tkept\rx\nb",
            false,
        );
        let out = output(vec![hostile]);
        for view in [View::HUMAN, unpainted()] {
            let shown = human(&out, &view);
            assert!(!shown.contains('\u{1b}') || view.paint, "{shown:?}");
            let body: Vec<&str> = shown.lines().skip(1).collect();
            assert_eq!(
                body,
                ["pre\\u{1b}[2J", "a\\u{1b}[31mred\tkept\\rx", "b"],
                "{shown:?}"
            );
        }
    }

    #[test]
    fn off_a_terminal_the_text_is_as_kept() {
        let raw = made(1, 1, None, None, "a\u{1b}[31mred\rx", false);
        let out = output(vec![raw]);
        assert!(plain(&out).contains("a\u{1b}[31mred\rx\n"));
        assert_eq!(human(&out, &View::PLAIN), plain(&out));
    }

    #[test]
    fn the_header_is_painted_only_for_a_person() {
        let out = output(vec![made(2, 1, Some((0, 2)), None, "text", true)]);
        let painted = human(&out, &View::HUMAN);
        let header = painted.lines().next().expect("a header");
        assert_eq!(
            header,
            format!(
                "\u{1b}[2m===\u{1b}[22m \u{1b}[1mbilbo:{ID}#r2\u{1b}[22m\t\u{1b}[2msession\u{1b}[22m\t\
                 \u{1b}[2m2026-11-07\u{1b}[22m\t\u{1b}[2m1 line\u{1b}[22m\t\u{1b}[2mpart 1/2\u{1b}[22m\t\
                 \u{1b}[2mcut\u{1b}[22m"
            )
        );
        assert_eq!(human(&out, &unpainted()), plain(&out));
        assert_eq!(painted.lines().nth(1), Some("text"));
    }
}
