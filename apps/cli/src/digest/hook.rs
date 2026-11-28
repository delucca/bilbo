use std::io::Read;

use serde::de::{Deserialize, DeserializeOwned, Deserializer, IgnoredAny, Visitor};

use super::transcript::Unread;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Given {
    pub session: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    Unread(String),
    TooLong(u64),
    NotAnObject,
    NoSession,
    NoPrompt(String),
    NoConversation,
    NoTranscript(String),
    NotRegular(String, String),
    Unreadable(String, String, String),
    NoUserInput(String),
}

impl Refused {
    pub fn session(&self) -> Option<&str> {
        match self {
            Refused::NoPrompt(session)
            | Refused::NoTranscript(session)
            | Refused::NotRegular(session, _)
            | Refused::Unreadable(session, ..)
            | Refused::NoUserInput(session) => Some(session),
            _ => None,
        }
    }

    pub fn sentence(&self) -> String {
        match self {
            Refused::Unread(error) => format!("cannot read standard input: {error}"),
            Refused::TooLong(limit) => {
                format!("the hook's input is longer than digest.input allows ({limit} bytes)")
            }
            Refused::NotAnObject => "the hook's input is not a JSON object".to_owned(),
            Refused::NoSession => "the hook's input has no session_id".to_owned(),
            Refused::NoPrompt(_) => "the hook's input has no prompt".to_owned(),
            Refused::NoConversation => "the hook's input has no conversationId".to_owned(),
            Refused::NoTranscript(_) => "the hook's input has no transcriptPath".to_owned(),
            Refused::NotRegular(_, path) => format!("the transcript {path} is not a regular file"),
            Refused::Unreadable(_, path, error) => {
                format!("cannot read the transcript {path}: {error}")
            }
            Refused::NoUserInput(_) => "the transcript's last steps hold no user input".to_owned(),
        }
    }
}

struct Raw(Vec<u8>);

struct RawVisitor;

impl Visitor<'_> for RawVisitor {
    type Value = Raw;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON string")
    }

    fn visit_bytes<E>(self, bytes: &[u8]) -> Result<Raw, E> {
        Ok(Raw(bytes.to_vec()))
    }
}

impl<'de> Deserialize<'de> for Raw {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Raw, D::Error> {
        deserializer.deserialize_bytes(RawVisitor)
    }
}

enum Field {
    Absent,
    Other,
    Whole(u64),
    Text(String),
}

