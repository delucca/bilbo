use std::fs;
use std::path::{Path, PathBuf};

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

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
    let folder = manifest.join("../instructions");
    watch(&folder);
    let mut table = String::new();
    for file in files(&folder)? {
        let name = file.display();
        let stem = file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty() && stem.bytes().all(|byte| byte.is_ascii_lowercase()))
            .ok_or_else(|| format!("{name}: the file name is not lower-case letters"))?;
        let text = fs::read_to_string(&file).map_err(|error| format!("{name}: {error}"))?;
        if text.contains([FIELD, RECORD]) {
            return Err(format!("{name}: holds a separator control character"));
        }
        table.push_str(&format!("{stem}{FIELD}{text}{RECORD}"));
    }
    fs::write(out.join("instructions.dat"), table)
        .map_err(|error| format!("{}: {error}", out.display()))
}

fn files(folder: &Path) -> Result<Vec<PathBuf>, String> {
    let name = folder.display();
    let mut files = Vec::new();
    for entry in fs::read_dir(folder).map_err(|error| format!("{name}: {error}"))? {
        let path = entry.map_err(|error| format!("{name}: {error}"))?.path();
        if path.extension().is_some_and(|extension| extension == "txt") {
            files.push(path);
        }
    }
    if files.is_empty() {
        return Err(format!("{name}: holds no .txt file"));
    }
    files.sort();
    Ok(files)
}
