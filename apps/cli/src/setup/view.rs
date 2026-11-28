use bilbo_setup::report::{Report, Status};

use crate::console::message::{Message, line};
use crate::console::view::View;

pub fn rows(report: &Report, view: &View) -> String {
    report
        .lines
        .iter()
        .map(|row| {
            format!(
                "{}\t{}\t{}\n",
                row.step,
                row.status.word(),
                line(view, &row.detail)
            )
        })
        .collect()
}

pub fn failed(report: &Report, removal: bool) -> Option<Message> {
    let names: Vec<&str> = report
        .lines
        .iter()
        .filter(|row| row.status == Status::Failed)
        .map(|row| row.step)
        .collect();
    let (noun, them) = match names.len() {
        0 => return None,
        1 => ("step", "it"),
        _ => ("steps", "them"),
    };
    Some(Message::error(format!(
        "{} {noun} failed: {}; bilbo setup{} tries {them} again",
        names.len(),
        names.join(", "),
        if removal { " --remove" } else { "" }
    )))
}

#[cfg(test)]
mod tests {
    use bilbo_setup::report::{Report, Status};

    use super::{failed, rows};
    use crate::console::view::View;

    fn report(lines: &[(&'static str, Status, &str)]) -> Report {
        let mut report = Report::default();
        for (step, status, detail) in lines {
            report.push(step, *status, (*detail).to_owned());
        }
        report
    }

    #[test]
    fn a_row_is_three_fields_on_one_line() {
        let report = report(&[
            (
                "claude",
                Status::Installed,
                "bilbo@bilbo from /a\tb\nc\u{1b}[0m",
            ),
            ("agy", Status::Skipped, "agy is not on PATH"),
        ]);
        let expected = "claude\tinstalled\tbilbo@bilbo from /a\\tb\\nc\\u{1b}[0m\nagy\tskipped\tagy is not on PATH\n";
        for view in [View::PLAIN, View::HUMAN] {
            let text = rows(&report, &view);
            assert_eq!(text, expected);
            for line in text.lines() {
                assert_eq!(line.split('\t').count(), 3, "{line}");
            }
        }
    }

    #[test]
    fn the_error_line_names_the_steps_that_failed() {
        let none = report(&[("claude", Status::Kept, "x"), ("agy", Status::Skipped, "y")]);
        assert!(failed(&none, false).is_none());
        let one = report(&[
            ("claude", Status::Kept, "x"),
            ("codex", Status::Failed, "y"),
        ]);
        assert_eq!(
            failed(&one, false).map(|message| message.text),
            Some("1 step failed: codex; bilbo setup tries it again".to_owned())
        );
        let two = report(&[
            ("codex", Status::Failed, "y"),
            ("agy", Status::Installed, "x"),
            ("refresh-timer", Status::Failed, "z"),
        ]);
        assert_eq!(
            failed(&two, false).map(|message| message.text),
            Some("2 steps failed: codex, refresh-timer; bilbo setup tries them again".to_owned())
        );
    }
}
