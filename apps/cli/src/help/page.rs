use clap::builder::StyledStr;
use clap::{Arg, ArgAction, Command};

use crate::console::outcome::Outcome;
use crate::console::theme::{Tone, paint};
use crate::console::view::View;

/// Usage forms joined with `\n  ` in `override_usage` sit two spaces under `Usage:`.
pub const VERB_TEMPLATE: &str = "{about}\n\nUsage:\n  {usage}\n\nOptions:\n{options}{after-help}";
pub const NO_OPTIONS_TEMPLATE: &str = "{about}\n\nUsage:\n  {usage}{after-help}";

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
    Outcome::new(text.clone())
        .with_human(move |view| text.lines().map(|line| bold(view, line) + "\n").collect())
}

pub fn bold(view: &View, line: &str) -> String {
    let strong = |text: &str| paint(view, Tone::Bold, text);
    if let Some(end) = heading_end(line) {
        return format!("{}{}", strong(&line[..end]), &line[end..]);
    }
    match column(line) {
        Some((indent, width)) => format!(
            "{}{}{}",
            &line[..indent],
            strong(&line[indent..indent + width]),
            &line[indent + width..]
        ),
        None => line.to_owned(),
    }
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
    use super::{bold, has_options, overview, page};
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
                "\n\nExit: ",
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
    fn an_unregistered_verb_has_no_page() {
        assert_eq!(page(&bilbo(), "frob"), None);
    }

    #[test]
    fn bold_marks_headings_and_left_columns_only() {
        assert_eq!(
            bold(&View::HUMAN, "Usage: bilbo <verb> [<args>]..."),
            "\x1b[1mUsage:\x1b[22m bilbo <verb> [<args>]..."
        );
        assert_eq!(
            bold(&View::HUMAN, "Paths, the first that applies:"),
            "\x1b[1mPaths, the first that applies:\x1b[22m"
        );
        assert_eq!(
            bold(&View::HUMAN, "Exit codes: 0 success"),
            "\x1b[1mExit codes:\x1b[22m 0 success"
        );
        assert_eq!(
            bold(&View::HUMAN, "  root    $BILBO_HOME"),
            "  \x1b[1mroot\x1b[22m    $BILBO_HOME"
        );
        for plain in [
            "Results go to stdout; diagnostics to stderr, prefixed 'bilbo: ' off a terminal.",
            "bilbo keeps durable memory for coding agents: it remembers their sessions and",
            "   continued",
            "      text that is not an option  x",
            "",
        ] {
            assert_eq!(bold(&View::HUMAN, plain), plain);
        }
    }

    #[test]
    fn bold_follows_the_option_rows_clap_renders() {
        let root = with_verb(bilbo(), probe::verb());
        let rows: Vec<String> = page(&root, "probe")
            .expect("a page")
            .lines()
            .filter(|line| line.trim_start().starts_with('-'))
            .map(|line| bold(&View::HUMAN, line))
            .collect();
        assert_eq!(rows.len(), 3, "{rows:?}");
        let page = page(&root, "probe").expect("a page");
        let block: Vec<String> = page
            .lines()
            .filter(|line| line.starts_with("  bilbo probe "))
            .map(|line| bold(&View::HUMAN, line))
            .collect();
        assert_eq!(block.len(), 3, "{block:?}");
        assert!(block.iter().all(|line| !line.contains('\x1b')), "{block:?}");
        for (row, column) in rows
            .iter()
            .zip(["-k, --kind <kind>", "--all", "--times <n>"])
        {
            assert!(row.contains(&format!("\x1b[1m{column}\x1b[22m")), "{row:?}");
        }
    }
}
