use std::io::Read;

use serde::de::{Deserialize, DeserializeOwned, Deserializer, IgnoredAny, Visitor};

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
}

impl Refused {
    pub fn session(&self) -> Option<&str> {
        match self {
            Refused::NoPrompt(session) => Some(session),
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

pub fn read(input: &mut dyn Read, limit: u64) -> Result<Given, Refused> {
    let mut bytes = Vec::new();
    input
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| Refused::Unread(error.to_string()))?;
    if u64::try_from(bytes.len()).is_ok_and(|read| read > limit) {
        return Err(Refused::TooLong(limit));
    }
    given(&bytes)
}

fn given(bytes: &[u8]) -> Result<Given, Refused> {
    std::str::from_utf8(bytes).map_err(|_| Refused::NotAnObject)?;
    value_at::<IgnoredAny>(bytes, 0)
        .filter(|(_, end)| skip(bytes, *end) == bytes.len())
        .ok_or(Refused::NotAnObject)?;
    let (session, prompt) = scan(bytes).ok_or(Refused::NotAnObject)?;
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

fn scan(bytes: &[u8]) -> Option<(Field, Field)> {
    let mut at = skip(bytes, 0);
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    at = skip(bytes, at + 1);
    let (mut session, mut prompt) = (Field::Absent, Field::Absent);
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
            let wanted = key == b"session_id" || key == b"prompt";
            let value = if wanted && bytes.get(at) == Some(&b'"') {
                let (Raw(text), next): (Raw, usize) = value_at(bytes, at)?;
                at = next;
                Field::Text(String::from_utf8_lossy(&text).into_owned())
            } else {
                let (_, next): (IgnoredAny, usize) = value_at(bytes, at)?;
                at = next;
                Field::Other
            };
            match key.as_slice() {
                b"session_id" => session = value,
                b"prompt" => prompt = value,
                _ => {}
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
    (skip(bytes, at) == bytes.len()).then_some((session, prompt))
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read};

    use super::{Given, Refused, read};

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
}
