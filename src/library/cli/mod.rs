//! `bilbo library`: its arguments, its output and the subcommand dispatch.

mod land;
mod list;
mod plan;
mod read;
mod reference;
mod show;
mod stage;

use std::io;
use std::path::{Path, PathBuf};

use crate::Failure;
use crate::host::terminal;
use crate::library::corpus::{self, SourceFile};
use crate::shared::markdown::{self, Section};
use crate::shared::store;

const VALUE_OPTIONS: [&str; 9] = [
    "--depth",
    "--origin",
    "--fetched",
    "--keep",
    "--title",
    "--budget-tokens",
    "--slice-bytes",
    "--slice-lines",
    "--part",
];

pub struct Output {
    /// stderr lines (without "bilbo: "), printed before stdout.
    pub warnings: Vec<String>,
    pub lines: Vec<String>,
    pub shape: Shape,
}

/// What a result holds, for the human view; the other subcommands have none.
pub enum Shape {
    Plain,
    Corpora(Vec<corpus::Listing>),
    Guide(Guide),
    Source(Source),
}

/// A corpus guide as its human view shows it.
pub struct Guide {
    pub name: String,
    pub path: PathBuf,
    /// The guide's lead paragraphs, as written.
    pub lead: Vec<String>,
    pub entries: Vec<Entry>,
}

/// An entry of the guide, or a source the guide does not name.
pub struct Entry {
    pub name: String,
    /// The entry's paragraphs as written; empty for a source with no entry.
    pub text: Vec<String>,
    /// `None` when the entry's source file is missing.
    pub facts: Option<Facts>,
    /// The entry still holds `corpus::STUB_SOURCE`.
    pub stub: bool,
    /// False for a source the guide has no entry for.
    pub listed: bool,
}

pub struct Facts {
    pub id: Option<String>,
    pub bytes: usize,
    pub tokens: usize,
    pub fetched: Option<String>,
    pub headings: usize,
    pub catalog: bool,
    pub capture: Option<String>,
}

/// A source as `library show` describes it.
pub struct Source {
    pub reference: String,
    pub title: String,
    pub origin: Option<String>,
    pub fetched: Option<String>,
    pub capture: Option<String>,
    pub id: Option<String>,
    pub path: PathBuf,
    pub start: usize,
    pub end: usize,
    pub tokens: usize,
    pub headings: usize,
    pub catalog: bool,
    /// The outline rows shown: first line, last line, tokens, heading path.
    pub sections: Vec<(usize, usize, usize, Vec<String>)>,
}

impl Output {
    fn lines(lines: Vec<String>) -> Output {
        Output {
            warnings: Vec::new(),
            lines,
            shape: Shape::Plain,
        }
    }

    /// The `library-browse` human view; `lines` is the plain one.
    pub fn view(&self, term: &terminal::Term) -> Vec<String> {
        match &self.shape {
            Shape::Plain => self.lines.clone(),
            Shape::Corpora(listing) => corpora(term, listing),
            Shape::Guide(guide) => guide_view(term, guide),
            Shape::Source(source) => source_view(term, source),
        }
    }
}

/// `text` as `markdown::plain` reads it, wrapped to `width` columns.
fn paragraphs(term: &terminal::Term, text: &[String], width: usize) -> Vec<Vec<String>> {
    text.iter()
        .map(|p| terminal::wrap(term, &markdown::plain(p), width, terminal::Long::Split))
        .collect()
}

/// `facts` folded to `width` and painted dim, one indented line each.
fn dim_lines(term: &terminal::Term, parts: &[String], indent: &str, width: usize) -> Vec<String> {
    let width = width.saturating_sub(indent.len());
    let lines = match parts.split_last() {
        Some((last, head)) => terminal::tail_lines(&head.join(" · "), last, " · ", width),
        None => Vec::new(),
    };
    lines
        .iter()
        .map(|l| format!("{indent}{}", terminal::paint(term, terminal::Tone::Dim, l)))
        .collect()
}

