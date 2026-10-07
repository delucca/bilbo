//! Terminal views: whether a stream gets the human view, how it paints, and the text helpers the
//! views share.

use std::path::{Path, PathBuf};

use crate::shared::{frontmatter, store};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// How one stream prints: decided once in `main`, passed to every view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    /// A terminal, and no agent runs bilbo: print the human view.
    pub human: bool,
    /// The human view may use escapes.
    pub paint: bool,
    /// Marks, `›` and `…` in Unicode, else their ASCII forms.
    pub unicode: bool,
    /// Content width in columns, 40..=100.
    pub width: usize,
    /// `$HOME` when absolute, for `~/` paths.
    pub home: Option<PathBuf>,
}

/// Probes the stream (a terminal or not, its columns) and decides.
pub fn open(stream: Stream, env: &store::Env) -> Term {
    let term = match stream {
        Stream::Stdout => console::Term::stdout(),
        Stream::Stderr => console::Term::stderr(),
    };
    let columns = term
        .size_checked()
        .map(|(_, columns)| columns as usize)
        .filter(|columns| *columns > 0);
    decide(term.is_term(), columns, env)
}

/// The rule of the `cli` spec's Terminal views and Human view escapes.
pub fn decide(terminal: bool, columns: Option<usize>, env: &store::Env) -> Term {
    let set = |v: &Option<std::ffi::OsString>| v.as_ref().is_some_and(|v| !v.is_empty());
    let human = terminal && !env.agent();
    let colour = !set(&env.no_color)
        && env.clicolor.as_deref() != Some("0".as_ref())
        && env
            .term
            .as_ref()
            .is_some_and(|t| !t.is_empty() && t != "dumb");
    let unicode = cfg!(target_os = "macos")
        || env
            .lang
            .as_ref()
            .and_then(|l| l.to_str())
            .is_some_and(|l| l.to_uppercase().ends_with("UTF-8"));
    let width = columns
        .or_else(|| {
            env.columns
                .as_ref()
                .and_then(|c| c.to_str())
                .and_then(|c| c.parse::<usize>().ok())
                .filter(|c| *c > 0)
        })
        .unwrap_or(80)
        .clamp(40, 100);
    Term {
        human,
        paint: human && colour,
        unicode,
        width,
        home: store::absolute(&env.home),
    }
}

/// Points console's colour flags, which cliclack reads, at stderr's decision: cliclack follows the
/// stdout flag, and bilbo prints nothing to stdout through console.
pub fn init(stderr: &Term) {
    console::set_colors_enabled(stderr.paint);
    console::set_colors_enabled_stderr(stderr.paint);
}

#[derive(Clone, Copy)]
pub enum Tone {
    Bold,
    Dim,
    Cyan,
    Green,
    Yellow,
    Red,
    Blue,
}

impl Tone {
    fn codes(self) -> (&'static str, &'static str) {
        match self {
            Tone::Bold => ("\x1b[1m", "\x1b[22m"),
            Tone::Dim => ("\x1b[2m", "\x1b[22m"),
            Tone::Red => ("\x1b[31m", "\x1b[39m"),
            Tone::Green => ("\x1b[32m", "\x1b[39m"),
            Tone::Yellow => ("\x1b[33m", "\x1b[39m"),
            Tone::Blue => ("\x1b[34m", "\x1b[39m"),
            Tone::Cyan => ("\x1b[36m", "\x1b[39m"),
        }
    }
}

/// `text` in `tone`, closed by the tone's own off-code so tones nest; `text` itself when
/// `!term.paint` or `text` is empty.
pub fn paint(term: &Term, tone: Tone, text: &str) -> String {
    if !term.paint || text.is_empty() {
        return text.to_string();
    }
    let (open, close) = tone.codes();
    format!("{open}{text}{close}")
}

#[derive(Clone, Copy)]
pub enum Mark {
    Done,
    Kept,
    Skipped,
    Warning,
    Error,
    Info,
    Path,
    Dot,
    Cut,
}

