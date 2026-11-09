use clap::builder::StyledStr;
use clap::{Arg, ArgAction, Command};

use crate::console::outcome::Outcome;
use crate::console::theme::{Style, paint};
use crate::console::view::View;

/// `about` is the verb's overview row, `before_help` its page's first line. Usage forms join
/// with `\n  `; an unindented line after them is a note, left out of usage errors. The painter
/// goes by shape: an indented `bilbo ` line is a command, `<word>` a slot.
pub const VERB_TEMPLATE: &str = "{before-help}Usage:\n  {usage}\n\nOptions:\n{options}{after-help}";
pub const NO_OPTIONS_TEMPLATE: &str = "{before-help}Usage:\n  {usage}{after-help}";

pub fn shape(command: Command) -> Command {
    let template = if has_options(&command) {
        VERB_TEMPLATE
    } else {
        NO_OPTIONS_TEMPLATE
    };
    command.help_template(template).disable_help_flag(true).arg(
        Arg::new("help")
            .short('h')
            .long("help")
            .action(ArgAction::Help)
            .hide(true),
    )
}

fn has_options(command: &Command) -> bool {
    command
        .get_arguments()
        .any(|arg| !arg.is_positional() && !arg.is_hide_set())
}

pub fn overview(root: &Command) -> String {
    render(root, None).unwrap_or_default()
}

pub fn page(root: &Command, verb: &str) -> Option<String> {
    render(root, Some(verb))
}

fn render(root: &Command, verb: Option<&str>) -> Option<String> {
    let mut root = root.clone();
    root.build();
    let command = match verb {
        Some(verb) => root.find_subcommand_mut(verb)?,
        None => &mut root,
    };
    Some(normalize(&command.render_help()))
}

fn normalize(styled: &StyledStr) -> String {
    let mut text = styled.to_string().trim_end().to_owned();
    text.push('\n');
    text
}

pub fn outcome(text: String) -> Outcome {
    Outcome::new(text.clone()).with_human(move |view| styled(view, &text))
}

fn styled(view: &View, page: &str) -> String {
    let mut opening = true;
    page.lines()
        .map(|line| {
            opening &= !line.is_empty();
            let shown = if opening {
                line.to_owned()
            } else {
                styled_line(view, line)
            };
            shown + "\n"
        })
        .collect()
}

fn styled_line(view: &View, line: &str) -> String {
    if let Some(end) = heading_end(line) {
        let (label, value) = line.split_at(end);
        let value = match value.strip_prefix(' ') {
            Some(command) if command.starts_with("bilbo ") => {
                format!(" {}", slotted(view, Some(Style::Command), command))
            }
            Some(note) => format!(" {}", paint(view, Style::Note, note)),
            None => value.to_owned(),
        };
        return paint(view, Style::Heading, label) + &value;
    }
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent == 0 {
        return note(view, line);
    }
    let (space, text) = line.split_at(indent);
    if text.starts_with("bilbo ") {
        return format!("{space}{}", slotted(view, Some(Style::Command), text));
    }
    match column(line) {
        Some((indent, width)) => format!(
            "{}{}{}",
            &line[..indent],
            slotted(view, Some(Style::Name), &line[indent..indent + width]),
            slotted(view, None, &line[indent + width..])
        ),
        None => slotted(view, None, line),
    }
}

fn note(view: &View, line: &str) -> String {
    let mut shown = String::new();
    let mut rest = line;
    while let Some(quote) = rest.find("'bilbo ") {
        let start = quote + 1;
        let Some(length) = rest[start..].find('\'') else {
            break;
        };
        shown += &paint(view, Style::Note, &rest[..start]);
        shown += &slotted(view, Some(Style::Command), &rest[start..start + length]);
        rest = &rest[start + length..];
    }
    shown + &paint(view, Style::Note, rest)
}

