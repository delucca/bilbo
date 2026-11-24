use std::io::{self, BufRead};
use std::path::PathBuf;

use crate::console::env::Env;
use crate::console::message::{Detail, Message, sanitize};
use crate::console::outcome::{Failure, Outcome};
use crate::console::theme::Mark;

pub trait Ways {
    fn typed(&mut self, env: &mut Env, input: &mut dyn BufRead) -> Result<Outcome, Failure>;
    fn editor(&mut self, env: &mut Env) -> Result<Outcome, Failure>;
    fn file(&mut self, env: &mut Env, target: PathBuf) -> Result<Outcome, Failure>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    End,
    Text(String),
    NotText,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    Nothing,
    Typed,
    Editor,
    File,
    Wrong(String),
    NotText,
}

pub fn question() -> Message {
    Message {
        mark: Mark::Info,
        text: "What should bilbo keep?\n1  a note typed here\n2  a note written in the editor\n3  a file or folder".to_owned(),
        details: vec![Detail::Text(
            "Type 1, 2 or 3, then Enter. An empty answer keeps nothing.".to_owned(),
        )],
    }
}

pub fn line(input: &mut dyn BufRead) -> Result<Line, Failure> {
    let mut bytes = Vec::new();
    let read = input
        .read_until(b'\n', &mut bytes)
        .map_err(|error: io::Error| {
            Failure::Refused(sanitize(&format!("cannot read the answer: {error}")))
        })?;
    if read == 0 {
        return Ok(Line::End);
    }
    Ok(String::from_utf8(bytes).map_or(Line::NotText, |text| Line::Text(text.trim().to_owned())))
}

pub fn answer(input: &mut dyn BufRead) -> Result<Answer, Failure> {
    Ok(match line(input)? {
        Line::End => Answer::Nothing,
        Line::NotText => Answer::NotText,
        Line::Text(text) => match text.as_str() {
            "" => Answer::Nothing,
            "1" => Answer::Typed,
            "2" => Answer::Editor,
            "3" => Answer::File,
            _ => Answer::Wrong(text),
        },
    })
}

fn nothing() -> Outcome {
    Outcome::new("").with_diagnostics(vec![Message::info("nothing kept")])
}