fn guide_view(term: &terminal::Term, guide: &Guide) -> Vec<String> {
    use terminal::{Mark, Tone};
    let mut out = vec![format!(
        "{}  {}",
        terminal::paint(term, Tone::Bold, &guide.name),
        terminal::paint(term, Tone::Dim, &terminal::tilde(term, &guide.path))
    )];
    let lead = paragraphs(term, &guide.lead, term.width);
    out.extend(lead.join(&String::new()));
    let indented = term.width.saturating_sub(2);
    for entry in &guide.entries {
        out.push(String::new());
        out.push(terminal::paint(term, Tone::Bold, &entry.name));
        let warn = |text: String| format!("  {}  {text}", terminal::mark(term, Mark::Warning));
        if !entry.listed {
            out.push(warn("no entry in guide.md".into()));
        } else if entry.stub {
            out.push(warn(terminal::paint(term, Tone::Yellow, "TODO")));
        } else {
            let text = paragraphs(term, &entry.text, indented);
            out.extend(text.join(&String::new()).iter().map(|l| format!("  {l}")));
        }
        match &entry.facts {
            None => out.push(warn("no source file".into())),
            Some(f) => {
                let mut parts = vec![
                    terminal::size(f.bytes as u64),
                    terminal::count(f.tokens as u64, "token", "tokens"),
                    terminal::count(f.headings as u64, "heading", "headings"),
                ];
                parts.extend(f.fetched.as_ref().map(|d| format!("fetched {d}")));
                parts.extend(f.catalog.then(|| "catalog".to_string()));
                parts.extend(f.capture.as_ref().map(|c| format!("capture {c}")));
                parts.extend(f.id.clone());
                out.extend(dim_lines(term, &parts, "  ", term.width));
            }
        }
    }
    out
}

fn source_view(term: &terminal::Term, source: &Source) -> Vec<String> {
    use terminal::Tone;
    let mut out = vec![format!(
        "{}  {}",
        terminal::paint(term, Tone::Bold, &source.title),
        terminal::paint(term, Tone::Cyan, &source.reference)
    )];
    let mut known: Vec<String> = Vec::new();
    known.extend(
        source
            .origin
            .as_ref()
            .map(|o| o.strip_prefix("url: ").unwrap_or(o).to_string()),
    );
    known.extend(source.fetched.as_ref().map(|d| format!("fetched {d}")));
    known.extend(source.capture.as_ref().map(|c| format!("capture {c}")));
    known.extend(source.id.clone());
    if !known.is_empty() {
        out.extend(dim_lines(term, &known, "", term.width));
    }
    let size = [
        format!("lines {}-{}", source.start, source.end),
        terminal::count(source.tokens as u64, "token", "tokens"),
        terminal::count(source.headings as u64, "heading", "headings"),
    ];
    out.extend(dim_lines(term, &size, "", term.width));
    out.push(terminal::paint(
        term,
        Tone::Dim,
        &terminal::tilde(term, &source.path),
    ));
    if source.catalog {
        let hint = format!("catalog: pick a section as {}#<anchor>", source.reference);
        out.push(terminal::paint(term, Tone::Dim, &hint));
    }
    out.push(String::new());
    let top = source.sections.iter().map(|s| s.3.len()).min().unwrap_or(0);
    let rows: Vec<Vec<String>> = source
        .sections
        .iter()
        .map(|(first, last, tokens, path)| {
            let heading = markdown::plain(path.last().map_or("", String::as_str));
            let heading = if path.len() <= top {
                terminal::paint(term, Tone::Bold, &heading)
            } else {
                format!("{}{heading}", "  ".repeat(path.len() - top))
            };
            vec![
                terminal::paint(term, Tone::Dim, &format!("{first}-{last}")),
                terminal::group(*tokens as u64),
                heading,
            ]
        })
        .collect();
    out.extend(
        terminal::table(&rows, &[true, true])
            .iter()
            .map(|row| terminal::cut(term, &format!("  {row}"), term.width)),
    );
    out
}

