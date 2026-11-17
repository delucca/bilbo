use std::fs;
use std::path::{Path, PathBuf};

use toml::de::{DeTable, DeValue};

const TEXTS: [&str; 2] = ["name", "source"];
const LISTS: [&str; 3] = ["function_words", "detected", "not_detected"];

#[expect(
    clippy::disallowed_methods,
    reason = "Cargo names the folders through its variables"
)]
fn folders() -> Result<(PathBuf, PathBuf), String> {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR is not set")?;
    let out = std::env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?;
    Ok((PathBuf::from(manifest), PathBuf::from(out)))
}

#[expect(
    clippy::disallowed_macros,
    reason = "Cargo reads its instructions from stdout"
)]
fn watch(folder: &Path) {
    println!("cargo::rerun-if-changed={}", folder.display());
}

fn main() -> Result<(), String> {
    let (manifest, out) = folders()?;
    let locales = manifest.join("../locales");
    watch(&locales);
    let mut table = String::from("pub static ALL: &[Language] = &[\n");
    for file in files(&locales)? {
        table.push_str(&language(&file)?);
    }
    table.push_str("];\n");
    fs::write(out.join("table.rs"), table).map_err(|error| format!("{}: {error}", out.display()))
}

fn files(locales: &Path) -> Result<Vec<PathBuf>, String> {
    let folder = locales.display();
    let mut files = Vec::new();
    for entry in fs::read_dir(locales).map_err(|error| format!("{folder}: {error}"))? {
        let path = entry.map_err(|error| format!("{folder}: {error}"))?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "toml")
        {
            files.push(path);
        }
    }
    if files.is_empty() {
        return Err(format!("{folder}: holds no .toml file"));
    }
    files.sort();
    Ok(files)
}

fn language(file: &Path) -> Result<String, String> {
    let name = file.display();
    let code = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| stem.len() == 2 && stem.bytes().all(|byte| byte.is_ascii_lowercase()))
        .ok_or_else(|| format!("{name}: the file name is not two lowercase letters"))?;
    let text = fs::read_to_string(file).map_err(|error| format!("{name}: {error}"))?;
    let root = DeTable::parse(&text).map_err(|error| syntax(file, &text, &error))?;
    let table = root.get_ref();
    for (key, _) in table {
        let key = key.get_ref();
        if !TEXTS.contains(&key.as_ref()) && !LISTS.contains(&key.as_ref()) {
            return Err(format!("{name}: unknown key {key}"));
        }
    }
    let mut out = format!("    Language {{\n        code: {code:?},\n");
    for key in TEXTS.into_iter().chain(LISTS) {
        let value = table
            .get(key)
            .ok_or_else(|| format!("{name}: missing key {key}"))?;
        let rendered = match (value.get_ref(), TEXTS.contains(&key)) {
            (DeValue::String(text), true) => format!("{:?}", text.as_ref()),
            (DeValue::Array(items), false) => {
                let mut words = Vec::new();
                for item in items {
                    let DeValue::String(word) = item.get_ref() else {
                        return Err(format!("{name}: {key} holds an item that is not a string"));
                    };
                    words.push(format!("{:?}", word.as_ref()));
                }
                format!("&[{}]", words.join(", "))
            }
            _ => return Err(format!("{name}: {key} has the wrong type")),
        };
        out.push_str(&format!("        {key}: {rendered},\n"));
    }
    out.push_str("    },\n");
    Ok(out)
}

fn syntax(file: &Path, text: &str, error: &toml::de::Error) -> String {
    let start = error.span().map_or(0, |span| span.start);
    let line = 1 + text
        .bytes()
        .take(start)
        .filter(|&byte| byte == b'\n')
        .count();
    let message = error.message().lines().next().unwrap_or_default();
    format!("{}:{line}: {message}", file.display())
}