fn path(env: &Env, text: &str) -> PathBuf {
    match (text.strip_prefix("~/"), (env.var)("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(text),
    }
}

fn file(env: &mut Env, input: &mut dyn BufRead, ways: &mut dyn Ways) -> Result<Outcome, Failure> {
    loop {
        (env.notice)(&Message::info("Which file or folder?"));
        match line(input)? {
            Line::End => return Ok(nothing()),
            Line::Text(text) if text.is_empty() => return Ok(nothing()),
            Line::Text(text) => {
                let target = path(env, &text);
                return ways.file(env, target);
            }
            Line::NotText => (env.notice)(&Message::warning("that answer is not text")),
        }
    }
}

pub fn ask(
    env: &mut Env,
    input: &mut dyn BufRead,
    ways: &mut dyn Ways,
) -> Result<Outcome, Failure> {
    loop {
        (env.notice)(&question());
        match answer(input)? {
            Answer::Nothing => return Ok(nothing()),
            Answer::Typed => {
                (env.notice)(&Message::info(
                    "Type the note, then Ctrl-D on a line of its own.",
                ));
                return ways.typed(env, input);
            }
            Answer::Editor => return ways.editor(env),
            Answer::File => return file(env, input, ways),
            Answer::Wrong(text) => {
                (env.notice)(&Message::warning(&format!("'{text}' is not 1, 2 or 3")));
            }
            Answer::NotText => (env.notice)(&Message::warning("that answer is not text")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::io::{self, BufRead, BufReader, Cursor, Read};
    use std::path::PathBuf;
    use std::rc::Rc;

    use super::{Answer, Ways, answer, ask};
    use crate::console::env::{Env, Who};
    use crate::console::message::Message;
    use crate::console::outcome::{Failure, Outcome};

    fn cursor(bytes: &[u8]) -> Cursor<Vec<u8>> {
        Cursor::new(bytes.to_vec())
    }

    #[test]
    fn an_answer_is_a_line_without_the_white_space_around_it() {
        assert_eq!(
            answer(&mut cursor(b"1\n")).expect("an answer"),
            Answer::Typed
        );
        assert_eq!(
            answer(&mut cursor(b" 2 \r\n")).expect("an answer"),
            Answer::Editor
        );
        assert_eq!(answer(&mut cursor(b"3")).expect("an answer"), Answer::File);
        assert_eq!(
            answer(&mut cursor(b"\n")).expect("an answer"),
            Answer::Nothing
        );
        assert_eq!(
            answer(&mut cursor(b"  \n")).expect("an answer"),
            Answer::Nothing
        );
        assert_eq!(
            answer(&mut cursor(b"")).expect("an answer"),
            Answer::Nothing
        );
        assert_eq!(
            answer(&mut cursor(b"x\n")).expect("an answer"),
            Answer::Wrong("x".to_owned())
        );
        assert_eq!(
            answer(&mut cursor(b"12\n")).expect("an answer"),
            Answer::Wrong("12".to_owned())
        );
    }

    #[derive(Default)]
    struct Noted {
        ran: Vec<String>,
        left: Vec<u8>,
    }

    impl Ways for Noted {
        fn typed(&mut self, _: &mut Env, input: &mut dyn BufRead) -> Result<Outcome, Failure> {
            self.ran.push("typed".to_owned());
            input.read_to_end(&mut self.left).expect("the note");
            Ok(Outcome::new(""))
        }
        fn editor(&mut self, _: &mut Env) -> Result<Outcome, Failure> {
            self.ran.push("editor".to_owned());
            Ok(Outcome::new(""))
        }
        fn file(&mut self, _: &mut Env, target: PathBuf) -> Result<Outcome, Failure> {
            self.ran.push(format!("file {}", target.display()));
            Ok(Outcome::new(""))
        }
    }

    fn env(home: Option<&str>, said: &Rc<RefCell<Vec<Vec<String>>>>) -> Env {
        let home = home.map(OsString::from);
        let said = Rc::clone(said);
        Env {
            var: Rc::new(move |name| if name == "HOME" { home.clone() } else { None }),
            stdin: Box::new(io::empty()),
            who: Who::Person,
            notice: Box::new(move |message: &Message| {
                said.borrow_mut().push(message.plain_lines());
            }),
        }
    }

    fn run(
        home: Option<&str>,
        bytes: &[u8],
    ) -> (Result<Outcome, Failure>, Noted, Vec<Vec<String>>) {
        let said = Rc::new(RefCell::new(Vec::new()));
        let mut env = env(home, &said);
        let mut ways = Noted::default();
        let mut input = BufReader::new(Cursor::new(bytes.to_vec()));
        let result = ask(&mut env, &mut input, &mut ways);
        let lines = said.borrow().clone();
        (result, ways, lines)
    }

    fn question() -> Vec<String> {
        super::question().plain_lines()
    }

    #[test]
    fn each_answer_runs_its_way_and_a_wrong_one_is_asked_again() {
        let (result, ways, said) = run(None, b"x\n2\n");
        assert!(result.is_ok());
        assert_eq!(ways.ran, ["editor"]);
        assert_eq!(
            said,
            [
                question(),
                vec!["bilbo: 'x' is not 1, 2 or 3".to_owned()],
                question()
            ]
        );

        let (_, ways, said) = run(Some("/home/me"), b"3\n~/notes\n");
        assert_eq!(ways.ran, ["file /home/me/notes"]);
        assert_eq!(
            said,
            [question(), vec!["bilbo: Which file or folder?".to_owned()]]
        );
        let (_, ways, _) = run(None, b"3\n~/notes\n");
        assert_eq!(ways.ran, ["file ~/notes"]);
        let (_, ways, _) = run(Some("/home/me"), b"3\n/etc/x y\n");
        assert_eq!(ways.ran, ["file /etc/x y"]);

        let (result, ways, said) = run(None, b"3\n\n");
        let nothing = result.expect("an outcome");
        assert!(ways.ran.is_empty());
        assert_eq!(
            nothing.stderr_lines(&crate::console::view::View::PLAIN),
            ["bilbo: nothing kept"]
        );
        assert_eq!(said.len(), 2);

        let (_, ways, said) = run(None, b"1\nThe beet seeds\nare in the drawer.\n");
        assert_eq!(ways.ran, ["typed"]);
        assert_eq!(ways.left, b"The beet seeds\nare in the drawer.\n");
        assert_eq!(
            said[1],
            vec!["bilbo: Type the note, then Ctrl-D on a line of its own.".to_owned()]
        );
    }

    struct Failing;

    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("the terminal closed"))
        }
    }

    #[test]
    fn the_end_of_input_a_failed_read_and_bytes_that_are_no_text_at_a_question() {
        let (result, ways, _) = run(None, b"1\n");
        assert!(result.is_ok());
        assert_eq!(ways.ran, ["typed"]);
        assert!(ways.left.is_empty());

        let said = Rc::new(RefCell::new(Vec::new()));
        let mut env = env(None, &said);
        let mut ways = Noted::default();
        let mut input = BufReader::new(Failing);
        let failure = ask(&mut env, &mut input, &mut ways).expect_err("a failure");
        assert_eq!(
            failure,
            Failure::Refused("cannot read the answer: the terminal closed".to_owned())
        );
        assert!(ways.ran.is_empty());

        let (result, ways, said) = run(None, b"\xff\xfe\n3\n/x\n");
        assert!(result.is_ok());
        assert_eq!(ways.ran, ["file /x"]);
        assert_eq!(
            said,
            [
                question(),
                vec!["bilbo: that answer is not text".to_owned()],
                question(),
                vec!["bilbo: Which file or folder?".to_owned()]
            ]
        );
    }
}
