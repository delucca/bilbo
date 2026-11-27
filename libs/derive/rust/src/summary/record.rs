use std::fmt;
use std::marker::PhantomData;

use bilbo_store::entries::Generation;
use bilbo_store::id::Id;
use serde::de::{Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use super::content_of;
use crate::instruction::number_of;
use crate::record::quoted;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notes {
    pub generation: Generation,
    pub records: u64,
    pub digest: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub text: String,
    pub instruction: String,
    pub provider: String,
    pub model: String,
    pub answered: Option<String>,
    pub made: u64,
    pub entry: Id,
    pub generation: Generation,
    pub records: u64,
    pub content: u128,
    pub notes: Option<Notes>,
    pub sent: u64,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
}

#[must_use]
pub fn write(summary: &Summary) -> String {
    let mut line = format!(
        r#"{{"summary":{{"text":{},"instruction":{},"by":{{"provider":{},"model":{}}},"#,
        quoted(&summary.text),
        quoted(&summary.instruction),
        quoted(&summary.provider),
        quoted(&summary.model)
    );
    if let Some(answered) = &summary.answered {
        line.push_str(&format!(r#""answered":{},"#, quoted(answered)));
    }
    line.push_str(&format!(
        r#""made":{},"of":{{"entry":{},"generation":{},"records":{},"content":"{:032x}""#,
        summary.made,
        quoted(&summary.entry.to_string()),
        summary.generation.0,
        summary.records,
        summary.content
    ));
    if let Some(notes) = &summary.notes {
        line.push_str(&format!(
            r#","notes":{{"generation":{},"records":{},"digest":"{:016x}"}}"#,
            notes.generation.0, notes.records, notes.digest
        ));
    }
    line.push_str(&format!(r#"}},"sent":{}"#, summary.sent));
    if let Some(tokens) = summary.tokens_in {
        line.push_str(&format!(r#","in":{tokens}"#));
    }
    if let Some(tokens) = summary.tokens_out {
        line.push_str(&format!(r#","out":{tokens}"#));
    }
    line.push_str("}}");
    line
}

#[must_use]
pub fn retirement(
    entry: Id,
    generation: Generation,
    records: u64,
    content: u128,
    made: u64,
) -> String {
    format!(
        r#"{{"summary":{{"retired":{made},"of":{{"entry":{},"generation":{},"records":{records},"content":"{content:032x}"}}}}}}"#,
        quoted(&entry.to_string()),
        generation.0
    )
}

trait Own: Sized {
    fn text(_: &str) -> Option<Self> {
        None
    }

    fn number(_: u64) -> Option<Self> {
        None
    }
}

impl Own for String {
    fn text(value: &str) -> Option<String> {
        Some(value.to_owned())
    }
}

impl Own for u64 {
    fn number(value: u64) -> Option<u64> {
        Some(value)
    }
}

struct Soft<T>(Option<T>);

struct SoftVisitor<T>(PhantomData<T>);

impl<'de, T: Own> Visitor<'de> for SoftVisitor<T> {
    type Value = Soft<T>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_str<E>(self, value: &str) -> Result<Soft<T>, E> {
        Ok(Soft(T::text(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Soft<T>, E> {
        Ok(Soft(T::number(value)))
    }

    fn visit_i64<E>(self, _: i64) -> Result<Soft<T>, E> {
        Ok(Soft(None))
    }

    fn visit_f64<E>(self, _: f64) -> Result<Soft<T>, E> {
        Ok(Soft(None))
    }

    fn visit_bool<E>(self, _: bool) -> Result<Soft<T>, E> {
        Ok(Soft(None))
    }

    fn visit_unit<E>(self) -> Result<Soft<T>, E> {
        Ok(Soft(None))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Soft<T>, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Soft(None))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Soft<T>, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Soft(None))
    }
}

impl<'de, T: Own> Deserialize<'de> for Soft<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Soft<T>, D::Error> {
        deserializer.deserialize_any(SoftVisitor(PhantomData))
    }
}

#[derive(Default)]
enum Member<T> {
    #[default]
    Missing,
    Null,
    Object(T),
    Other,
}

impl<T> Member<T> {
    fn object(self) -> Option<T> {
        match self {
            Member::Object(held) => Some(held),
            _ => None,
        }
    }
}

trait Fields: Sized {
    fn fields<'de, A: MapAccess<'de>>(map: A) -> Result<Self, A::Error>;
}

struct MemberVisitor<T>(PhantomData<T>);

impl<'de, T: Fields> Visitor<'de> for MemberVisitor<T> {
    type Value = Member<T>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Member<T>, A::Error> {
        T::fields(map).map(Member::Object)
    }

    fn visit_unit<E>(self) -> Result<Member<T>, E> {
        Ok(Member::Null)
    }

    fn visit_str<E>(self, _: &str) -> Result<Member<T>, E> {
        Ok(Member::Other)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Member<T>, E> {
        Ok(Member::Other)
    }

    fn visit_i64<E>(self, _: i64) -> Result<Member<T>, E> {
        Ok(Member::Other)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Member<T>, E> {
        Ok(Member::Other)
    }

    fn visit_bool<E>(self, _: bool) -> Result<Member<T>, E> {
        Ok(Member::Other)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Member<T>, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Member::Other)
    }
}

impl<'de, T: Fields> Deserialize<'de> for Member<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Member<T>, D::Error> {
        deserializer.deserialize_any(MemberVisitor(PhantomData))
    }
}

#[derive(Default)]
struct RawNotes {
    generation: Option<u64>,
    records: Option<u64>,
    digest: Option<String>,
}

#[derive(Default)]
struct RawOf {
    entry: Option<String>,
    generation: Option<u64>,
    records: Option<u64>,
    content: Option<String>,
    notes: Member<RawNotes>,
}

#[derive(Default)]
struct RawBy {
    provider: Option<String>,
    model: Option<String>,
}

#[derive(Default)]
struct RawSummary {
    text: Option<String>,
    instruction: Option<String>,
    by: Member<RawBy>,
    answered: Option<String>,
    made: Option<u64>,
    of: Member<RawOf>,
    sent: Option<u64>,
    tokens_in: Option<u64>,
    tokens_out: Option<u64>,
}

struct Whole {
    summary: Member<RawSummary>,
}

struct Members<'a, A>(&'a mut A);

impl<'de, A: MapAccess<'de>> Members<'_, A> {
    fn own<T: Own>(&mut self) -> Result<Option<T>, A::Error> {
        Ok(self.0.next_value::<Soft<T>>()?.0)
    }

    fn skip(&mut self) -> Result<(), A::Error> {
        self.0.next_value::<IgnoredAny>().map(|_| ())
    }
}

impl Fields for RawNotes {
    fn fields<'de, A: MapAccess<'de>>(mut map: A) -> Result<RawNotes, A::Error> {
        let mut raw = RawNotes::default();
        while let Some(name) = map.next_key::<String>()? {
            let mut members = Members(&mut map);
            match name.as_str() {
                "generation" => raw.generation = members.own()?,
                "records" => raw.records = members.own()?,
                "digest" => raw.digest = members.own()?,
                _ => members.skip()?,
            }
        }
        Ok(raw)
    }
}

impl Fields for RawOf {
    fn fields<'de, A: MapAccess<'de>>(mut map: A) -> Result<RawOf, A::Error> {
        let mut raw = RawOf::default();
        while let Some(name) = map.next_key::<String>()? {
            if name == "notes" {
                raw.notes = map.next_value()?;
                continue;
            }
            let mut members = Members(&mut map);
            match name.as_str() {
                "entry" => raw.entry = members.own()?,
                "generation" => raw.generation = members.own()?,
                "records" => raw.records = members.own()?,
                "content" => raw.content = members.own()?,
                _ => members.skip()?,
            }
        }
        Ok(raw)
    }
}

impl Fields for RawBy {
    fn fields<'de, A: MapAccess<'de>>(mut map: A) -> Result<RawBy, A::Error> {
        let mut raw = RawBy::default();
        while let Some(name) = map.next_key::<String>()? {
            let mut members = Members(&mut map);
            match name.as_str() {
                "provider" => raw.provider = members.own()?,
                "model" => raw.model = members.own()?,
                _ => members.skip()?,
            }
        }
        Ok(raw)
    }
}

impl Fields for RawSummary {
    fn fields<'de, A: MapAccess<'de>>(mut map: A) -> Result<RawSummary, A::Error> {
        let mut raw = RawSummary::default();
        while let Some(name) = map.next_key::<String>()? {
            match name.as_str() {
                "by" => raw.by = map.next_value()?,
                "of" => raw.of = map.next_value()?,
                _ => {
                    let mut members = Members(&mut map);
                    match name.as_str() {
                        "text" => raw.text = members.own()?,
                        "instruction" => raw.instruction = members.own()?,
                        "answered" => raw.answered = members.own()?,
                        "made" => raw.made = members.own()?,
                        "sent" => raw.sent = members.own()?,
                        "in" => raw.tokens_in = members.own()?,
                        "out" => raw.tokens_out = members.own()?,
                        _ => members.skip()?,
                    }
                }
            }
        }
        Ok(raw)
    }
}

impl Fields for Whole {
    fn fields<'de, A: MapAccess<'de>>(mut map: A) -> Result<Whole, A::Error> {
        let mut whole = Whole {
            summary: Member::Missing,
        };
        while let Some(name) = map.next_key::<String>()? {
            if name == "summary" {
                whole.summary = map.next_value()?;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(whole)
    }
}

struct WholeVisitor;

impl<'de> Visitor<'de> for WholeVisitor {
    type Value = Whole;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Whole, A::Error> {
        Whole::fields(map)
    }
}

impl<'de> Deserialize<'de> for Whole {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Whole, D::Error> {
        deserializer.deserialize_map(WholeVisitor)
    }
}

fn notes(raw: Member<RawNotes>) -> Option<Option<Notes>> {
    let held = match raw {
        Member::Missing | Member::Null => return Some(None),
        Member::Object(held) => held,
        Member::Other => return None,
    };
    let digest = number_of(held.digest.as_deref()?)?;
    Some(Some(Notes {
        generation: Generation(u32::try_from(held.generation?).ok()?),
        records: held.records?,
        digest,
    }))
}

#[must_use]
pub fn read(bytes: &[u8]) -> Option<Summary> {
    std::str::from_utf8(bytes).ok()?;
    let whole: Whole = serde_json::from_slice(bytes).ok()?;
    let summary = whole.summary.object()?;
    let by = summary.by.object()?;
    let of = summary.of.object()?;
    let content = content_of(of.content.as_deref()?)?;
    Some(Summary {
        text: summary.text.filter(|words| !words.is_empty())?,
        instruction: summary.instruction?,
        provider: by.provider?,
        model: by.model?,
        answered: summary.answered,
        made: summary.made?,
        entry: Id::parse(&of.entry?)?,
        generation: Generation(u32::try_from(of.generation?).ok()?),
        records: of.records?,
        content,
        notes: notes(of.notes)?,
        sent: summary.sent?,
        tokens_in: summary.tokens_in,
        tokens_out: summary.tokens_out,
    })
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;
    use serde_json::Value;

    use super::{Notes, Summary, read, retirement, write};

    const ENTRY: &str = "01M6ZGE4KTBYYH07AS2KVMSCAF";
    const EXAMPLE: &str = r#"{"summary":{"text":"Thinning the carrots of the north bed: 142 seedlings counted in four rows.","instruction":"7fdf9f0d93f443e3","by":{"provider":"anthropic","model":"claude-haiku-5-5"},"answered":"claude-haiku-5-5","made":1794107564,"of":{"entry":"01M6ZGE4KTBYYH07AS2KVMSCAF","generation":1,"records":23,"content":"0000000000000000000000000000abcd","notes":{"generation":1,"records":8,"digest":"00000000000000ff"}},"sent":412,"in":790,"out":61}}"#;
    const RUN: &str = r#"{"run":{"kind":"session","instruction":"9f86d081884c7d65","by":{"provider":"anthropic","model":"claude-haiku-5-5"},"answered":"claude-haiku-5-5","same":true,"made":1794107564,"of":{"entry":"01M6ZGE4KTBYYH07AS2KVMSCAF","generation":1,"records":[1,811],"total":1204},"sent":19874,"in":29411,"out":2210,"written":14,"kept":12,"range":1,"names":1,"passes":0,"passed":false}}"#;

    fn id() -> Id {
        Id::parse(ENTRY).expect("id")
    }

    fn example() -> Summary {
        Summary {
            text: "Thinning the carrots of the north bed: 142 seedlings counted in four rows."
                .to_owned(),
            instruction: "7fdf9f0d93f443e3".to_owned(),
            provider: "anthropic".to_owned(),
            model: "claude-haiku-5-5".to_owned(),
            answered: Some("claude-haiku-5-5".to_owned()),
            made: 1_794_107_564,
            entry: id(),
            generation: Generation(1),
            records: 23,
            content: 0xabcd,
            notes: Some(Notes {
                generation: Generation(1),
                records: 8,
                digest: 0xff,
            }),
            sent: 412,
            tokens_in: Some(790),
            tokens_out: Some(61),
        }
    }

    fn bare() -> Summary {
        Summary {
            text: "The garden log.".to_owned(),
            answered: None,
            generation: Generation(2),
            records: 34,
            notes: None,
            sent: 300,
            tokens_in: None,
            tokens_out: None,
            ..example()
        }
    }

    fn changed(path: &[&str], to: Option<Value>) -> String {
        let mut value: Value = serde_json::from_str(EXAMPLE).expect("json");
        let mut at = &mut value;
        for name in &path[..path.len() - 1] {
            at = at.get_mut(*name).expect("a field");
        }
        let fields = at.as_object_mut().expect("an object");
        match to {
            Some(to) => fields.insert((*path.last().expect("a name")).to_owned(), to),
            None => fields.remove(*path.last().expect("a name")),
        };
        value.to_string()
    }

    #[test]
    fn a_record_is_one_json_line_in_the_plans_order() {
        assert_eq!(write(&example()), EXAMPLE);
        assert_eq!(
            write(&bare()),
            r#"{"summary":{"text":"The garden log.","instruction":"7fdf9f0d93f443e3","by":{"provider":"anthropic","model":"claude-haiku-5-5"},"made":1794107564,"of":{"entry":"01M6ZGE4KTBYYH07AS2KVMSCAF","generation":2,"records":34,"content":"0000000000000000000000000000abcd"},"sent":300}}"#
        );
    }

    #[test]
    fn a_record_reads_back_and_passes_over_a_field_it_does_not_know() {
        for summary in [example(), bare()] {
            let line = write(&summary);
            assert_eq!(read(line.as_bytes()), Some(summary.clone()));
            assert_eq!(read(format!("{line}\n").as_bytes()), Some(summary.clone()));
            let inner = line.replacen(r#"{"summary":{"#, r#"{"summary":{"author":"x","#, 1);
            assert_eq!(read(inner.as_bytes()), Some(summary.clone()));
            let outer = line.replacen(r#"{"summary":"#, r#"{"author":"x","summary":"#, 1);
            assert_eq!(read(outer.as_bytes()), Some(summary));
        }
        let deep = format!("{}{}", "[".repeat(130), "]".repeat(130));
        for opening in [
            r#"{"#,
            r#"{"summary":{"#,
            r#""by":{"#,
            r#""of":{"#,
            r#""notes":{"#,
        ] {
            let extra = format!(r#"{opening}"extra":{deep},"#);
            let line = EXAMPLE.replacen(opening, &extra, 1);
            assert_eq!(read(line.as_bytes()), Some(example()), "{opening}");
        }
        for opening in [r#"{"#, r#"{"summary":{"#] {
            let extra = format!(r#"{opening}"extra":"a }} ] , \" b \\","#);
            let line = EXAMPLE.replacen(opening, &extra, 1);
            assert_eq!(read(line.as_bytes()), Some(example()), "{opening}");
        }
        let value: Value = serde_json::from_str(EXAMPLE).expect("json");
        let pretty = serde_json::to_string_pretty(&value).expect("pretty");
        assert_eq!(read(pretty.as_bytes()), Some(example()));
        let twice = EXAMPLE.replacen(r#""made":1794107564"#, r#""made":1,"made":1794107564"#, 1);
        assert_eq!(read(twice.as_bytes()), Some(example()));
        let without_answer = Summary {
            answered: None,
            ..example()
        };
        for (from, to, expected) in [
            (
                r#""answered":"claude-haiku-5-5""#,
                r#""answered":null"#,
                without_answer.clone(),
            ),
            (
                r#""answered":"claude-haiku-5-5""#,
                r#""answered":3"#,
                without_answer,
            ),
            (
                r#""in":790"#,
                r#""in":"12""#,
                Summary {
                    tokens_in: None,
                    ..example()
                },
            ),
            (
                r#""out":61"#,
                r#""out":[[[1]]]"#,
                Summary {
                    tokens_out: None,
                    ..example()
                },
            ),
        ] {
            let line = EXAMPLE.replacen(from, to, 1);
            assert_eq!(read(line.as_bytes()), Some(expected), "{to}");
        }
        for (from, to) in [
            (r#""by":{"#, r#""by":null,"by":{"#),
            (r#"{"summary":{"#, r#"{"summary":3,"summary":{"#),
            (r#""notes":{"#, r#""notes":"x","notes":{"#),
        ] {
            let line = EXAMPLE.replacen(from, to, 1);
            assert_eq!(read(line.as_bytes()), Some(example()), "{to}");
        }
        let no_notes = EXAMPLE.replacen(
            r#","notes":{"generation":1,"records":8,"digest":"00000000000000ff"}"#,
            r#","notes":null"#,
            1,
        );
        assert_eq!(
            read(no_notes.as_bytes()),
            Some(Summary {
                notes: None,
                ..example()
            })
        );
    }

    #[test]
    fn what_is_no_summarys_record_reads_as_none() {
        let mut cases: Vec<String> = vec![
            "not json".to_owned(),
            "{}".to_owned(),
            r#"{"summary":3}"#.to_owned(),
            RUN.to_owned(),
        ];
        for path in [
            &["summary", "text"][..],
            &["summary", "instruction"],
            &["summary", "by", "provider"],
            &["summary", "by", "model"],
            &["summary", "made"],
            &["summary", "of", "entry"],
            &["summary", "of", "generation"],
            &["summary", "of", "records"],
            &["summary", "of", "content"],
            &["summary", "sent"],
        ] {
            cases.push(changed(path, None));
        }
        for (path, to) in [
            (&["summary", "text"][..], "\"\""),
            (&["summary", "of", "entry"], "\"x\""),
            (&["summary", "of", "content"], "\"abcd\""),
            (&["summary", "of", "notes", "digest"], "\"ff\""),
            (
                &["summary", "of", "notes", "digest"],
                "\"zzzzzzzzzzzzzzzz\"",
            ),
        ] {
            cases.push(changed(path, Some(serde_json::from_str(to).expect("json"))));
        }
        cases.push(changed(
            &["summary", "of", "content"],
            Some(Value::String("z".repeat(32))),
        ));
        cases.push(changed(&["summary", "of", "notes", "digest"], None));
        cases.push(format!("{EXAMPLE} x"));
        cases.push(EXAMPLE[..EXAMPLE.len() - 1].to_owned());
        cases.push(format!("[{EXAMPLE}]"));
        cases.push(r#"{"summary":{"text":"a"#.to_owned());
        cases.push(EXAMPLE.replacen(r#""made":1794107564"#, r#""made":"1794107564""#, 1));
        cases.push(EXAMPLE.replacen(
            r#""by":{"provider":"anthropic","model":"claude-haiku-5-5"}"#,
            r#""by":{"provider":"anthropic","model":"claude-haiku-5-5"},"by":null"#,
            1,
        ));
        for case in cases {
            assert_eq!(read(case.as_bytes()), None, "{case}");
        }
        let mut raw = br#"{"extra":"a"#.to_vec();
        raw.extend_from_slice(&[0xff]);
        raw.extend_from_slice(br#"b","#);
        raw.extend_from_slice(&EXAMPLE.as_bytes()[1..]);
        assert_eq!(read(&raw), None);
    }

    #[test]
    fn a_digest_is_sixteen_hex_digits_whatever_its_value() {
        for (digest, text) in [
            (0, "0000000000000000"),
            (255, "00000000000000ff"),
            (u64::MAX, "ffffffffffffffff"),
        ] {
            let summary = Summary {
                notes: Some(Notes {
                    generation: Generation(1),
                    records: 8,
                    digest,
                }),
                ..example()
            };
            let line = write(&summary);
            assert!(line.contains(&format!(r#""digest":"{text}""#)), "{line}");
            assert_eq!(read(line.as_bytes()), Some(summary));
        }
    }

    #[test]
    fn a_retirement_is_one_line_that_no_summary_reads_from() {
        let line = retirement(id(), Generation(3), 0, 0xabcd, 1_794_107_564);
        assert_eq!(
            line,
            r#"{"summary":{"retired":1794107564,"of":{"entry":"01M6ZGE4KTBYYH07AS2KVMSCAF","generation":3,"records":0,"content":"0000000000000000000000000000abcd"}}}"#
        );
        assert_eq!(read(line.as_bytes()), None);
        assert_eq!(read(format!("{line}\n").as_bytes()), None);
        assert_eq!(
            bilbo_store::opaque::filter(line.as_bytes()).as_ref(),
            line.as_bytes()
        );
    }
}