fn corpora(term: &terminal::Term, listing: &[corpus::Listing]) -> Vec<String> {
    use terminal::Tone;
    if listing.is_empty() {
        return vec![format!(
            "{}  No corpora yet; ask your agent to ingest a page",
            terminal::mark(term, terminal::Mark::Skipped)
        )];
    }
    let titled = listing
        .iter()
        .any(|c| c.title.as_deref().is_some_and(|t| t != c.name));
    let dim = |text: &str| terminal::paint(term, Tone::Dim, text);
    let mut head = vec![dim("CORPUS"), dim("SOURCES"), dim("SIZE"), dim("TOKENS")];
    if titled {
        head.push(dim("TITLE"));
    }
    let mut rows = vec![head];
    for c in listing {
        let mut row = vec![
            terminal::paint(term, Tone::Bold, &c.name),
            terminal::group(c.sources as u64),
            terminal::size(c.bytes as u64),
            terminal::group(c.tokens as u64),
        ];
        if titled {
            row.push(c.title.clone().unwrap_or_else(|| "-".into()));
        }
        rows.push(row);
    }
    terminal::table(&rows, &[false, true, true, true])
}

fn usage(message: impl Into<String>) -> Failure {
    Failure::Usage(message.into())
}

fn refused(message: impl Into<String>) -> Failure {
    Failure::Refused(message.into())
}

fn io_failure(what: &'static str, path: &Path) -> impl Fn(io::Error) -> Failure {
    let path = path.to_path_buf();
    move |e| refused(format!("cannot {what} {}: {e}", path.display()))
}

struct Args {
    positional: Vec<String>,
    options: Vec<(String, String)>,
    replace: bool,
    force: bool,
    html: bool,
}

impl Args {
    fn parse(args: &[String]) -> Result<Args, Failure> {
        let mut parsed = Args {
            positional: Vec::new(),
            options: Vec::new(),
            replace: false,
            force: false,
            html: false,
        };
        let mut options_ended = false;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let is_option = arg.chars().nth(1).is_some_and(|c| !c.is_whitespace());
            if options_ended || !arg.starts_with('-') || !is_option {
                parsed.positional.push(arg.clone());
                continue;
            }
            if arg == "--" {
                options_ended = true;
                continue;
            }
            let (name, inline) = match arg.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (arg.as_str(), None),
            };
            if name == "--replace" {
                if inline.is_some() {
                    return Err(usage("--replace takes no value"));
                }
                parsed.replace = true;
            } else if name == "--html" {
                if inline.is_some() {
                    return Err(usage("--html takes no value"));
                }
                parsed.html = true;
            } else if name == "--force" {
                if inline.is_some() {
                    return Err(usage("--force takes no value"));
                }
                parsed.force = true;
            } else if VALUE_OPTIONS.contains(&name) {
                let value = match inline {
                    Some(value) => value,
                    None => iter
                        .next()
                        .ok_or_else(|| usage(format!("{name} needs a value")))?
                        .clone(),
                };
                if name != "--keep" && parsed.options.iter().any(|(n, _)| n == name) {
                    return Err(usage(format!("{name} given more than once")));
                }
                parsed.options.push((name.to_string(), value));
            } else {
                return Err(usage(format!("unknown option '{name}'")));
            }
        }
        Ok(parsed)
    }

    fn one(&self, name: &str) -> Option<&str> {
        self.all(name).into_iter().next()
    }

    fn all(&self, name: &str) -> Vec<&str> {
        self.options
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// Refuses every option that is not in `taken`.
    fn only(&self, taken: &[&str]) -> Result<(), Failure> {
        let given = self.options.iter().map(|(n, _)| n.as_str());
        let given = given
            .chain(self.replace.then_some("--replace"))
            .chain(self.force.then_some("--force"))
            .chain(self.html.then_some("--html"));
        match given.into_iter().find(|name| !taken.contains(name)) {
            Some(name) => Err(usage(format!("option '{name}' does not apply here"))),
            None => Ok(()),
        }
    }

    /// The operands after the subcommand word, exactly as many as `names`.
    fn operands<const N: usize>(&self, names: [&str; N]) -> Result<[&str; N], Failure> {
        let rest = &self.positional[1..];
        if let Some(extra) = rest.get(N) {
            return Err(usage(format!("unexpected argument '{extra}'")));
        }
        let mut out = [""; N];
        for (i, name) in names.iter().enumerate() {
            out[i] = rest
                .get(i)
                .ok_or_else(|| usage(format!("missing {name}")))?;
        }
        Ok(out)
    }
}