impl Field {
    fn text(self) -> Option<String> {
        match self {
            Field::Text(text) if !text.is_empty() => Some(text),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Heard {
    Turn(Given),
    Quiet,
}

fn taken(input: &mut dyn Read, limit: u64) -> Result<Vec<u8>, Refused> {
    let mut bytes = Vec::new();
    input
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| Refused::Unread(error.to_string()))?;
    if u64::try_from(bytes.len()).is_ok_and(|read| read > limit) {
        return Err(Refused::TooLong(limit));
    }
    Ok(bytes)
}

fn whole_object(bytes: &[u8]) -> Result<(), Refused> {
    std::str::from_utf8(bytes).map_err(|_| Refused::NotAnObject)?;
    value_at::<IgnoredAny>(bytes, 0)
        .filter(|(_, end)| skip(bytes, *end) == bytes.len())
        .ok_or(Refused::NotAnObject)?;
    Ok(())
}

pub fn read(input: &mut dyn Read, limit: u64) -> Result<Given, Refused> {
    let bytes = taken(input, limit)?;
    given(&bytes)
}

pub fn read_agy(
    input: &mut dyn Read,
    limit: u64,
    tail: &dyn Fn(&str) -> Result<Vec<u8>, Unread>,
) -> Result<Heard, Refused> {
    let bytes = taken(input, limit)?;
    whole_object(&bytes)?;
    let [conversation, path, number] = scan(
        &bytes,
        [
            b"conversationId".as_slice(),
            b"transcriptPath",
            b"invocationNum",
        ],
    )
    .ok_or(Refused::NotAnObject)?;
    if !matches!(number, Field::Absent | Field::Whole(0)) {
        return Ok(Heard::Quiet);
    }
    let session = conversation.text().ok_or(Refused::NoConversation)?;
    let path = path
        .text()
        .ok_or_else(|| Refused::NoTranscript(session.clone()))?;
    let steps = tail(&path).map_err(|error| match error {
        Unread::NotRegular => Refused::NotRegular(session.clone(), path.clone()),
        Unread::Failed(error) => Refused::Unreadable(session.clone(), path.clone(), error),
    })?;
    match last_user_input(&steps) {
        Some(prompt) => Ok(Heard::Turn(Given { session, prompt })),
        None => Err(Refused::NoUserInput(session)),
    }
}

fn last_user_input(steps: &[u8]) -> Option<String> {
    for line in steps.rsplit(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        whole_object(line).ok()?;
        let [kind, content] = scan(line, [b"type".as_slice(), b"content"])?;
        match kind.text()?.as_str() {
            "SYSTEM_MESSAGE" => continue,
            "USER_INPUT" => return request(&content.text()?),
            _ => return None,
        }
    }
    None
}

fn request(content: &str) -> Option<String> {
    const OPEN: &str = "<USER_REQUEST>";
    const CLOSE: &str = "</USER_REQUEST>";
    let body = content
        .find(OPEN)
        .and_then(|open| {
            let after = &content[open + OPEN.len()..];
            after.find(CLOSE).map(|close| &after[..close])
        })
        .unwrap_or(content)
        .trim();
    (!body.is_empty()).then(|| body.to_owned())
}

fn given(bytes: &[u8]) -> Result<Given, Refused> {
    whole_object(bytes)?;
    let [session, prompt] =
        scan(bytes, [b"session_id".as_slice(), b"prompt"]).ok_or(Refused::NotAnObject)?;
    let session = session.text().ok_or(Refused::NoSession)?;
    match prompt.text() {
        Some(prompt) => Ok(Given { session, prompt }),
        None => Err(Refused::NoPrompt(session)),
    }
}

fn skip(bytes: &[u8], mut at: usize) -> usize {
    while bytes
        .get(at)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
    {
        at += 1;
    }
    at
}

fn value_at<T: DeserializeOwned>(bytes: &[u8], at: usize) -> Option<(T, usize)> {
    let mut stream = serde_json::Deserializer::from_slice(bytes.get(at..)?).into_iter::<T>();
    let value = stream.next()?.ok()?;
    Some((value, at + stream.byte_offset()))
}

fn scan<const N: usize>(bytes: &[u8], names: [&[u8]; N]) -> Option<[Field; N]> {
    let mut at = skip(bytes, 0);
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    at = skip(bytes, at + 1);
    let mut fields = names.map(|_| Field::Absent);
    if bytes.get(at) == Some(&b'}') {
        at += 1;
    } else {
        loop {
            let (Raw(key), next): (Raw, usize) = value_at(bytes, at)?;
            at = skip(bytes, next);
            if bytes.get(at) != Some(&b':') {
                return None;
            }
            at = skip(bytes, at + 1);
            let wanted = names.iter().position(|name| *name == key.as_slice());
            let value = match (wanted, bytes.get(at)) {
                (Some(_), Some(b'"')) => {
                    let (Raw(text), next): (Raw, usize) = value_at(bytes, at)?;
                    at = next;
                    Field::Text(String::from_utf8_lossy(&text).into_owned())
                }
                (Some(_), Some(b'0'..=b'9')) => {
                    let (number, next): (serde_json::Number, usize) = value_at(bytes, at)?;
                    at = next;
                    number.as_u64().map_or(Field::Other, Field::Whole)
                }
                _ => {
                    let (_, next): (IgnoredAny, usize) = value_at(bytes, at)?;
                    at = next;
                    Field::Other
                }
            };
            if let Some(place) = wanted {
                fields[place] = value;
            }
            at = skip(bytes, at);
            match bytes.get(at) {
                Some(b',') => at = skip(bytes, at + 1),
                Some(b'}') => {
                    at += 1;
                    break;
                }
                _ => return None,
            }
        }
    }
    (skip(bytes, at) == bytes.len()).then_some(fields)
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read};

    use super::{Given, Heard, Refused, Unread, read, read_agy};

    const LIMIT: u64 = 4096;

    fn of(text: &[u8]) -> Result<Given, Refused> {
        read(&mut &text[..], LIMIT)
    }

    fn given(session: &str, prompt: &str) -> Result<Given, Refused> {
        Ok(Given {
            session: session.to_owned(),
            prompt: prompt.to_owned(),
        })
    }

    #[test]
    fn a_claude_code_and_a_codex_input_give_the_session_and_the_prompt() {
        let claude = br#"{"session_id":"s-1","prompt_id":"p","transcript_path":"/h/t.jsonl","cwd":"/h","permission_mode":"default","hook_event_name":"UserPromptSubmit","prompt":"thin the\ncarrot rows \"now\"","session_title":"x"}"#;
        let codex = br#"{"session_id":"s-1","transcript_path":null,"cwd":"/h","hook_event_name":"UserPromptSubmit","model":"m","permission_mode":"default","turn_id":"t","prompt":"thin the\ncarrot rows \"now\""}"#;
        let wanted = given("s-1", "thin the\ncarrot rows \"now\"");
        assert_eq!(of(claude), wanted);
        assert_eq!(of(codex), wanted);
    }

    struct Fails(bool);

    impl Read for Fails {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.0 {
                return Err(io::Error::other("the pipe broke"));
            }
            self.0 = true;
            buffer[0] = b'{';
            Ok(1)
        }
    }

