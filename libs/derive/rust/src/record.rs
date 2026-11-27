use bilbo_store::entries::Generation;
use bilbo_store::id::Id;
use serde_json::{Map, Value, to_string};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub kind: String,
    pub instruction: String,
    pub provider: String,
    pub model: String,
    pub answered: Option<String>,
    pub same: bool,
    pub made: u64,
    pub entry: Id,
    pub generation: Generation,
    pub first: u64,
    pub last: u64,
    pub total: u64,
    pub sent: u64,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub written: u64,
    pub kept: u64,
    pub range: u64,
    pub names: u64,
    pub passes: u64,
    pub passed: bool,
}

pub(crate) fn quoted(text: &str) -> String {
    to_string(text).unwrap_or_default()
}

#[must_use]
pub fn write(run: &Run) -> String {
    let mut line = format!(
        r#"{{"run":{{"kind":{},"instruction":{},"by":{{"provider":{},"model":{}}},"#,
        quoted(&run.kind),
        quoted(&run.instruction),
        quoted(&run.provider),
        quoted(&run.model)
    );
    if let Some(answered) = &run.answered {
        line.push_str(&format!(r#""answered":{},"#, quoted(answered)));
    }
    line.push_str(&format!(
        r#""same":{},"made":{},"of":{{"entry":{},"generation":{},"records":[{},{}],"total":{}}},"sent":{},"#,
        run.same,
        run.made,
        quoted(&run.entry.to_string()),
        run.generation.0,
        run.first,
        run.last,
        run.total,
        run.sent
    ));
    if let Some(tokens) = run.tokens_in {
        line.push_str(&format!(r#""in":{tokens},"#));
    }
    if let Some(tokens) = run.tokens_out {
        line.push_str(&format!(r#""out":{tokens},"#));
    }
    line.push_str(&format!(
        r#""written":{},"kept":{},"range":{},"names":{},"passes":{},"passed":{}}}}}"#,
        run.written, run.kept, run.range, run.names, run.passes, run.passed
    ));
    line
}

fn text(fields: &Map<String, Value>, name: &str) -> Option<String> {
    fields.get(name)?.as_str().map(str::to_owned)
}

fn count(fields: &Map<String, Value>, name: &str) -> Option<u64> {
    fields.get(name)?.as_u64()
}

fn object<'a>(fields: &'a Map<String, Value>, name: &str) -> Option<&'a Map<String, Value>> {
    fields.get(name)?.as_object()
}

#[must_use]
pub fn read(bytes: &[u8]) -> Option<Run> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let run = object(value.as_object()?, "run")?;
    let by = object(run, "by")?;
    let of = object(run, "of")?;
    let [first, last] = of.get("records")?.as_array()?.as_slice() else {
        return None;
    };
    Some(Run {
        kind: text(run, "kind")?,
        instruction: text(run, "instruction")?,
        provider: text(by, "provider")?,
        model: text(by, "model")?,
        answered: text(run, "answered"),
        same: run.get("same")?.as_bool()?,
        made: count(run, "made")?,
        entry: Id::parse(&text(of, "entry")?)?,
        generation: Generation(u32::try_from(count(of, "generation")?).ok()?),
        first: first.as_u64()?,
        last: last.as_u64()?,
        total: count(of, "total")?,
        sent: count(run, "sent")?,
        tokens_in: count(run, "in"),
        tokens_out: count(run, "out"),
        written: count(run, "written")?,
        kept: count(run, "kept")?,
        range: count(run, "range")?,
        names: count(run, "names")?,
        passes: count(run, "passes")?,
        passed: run.get("passed")?.as_bool()?,
    })
}

#[must_use]
pub fn next_same(previous: Option<&Run>, instruction: &str, provider: &str, model: &str) -> bool {
    previous.is_none_or(|run| {
        run.same && run.instruction == instruction && run.provider == provider && run.model == model
    })
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;
    use serde_json::Value;

    use super::{Run, next_same, read, write};

    const EXAMPLE: &str = r#"{"run":{"kind":"session","instruction":"9f86d081884c7d65","by":{"provider":"anthropic","model":"claude-haiku-5-5"},"answered":"claude-haiku-5-5","same":true,"made":1794107564,"of":{"entry":"01M6ZGE4KTBYYH07AS2KVMSCAF","generation":1,"records":[1,811],"total":1204},"sent":19874,"in":29411,"out":2210,"written":14,"kept":12,"range":1,"names":1,"passes":0,"passed":false}}"#;

    fn example() -> Run {
        Run {
            kind: "session".to_owned(),
            instruction: "9f86d081884c7d65".to_owned(),
            provider: "anthropic".to_owned(),
            model: "claude-haiku-5-5".to_owned(),
            answered: Some("claude-haiku-5-5".to_owned()),
            same: true,
            made: 1_794_107_564,
            entry: Id::parse("01M6ZGE4KTBYYH07AS2KVMSCAF").expect("id"),
            generation: Generation(1),
            first: 1,
            last: 811,
            total: 1204,
            sent: 19874,
            tokens_in: Some(29411),
            tokens_out: Some(2210),
            written: 14,
            kept: 12,
            range: 1,
            names: 1,
            passes: 0,
            passed: false,
        }
    }

    #[test]
    fn a_run_record_is_one_json_line_with_its_fields_in_order() {
        assert_eq!(write(&example()), EXAMPLE);
    }

    #[test]
    fn a_written_run_record_reads_back_the_same() {
        let mut passed = example();
        passed.passed = true;
        passed.passes = 2;
        passed.same = false;
        passed.answered = None;
        passed.kind = "a \"kind\"".to_owned();
        for run in [example(), passed] {
            assert_eq!(read(write(&run).as_bytes()), Some(run));
        }
    }

    #[test]
    fn a_record_that_is_no_run_record_reads_as_nothing() {
        let without_total = EXAMPLE.replace(r#","total":1204"#, "");
        assert_ne!(without_total, EXAMPLE);
        for bytes in [
            "{}",
            r#"{"text":"a"}"#,
            "x",
            r#"{"run":{}}"#,
            without_total.as_str(),
        ] {
            assert_eq!(read(bytes.as_bytes()), None, "{bytes}");
        }
    }

    #[test]
    fn a_run_record_has_no_text() {
        let value: Value = serde_json::from_str(&write(&example())).expect("json");
        assert!(value["run"].get("text").is_none());
        assert!(value.get("text").is_none());
    }

    #[test]
    fn the_counts_a_provider_did_not_give_are_absent() {
        let mut run = example();
        run.answered = None;
        run.tokens_in = None;
        run.tokens_out = None;
        let value: Value = serde_json::from_str(&write(&run)).expect("json");
        for name in ["answered", "in", "out"] {
            assert!(value["run"].get(name).is_none(), "{name}");
        }
    }

    #[test]
    fn same_holds_while_every_run_of_the_generation_names_the_same_three() {
        let first = example();
        assert!(next_same(None, "a", "b", "c"));
        assert!(next_same(
            Some(&first),
            &first.instruction,
            &first.provider,
            &first.model
        ));
        assert!(!next_same(
            Some(&first),
            "other",
            &first.provider,
            &first.model
        ));
        assert!(!next_same(
            Some(&first),
            &first.instruction,
            "other",
            &first.model
        ));
        assert!(!next_same(
            Some(&first),
            &first.instruction,
            &first.provider,
            "other"
        ));
        let mut apart = example();
        apart.same = false;
        assert!(!next_same(
            Some(&apart),
            &apart.instruction,
            &apart.provider,
            &apart.model
        ));
    }

    #[test]
    fn a_passes_record_holds_no_model_and_reads_back() {
        let mut pass = example();
        pass.model = String::new();
        pass.answered = None;
        pass.passed = true;
        pass.passes = 1;
        let text = write(&pass);
        assert!(text.contains(r#""model":"""#), "{text}");
        assert_eq!(read(text.as_bytes()), Some(pass.clone()));
        assert!(!next_same(
            Some(&pass),
            &pass.instruction,
            &pass.provider,
            "model-a"
        ));
    }
}