pub fn run(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    let args = Args::parse(args)?;
    match args.positional.first().map(String::as_str) {
        None => {
            args.only(&[])?;
            list::run(env)
        }
        Some("show") => {
            args.only(&["--depth"])?;
            show::run(&args, env)
        }
        Some("stage") => {
            args.only(&["--origin", "--fetched", "--html"])?;
            stage::run(&args, env)
        }
        Some("land") => {
            args.only(&["--keep", "--title", "--replace", "--force"])?;
            land::run(&args, env)
        }
        Some("plan") => {
            args.only(&["--budget-tokens", "--slice-bytes", "--slice-lines"])?;
            plan::run(&args, env)
        }
        Some("read") => {
            args.only(&["--part"])?;
            read::run(&args, env)
        }
        Some(name) => {
            args.only(&[])?;
            if let Some(extra) = args.positional.get(1) {
                return Err(usage(format!("unexpected argument '{extra}'")));
            }
            if !store::is_topic(name) {
                return Err(usage(format!(
                    "invalid corpus '{name}': use segments of a-z and 0-9 joined by single hyphens"
                )));
            }
            list::show_corpus(name, env)
        }
    }
}

fn root(env: &store::Env) -> Result<PathBuf, Failure> {
    store::root(env).map_err(Failure::Config)
}

fn sections(file: &SourceFile) -> Vec<Section> {
    markdown::outline(&markdown::lines(&file.text), file.source.body_start)
}

fn or_dash(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("-")
}

fn grouped(n: usize) -> String {
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

fn today() -> String {
    jiff::Zoned::now().date().to_string()
}

fn state_failure() -> Failure {
    Failure::Config(
        "cannot find the state folder: set XDG_STATE_HOME, or HOME, to an absolute path".into(),
    )
}

/// The line of the first level-1 heading outside fences and its text.
fn capture_title(lines: &[&str]) -> Option<(usize, String)> {
    markdown::outside_fences(lines)
        .into_iter()
        .find_map(|i| match markdown::heading(lines[i]) {
            Some((1, text)) => Some((i + 1, text)),
            _ => None,
        })
}

/// `bilbo library --help`; its Usage block is also the synopsis a usage error shows.
pub const HELP: &str = r#"bilbo library: browse the library, add a source to it, and read sources in
full through a plan.

Usage:
  bilbo library [<corpus>]
  bilbo library show <ref> [--depth <n>]
  bilbo library stage <url>
  bilbo library stage <file> --origin "<url|doc>: <value>"
                      [--fetched <YYYY-MM-DD>] [--html]
  bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]...
                     [--title <text>] [--replace [--force]]
  bilbo library plan <ref>... [--budget-tokens <n>] [--slice-bytes <n>]
                     [--slice-lines <n>]
  bilbo library read <plan> <slice>... [--part <k>/<n>]
<ref> is <corpus>/<name> or a source id, then optionally #<anchor>.
Options go before or after the operands, --<option>=<value> works too, and
-- ends the options.

Browse:
  library            One row per corpus: its sources, size and guide title
  library <corpus>   The corpus's guide, with a facts line under each entry
  show <ref>         A source's header, then one row per section: its lines,
                     tokens and heading path. #<anchor> narrows the rows to
                     that section, --depth <n> to n heading levels

Add a source: stage it, pick the line ranges that are the page itself, land:
  stage <url>        Fetch the page; print the stage id, its lines, title,
                     a suggested --keep range and its headings
  stage <file>       Copy a text file instead. --origin is required, --html
                     converts a saved page, --fetched defaults to today
  land               Write the source from the --keep ranges of the stage and
                     add its guide entry. --keep may repeat. --title defaults
                     to the first '# ' heading. --replace overwrites a source,
                     keeping its id; --force lets the citations of it degrade
stage writes only to the state folder; land writes the store.

Read in full:
  plan <ref>...      Cut the picks into slices and partitions; print the plan
                     id, the partitions and one row per slice.
                     --budget-tokens per partition (default 60000),
                     --slice-bytes (default 24000), --slice-lines (none)
  read               Print the named slices with line numbers, and log them
                     for 'bilbo cite --plan'. --part <k>/<n> prints the k-th
                     of n runs of each slice

Exit: 0 success; 1 refused (no such corpus, source, stage or plan, a failed
fetch, a source that exists or changed, or citations that would degrade);
2 usage or config error.

