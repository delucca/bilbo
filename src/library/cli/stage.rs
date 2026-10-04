//! `bilbo library stage`.

use std::fs;
use std::path::Path;

use super::{Args, Output, capture_title, io_failure, refused, root, state_failure, today, usage};
use crate::Failure;
use crate::library::corpus;
use crate::library::hash;
use crate::library::source;
use crate::shared::frontmatter;
use crate::shared::markdown;
use crate::shared::store;

fn is_url(arg: &str) -> bool {
    arg.starts_with("http://") || arg.starts_with("https://")
}

/// Usage errors of a URL argument, found before any request.
fn check_url(args: &Args, url: &str) -> Result<(), Failure> {
    for (given, name) in [
        (args.one("--origin").is_some(), "--origin"),
        (args.one("--fetched").is_some(), "--fetched"),
        (args.html, "--html"),
    ] {
        if given {
            return Err(usage(format!(
                "{name} does not apply to a URL: bilbo sets the origin and the date, and reads the media type"
            )));
        }
    }
    if let Some((_, fragment)) = url.split_once('#') {
        return Err(usage(format!(
            "the URL '{url}' holds the fragment '#{fragment}': stage the whole page, without it"
        )));
    }
    if url
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\\')
    {
        return Err(usage(format!(
            "the URL '{url}' holds whitespace, '\"' or '\\'"
        )));
    }
    Ok(())
}

/// `<letters>://` with a scheme bilbo does not fetch.
fn other_scheme(arg: &str) -> Option<&str> {
    let (scheme, _) = arg.split_once("://")?;
    (!scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphabetic())).then_some(scheme)
}

/// LF line endings and a final newline.
fn normalize(text: &str) -> String {
    let mut capture = text.replace("\r\n", "\n").replace('\r', "\n");
    if !capture.ends_with('\n') {
        capture.push('\n');
    }
    capture
}

/// What a stage holds before it is written.
struct Capture {
    text: String,
    origin: String,
    fetched: String,
    label: &'static str,
    raw: Option<Vec<u8>>,
    fetch: Option<serde_json::Value>,
    /// `media type:` and `final url:` lines, for a URL.
    fetch_lines: Vec<String>,
    page: Option<crate::library::html::Conversion>,
}

fn utf8_text(bytes: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(bytes).ok()?;
    Some(text.strip_prefix('\u{feff}').unwrap_or(text))
}

fn capture_url(url: &str) -> Result<Capture, Failure> {
    use crate::library::fetch::{self, Kind};
    let answer = fetch::get(url).map_err(refused)?;
    let fetched_at = jiff::Zoned::now();
    let media = answer.media_type.clone();
    let kind = fetch::kind(media.as_deref(), &answer.bytes);
    let (text, page) = match kind {
        Kind::Pdf => {
            return Err(refused(format!(
                "{url} is a PDF; extract its text with a PDF tool and stage that text file with --origin \"url: {url}\""
            )));
        }
        Kind::Other(media) => {
            return Err(refused(format!(
                "{url} answered {media}, which bilbo does not stage: it takes HTML and text"
            )));
        }
        Kind::Html | Kind::Text => {
            let text = utf8_text(&answer.bytes)
                .ok_or_else(|| refused(format!("{url} is not valid UTF-8, so it is not text")))?;
            if kind == Kind::Html {
                let page = crate::library::html::convert(text);
                if page.markdown.trim().is_empty() {
                    return Err(refused(format!("{url} converted to no text")));
                }
                (normalize(&page.markdown), Some(page))
            } else {
                if text.trim().is_empty() {
                    return Err(refused(format!("{url} holds only whitespace")));
                }
                (normalize(text), None)
            }
        }
    };
    let mut fetch_lines = vec![format!("media type: {}", media.as_deref().unwrap_or("-"))];
    if answer.redirected {
        fetch_lines.push(format!("final url: {}", answer.final_url));
    }
    let fetch = serde_json::json!({
        "url": url,
        "final_url": answer.final_url,
        "status": answer.status,
        "media_type": media,
        "fetched_at": fetched_at.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        "converter": page
            .as_ref()
            .map(|_| format!("bilbo {}", env!("CARGO_PKG_VERSION"))),
    });
    Ok(Capture {
        text,
        origin: format!("url: {url}"),
        fetched: today(),
        label: "fetched",
        raw: Some(answer.bytes),
        fetch: Some(fetch),
        fetch_lines,
        page,
    })
}

fn capture_file(args: &Args, file: &str) -> Result<Capture, Failure> {
    let origin = args
        .one("--origin")
        .ok_or_else(|| usage("missing --origin \"<url|doc>: <value>\""))?;
    if !source::valid_origin(origin) {
        return Err(usage(format!(
            "invalid --origin '{origin}': write it as \"<url or doc>: <value>\""
        )));
    }
    let fetched = match args.one("--fetched") {
        Some(date) if source::is_date(date) => date.to_string(),
        Some(date) => {
            return Err(usage(format!(
                "--fetched '{date}' is not YYYY-MM-DD, a real date"
            )));
        }
        None => today(),
    };
    let path = Path::new(file);
    let bytes = fs::read(path).map_err(io_failure("read", path))?;
    let text = utf8_text(&bytes)
        .ok_or_else(|| refused(format!("{file} is not valid UTF-8, so it is not text")))?;
    if text.trim().is_empty() {
        return Err(refused(format!("{file} holds only whitespace")));
    }
    let (text, page) = if args.html {
        let page = crate::library::html::convert(text);
        if page.markdown.trim().is_empty() {
            return Err(refused(format!("{file} converted to no text")));
        }
        (normalize(&page.markdown), Some(page))
    } else {
        (normalize(text), None)
    };
    Ok(Capture {
        text,
        origin: origin.to_string(),
        fetched,
        label: "external",
        raw: args.html.then_some(bytes),
        fetch: None,
        fetch_lines: Vec::new(),
        page,
    })
}

