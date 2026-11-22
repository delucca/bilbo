use bilbo_settings::error::Error;
use bilbo_settings::key::{Value, quote, segment};
use serde_json::{Map, Number, Value as Json};

const TEXT_TABLE: &str = "must be a table of texts";
const FIELD_TABLE: &str = "must be a table of fields";
const FIELD_VALUES: &str = "must hold no date, inf or nan";

pub fn json(table: &Value<'_>) -> Result<Map<String, Json>, Error> {
    fields(table, &[])
}

pub fn fields(table: &Value<'_>, own: &[&str]) -> Result<Map<String, Json>, Error> {
    let entries = table.fields().ok_or_else(|| table.refused(FIELD_TABLE))?;
    let mut map = Map::new();
    for (name, value) in entries {
        if let Some(held) = own_name(name, own) {
            return Err(table.refused(&format!(
                "must not hold {held}: a field bilbo writes itself, one that offers the model a tool, or one that keeps the records at the provider"
            )));
        }
        let converted = convert(&value).ok_or_else(|| table.refused(FIELD_VALUES))?;
        map.insert(name.to_owned(), converted);
    }
    Ok(map)
}

pub fn headers(table: &Value<'_>, own: &[&str]) -> Result<Vec<(String, String)>, Error> {
    let entries = table.fields().ok_or_else(|| table.refused(TEXT_TABLE))?;
    let mut headers = Vec::new();
    for (name, value) in entries {
        if let Some(held) = own_name(name, own) {
            return Err(table.refused(&format!(
                "must not hold {}, a header bilbo writes itself",
                held.to_ascii_lowercase()
            )));
        }
        let text = value
            .text()
            .ok_or_else(|| table.refused("must hold texts alone"))?;
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(table.refused("must name its headers with letters, digits and dashes"));
        }
        if text.chars().any(|c| c.is_control() && c != '\t') {
            return Err(table.refused("must hold no line end or control character"));
        }
        headers.push((name.to_owned(), text.to_owned()));
    }
    Ok(headers)
}

pub fn show_fields(map: &Map<String, Json>) -> String {
    bilbo_settings::key::inline(
        map.iter()
            .map(|(name, value)| format!("{} = {}", segment(name), show_value(value))),
    )
}

fn show_value(value: &Json) -> String {
    match value {
        Json::String(text) => quote(text),
        Json::Array(items) => format!(
            "[{}]",
            items.iter().map(show_value).collect::<Vec<_>>().join(", ")
        ),
        Json::Object(map) => show_fields(map),
        other => other.to_string(),
    }
}

fn own_name<'a>(name: &str, own: &[&'a str]) -> Option<&'a str> {
    own.iter()
        .copied()
        .find(|item| item.eq_ignore_ascii_case(name))
}