/// Pieces are painted side by side, never nested: bold and dim share a close code.
fn slotted(view: &View, style: Option<Style>, text: &str) -> String {
    let literal = |piece: &str| match style {
        Some(style) => paint(view, style, piece),
        None => piece.to_owned(),
    };
    let mut shown = String::new();
    let (mut from, mut at) = (0, 0);
    while let Some(open) = text[at..].find('<').map(|found| at + found) {
        let close = text[open + 1..]
            .find('>')
            .map(|found| open + 1 + found)
            .filter(|&close| is_slot(&text[open + 1..close]));
        match close {
            Some(close) => {
                shown += &literal(&text[from..open]);
                shown += &paint(view, Style::Slot, &text[open..=close]);
                from = close + 1;
                at = from;
            }
            None => at = open + 1,
        }
    }
    shown + &literal(&text[from..])
}

fn is_slot(name: &str) -> bool {
    !name.is_empty() && !name.contains(|c: char| c == '<' || c.is_whitespace())
}

/// clap indents a long-only option six spaces, under options with a short flag.
fn column(line: &str) -> Option<(usize, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let width = rest.find("  ")?;
    let long_only = indent == 6 && rest.starts_with("--");
    ((indent == 2 && !rest.is_empty()) || long_only).then_some((indent, width))
}

fn heading_end(line: &str) -> Option<usize> {
    if !line.starts_with(|c: char| c.is_ascii_uppercase()) {
        return None;
    }
    let at = line.find(':')?;
    let (label, rest) = (&line[..at], &line[at + 1..]);
    let words = label
        .chars()
        .all(|c| c.is_ascii_alphabetic() || c == ' ' || c == ',');
    let ends = rest.is_empty() || (rest.starts_with(' ') && label.split(' ').count() <= 3);
    (words && ends).then_some(at + 1)
}

#[cfg(test)]
mod tests {
    use super::{has_options, overview, page, slotted, styled, styled_line};
    use crate::bilbo;
    use crate::console::args::verbs;
    use crate::console::view::View;
    use crate::probe::{self, with_verb};

    fn every_page() -> Vec<(String, String)> {
        let root = with_verb(bilbo(), probe::verb());
        let mut pages = vec![("overview".to_owned(), overview(&root))];
        for verb in verbs(&root) {
            let text = page(&root, &verb).expect("a registered verb has a page");
            pages.push((verb, text));
        }
        pages
    }

    #[test]
    fn no_help_line_is_longer_than_80_columns() {
        for (name, text) in every_page() {
            for line in text.lines() {
                assert!(line.chars().count() <= 80, "{name}: {line:?}");
            }
        }
    }

    #[test]
    fn every_verb_page_has_its_sections_in_order() {
        let root = with_verb(bilbo(), probe::verb());
        for (name, text) in every_page().into_iter().skip(1) {
            assert!(
                text.starts_with(&format!("bilbo {name}: ")),
                "{name}: {text}"
            );
            let verb = root.find_subcommand(&name).expect("a registered verb");
            let options = has_options(verb).then_some("\n\nOptions:\n  ");
            assert_eq!(
                text.contains("Options:"),
                options.is_some(),
                "{name}: {text}"
            );
            let mut from = 0;
            for heading in ["\n\nUsage:\n  bilbo "].into_iter().chain(options).chain([
                "\n\nOutput:\n",
                "\n\nExit:\n  0  ",
                "\n\nExamples:\n",
                "\n\nDocs: ",
            ]) {
                let at = text[from..].find(heading).unwrap_or_else(|| {
                    panic!("{name}: {heading:?} missing or out of order in\n{text}")
                });
                from += at + heading.len();
            }
            assert!(!text.contains("--help"), "{name}: {text}");
        }
    }

    #[test]
    fn a_verb_page_is_at_most_30_lines_and_the_overview_at_most_20() {
        for (name, text) in every_page() {
            let most = if name == "overview" { 20 } else { 30 };
            let lines = text.lines().count();
            assert!(lines <= most, "{name}: {lines} lines");
        }
    }

    #[test]
    fn no_page_has_more_than_two_unindented_lines_in_a_row() {
        for (name, text) in every_page() {
            let opening = text.lines().take_while(|line| !line.is_empty()).count();
            let most = if name == "overview" { 2 } else { 1 };
            assert!(opening <= most, "{name}: {opening} opening lines");
            let mut run = 0;
            for line in text.lines().skip(opening) {
                run = if line.is_empty() || line.starts_with(' ') {
                    0
                } else {
                    run + 1
                };
                assert!(run <= 2, "{name}: a paragraph at {line:?}");
            }
        }
    }