Examples:
  bilbo library go
  bilbo library show go/effective-go --depth 2
  bilbo library stage https://go.dev/doc/effective_go
  bilbo library land <stage> go/effective-go --keep 12-840
  bilbo library plan 'go/effective-go#Concurrency' go/errors
  bilbo library read <plan> 1 2

Docs: https://github.com/delucca/bilbo/wiki/Commands#library
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(title: &str) -> Vec<corpus::Listing> {
        let row = |name: &str, sources, kb: usize, tokens, title: &str| corpus::Listing {
            name: name.into(),
            sources,
            bytes: kb * 1024,
            tokens,
            title: Some(title.into()),
        };
        vec![
            row("rust", 117, 1949, 779594, "rust"),
            row("writing", 16, 193, 76903, title),
        ]
    }

    fn view(listing: Vec<corpus::Listing>, term: &terminal::Term) -> Vec<String> {
        Output {
            shape: Shape::Corpora(listing),
            ..Output::lines(Vec::new())
        }
        .view(term)
    }

    #[test]
    fn the_corpus_table() {
        assert_eq!(
            view(listing("writing"), &terminal::fixed(100, false, true)),
            [
                "CORPUS   SOURCES    SIZE   TOKENS",
                "rust         117  1.9 MB  779,594",
                "writing       16  193 KB   76,903",
            ]
        );
        assert_eq!(
            view(listing("writing"), &terminal::fixed(100, true, true))[..2],
            [
                terminal::styled("{d}CORPUS{/d}   {d}SOURCES{/d}    {d}SIZE{/d}   {d}TOKENS{/d}"),
                terminal::styled("{b}rust{/b}         117  1.9 MB  779,594"),
            ]
        );
        assert_eq!(
            view(
                listing("Writing guides"),
                &terminal::fixed(100, false, true)
            ),
            [
                "CORPUS   SOURCES    SIZE   TOKENS  TITLE",
                "rust         117  1.9 MB  779,594  rust",
                "writing       16  193 KB   76,903  Writing guides",
            ]
        );
    }

    #[test]
    fn no_corpora_says_so() {
        assert_eq!(
            view(Vec::new(), &terminal::fixed(100, false, true)),
            ["○  No corpora yet; ask your agent to ingest a page"]
        );
        assert_eq!(
            view(Vec::new(), &terminal::fixed(100, false, false)),
            ["-  No corpora yet; ask your agent to ingest a page"]
        );
    }

    fn guide() -> Guide {
        Guide {
            name: "writing".into(),
            path: "/home/a/.local/share/bilbo/library/writing/guide.md".into(),
            lead: vec![
                "Read by write-one-pager and the writing concern of the review skill. It holds guides to the one-pager, Amazon's six-pager and PR/FAQ, the Minto pyramid and McKinsey-style communication, and two pieces on visual hierarchy."
                    .into(),
            ],
            entries: vec![Entry {
                name: "a-guide-to-amazon-one-pager-six-pager".into(),
                text: vec![
                    "Explains Amazon's internal one-pager and six-pager document formats, why Amazon replaced slide decks with narrative documents, and how the practice reduces meeting overhead."
                        .into(),
                ],
                facts: Some(Facts {
                    id: Some("01M3EZ86WZ6JZHEF7DT1J8C08V".into()),
                    bytes: 7 * 1024,
                    tokens: 2584,
                    fetched: Some("2026-08-21".into()),
                    headings: 14,
                    catalog: false,
                    capture: Some("external".into()),
                }),
                stub: false,
                listed: true,
            }],
        }
    }

    fn shown(shape: Shape, term: &terminal::Term) -> Vec<String> {
        Output {
            shape,
            ..Output::lines(Vec::new())
        }
        .view(term)
    }

    #[test]
    fn a_guide_on_a_terminal() {
        let want = "writing  ~/.local/share/bilbo/library/writing/guide.md
Read by write-one-pager and the writing concern of the review skill. It holds guides to the
one-pager, Amazon's six-pager and PR/FAQ, the Minto pyramid and McKinsey-style communication, and
two pieces on visual hierarchy.

a-guide-to-amazon-one-pager-six-pager
  Explains Amazon's internal one-pager and six-pager document formats, why Amazon replaced slide
  decks with narrative documents, and how the practice reduces meeting overhead.
  7 KB · 2,584 tokens · 14 headings · fetched 2026-08-21 · capture external
  01M3EZ86WZ6JZHEF7DT1J8C08V";
        assert_eq!(
            shown(Shape::Guide(guide()), &terminal::fixed(100, false, true)),
            want.lines().collect::<Vec<_>>()
        );
        let painted = shown(Shape::Guide(guide()), &terminal::fixed(100, true, true));
        assert_eq!(
            painted[0],
            terminal::styled(
                "{b}writing{/b}  {d}~/.local/share/bilbo/library/writing/guide.md{/d}"
            )
        );
        assert_eq!(
            painted[5],
            terminal::styled("{b}a-guide-to-amazon-one-pager-six-pager{/b}")
        );
        let plain: Vec<String> = painted.iter().map(|l| terminal::stripped(l)).collect();
        assert_eq!(plain, want.lines().collect::<Vec<_>>());
    }

    #[test]
    fn a_stub_entry() {
        let mut stubbed = guide();
        stubbed.entries[0].stub = true;
        let lines = shown(Shape::Guide(stubbed), &terminal::fixed(100, true, true));
        assert_eq!(lines[6], terminal::styled("  {y}▲{/y}  {y}TODO{/y}"));
    }

    fn source(sections: Vec<(usize, usize, usize, Vec<String>)>, catalog: bool) -> Source {
        Source {
            reference: "writing/communication-the-mckinsey-way".into(),
            title: "Communication - The McKinsey Way".into(),
            origin: Some("url: https://www.stratechi.com/business-communication/".into()),
            fetched: Some("2026-08-21".into()),
            capture: Some("external".into()),
            id: Some("01M3EZ8729JWSXBHA6H77RK54F".into()),
            path: "/home/a/.local/share/bilbo/library/writing/communication-the-mckinsey-way.md"
                .into(),
            start: 8,
            end: 96,
            tokens: 4453,
            headings: 6,
            catalog,
            sections,
        }
    }

    fn path(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_source_on_a_terminal() {
        let top = "Business COMMINICATION";
        let sections = vec![
            (10, 96, 4439, path(&[top])),
            (22, 39, 1221, path(&[top, "The right communication medium"])),
            (40, 53, 734, path(&[top, "Written communication"])),
            (54, 67, 635, path(&[top, "Verbal communication"])),
            (68, 79, 662, path(&[top, "Presentation skills"])),
            (80, 96, 434, path(&[top, "Be comfortable in your own skin"])),
        ];
        let want = "Communication - The McKinsey Way  writing/communication-the-mckinsey-way
https://www.stratechi.com/business-communication/ · fetched 2026-08-21 · capture external
01M3EZ8729JWSXBHA6H77RK54F
lines 8-96 · 4,453 tokens · 6 headings
~/.local/share/bilbo/library/writing/communication-the-mckinsey-way.md

  10-96  4,439  Business COMMINICATION
  22-39  1,221    The right communication medium
  40-53    734    Written communication
  54-67    635    Verbal communication
  68-79    662    Presentation skills
  80-96    434    Be comfortable in your own skin";
        let lines = shown(
            Shape::Source(source(sections.clone(), false)),
            &terminal::fixed(100, false, true),
        );
        assert_eq!(lines, want.lines().collect::<Vec<_>>());
        let painted = shown(
            Shape::Source(source(sections, false)),
            &terminal::fixed(100, true, true),
        );
        assert_eq!(
            painted[6],
            terminal::styled("  {d}10-96{/d}  4,439  {b}Business COMMINICATION{/b}")
        );
    }

    #[test]
    fn escaped_headings_read_as_written() {
        let sections = vec![
            (3, 40, 900, path(&["unnecessary\\_clippy\\_cfg"])),
            (
                5,
                20,
                300,
                path(&["unnecessary\\_clippy\\_cfg", "What it does"]),
            ),
        ];
        let lines = shown(
            Shape::Source(source(sections, true)),
            &terminal::fixed(100, false, true),
        );
        assert_eq!(
            lines[lines.len() - 2..],
            [
                "  3-40  900  unnecessary_clippy_cfg",
                "  5-20  300    What it does",
            ]
        );
        assert!(
            lines.contains(
                &"catalog: pick a section as writing/communication-the-mckinsey-way#<anchor>"
                    .to_string()
            )
        );
    }
}