pub fn texts(map: &Map<String, Json>) -> Vec<String> {
    fn collect(value: &Json, found: &mut Vec<String>) {
        match value {
            Json::String(text) => found.push(text.clone()),
            Json::Array(items) => items.iter().for_each(|item| collect(item, found)),
            Json::Object(fields) => fields.iter().for_each(|(name, item)| {
                found.push(name.clone());
                collect(item, found);
            }),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for (name, value) in map {
        found.push(name.clone());
        collect(value, &mut found);
    }
    found
}

pub fn reserved(name: &str, list: &[&str]) -> bool {
    list.iter().any(|item| item.eq_ignore_ascii_case(name))
}

fn convert(value: &Value<'_>) -> Option<Json> {
    if let Some(text) = value.text() {
        return Some(Json::String(text.to_owned()));
    }
    if let Some(flag) = value.boolean() {
        return Some(Json::Bool(flag));
    }
    if let Some(number) = value.integer() {
        return Some(Json::Number(number.into()));
    }
    if let Some(number) = value.decimal() {
        return Number::from_f64(number).map(Json::Number);
    }
    if let Some(items) = value.items() {
        return items
            .iter()
            .map(convert)
            .collect::<Option<Vec<_>>>()
            .map(Json::Array);
    }
    let entries = value.fields()?;
    let mut map = Map::new();
    for (name, item) in entries {
        map.insert(name.to_owned(), convert(&item)?);
    }
    Some(Json::Object(map))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bilbo_settings::error::{At, Error};
    use bilbo_settings::file::File;
    use bilbo_settings::key::{About, Default, Key, Source};
    use serde_json::json;

    use bilbo_settings::key::show_texts;

    use super::{headers, reserved, show_fields};

    const ABOUT: About = About {
        name: "t.table",
        variable: None,
        default: Default::Toml("{}"),
        allowed: "a table",
        doc: "A table.",
    };

    fn read<T>(
        parse: fn(&bilbo_settings::key::Value<'_>) -> Result<T, Error>,
        text: &str,
    ) -> Result<T, Error> {
        let key = Key {
            about: ABOUT,
            parse,
            show: |_| String::new(),
        };
        let file = File::parse(Path::new("config.toml"), text.as_bytes()).unwrap();
        let var = |_: &str| None;
        key.resolve(&Source {
            var: &var,
            file: &file,
        })
        .map(|resolved| resolved.value)
    }

    #[test]
    fn a_table_of_toml_values_becomes_the_same_json() {
        let text = "[t.table]\ntext = \"a\"\nwhole = 3\ndecimal = 0.5\nflag = true\nlist = [1, \"b\", [false]]\n[t.table.inner.deeper]\nx = 7\n";
        let map = read(super::json, text).unwrap();
        assert_eq!(
            serde_json::Value::Object(map),
            json!({
                "text": "a",
                "whole": 3,
                "decimal": 0.5,
                "flag": true,
                "list": [1, "b", [false]],
                "inner": {"deeper": {"x": 7}},
            })
        );
        let error = read(super::json, "[t.table]\nwhen = 1979-05-27\n").unwrap_err();
        assert_eq!(
            error,
            Error::Refused {
                at: At::Line {
                    path: Path::new("config.toml").to_path_buf(),
                    line: 1,
                    key: "t.table".to_owned(),
                },
                reason: "must hold no date, inf or nan".to_owned(),
            }
        );
    }

    #[test]
    fn the_texts_of_a_table_are_its_names_and_texts_at_any_depth() {
        let text =
            "[t.table]\na = \"x\"\ne = 2\n[t.table.b]\nc = [\"y\", 1, true, { d = \"z\" }]\n";
        let map = read(super::json, text).unwrap();
        assert_eq!(super::texts(&map), ["a", "x", "b", "c", "y", "d", "z", "e"]);
        assert!(super::texts(&serde_json::Map::new()).is_empty());
    }

    fn own_headers(value: &bilbo_settings::key::Value<'_>) -> Result<Vec<(String, String)>, Error> {
        headers(value, &["authorization", "Host"])
    }

    #[test]
    fn a_headers_table_is_checked_entry_by_entry_in_one_order() {
        let taken = read(own_headers, "[t.table]\nX-A1 = \"one\"\nTab = \"a\\tb\"\n").unwrap();
        assert_eq!(
            taken,
            [
                ("X-A1".to_owned(), "one".to_owned()),
                ("Tab".to_owned(), "a\tb".to_owned())
            ]
        );
        for (text, reason) in [
            ("t.table = \"secret-ONE\"\n", "must be a table of texts"),
            ("[t.table]\nsecret-ONE = 7391\n", "must hold texts alone"),
            (
                "[t.table]\n\"secret ONE\" = \"x\"\n",
                "must name its headers with letters, digits and dashes",
            ),
            (
                "[t.table]\nX-A = \"secret-one\\ntwo\"\n",
                "must hold no line end or control character",
            ),
            (
                "[t.table]\nX-A = \"secret-one\\u0001\"\n",
                "must hold no line end or control character",
            ),
            (
                "[t.table]\nX-A = \"secret-one\\u001b\"\n",
                "must hold no line end or control character",
            ),
            (
                "[t.table]\nX-A = \"secret-one\\u007f\"\n",
                "must hold no line end or control character",
            ),
            (
                "[t.table]\nAUTHORIZATION = 7\n",
                "must not hold authorization, a header bilbo writes itself",
            ),
            (
                "[t.table]\nhost = \"x\"\n",
                "must not hold host, a header bilbo writes itself",
            ),
        ] {
            let error = read(own_headers, text).unwrap_err();
            let Error::Refused {
                at: found,
                reason: given,
            } = &error
            else {
                panic!("{text}: {error:?}");
            };
            assert_eq!(given, reason, "{text}");
            assert!(
                matches!(found, At::Line { key, line: 1, .. } if key == "t.table"),
                "{text}"
            );
            let printed = error.to_string();
            assert!(
                !printed.contains("secret") && !printed.contains("7391"),
                "{text}"
            );
        }
        assert!(reserved("Content-Type", &["host", "content-type"]));
        assert!(!reserved("X-A1", &["host", "content-type"]));
    }

    #[test]
    fn a_table_of_fields_is_printed_as_toml_that_reads_back() {
        let texts = [
            ("X-A".to_owned(), "1".to_owned()),
            ("a.b".to_owned(), "2".to_owned()),
        ];
        let text = "[t.table]\neffort = \"low\"\nn = 2\ndeep = { list = [\"x\", true] }\n";
        let fields = read(super::json, text).unwrap();
        let shown = show_fields(&fields);
        assert_eq!(
            shown,
            "{ deep = { list = [\"x\", true] }, effort = \"low\", n = 2 }"
        );
        assert_eq!(show_fields(&serde_json::Map::new()), "{}");
        let again = read(super::json, &format!("t.table = {shown}\n")).unwrap();
        assert_eq!(again, fields);
        let again = read(super::json, &format!("t.table = {}\n", show_texts(&texts))).unwrap();
        assert_eq!(again["X-A"], "1");
        assert_eq!(again["a.b"], "2");
    }
}