/// The `origin` of every source in the library is read, and nothing is written.
fn existing_sources(root: &Path, origin: &str) -> Result<Vec<String>, Failure> {
    let library = store::library_dir(root);
    let mut found = Vec::new();
    for (corpus, dir) in corpus::corpus_dirs(root).map_err(io_failure("read", &library))? {
        for (name, found_origin) in corpus::read_origins(&dir).map_err(io_failure("read", &dir))? {
            if found_origin.as_deref() == Some(origin) {
                found.push(format!("existing: {corpus}/{name}"));
            }
        }
    }
    Ok(found)
}

const LOST_LINES: usize = 10;

fn lost_heading_warnings(page: &crate::library::html::Conversion) -> Vec<String> {
    let lost = crate::library::html::lost_headings(page);
    let mut out: Vec<String> = lost
        .iter()
        .take(LOST_LINES)
        .map(|(level, text)| {
            format!("heading lost: <h{level}> '{text}' is not a heading in the capture")
        })
        .collect();
    if lost.len() > LOST_LINES {
        out.push(format!("heading lost: {} more", lost.len() - LOST_LINES));
    }
    out
}

pub fn run(args: &Args, env: &store::Env) -> Result<Output, Failure> {
    let [target] = args.operands(["<url>|<file>"])?;
    let url = is_url(target);
    if url {
        check_url(args, target)?;
    } else if let Some(scheme) = other_scheme(target) {
        return Err(usage(format!(
            "cannot fetch '{target}': bilbo fetches http and https, not {scheme}"
        )));
    }
    let staging = store::staging_dir(env).ok_or_else(state_failure)?;
    let captured = if url {
        capture_url(target)?
    } else {
        capture_file(args, target)?
    };
    let root = root(env)?;
    let existing = existing_sources(&root, &captured.origin)?;
    let capture = &captured.text;
    let lines = markdown::lines(capture);

    let content = captured
        .page
        .as_ref()
        .and_then(|p| p.content.as_deref())
        .and_then(|c| crate::library::html::content_lines(capture, c));
    let title = content
        .and_then(|(a, b)| {
            markdown::outside_fences(&lines)
                .into_iter()
                .filter(|i| (a..=b).contains(&(i + 1)))
                .find_map(|i| match markdown::heading(lines[i]) {
                    Some((1, text)) => Some((i + 1, text)),
                    _ => None,
                })
        })
        .or_else(|| capture_title(&lines));
    let blank = |l: &&str| l.trim().is_empty();
    let last = match content {
        Some((_, b)) => lines[..b]
            .iter()
            .rposition(|l| !blank(l))
            .map_or(0, |i| i + 1),
        None => lines.iter().rposition(|l| !blank(l)).map_or(0, |i| i + 1),
    };
    let first = match (&title, content) {
        (Some((line, _)), None) => line + 1,
        (Some((line, _)), Some((a, b))) if (a..=b).contains(line) => line + 1,
        (_, Some((a, _))) => a,
        (None, None) => lines.iter().position(|l| !blank(l)).map_or(1, |i| i + 1),
    };

    let id =
        frontmatter::mint_ulid().map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?;
    let folder = staging.join(&id);
    fs::create_dir_all(&staging).map_err(io_failure("create", &staging))?;
    fs::create_dir(&folder).map_err(io_failure("create", &folder))?;
    let record = serde_json::json!({
        "origin": captured.origin,
        "fetched": captured.fetched,
        "capture": captured.label,
        "sha256": hash::sha256_hex(capture.as_bytes()),
    });
    let capture_path = folder.join("capture.md");
    let written = (|| {
        fs::write(&capture_path, capture).map_err(io_failure("write", &capture_path))?;
        if let Some(raw) = &captured.raw {
            let raw_path = folder.join("raw");
            fs::write(&raw_path, raw).map_err(io_failure("write", &raw_path))?;
        }
        if let Some(fetch) = &captured.fetch {
            let fetch_path = folder.join("fetch.json");
            fs::write(&fetch_path, fetch.to_string()).map_err(io_failure("write", &fetch_path))?;
        }
        let record_path = folder.join("stage.json");
        fs::write(&record_path, record.to_string()).map_err(io_failure("write", &record_path))
    })();
    if let Err(failure) = written {
        let _ = fs::remove_dir_all(&folder);
        return Err(failure);
    }

    let mut out = vec![
        format!("stage: {id}"),
        format!("capture: {}", capture_path.display()),
    ];
    if captured.raw.is_some() {
        out.push(format!("raw: {}", folder.join("raw").display()));
    }
    out.extend(captured.fetch_lines.iter().cloned());
    if captured.page.is_some() {
        out.push(match content {
            Some((a, b)) => format!("content: {a}-{b}"),
            None => "content: -".into(),
        });
    }
    out.extend(existing);
    out.push(format!("lines: {}", lines.len()));
    out.push(format!("tokens: {}", markdown::tokens(capture.len())));
    out.push(format!(
        "title: {}",
        title.as_ref().map_or("-", |(_, text)| text)
    ));
    out.push(if first <= last {
        format!("keep: {first}-{last}")
    } else {
        "keep: -".into()
    });
    out.push(String::new());
    for i in markdown::outside_fences(&lines) {
        if markdown::heading(lines[i]).is_some_and(|(level, _)| level <= 2) {
            out.push(format!("{}\t{}", i + 1, lines[i]));
        }
    }
    let mut warnings = source::capture_warnings(&lines);
    if let Some(page) = &captured.page {
        warnings.extend(lost_heading_warnings(page));
    }
    Ok(Output {
        warnings,
        lines: out,
    })
}
