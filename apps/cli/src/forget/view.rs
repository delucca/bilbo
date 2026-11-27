use bilbo_entries::forget::{Gone, Named, Output};
use bilbo_store::id::Id;

use crate::console::message::Message;

pub fn counts(output: &Output) -> String {
    let forgotten = output
        .named
        .iter()
        .filter(|named| named.gone.forgot)
        .count();
    let belonging: usize = output.named.iter().map(|named| named.with.len()).sum();
    let unchanged = output.named.len() - forgotten;
    format!("forgotten {forgotten}, belonging {belonging}, unchanged {unchanged}\n")
}

fn said(gone: &Gone, with: Option<Id>, read: &dyn Fn(&str) -> bool) -> Message {
    let id = gone.id;
    let whose = with.map_or_else(String::new, |named| format!(" with {named}"));
    let hint = match gone.held {
        Some(generation) if read(&gone.kind) => format!(
            "; bilbo read {id}#g{} prints the outline of what it held",
            generation.0
        ),
        _ => String::new(),
    };
    Message::info(&format!(
        "forgot {id}{whose}: its label was {}{hint}",
        gone.had
    ))
}

pub fn lines(output: &Output, read: &dyn Fn(&str) -> bool) -> Vec<Message> {
    let mut found = Vec::new();
    for Named { gone, with } in &output.named {
        if gone.forgot {
            found.push(said(gone, None, read));
        }
        for member in with {
            found.push(said(member, Some(gone.id), read));
        }
    }
    found
}

pub fn stopped(named: &[Id]) -> String {
    let ids: Vec<String> = named.iter().map(ToString::to_string).collect();
    format!(
        "stopped part way: bilbo forget {} finishes it",
        ids.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use bilbo_entries::forget::{Gone, Named, Output};
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;
    use bilbo_store::labels::CaptureLabel;

    use super::{counts, lines, stopped};

    const A: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DA";
    const B: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DB";
    const C: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DC";

    fn id(text: &str) -> Id {
        Id::parse(text).expect("an id")
    }

    fn gone(text: &str, forgot: bool, held: Option<u32>, kind: &str, had: &str) -> Gone {
        Gone {
            id: id(text),
            forgot,
            held: held.map(Generation),
            kind: kind.to_owned(),
            had: CaptureLabel::new(had.to_owned()).expect("a label"),
        }
    }

    fn named(gone: Gone, with: Vec<Gone>) -> Named {
        Named { gone, with }
    }

    fn plain(messages: &[crate::console::message::Message]) -> Vec<String> {
        messages
            .iter()
            .flat_map(crate::console::message::Message::plain_lines)
            .collect()
    }

    fn reads(kind: &str) -> bool {
        kind != "summary"
    }

    #[test]
    fn the_counts_are_one_line() {
        let held = |text: &str| gone(text, true, Some(1), "markdown", "file:/n/a.md");
        let one = Output {
            named: vec![named(held(A), vec![])],
        };
        assert_eq!(counts(&one), "forgotten 1, belonging 0, unchanged 0\n");
        let three = Output {
            named: vec![named(
                held(A),
                vec![
                    held(B),
                    held(C),
                    gone(C, true, None, "summary", "summary:x"),
                ],
            )],
        };
        assert_eq!(counts(&three), "forgotten 1, belonging 3, unchanged 0\n");
        let none = Output {
            named: vec![named(
                gone(A, false, Some(1), "markdown", "file:/n/a.md"),
                vec![],
            )],
        };
        assert_eq!(counts(&none), "forgotten 0, belonging 0, unchanged 1\n");
    }

    #[test]
    fn each_entry_forgotten_has_its_line() {
        let output = Output {
            named: vec![
                named(
                    gone(A, true, Some(2), "session", "file:/home/me/a.jsonl"),
                    vec![
                        gone(B, true, Some(1), "session", "file:/home/me/b.jsonl"),
                        gone(C, true, None, "markdown", "file:/home/me/c.md"),
                        gone(A, true, Some(1), "summary", "summary:x"),
                    ],
                ),
                named(gone(B, false, Some(1), "markdown", "file:/n/x.md"), vec![]),
            ],
        };
        assert_eq!(
            plain(&lines(&output, &reads)),
            [
                format!(
                    "bilbo: forgot {A}: its label was file:/home/me/a.jsonl; bilbo read {A}#g2 prints the outline of what it held"
                ),
                format!(
                    "bilbo: forgot {B} with {A}: its label was file:/home/me/b.jsonl; bilbo read {B}#g1 prints the outline of what it held"
                ),
                format!("bilbo: forgot {C} with {A}: its label was file:/home/me/c.md"),
                format!("bilbo: forgot {A} with {A}: its label was summary:x"),
            ]
        );
        let odd = Output {
            named: vec![named(
                gone(A, true, None, "markdown", "file:/n/a\tb\nc.md"),
                vec![],
            )],
        };
        let said = plain(&lines(&odd, &reads));
        assert_eq!(said.len(), 1);
        assert_eq!(
            said[0],
            format!("bilbo: forgot {A}: its label was file:/n/a\\tb\\nc.md")
        );
    }

    #[test]
    fn a_run_that_stopped_names_its_entries_by_id() {
        assert_eq!(
            stopped(&[id(A)]),
            format!("stopped part way: bilbo forget {A} finishes it")
        );
        assert_eq!(
            stopped(&[id(C), id(A), id(B)]),
            format!("stopped part way: bilbo forget {C} {A} {B} finishes it")
        );
    }
}