    #[test]
    fn an_input_that_is_no_prompt_says_why() {
        let many = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let rows: Vec<(Vec<u8>, Result<Given, Refused>)> = vec![
            (b"".to_vec(), Err(Refused::NotAnObject)),
            (b"   \n".to_vec(), Err(Refused::NotAnObject)),
            (b"\xff\xfe".to_vec(), Err(Refused::NotAnObject)),
            (b"not json".to_vec(), Err(Refused::NotAnObject)),
            (b"[1,2]".to_vec(), Err(Refused::NotAnObject)),
            (br#"{"prompt":"p"}"#.to_vec(), Err(Refused::NoSession)),
            (
                br#"{"session_id":7,"prompt":"p"}"#.to_vec(),
                Err(Refused::NoSession),
            ),
            (
                br#"{"session_id":"","prompt":"p"}"#.to_vec(),
                Err(Refused::NoSession),
            ),
            (
                br#"{"session_id":"s"}"#.to_vec(),
                Err(Refused::NoPrompt("s".to_owned())),
            ),
            (
                br#"{"session_id":"s","prompt":""}"#.to_vec(),
                Err(Refused::NoPrompt("s".to_owned())),
            ),
            (
                br#"{"session_id":"s","prompt":7}"#.to_vec(),
                Err(Refused::NoPrompt("s".to_owned())),
            ),
            (
                br#"{"session_id":"s","prompt":["a"]}"#.to_vec(),
                Err(Refused::NoPrompt("s".to_owned())),
            ),
            (
                br#"{"session_id":"s","prompt":null}"#.to_vec(),
                Err(Refused::NoPrompt("s".to_owned())),
            ),
            (
                br#"{"session_id":"a","session_id":"b","prompt":"x","prompt":"y"}"#.to_vec(),
                given("b", "y"),
            ),
            (
                br#"{"session_id":"a","session_id":7,"prompt":"p"}"#.to_vec(),
                Err(Refused::NoSession),
            ),
            (
                format!(r#"{{"session_id":"s","prompt":"p","other":{}}}"#, many(130)).into_bytes(),
                given("s", "p"),
            ),
            (
                format!(r#"{{"session_id":{},"prompt":"p"}}"#, many(130)).into_bytes(),
                Err(Refused::NoSession),
            ),
            (
                br#"{"session_id":"s","prompt":"p","other":nope}"#.to_vec(),
                Err(Refused::NotAnObject),
            ),
            (
                b"{\"session_id\":\"s\",\"prompt\":\"p\",\"other\":\"\xff\"}".to_vec(),
                Err(Refused::NotAnObject),
            ),
            (
                br#"{"other":{"session_id":"inner"},"prompt":"p"}"#.to_vec(),
                Err(Refused::NoSession),
            ),
            (
                br#"{"session_id":"s","prompt":"p"} x"#.to_vec(),
                Err(Refused::NotAnObject),
            ),
            (
                b"  {\"session_id\":\"s\",\"prompt\":\"p\"}\n\n".to_vec(),
                given("s", "p"),
            ),
        ];
        let half = |bytes: [u8; 3]| String::from_utf8_lossy(&bytes).into_owned();
        let (high, low) = (half([0xED, 0xA0, 0xBD]), half([0xED, 0xB8, 0x80]));
        let mut rows = rows;
        rows.extend([
            (
                br#"{"session_id":"s","prompt":"ab\ud83d"}"#.to_vec(),
                given("s", &format!("ab{high}")),
            ),
            (
                br#"{"session_id":"s","prompt":"a\ude00b"}"#.to_vec(),
                given("s", &format!("a{low}b")),
            ),
            (
                br#"{"session_id":"s","prompt":"\ud83d\n"}"#.to_vec(),
                given("s", &format!("{high}\n")),
            ),
            (
                br#"{"session_id":"s","prompt":"\ud83d\ud83d\ude00"}"#.to_vec(),
                given("s", &format!("{high}\u{1F600}")),
            ),
            (
                br#"{"session_id":"x\ud83d","prompt":"p"}"#.to_vec(),
                given(&format!("x{high}"), "p"),
            ),
            (
                br#"{"pro\ud83dmpt":"odd","session_id":"s","prompt":"p"}"#.to_vec(),
                given("s", "p"),
            ),
            (
                br#"{"pro\ud83dmpt":"odd","session_id":"s"}"#.to_vec(),
                Err(Refused::NoPrompt("s".to_owned())),
            ),
            (
                br#"{"prompt\ud83d":"odd","session_id":"s","prompt":"p"}"#.to_vec(),
                given("s", "p"),
            ),
            (
                br#"{"prompt\ud83d":"odd","session_id":"s"}"#.to_vec(),
                Err(Refused::NoPrompt("s".to_owned())),
            ),
            (
                b"{\"session_id\":\"s\",\"prompt\":\"a\tb\"}".to_vec(),
                Err(Refused::NotAnObject),
            ),
            (
                b"{\"session_id\":\"s\n\",\"prompt\":\"p\"}".to_vec(),
                Err(Refused::NotAnObject),
            ),
            (
                b"{\"session_id\":\"s\",\"prompt\":\"p\",\"x\0y\":1}".to_vec(),
                Err(Refused::NotAnObject),
            ),
            (
                b"{\"session_id\":\"s\",\"prompt\":\"p\",\"other\":\"a\nb\"}".to_vec(),
                Err(Refused::NotAnObject),
            ),
            (
                br#"{"session_id":"s","prompt":"a\tb\n"}"#.to_vec(),
                given("s", "a\tb\n"),
            ),
            (
                br#"{"session_id":"s","prompt":"p","other":"\ud83d"}"#.to_vec(),
                given("s", "p"),
            ),
            (
                b"{\"session_id\":\"s\",\"prompt\":\"\xED\xA0\x81\"}".to_vec(),
                Err(Refused::NotAnObject),
            ),
            (
                br#"{"session_id":"s","prompt":"\ud83"}"#.to_vec(),
                Err(Refused::NotAnObject),
            ),
            (
                br#"{"session_id":"s","prompt":" \t\n "}"#.to_vec(),
                given("s", " \t\n "),
            ),
        ]);
        for (input, wanted) in rows {
            assert_eq!(of(&input), wanted, "{}", String::from_utf8_lossy(&input));
        }

        let exact = br#"{"session_id":"s","prompt":"p"}"#;
        let limit = exact.len() as u64;
        assert_eq!(read(&mut &exact[..], limit), given("s", "p"));
        let over = [&exact[..], b" "].concat();
        assert_eq!(
            read(&mut &over[..], limit),
            Err(Refused::TooLong(limit)),
            "one byte over the limit, though the start is a whole object"
        );
        assert_eq!(
            read(&mut Fails(false), LIMIT),
            Err(Refused::Unread("the pipe broke".to_owned()))
        );
        assert_eq!(
            Refused::TooLong(4096).sentence(),
            "the hook's input is longer than digest.input allows (4096 bytes)"
        );
        assert_eq!(
            Refused::Unread("boom".to_owned()).sentence(),
            "cannot read standard input: boom"
        );
        assert_eq!(
            Refused::NoPrompt("s".to_owned()).session(),
            Some("s"),
            "the session is kept"
        );
        assert_eq!(Refused::NoSession.session(), None);
    }

    fn heard(input: &[u8], steps: &[u8]) -> Result<Heard, Refused> {
        read_agy(&mut &input[..], LIMIT, &|_| Ok(steps.to_vec()))
    }

    fn turn(session: &str, prompt: &str) -> Result<Heard, Refused> {
        Ok(Heard::Turn(Given {
            session: session.to_owned(),
            prompt: prompt.to_owned(),
        }))
    }

    const AGY: &[u8] =
        br#"{"conversationId":"c-1","transcriptPath":"/h/t.jsonl","invocationNum":0}"#;

    #[test]
    fn an_agy_input_gives_the_conversation_and_the_last_user_input() {
        let user = |text: &str| format!("{{\"type\":\"USER_INPUT\",\"content\":{text:?}}}\n");
        let system = "{\"type\":\"SYSTEM_MESSAGE\",\"content\":\"ephemeral\"}\n";
        let rows: Vec<(String, Result<Heard, Refused>)> = vec![
            (
                user("thin the carrot rows"),
                turn("c-1", "thin the carrot rows"),
            ),
            (
                user(
                    "<ADDITIONAL_METADATA>x</ADDITIONAL_METADATA>\n<USER_REQUEST>\n thin rows \n</USER_REQUEST>\ntail",
                ),
                turn("c-1", "thin rows"),
            ),
            (
                format!("{}{}{system}{system}", user("first"), user("last")),
                turn("c-1", "last"),
            ),
            (
                format!("{}{system}\n  \n", user("kept")),
                turn("c-1", "kept"),
            ),
            (
                user("<USER_REQUEST>no close"),
                turn("c-1", "<USER_REQUEST>no close"),
            ),
        ];
        for (steps, wanted) in rows {
            assert_eq!(heard(AGY, steps.as_bytes()), wanted, "{steps}");
        }
        let both =
            br#"{"session_id":"other","prompt":"no","conversationId":"c-1","transcriptPath":"/t"}"#;
        assert_eq!(heard(both, user("yes").as_bytes()), turn("c-1", "yes"));
    }

    #[test]
    fn an_agy_call_that_starts_no_turn_is_no_prompt_and_no_error() {
        let with = |number: &str| {
            format!(r#"{{"conversationId":"c","transcriptPath":"/t","invocationNum":{number}}}"#)
        };
        let steps = b"{\"type\":\"USER_INPUT\",\"content\":\"p\"}\n";
        for quiet in [
            "1",
            "7",
            "\"0\"",
            "0.5",
            "-1",
            "null",
            "1e2",
            "99999999999999999999",
        ] {
            assert_eq!(
                heard(with(quiet).as_bytes(), steps),
                Ok(Heard::Quiet),
                "{quiet}"
            );
        }
        assert_eq!(heard(with("0").as_bytes(), steps), turn("c", "p"));
        let absent = br#"{"conversationId":"c","transcriptPath":"/t"}"#;
        assert_eq!(heard(absent, steps), turn("c", "p"));
        let nothing = br#"{"invocationNum":3}"#;
        assert_eq!(heard(nothing, b""), Ok(Heard::Quiet));
    }

    #[test]
    fn an_agy_input_that_is_no_prompt_says_why() {
        let steps = b"{\"type\":\"USER_INPUT\",\"content\":\"p\"}\n";
        let rows: Vec<(&[u8], Refused, &str)> = vec![
            (
                br#"{"transcriptPath":"/t"}"#,
                Refused::NoConversation,
                "the hook's input has no conversationId",
            ),
            (
                br#"{"conversationId":7,"transcriptPath":"/t"}"#,
                Refused::NoConversation,
                "the hook's input has no conversationId",
            ),
            (
                br#"{"conversationId":"","transcriptPath":"/t"}"#,
                Refused::NoConversation,
                "the hook's input has no conversationId",
            ),
            (
                br#"{"conversationId":"c"}"#,
                Refused::NoTranscript("c".to_owned()),
                "the hook's input has no transcriptPath",
            ),
            (
                br#"{"conversationId":"c","transcriptPath":""}"#,
                Refused::NoTranscript("c".to_owned()),
                "the hook's input has no transcriptPath",
            ),
            (
                b"[1]",
                Refused::NotAnObject,
                "the hook's input is not a JSON object",
            ),
        ];
        for (input, refused, sentence) in rows {
            let got = heard(input, steps).expect_err("no prompt");
            assert_eq!(got, refused);
            assert_eq!(got.sentence(), sentence);
        }
        let refused = |unread: Unread| {
            read_agy(&mut &AGY[..], LIMIT, &|_| Err(unread.clone())).expect_err("no prompt")
        };
        let not = refused(Unread::NotRegular);
        assert_eq!(
            not.sentence(),
            "the transcript /h/t.jsonl is not a regular file"
        );
        assert_eq!(not.session(), Some("c-1"));
        let failed = refused(Unread::Failed("No such file".to_owned()));
        assert_eq!(
            failed.sentence(),
            "cannot read the transcript /h/t.jsonl: No such file"
        );
        let empty = "the transcript's last steps hold no user input";
        for tail in [
            &b""[..],
            b"\n \n",
            b"{\"type\":\"USER_INPUT\",\"content\":\"old\"}\n{\"type\":\"TOOL_CALL\"}\n",
            b"{\"type\":\"USER_INPUT\",\"content\":\"old\"}\nnot json\n",
            b"st\":1}\n{\"type\":\"SYSTEM_MESSAGE\",\"content\":\"s\"}\n",
            b"{\"type\":\"USER_INPUT\",\"content\":\"  \"}\n",
            b"{\"type\":\"USER_INPUT\",\"content\":7}\n",
            b"{\"type\":\"USER_INPUT\"}\n",
            b"[1]\n",
        ] {
            let got = heard(AGY, tail).expect_err("no prompt");
            assert_eq!(got.sentence(), empty, "{}", String::from_utf8_lossy(tail));
            assert_eq!(got.session(), Some("c-1"));
        }
        assert_eq!(
            read_agy(&mut Fails(false), LIMIT, &|_| Ok(Vec::new())),
            Err(Refused::Unread("the pipe broke".to_owned()))
        );
        let over = [AGY, b" "].concat();
        assert_eq!(
            read_agy(&mut &over[..], AGY.len() as u64, &|_| Ok(Vec::new())),
            Err(Refused::TooLong(AGY.len() as u64))
        );
    }

    #[test]
    fn an_agy_input_passes_over_what_it_does_not_read() {
        let steps = b"{\"type\":\"USER_INPUT\",\"content\":\"p\"}\n";
        let many = "[".repeat(130) + &"]".repeat(130);
        let rows: Vec<(String, Result<Heard, Refused>)> = vec![
            (
                format!(r#"{{"conversationId":"c","other":{many},"transcriptPath":"/t"}}"#),
                turn("c", "p"),
            ),
            (
                r#"{"conversationId":"a","conversationId":"b","transcriptPath":"/t"}"#.to_owned(),
                turn("b", "p"),
            ),
            (
                r#"{"conversationId":"c","transcriptPath":"/t","invocationNum":5,"invocationNum":0}"#.to_owned(),
                turn("c", "p"),
            ),
            (
                r#"{"conversationId":"c","transcriptPath":"/t","other":"\ud83d"}"#.to_owned(),
                turn("c", "p"),
            ),
            (
                format!(r#"{{"invocationNum":2,"conversationId":{many},"transcriptPath":7}}"#),
                Ok(Heard::Quiet),
            ),
        ];
        for (input, wanted) in rows {
            assert_eq!(heard(input.as_bytes(), steps), wanted, "{input}");
        }
    }
}