    #[test]
    fn every_page_has_one_docs_line_and_it_is_a_url() {
        for (name, text) in every_page() {
            let docs: Vec<&str> = text
                .lines()
                .filter_map(|line| line.strip_prefix("Docs: "))
                .collect();
            assert_eq!(docs.len(), 1, "{name}: {docs:?}");
            assert!(docs[0].starts_with("https://"), "{name}: {docs:?}");
        }
    }

    #[test]
    fn an_unregistered_verb_has_no_page() {
        assert_eq!(page(&bilbo(), "frob"), None);
    }

    fn strip(text: &str) -> String {
        let mut plain = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                chars.by_ref().find(|&c| c == 'm');
            } else {
                plain.push(c);
            }
        }
        plain
    }

    #[test]
    fn each_help_line_takes_the_style_of_its_role() {
        for (line, expected) in [
            ("Verbs:", "E[1mVerbs:E[22m"),
            (
                "Usage: bilbo <verb> [<args>]",
                "E[1mUsage:E[22m E[36mbilbo E[39mE[2m<verb>E[22mE[36m [E[39mE[2m<args>E[22mE[36m]E[39m",
            ),
            (
                "Docs: https://github.com/delucca/bilbo/wiki",
                "E[1mDocs:E[22m E[2mhttps://github.com/delucca/bilbo/wikiE[22m",
            ),
            (
                "  config    Show each setting",
                "  E[1mconfigE[22m    Show each setting",
            ),
            (
                "  2  usage or config error: an unknown key",
                "  E[1m2E[22m  usage or config error: an unknown key",
            ),
            (
                "  bilbo probe [options] <word>",
                "  E[36mbilbo probe [options] E[39mE[2m<word>E[22m",
            ),
            (
                "  bilbo remember notes/",
                "  E[36mbilbo remember notes/E[39m",
            ),
            (
                "  -k, --kind <kind>  Only this kind of note",
                "  E[1m-k, --kind E[22mE[2m<kind>E[22m  Only this kind of note",
            ),
            (
                "      --project <project>  Label each new entry with this project",
                "      E[1m--project E[22mE[2m<project>E[22m  Label each new entry with this project",
            ),
            (
                "      --all          Every kind",
                "      E[1m--allE[22m          Every kind",
            ),
            (
                "  new <n>, appended <n>",
                "  new E[2m<n>E[22m, appended E[2m<n>E[22m",
            ),
            (
                "  <key> = <value> (<origin>)",
                "  E[2m<key>E[22m = E[2m<value>E[22m (E[2m<origin>E[22m)",
            ),
            (
                "  verb  Label it <project>",
                "  E[1mverbE[22m  Label it E[2m<project>E[22m",
            ),
            ("  a <> b < c <two words> d", "  a <> b < c <two words> d"),
            (
                "Run 'bilbo <verb> --help' for its options, output and exit codes.",
                "E[2mRun 'E[22mE[36mbilbo E[39mE[2m<verb>E[22mE[36m --helpE[39mE[2m' for its options, output and exit codes.E[22m",
            ),
            (
                "Use 'bilbo a' or 'bilbo b <n>'.",
                "E[2mUse 'E[22mE[36mbilbo aE[39mE[2m' or 'E[22mE[36mbilbo b E[39mE[2m<n>E[22mE[2m'.E[22m",
            ),
            ("Run 'bilbo x", "E[2mRun 'bilbo xE[22m"),
            (
                "variable or a flag. With <key>, its value alone, a string without its quotes.",
                "E[2mvariable or a flag. With <key>, its value alone, a string without its quotes.E[22m",
            ),
            ("   continued", "   continued"),
            (
                "      text that is not an option  x",
                "      text that is not an option  x",
            ),
            ("", ""),
        ] {
            assert_eq!(
                styled_line(&View::HUMAN, line),
                expected.replace("E[", "\x1b["),
                "{line:?}"
            );
        }
    }

    #[test]
    fn a_slot_is_a_bracketed_word() {
        let pieces = |text: &str| slotted(&View::HUMAN, None, text).replace('\x1b', "E");
        assert_eq!(pieces("<n>"), "E[2m<n>E[22m");
        assert_eq!(pieces("<file|folder>"), "E[2m<file|folder>E[22m");
        assert_eq!(pieces("x<n>y"), "xE[2m<n>E[22my");
        assert_eq!(pieces("<<n>>"), "<E[2m<n>E[22m>");
        for text in ["<>", "a < b", "<two words>", "<", "a > b"] {
            assert_eq!(pieces(text), text);
        }
    }

    #[test]
    fn clap_option_rows_are_names_and_usage_forms_commands() {
        let root = with_verb(bilbo(), probe::verb());
        let page = page(&root, "probe").expect("a page");
        let rows: Vec<String> = page
            .lines()
            .filter(|line| line.trim_start().starts_with('-'))
            .map(|line| styled_line(&View::HUMAN, line))
            .collect();
        assert_eq!(rows.len(), 3, "{rows:?}");
        for (row, name) in rows.iter().zip([
            "\x1b[1m-k, --kind \x1b[22m\x1b[2m<kind>\x1b[22m",
            "\x1b[1m--all\x1b[22m",
            "\x1b[1m--times \x1b[22m\x1b[2m<n>\x1b[22m",
        ]) {
            assert!(row.contains(name), "{row:?}");
        }
        let forms: Vec<String> = page
            .lines()
            .filter(|line| line.starts_with("  bilbo probe "))
            .map(|line| styled_line(&View::HUMAN, line))
            .collect();
        assert_eq!(forms.len(), 3, "{forms:?}");
        for needle in [
            "\x1b[36mbilbo probe [options] \x1b[39m\x1b[2m<word>\x1b[22m",
            "\x1b[36mbilbo probe --all \x1b[39m\x1b[2m<word>\x1b[22m",
        ] {
            assert!(forms.iter().any(|form| form.contains(needle)), "{forms:?}");
        }
    }

    #[test]
    fn the_opening_lines_stay_plain() {
        for (name, text) in every_page() {
            let opening = if name == "overview" { 2 } else { 1 };
            let human = styled(&View::HUMAN, &text);
            for line in human.lines().take(opening) {
                assert!(!line.contains('\x1b'), "{name}: {line:?}");
            }
        }
    }

    #[test]
    fn a_painted_page_is_the_plain_page_plus_escapes() {
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        for (name, text) in every_page() {
            assert_eq!(strip(&styled(&View::HUMAN, &text)), text, "{name}");
            assert_eq!(styled(&unpainted, &text), text, "{name}");
        }
    }

    /// The weight and colour a line leaves open; 22 closes both bold and dim.
    fn left_open(line: &str) -> (bool, bool) {
        let (mut weight, mut colour) = (false, false);
        for code in line.split('\x1b').skip(1) {
            let param = code
                .strip_prefix('[')
                .and_then(|c| c.split_once('m'))
                .map(|(p, _)| p);
            match param {
                Some("1" | "2") => weight = true,
                Some("22") => weight = false,
                Some("31" | "32" | "33" | "34" | "35" | "36") => colour = true,
                Some("39") => colour = false,
                other => panic!("unexpected code {other:?} in {line:?}"),
            }
        }
        (weight, colour)
    }

    #[test]
    fn no_style_runs_past_a_line_end() {
        for (name, text) in every_page() {
            for line in styled(&View::HUMAN, &text).lines() {
                assert_eq!(left_open(line), (false, false), "{name}: {line:?}");
            }
        }
    }

    #[test]
    fn the_line_end_check_sees_a_colour_or_a_weight_left_open() {
        assert_eq!(left_open("\x1b[36mbilbo \x1b[2m<n>\x1b[22m"), (false, true));
        assert_eq!(left_open("\x1b[1mUsage:\x1b[39m"), (true, false));
        assert_eq!(
            left_open("\x1b[1mUsage:\x1b[22m \x1b[36mx\x1b[39m"),
            (false, false)
        );
        assert_eq!(left_open("plain"), (false, false));
    }
}