/// The mark in Unicode, or in the ASCII form cliclack falls back to.
pub fn glyph(term: &Term, mark: Mark) -> &'static str {
    let (unicode, ascii) = match mark {
        Mark::Done => ("◆", "*"),
        Mark::Kept => ("◇", "o"),
        Mark::Skipped => ("○", "-"),
        Mark::Warning => ("▲", "!"),
        Mark::Error => ("■", "x"),
        Mark::Info => ("●", "•"),
        Mark::Path => ("›", ">"),
        Mark::Dot => ("·", "-"),
        Mark::Cut => ("…", "..."),
    };
    if term.unicode { unicode } else { ascii }
}

/// The glyph in its tone.
pub fn mark(term: &Term, mark: Mark) -> String {
    let tone = match mark {
        Mark::Done => Tone::Green,
        Mark::Warning => Tone::Yellow,
        Mark::Error => Tone::Red,
        Mark::Info => Tone::Blue,
        Mark::Kept | Mark::Skipped | Mark::Path | Mark::Dot | Mark::Cut => Tone::Dim,
    };
    paint(term, tone, glyph(term, mark))
}

/// `<mark>  <text>` wrapped to `term.width`, later lines indented three columns.
pub fn marked(term: &Term, lead: Mark, text: &str) -> Vec<String> {
    let lead = mark(term, lead);
    let width = term.width.saturating_sub(3).max(1);
    let mut lines = Vec::new();
    for physical in text.split('\n') {
        for line in wrap(term, physical, width, Long::Split) {
            lines.push(match lines.is_empty() {
                true => format!("{lead}  {line}"),
                false if line.is_empty() => line,
                false => format!("   {line}"),
            });
        }
    }
    lines
}

/// Display columns of `text`, escapes ignored.
pub fn width_of(text: &str) -> usize {
    console::measure_text_width(text)
}

/// `text` cut to `width` columns with a dim `…` (or `...`) when longer; escapes kept.
pub fn cut(term: &Term, text: &str, width: usize) -> String {
    if width_of(text) <= width {
        return text.to_string();
    }
    let cut = console::truncate_str(text, width, &mark(term, Mark::Cut)).into_owned();
    squeeze(cut)
}

/// `text` without an escape that opens and closes with nothing between.
fn squeeze(mut text: String) -> String {
    const EMPTY: [&str; 7] = [
        "\x1b[1m\x1b[22m",
        "\x1b[2m\x1b[22m",
        "\x1b[31m\x1b[39m",
        "\x1b[32m\x1b[39m",
        "\x1b[33m\x1b[39m",
        "\x1b[34m\x1b[39m",
        "\x1b[36m\x1b[39m",
    ];
    loop {
        let before = text.len();
        for empty in EMPTY {
            text = text.replace(empty, "");
        }
        if text.len() == before {
            return text;
        }
    }
}

/// What `wrap` does with a word wider than the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Long {
    /// Cut it with a dim `…`: for previews.
    Cut,
    /// Split it into pieces by display columns, so every character stays.
    Split,
}

/// Greedy word wrap of `text` (split on single spaces, widths by `width_of`, so painted words
/// keep their codes) to lines of at most `width` columns; a word wider than `width` is cut or
/// split as `long` says (a painted word is always cut).
pub fn wrap(term: &Term, text: &str, width: usize, long: Long) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        if long == Long::Split && width_of(word) > width && !word.contains('\x1b') {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let mut pieces = fold(word, width);
            line = pieces.pop().unwrap_or_default();
            lines.extend(pieces);
            continue;
        }
        let word = cut(term, word, width);
        if line.is_empty() {
            line = word;
        } else if width_of(&line) + 1 + width_of(&word) <= width {
            line.push(' ');
            line.push_str(&word);
        } else {
            lines.push(std::mem::replace(&mut line, word));
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// `head` and `tail` joined by `sep` on one line when they fit `width`; else `head` folded, then
/// `tail` on its own lines, broken only after a `/` (a tail without one is split by columns).
pub fn tail_lines(head: &str, tail: &str, sep: &str, width: usize) -> Vec<String> {
    if head.is_empty() {
        return break_path(tail, width);
    }
    let joined = format!("{head}{sep}{tail}");
    if width_of(&joined) <= width {
        return vec![joined];
    }
    let mut lines = fold(head, width);
    lines.extend(break_path(tail, width));
    lines
}

fn break_path(path: &str, width: usize) -> Vec<String> {
    if width_of(path) <= width {
        return vec![path.to_string()];
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    for segment in path.split_inclusive('/') {
        if !line.is_empty() && width_of(&line) + width_of(segment) > width {
            lines.push(std::mem::take(&mut line));
        }
        line.push_str(segment);
    }
    let mut out = Vec::new();
    for line in lines.into_iter().chain(Some(line)) {
        out.extend(fold(&line, width));
    }
    out
}

/// Like `wrap` on unpainted text, but a word wider than `width` is split into `width`-column
/// pieces instead of cut: for info lines whose long token is a path a person copies.
pub fn fold(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let mut word = word.to_string();
        if !line.is_empty() && width_of(&line) + 1 + width_of(&word) <= width {
            line.push(' ');
            line.push_str(&word);
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        while width_of(&word) > width {
            let mut cut_at = 0;
            let mut columns = 0;
            for c in word.chars() {
                let w = width_of(c.encode_utf8(&mut [0; 4]));
                if columns + w > width {
                    break;
                }
                columns += w;
                cut_at += c.len_utf8();
            }
            if cut_at == 0 {
                cut_at = word.chars().next().map_or(0, char::len_utf8);
            }
            let rest = word.split_off(cut_at);
            lines.push(std::mem::replace(&mut word, rest));
        }
        line = word;
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

pub enum Align {
    Left,
    Right,
}

/// `text` padded with spaces to `width` columns, on the right (`Align::Left`) or left
/// (`Align::Right`); escapes ignored.
pub fn pad(text: &str, width: usize, align: Align) -> String {
    let fill = " ".repeat(width.saturating_sub(width_of(text)));
    match align {
        Align::Left => format!("{text}{fill}"),
        Align::Right => format!("{fill}{text}"),
    }
}

/// Aligned table rows: each row's cells padded to the column's widest, two spaces apart, no line
/// ending in spaces. `right[i]` right-aligns column i.
pub fn table(rows: &[Vec<String>], right: &[bool]) -> Vec<String> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|i| {
            rows.iter()
                .filter_map(|row| row.get(i))
                .map(|cell| width_of(cell))
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(i, cell)| {
                    if right.get(i).copied().unwrap_or(false) {
                        pad(cell, widths[i], Align::Right)
                    } else {
                        pad(cell, widths[i], Align::Left)
                    }
                })
                .collect();
            cells.join("  ").trim_end().to_string()
        })
        .collect()
}

/// `path` from `~/` when it is under `term.home`, else as is.
pub fn tilde(term: &Term, path: &Path) -> String {
    match term.home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// `text` with each `<home>/` that starts it or follows a space, `(` or `//` written `~/`
/// (`file:///home/a/sync` becomes `file://~/sync`).
pub fn tilde_text(term: &Term, text: &str) -> String {
    let Some(home) = term.home.as_deref() else {
        return text.to_string();
    };
    let home = format!("{}/", home.display());
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(&home) {
        let before = &rest[..at];
        let lead = (out.is_empty() && before.is_empty())
            || before.ends_with([' ', '('])
            || before.ends_with("//");
        out.push_str(before);
        out.push_str(if lead { "~/" } else { &home });
        rest = &rest[at + home.len()..];
    }
    out.push_str(rest);
    out
}

/// A step of a report (`setup`, `device init` and `recover`): `<step> <status>[: <detail>]`.
pub struct Step {
    pub step: String,
    pub status: String,
    pub detail: Option<String>,
}

/// One line per step: the status's mark, then step, status and detail in aligned columns.
pub fn steps(term: &Term, steps: &[Step]) -> Vec<String> {
    let rows: Vec<Vec<String>> = steps
        .iter()
        .map(|s| {
            let (lead, tone) = match s.status.as_str() {
                "created" | "installed" | "written" | "updated" | "ok" | "removed"
                | "recovered" => (Mark::Done, Tone::Green),
                "kept" => (Mark::Kept, Tone::Dim),
                "skipped" => (Mark::Skipped, Tone::Dim),
                "failed" => (Mark::Error, Tone::Red),
                _ => (Mark::Warning, Tone::Yellow),
            };
            vec![
                mark(term, lead),
                s.step.clone(),
                paint(term, tone, &s.status),
                s.detail
                    .as_deref()
                    .map(|d| tilde_text(term, d))
                    .unwrap_or_default(),
            ]
        })
        .collect();
    table(&rows, &[])
}

/// The age of a `created`-form time at `now`: "just now", "N min ago", "1 hour ago", "N hours
/// ago", "yesterday", "N days ago" (up to 30 days), else the date as written; a future time gives
/// the date, and a `created` that does not parse comes back as is.
pub fn ago(now: jiff::Timestamp, created: &str) -> String {
    let Some(then) = frontmatter::created_time(created) else {
        return created.to_string();
    };
    let seconds = now.as_second() - then.as_second();
    match seconds {
        ..0 => created[..10].to_string(),
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..7200 => "1 hour ago".into(),
        7200..86400 => format!("{} hours ago", seconds / 3600),
        86400..172800 => "yesterday".into(),
        172800..=2592000 => format!("{} days ago", seconds / 86400),
        _ => created[..10].to_string(),
    }
}

/// 779594 -> "779,594".
pub fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 1024-based, whole KB rounded up under 1024 KB ("193 KB"), else one decimal ("1.9 MB").
pub fn size(bytes: u64) -> String {
    let kb = bytes.div_ceil(1024);
    if kb < 1024 {
        format!("{kb} KB")
    } else {
        format!("{:.1} MB", bytes as f64 / 1048576.0)
    }
}

/// "1 note" / "2 notes": the count grouped, with `one` or `many`.
pub fn count(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", group(n), if n == 1 { one } else { many })
}

#[cfg(test)]
pub fn fixed(width: usize, paint: bool, unicode: bool) -> Term {
    Term {
        human: true,
        paint,
        unicode,
        width,
        home: Some("/home/a".into()),
    }
}

/// An expected string with its escape codes written as `{b}` `{/b}`, `{d}`, `{c}`, `{g}`, `{y}`,
/// `{r}` and `{u}`.
#[cfg(test)]
pub fn styled(notation: &str) -> String {
    const CODES: [(&str, &str); 14] = [
        ("{b}", "\x1b[1m"),
        ("{/b}", "\x1b[22m"),
        ("{d}", "\x1b[2m"),
        ("{/d}", "\x1b[22m"),
        ("{c}", "\x1b[36m"),
        ("{/c}", "\x1b[39m"),
        ("{g}", "\x1b[32m"),
        ("{/g}", "\x1b[39m"),
        ("{y}", "\x1b[33m"),
        ("{/y}", "\x1b[39m"),
        ("{r}", "\x1b[31m"),
        ("{/r}", "\x1b[39m"),
        ("{u}", "\x1b[34m"),
        ("{/u}", "\x1b[39m"),
    ];
    CODES
        .iter()
        .fold(notation.to_string(), |text, (name, code)| {
            text.replace(name, code)
        })
}

/// `line` without its escape codes.
#[cfg(test)]
pub fn stripped(line: &str) -> String {
    console::strip_ansi_codes(line).into_owned()
}

/// An expected string without its notation.
#[cfg(test)]
pub fn plain(notation: &str) -> String {
    console::strip_ansi_codes(&styled(notation)).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> store::Env {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        store::Env::from_vars(move |name| {
            vars.iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str().into())
        })
    }

    #[test]
    fn decide_follows_the_cli_spec() {
        // (terminal, columns, vars) -> (human, paint, width); width None where it is unused
        #[allow(clippy::type_complexity)]
        let table: [(
            bool,
            Option<usize>,
            &[(&str, &str)],
            bool,
            bool,
            Option<usize>,
        ); 21] = [
            (
                true,
                Some(120),
                &[("TERM", "xterm-256color")],
                true,
                true,
                Some(100),
            ),
            (true, Some(60), &[("TERM", "xterm")], true, true, Some(60)),
            (true, Some(20), &[("TERM", "xterm")], true, true, Some(40)),
            (
                true,
                None,
                &[("TERM", "xterm"), ("COLUMNS", "70")],
                true,
                true,
                Some(70),
            ),
            (
                true,
                None,
                &[("TERM", "xterm"), ("COLUMNS", "abc")],
                true,
                true,
                Some(80),
            ),
            (
                true,
                None,
                &[("TERM", "xterm"), ("COLUMNS", "0")],
                true,
                true,
                Some(80),
            ),
            (false, Some(120), &[("TERM", "xterm")], false, false, None),
            (
                true,
                Some(100),
                &[("TERM", "xterm"), ("AI_AGENT", "x")],
                false,
                false,
                None,
            ),
            (
                true,
                Some(100),
                &[("TERM", "xterm"), ("CLAUDE_CODE_CHILD_SESSION", "1")],
                false,
                false,
                None,
            ),
            (
                true,
                Some(100),
                &[("TERM", "xterm"), ("CODEX_CI", "1")],
                false,
                false,
                None,
            ),
            (
                true,
                Some(100),
                &[("TERM", "xterm"), ("CODEX_THREAD_ID", "t")],
                false,
                false,
                None,
            ),
            (
                true,
                Some(100),
                &[("TERM", "xterm"), ("CLAUDECODE", "1")],
                true,
                true,
                Some(100),
            ),
            (
                true,
                Some(100),
                &[("TERM", "xterm"), ("AI_AGENT", "")],
                true,
                true,
                Some(100),
            ),
            (
                true,
                Some(100),
                &[("TERM", "xterm"), ("NO_COLOR", "1")],
                true,
                false,
                Some(100),
            ),
            (
                true,
                Some(100),
                &[("TERM", "xterm"), ("NO_COLOR", "")],
                true,
                true,
                Some(100),
            ),
            (
                true,
                Some(100),
                &[("TERM", "xterm"), ("CLICOLOR", "0")],
                true,
                false,
                Some(100),
            ),
            (
                true,
                Some(100),
                &[("TERM", "xterm"), ("CLICOLOR", "1")],
                true,
                true,
                Some(100),
            ),
            (true, Some(100), &[("TERM", "dumb")], true, false, Some(100)),
            (true, Some(100), &[], true, false, Some(100)),
            (true, Some(100), &[("TERM", "")], true, false, Some(100)),
            (
                false,
                None,
                &[
                    ("TERM", "xterm"),
                    ("CLICOLOR_FORCE", "1"),
                    ("FORCE_COLOR", "1"),
                ],
                false,
                false,
                None,
            ),
        ];
        for (terminal, columns, vars, human, paint, width) in table {
            let t = decide(terminal, columns, &env(vars));
            assert_eq!((t.human, t.paint), (human, paint), "{terminal} {vars:?}");
            if let Some(width) = width {
                assert_eq!(t.width, width, "{vars:?}");
            }
        }
    }

    #[test]
    fn home_is_kept_only_when_absolute() {
        assert_eq!(
            decide(true, None, &env(&[("HOME", "/home/a")])).home,
            Some(PathBuf::from("/home/a"))
        );
        assert_eq!(decide(true, None, &env(&[("HOME", "a")])).home, None);
        assert_eq!(decide(true, None, &env(&[])).home, None);
    }

    #[test]
    fn unicode_follows_console_rule() {
        let on_macos = cfg!(target_os = "macos");
        let lang = |l: &str| decide(true, None, &env(&[("LANG", l)])).unicode;
        assert!(lang("en_US.UTF-8"));
        assert!(lang("en_US.utf-8"));
        assert_eq!(lang("C"), on_macos);
        assert_eq!(decide(true, None, &env(&[])).unicode, on_macos);
    }

    #[test]
    fn paint_closes_with_its_own_code() {
        let on = fixed(80, true, true);
        let off = fixed(80, false, true);
        for (tone, open, close) in [
            (Tone::Bold, "1", "22"),
            (Tone::Dim, "2", "22"),
            (Tone::Red, "31", "39"),
            (Tone::Green, "32", "39"),
            (Tone::Yellow, "33", "39"),
            (Tone::Blue, "34", "39"),
            (Tone::Cyan, "36", "39"),
        ] {
            assert_eq!(paint(&on, tone, "x"), format!("\x1b[{open}mx\x1b[{close}m"));
        }
        assert_eq!(paint(&off, Tone::Bold, "x"), "x");
        assert_eq!(paint(&on, Tone::Bold, ""), "");
        assert_eq!(
            paint(
                &on,
                Tone::Dim,
                &format!("{} · 5 days ago", paint(&on, Tone::Cyan, "decision"))
            ),
            styled("{d}{c}decision{/c} · 5 days ago{/d}")
        );
    }

    #[test]
    fn marks_match_cliclack() {
        let (uni, ascii) = (fixed(80, false, true), fixed(80, false, false));
        for (m, u, a) in [
            (Mark::Done, "◆", "*"),
            (Mark::Kept, "◇", "o"),
            (Mark::Skipped, "○", "-"),
            (Mark::Warning, "▲", "!"),
            (Mark::Error, "■", "x"),
            (Mark::Info, "●", "•"),
            (Mark::Path, "›", ">"),
            (Mark::Dot, "·", "-"),
            (Mark::Cut, "…", "..."),
        ] {
            assert_eq!(glyph(&uni, m), u);
            assert_eq!(glyph(&ascii, m), a);
        }
        let on = fixed(80, true, true);
        assert_eq!(mark(&on, Mark::Done), styled("{g}◆{/g}"));
        assert_eq!(mark(&on, Mark::Error), styled("{r}■{/r}"));
        assert_eq!(mark(&on, Mark::Warning), styled("{y}▲{/y}"));
        assert_eq!(mark(&on, Mark::Info), styled("{u}●{/u}"));
        for m in [Mark::Kept, Mark::Skipped, Mark::Path, Mark::Dot, Mark::Cut] {
            assert_eq!(
                mark(&on, m),
                styled(&format!("{{d}}{}{{/d}}", glyph(&on, m)))
            );
        }
    }

    #[test]
    fn marked_wraps_under_the_text() {
        let t = fixed(40, false, true);
        let lines = marked(
            &t,
            Mark::Warning,
            "embedder unavailable (embedder http://127.0.0.1:8081 unreachable: Connection refused (os error 61)); keyword results only",
        );
        assert_eq!(
            lines,
            [
                "▲  embedder unavailable (embedder",
                "   http://127.0.0.1:8081 unreachable:",
                "   Connection refused (os error 61));",
                "   keyword results only",
            ]
        );
        assert!(lines.iter().all(|l| width_of(l) <= 40));
        assert_eq!(marked(&t, Mark::Warning, "a\nb"), ["▲  a", "   b"]);
    }

    #[test]
    fn cut_and_wrap_measure_display_columns() {
        let (off, ascii) = (fixed(80, false, true), fixed(80, false, false));
        assert_eq!(width_of("日本"), 4);
        assert_eq!(cut(&off, "abcdefgh", 5), "abcd…");
        assert_eq!(cut(&ascii, "abcdefgh", 5), "ab...");
        assert_eq!(cut(&off, "abc", 5), "abc");
        let on = fixed(80, true, true);
        assert_eq!(
            cut(&on, &paint(&on, Tone::Bold, "abcdef"), 4),
            styled("{b}abc{d}…{/d}{/b}")
        );
        assert_eq!(wrap(&off, "aaa bbb ccc", 7, Long::Cut), ["aaa bbb", "ccc"]);
        assert_eq!(wrap(&off, "abcdefghijkl", 5, Long::Cut), ["abcd…"]);
        assert_eq!(wrap(&off, "", 5, Long::Cut), [""]);
    }

    #[test]
    fn split_keeps_every_character_of_a_long_word() {
        let off = fixed(40, false, true);
        let path = format!("/{}", "segment/".repeat(15));
        let message = format!("no scope syncs; set it in {path} now");
        let lines = marked(&off, Mark::Warning, &message);
        assert!(lines.iter().all(|l| width_of(l) <= 40));
        assert!(lines.iter().all(|l| !l.contains('…')));
        let joined: String = lines
            .iter()
            .map(|l| l.trim_start_matches("▲  ").trim_start())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(joined.replace(' ', ""), message.replace(' ', ""));
        assert_eq!(
            wrap(&off, "ab abcdefghij", 4, Long::Split),
            ["ab", "abcd", "efgh", "ij"]
        );
    }

    #[test]
    fn a_tail_breaks_after_a_slash_never_inside_a_segment() {
        let path = "~/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:9";
        assert_eq!(
            tail_lines("gotcha · 17 min ago", path, " · ", 46),
            [
                "gotcha · 17 min ago",
                "~/.local/share/bilbo/notes/",
                "gotcha-sqlite-busy-timeout.md:9"
            ]
        );
        assert_eq!(
            tail_lines("gotcha", "~/a/b.md:9", " · ", 46),
            ["gotcha · ~/a/b.md:9"]
        );
        assert_eq!(tail_lines("", "01M3EZ86", " · ", 4), ["01M3", "EZ86"]);
    }

    #[test]
    fn a_cut_leaves_no_empty_escape_pairs() {
        let on = fixed(80, true, true);
        let text = format!("{} {}", paint(&on, Tone::Bold, "abcdef"), "ghijkl");
        assert!(!cut(&on, &text, 4).contains("\x1b[22m\x1b[2m\x1b[22m"));
        assert_eq!(squeeze("a\x1b[2m\x1b[22mb".into()), "ab");
    }

    #[test]
    fn an_unknown_step_status_is_a_warning() {
        let on = fixed(80, false, true);
        let rows = [Step {
            step: "x".into(),
            status: "weird".into(),
            detail: None,
        }];
        assert_eq!(steps(&on, &rows), ["▲  x  weird"]);
    }

    #[test]
    fn fold_splits_long_words() {
        assert_eq!(
            fold(
                "gotcha · 17 min ago · ~/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:9",
                46
            ),
            [
                "gotcha · 17 min ago ·",
                "~/.local/share/bilbo/notes/gotcha-sqlite-busy-",
                "timeout.md:9"
            ]
        );
    }

    #[test]
    fn table_pads_two_apart() {
        let rows = vec![
            vec!["SCOPE".to_string(), "NOTES".into()],
            vec!["personal".into(), "6".into()],
            vec!["work".into(), "12".into()],
        ];
        assert_eq!(
            table(&rows, &[false, true]),
            ["SCOPE     NOTES", "personal      6", "work         12"]
        );
        assert_eq!(pad("日本", 5, Align::Left), "日本 ");
    }

    #[test]
    fn tilde_shortens_home() {
        let t = fixed(80, false, true);
        assert_eq!(
            tilde(&t, Path::new("/home/a/.local/share/bilbo/notes/x.md")),
            "~/.local/share/bilbo/notes/x.md"
        );
        assert_eq!(tilde(&t, Path::new("/home/ab/x")), "/home/ab/x");
        assert_eq!(tilde(&t, Path::new("/home/a")), "~");
    }

    #[test]
    fn tilde_text_rewrites_a_home_that_starts_a_word() {
        let t = fixed(80, false, true);
        for (text, want) in [
            (
                "watching /home/a/.local/share/bilbo/notes",
                "watching ~/.local/share/bilbo/notes",
            ),
            ("(/home/a/x)", "(~/x)"),
            ("/home/ab/x", "/home/ab/x"),
            ("file:///home/a/sync", "file://~/sync"),
            ("x/home/a/y", "x/home/a/y"),
            ("/home/a/x and /home/a/y", "~/x and ~/y"),
        ] {
            assert_eq!(tilde_text(&t, text), want, "{text}");
        }
    }

    #[test]
    fn steps_align_and_take_the_tone_of_their_status() {
        let steps_of = |rows: &[(&str, &str, Option<&str>)]| -> Vec<Step> {
            rows.iter()
                .map(|(step, status, detail)| Step {
                    step: step.to_string(),
                    status: status.to_string(),
                    detail: detail.map(str::to_string),
                })
                .collect()
        };
        let report = steps_of(&[
            ("store", "kept", Some("/home/a/notes")),
            ("key", "skipped", Some("local embedder")),
            ("timer", "failed", Some("launchctl not found")),
            ("claude", "installed", None),
            ("scope work", "unsealed", Some("copy the store")),
        ]);
        let painted = steps(&fixed(100, true, true), &report);
        assert_eq!(
            painted,
            [
                styled("{d}◇{/d}  store       {d}kept{/d}       ~/notes"),
                styled("{d}○{/d}  key         {d}skipped{/d}    local embedder"),
                styled("{r}■{/r}  timer       {r}failed{/r}     launchctl not found"),
                styled("{g}◆{/g}  claude      {g}installed{/g}"),
                styled("{y}▲{/y}  scope work  {y}unsealed{/y}   copy the store"),
            ]
        );
        let unpainted = steps(&fixed(100, false, true), &report);
        let stripped: Vec<String> = painted
            .iter()
            .map(|l| console::strip_ansi_codes(l).into_owned())
            .collect();
        assert_eq!(unpainted, stripped);
        assert_eq!(
            steps(&fixed(100, false, false), &report)[0],
            "o  store       kept       ~/notes"
        );
    }

    #[test]
    fn ago_reads_created_times() {
        let now: jiff::Timestamp = "2026-10-07T01:00:00-03:00".parse().unwrap();
        for (created, want) in [
            ("2026-10-07T00:59-03:00", "1 min ago"),
            ("2026-10-07T01:00-03:00", "just now"),
            ("2026-10-07T00:43-03:00", "17 min ago"),
            ("2026-10-06T23:00-03:00", "2 hours ago"),
            ("2026-10-07T00:00-03:00", "1 hour ago"),
            ("2026-10-06T00:30-03:00", "yesterday"),
            ("2026-10-02T00:00-03:00", "5 days ago"),
            ("2026-09-07T01:00-03:00", "30 days ago"),
            ("2026-08-02T10:00-03:00", "2026-08-02"),
            ("2026-10-08T00:00-03:00", "2026-10-08"),
            ("never", "never"),
            ("-", "-"),
        ] {
            assert_eq!(ago(now, created), want, "{created}");
        }
    }

    #[test]
    fn numbers_group_and_sizes() {
        assert_eq!(group(779594), "779,594");
        assert_eq!(group(999), "999");
        assert_eq!(group(1000), "1,000");
        assert_eq!(group(0), "0");
        assert_eq!(size(193 * 1024), "193 KB");
        assert_eq!(size(1949 * 1024), "1.9 MB");
        assert_eq!(size(1), "1 KB");
        assert_eq!(count(1, "note", "notes"), "1 note");
        assert_eq!(count(0, "note", "notes"), "0 notes");
        assert_eq!(count(4620, "passage", "passages"), "4,620 passages");
    }

    #[test]
    fn notation_expands() {
        assert_eq!(styled("{b}x{/b}"), "\x1b[1mx\x1b[22m");
        assert_eq!(plain("{b}x{/b} {c}y{/c}"), "x y");
    }
}
