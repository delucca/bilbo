use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::frontmatter;
use crate::markdown::{self, Section};

pub const BUDGET_TOKENS: usize = 60_000;
pub const MIN_BUDGET_TOKENS: usize = 1_000;
pub const SLICE_BYTES: usize = 24_000;
pub const MIN_SLICE_BYTES: usize = 1_000;
pub const MAX_SLICE_BYTES: usize = 30_000;
pub const MIN_SLICE_LINES: usize = 10;
pub const MAX_PARTS: usize = 8;
const KEEP: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub budget_tokens: usize,
    pub slice_bytes: usize,
    pub slice_lines: Option<usize>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            budget_tokens: BUDGET_TOKENS,
            slice_bytes: SLICE_BYTES,
            slice_lines: None,
        }
    }
}

/// A source, or a section of it, to read: the lines `start` to `end`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    /// The reference as the agent gave it, with its `#<anchor>`.
    pub reference: String,
    pub id: String,
    pub corpus: String,
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub digest: String,
    /// The physical line the source's body started on; a frontmatter of another length shifts every line.
    #[serde(default)]
    pub body_start: usize,
}

impl Pick {
    pub fn label(&self) -> String {
        format!("{}/{}", self.corpus, self.name)
    }

    /// `<corpus>` and `<name>` as the plan recorded them.
    pub fn place(&self) -> (String, String) {
        (self.corpus.clone(), self.name.clone())
    }

    /// `name`, or `name#<anchor>` when the reference had an anchor.
    fn shown(&self, name: &str) -> String {
        match self.reference.split_once('#') {
            Some((_, anchor)) => format!("{name}#{anchor}"),
            None => name.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    /// Index into the plan's picks.
    pub pick: usize,
    pub start: usize,
    pub end: usize,
    /// The bytes `library read` prints for it, as estimated when the plan was cut.
    pub bytes: usize,
    pub tokens: usize,
    pub partition: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub id: String,
    pub root: String,
    pub created: String,
    pub options: Options,
    pub picks: Vec<Pick>,
    pub slices: Vec<Slice>,
}

/// What a pick's source holds, for cutting and printing.
pub struct Material<'a> {
    /// Physical lines, as `markdown::lines` numbers them.
    pub lines: &'a [&'a str],
    pub sections: &'a [Section],
}

impl Material<'_> {
    fn line(&self, number: usize) -> &str {
        self.lines.get(number - 1).copied().unwrap_or("")
    }
}

/// The heading path of the deepest section that holds `line`.
pub fn heading_path(sections: &[Section], line: usize) -> Option<String> {
    sections
        .iter()
        .rfind(|s| s.start <= line && line <= s.end)
        .map(Section::path_text)
}

fn digits(n: usize) -> usize {
    n.to_string().len()
}

/// Whether two picks of one source share a line, as the indexes of the first such pair.
pub fn overlap(picks: &[Pick]) -> Option<(usize, usize)> {
    (0..picks.len()).find_map(|j| {
        (0..j)
            .find(|&i| {
                picks[i].id == picks[j].id
                    && picks[i].start <= picks[j].end
                    && picks[j].start <= picks[i].end
            })
            .map(|i| (i, j))
    })
}

struct Walk<'a> {
    material: &'a Material<'a>,
    label: String,
    id: &'a str,
    /// The widest number a slice index or the slice count can print as.
    width: String,
    options: &'a Options,
    /// `print[n]`: the bytes the lines up to `n` print, each as `<n>\t<text>\n`.
    print: Vec<usize>,
    /// `raw[n]`: the bytes of the lines up to `n`, each with its newline.
    raw: Vec<usize>,
}

impl<'a> Walk<'a> {
    fn new(pick: &'a Pick, material: &'a Material<'a>, options: &'a Options, width: usize) -> Self {
        let upto = material.lines.len().max(pick.end);
        let mut print = vec![0];
        let mut raw = vec![0];
        for n in 1..=upto {
            let len = material.line(n).len();
            print.push(print[n - 1] + digits(n) + 1 + len + 1);
            raw.push(raw[n - 1] + len + 1);
        }
        Walk {
            material,
            label: pick.label(),
            id: &pick.id,
            width: "9".repeat(width),
            options,
            print,
            raw,
        }
    }

    fn in_len(&self, line: usize) -> usize {
        let path = heading_path(self.material.sections, line);
        format!("-- in: {} --\n", path.as_deref().unwrap_or("-")).len()
    }

    /// The bytes of the slice `a` to `b`, whose `-- in: --` line is `in_len` bytes.
    fn size(&self, a: usize, b: usize, in_len: usize) -> usize {
        let w = &self.width;
        let fixed = format!(
            "-- slice {w}/{w}: {} {} lines {a}-{b} --\n-- end slice {w}/{w} --\n",
            self.label, self.id
        );
        fixed.len() + in_len + self.print[b] - self.print[a - 1]
    }

    fn fits(&self, a: usize, b: usize, in_len: usize) -> bool {
        self.size(a, b, in_len) <= self.options.slice_bytes
            && self.options.slice_lines.is_none_or(|n| b - a < n)
    }

    /// An oversized run `a` to `b` as pieces that end at a blank line when they can, else at the limits.
    fn pieces(&self, a: usize, b: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut p = a;
        loop {
            let in_len = self.in_len(p);
            if p == b || self.fits(p, b, in_len) {
                out.push((p, b));
                return out;
            }
            let (mut count, mut high) = (0, b - p);
            while count < high {
                let mid = (count + high) / 2;
                if self.fits(p, p + mid, in_len) {
                    count = mid + 1;
                } else {
                    high = mid;
                }
            }
            let longest = if count == 0 { p } else { p + count - 1 };
            let end = (p + 1..=longest)
                .rev()
                .find(|&e| self.material.line(e).trim().is_empty())
                .unwrap_or(longest);
            out.push((p, end));
            p = end + 1;
        }
    }

    /// The slices of the pick `start` to `end`, cut at section starts.
    fn cut(&self, start: usize, end: usize) -> Vec<(usize, usize)> {
        let mut starts = vec![start];
        starts.extend(
            self.material
                .sections
                .iter()
                .map(|s| s.start)
                .filter(|&s| s > start && s <= end),
        );
        let mut out = Vec::new();
        let mut open: Option<(usize, usize, usize)> = None;
        for (i, &a) in starts.iter().enumerate() {
            let b = starts.get(i + 1).map_or(end, |next| next - 1);
            if let Some((first, _, in_len)) = open {
                if self.fits(first, b, in_len) {
                    open = Some((first, b, in_len));
                    continue;
                }
                out.extend(open.take().map(|(first, last, _)| (first, last)));
            }
            let in_len = self.in_len(a);
            if self.fits(a, b, in_len) {
                open = Some((a, b, in_len));
            } else {
                out.extend(self.pieces(a, b));
            }
        }
        out.extend(open.map(|(first, last, _)| (first, last)));
        out
    }
}

/// Cuts `picks` into slices and the slices into partitions. `materials[i]` is the source of `picks[i]`.
pub fn build(
    id: String,
    root: String,
    created: String,
    options: Options,
    picks: Vec<Pick>,
    materials: &[Material],
) -> Plan {
    let mut width = 1;
    let mut slices = loop {
        let mut slices = Vec::new();
        for (i, pick) in picks.iter().enumerate() {
            let walk = Walk::new(pick, &materials[i], &options, width);
            for (a, b) in walk.cut(pick.start, pick.end) {
                slices.push(Slice {
                    pick: i,
                    start: a,
                    end: b,
                    bytes: walk.size(a, b, walk.in_len(a)),
                    tokens: markdown::tokens(walk.raw[b] - walk.raw[a - 1]),
                    partition: 0,
                });
            }
        }
        if digits(slices.len()) <= width {
            break slices;
        }
        width = digits(slices.len());
    };
    partition(&mut slices, options.budget_tokens);
    Plan {
        id,
        root,
        created,
        options,
        picks,
        slices,
    }
}

/// Next-fit: a slice joins the current partition when it fits the budget, else opens the next.
fn partition(slices: &mut [Slice], budget: usize) {
    let mut number = 0;
    let mut held = 0;
    for slice in slices {
        if number == 0 || held + slice.tokens > budget {
            number += 1;
            held = 0;
        }
        held += slice.tokens;
        slice.partition = number;
    }
}

pub struct Partition {
    pub number: usize,
    pub first: usize,
    pub last: usize,
    pub tokens: usize,
}

impl Plan {
    pub fn tokens(&self) -> usize {
        self.slices.iter().map(|s| s.tokens).sum()
    }

    /// The partitions, each a run of consecutive slices, as slice numbers from 1.
    pub fn partitions(&self) -> Vec<Partition> {
        let mut out: Vec<Partition> = Vec::new();
        for (i, slice) in self.slices.iter().enumerate() {
            match out.last_mut() {
                Some(last) if last.number == slice.partition => {
                    last.last = i + 1;
                    last.tokens += slice.tokens;
                }
                _ => out.push(Partition {
                    number: slice.partition,
                    first: i + 1,
                    last: i + 1,
                    tokens: slice.tokens,
                }),
            }
        }
        out
    }
}

/// What `cite` asks of a plan.
impl Plan {
    pub fn has_source(&self, id: &str) -> bool {
        self.picks.iter().any(|p| p.id == id)
    }

    /// Whether the plan picked the source as it is now: the same body, starting on the same line.
    pub fn is_current(&self, id: &str, digest: Option<&str>, body_start: usize) -> bool {
        self.picks
            .iter()
            .any(|p| p.id == id && Some(p.digest.as_str()) == digest && p.body_start == body_start)
    }

    /// The number, from 1, of the slice of source `id` that holds `line`.
    pub fn slice_at(&self, id: &str, line: usize) -> Option<usize> {
        self.slices
            .iter()
            .position(|s| self.picks[s.pick].id == id && s.start <= line && line <= s.end)
            .map(|i| i + 1)
    }

    /// Reads the coverage of the plan from its read log: a slice is read when every line of it is.
    pub fn coverage(&self, log: &[LogEntry]) -> Coverage {
        let mut ranges: Vec<(&str, Vec<(usize, usize)>)> = Vec::new();
        for pick in &self.picks {
            if !ranges.iter().any(|(id, _)| *id == pick.id) {
                ranges.push((&pick.id, read_ranges(log, &pick.id, &pick.digest)));
            }
        }
        let mut cov = Coverage {
            read_slices: 0,
            total_slices: self.slices.len(),
            read_tokens: 0,
            total_tokens: self.tokens(),
            unread: Vec::new(),
        };
        let mut last: Option<(usize, usize)> = None;
        for (i, slice) in self.slices.iter().enumerate() {
            let pick = &self.picks[slice.pick];
            let held = ranges.iter().find(|(id, _)| *id == pick.id);
            if held.is_some_and(|(_, r)| covers(r, slice.start, slice.end)) {
                cov.read_slices += 1;
                cov.read_tokens += slice.tokens;
                last = None;
                continue;
            }
            match (cov.unread.last_mut(), last) {
                (Some(run), Some((pick_index, end)))
                    if self.picks[pick_index].id == pick.id && end + 1 == slice.start =>
                {
                    run.end = slice.end;
                    run.last = i + 1;
                }
                _ => cov.unread.push(Unread {
                    id: pick.id.clone(),
                    label: pick.label(),
                    start: slice.start,
                    end: slice.end,
                    first: i + 1,
                    last: i + 1,
                }),
            }
            last = Some((slice.pick, slice.end));
        }
        cov
    }

    /// The picks per corpus, in the order the corpora first appear; `place` gives a pick's `(corpus, name)`.
    pub fn picked(&self, place: impl Fn(&Pick) -> (String, String)) -> Vec<PickedCorpus> {
        let mut out: Vec<PickedCorpus> = Vec::new();
        for (i, pick) in self.picks.iter().enumerate() {
            let (corpus, name) = place(pick);
            let at = match out.iter().position(|c| c.corpus == corpus) {
                Some(at) => at,
                None => {
                    out.push(PickedCorpus {
                        corpus,
                        sources: 0,
                        picks: Vec::new(),
                    });
                    out.len() - 1
                }
            };
            let group = &mut out[at];
            if !self.picks[..i].iter().any(|p| p.id == pick.id) {
                group.sources += 1;
            }
            group.picks.push(pick.shown(&name));
        }
        out
    }

    /// `picked: plan <plan>: <corpus> <k> of <n> sources (<picks>)`, `<n>` from `sources_in`.
    pub fn picked_line(
        &self,
        place: impl Fn(&Pick) -> (String, String),
        sources_in: impl Fn(&str) -> usize,
    ) -> String {
        let groups: Vec<String> = self
            .picked(place)
            .iter()
            .map(|c| {
                format!(
                    "{} {} of {} sources ({})",
                    c.corpus,
                    c.sources,
                    sources_in(&c.corpus),
                    c.picks.join(", ")
                )
            })
            .collect();
        format!("picked: plan {}: {}", self.id, groups.join("; "))
    }
}

/// A run of unread slices of one pick.
#[derive(Debug, PartialEq, Eq)]
pub struct Unread {
    pub id: String,
    /// `<corpus>/<name>` when the plan was made.
    pub label: String,
    pub start: usize,
    pub end: usize,
    /// The first and last slice of the run, from 1.
    pub first: usize,
    pub last: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Coverage {
    pub read_slices: usize,
    pub total_slices: usize,
    pub read_tokens: usize,
    pub total_tokens: usize,
    pub unread: Vec<Unread>,
}

impl Coverage {
    /// `coverage: plan <plan>: read <r> of <s> slices (<t> of <T> tokens); not read: <runs>`, each run named by
    /// `label`.
    pub fn line_with(&self, plan: &str, label: impl Fn(&Unread) -> String) -> String {
        let runs = if self.unread.is_empty() {
            "none".to_string()
        } else {
            self.unread
                .iter()
                .map(|run| {
                    let slices = if run.first == run.last {
                        format!("slice {}", run.first)
                    } else {
                        format!("slices {}-{}", run.first, run.last)
                    };
                    format!("{} lines {}-{} ({slices})", label(run), run.start, run.end)
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "coverage: plan {plan}: read {} of {} slices ({} of {} tokens); not read: {runs}",
            self.read_slices, self.total_slices, self.read_tokens, self.total_tokens
        )
    }

    #[cfg(test)]
    fn line(&self, plan: &str) -> String {
        self.line_with(plan, |run| run.label.clone())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PickedCorpus {
    pub corpus: String,
    /// Distinct sources picked.
    pub sources: usize,
    /// Each pick as `<name>` or `<name>#<anchor>`.
    pub picks: Vec<String>,
}

pub fn plan_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

pub fn log_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.log"))
}

/// Writes the plan to `<dir>/<plan>.json`, never over a plan that is there.
pub fn save(dir: &Path, plan: &Plan) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let json = serde_json::to_string(plan).map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(plan_path(dir, &plan.id))?;
    file.write_all(json.as_bytes())
}

/// The plan `id`; `None` when `<dir>` holds no such plan, the message when it cannot be read.
pub fn load(dir: &Path, id: &str) -> Result<Option<Plan>, String> {
    let path = plan_path(dir, id);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Removes the plans, with their logs, that nothing has changed in 30 days; returns how many.
pub fn prune(dir: &Path, now: SystemTime) -> usize {
    let Ok(read) = fs::read_dir(dir) else {
        return 0;
    };
    let mut plans: Vec<(String, SystemTime, Vec<PathBuf>)> = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name
            .strip_suffix(".json")
            .or_else(|| name.strip_suffix(".log"))
            .filter(|stem| frontmatter::is_ulid(stem))
        else {
            continue;
        };
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        match plans.iter_mut().find(|(s, _, _)| s == stem) {
            Some((_, newest, paths)) => {
                *newest = (*newest).max(modified);
                paths.push(entry.path());
            }
            None => plans.push((stem.to_string(), modified, vec![entry.path()])),
        }
    }
    let mut removed = 0;
    for (_, newest, paths) in plans {
        if now.duration_since(newest).is_ok_and(|age| age > KEEP) {
            paths.iter().for_each(|p| {
                let _ = fs::remove_file(p);
            });
            removed += 1;
        }
    }
    removed
}

/// One printed run: slice `slice`, lines `start` to `end` of source `id` at `digest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub slice: usize,
    pub id: String,
    pub digest: String,
    pub start: usize,
    pub end: usize,
    pub time: String,
}

impl LogEntry {
    /// `<slice>\t<id>\t<digest>\t<start>-<end>\t<time>`
    fn line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}-{}\t{}\n",
            self.slice, self.id, self.digest, self.start, self.end, self.time
        )
    }

    fn parse(line: &str) -> Option<LogEntry> {
        let mut parts = line.split('\t');
        let slice = parts.next()?.parse().ok()?;
        let id = parts.next()?.to_string();
        let digest = parts.next()?.to_string();
        let (start, end) = parts.next()?.split_once('-')?;
        let time = parts.next()?.to_string();
        Some(LogEntry {
            slice,
            id,
            digest,
            start: start.parse().ok()?,
            end: end.parse().ok()?,
            time,
        })
    }
}

/// A log entry for the run `start` to `end` of `slice`, stamped now.
pub fn entry(slice: usize, pick: &Pick, start: usize, end: usize) -> LogEntry {
    LogEntry {
        slice,
        id: pick.id.clone(),
        digest: pick.digest.clone(),
        start,
        end,
        time: jiff::Timestamp::now().to_string(),
    }
}

/// Appends the entries to the plan's log under an exclusive lock, in one write.
pub fn append_log(dir: &Path, plan: &str, entries: &[LogEntry]) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(log_path(dir, plan))?;
    file.lock()?;
    let text: String = entries.iter().map(LogEntry::line).collect();
    file.write_all(text.as_bytes())
}

/// The entries of the plan's log, none when it has no log; a line that does not parse is skipped.
pub fn load_log(dir: &Path, plan: &str) -> io::Result<Vec<LogEntry>> {
    let mut file = match fs::File::open(log_path(dir, plan)) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    file.lock_shared()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(LogEntry::parse)
        .collect())
}

/// The lines the log records as printed for source `id` at `digest`, as sorted, merged inclusive ranges.
pub fn read_ranges(log: &[LogEntry], id: &str, digest: &str) -> Vec<(usize, usize)> {
    let mut found: Vec<(usize, usize)> = log
        .iter()
        .filter(|e| e.id == id && e.digest == digest)
        .map(|e| (e.start, e.end))
        .collect();
    found.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in found {
        match merged.last_mut() {
            Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    merged
}

/// Whether the merged `ranges` hold every line from `a` to `b`.
pub fn covers(ranges: &[(usize, usize)], a: usize, b: usize) -> bool {
    ranges.iter().any(|&(first, last)| first <= a && b <= last)
}

/// One slice, or one part of it, as `library read` prints it.
#[derive(Debug)]
pub struct Rendered {
    pub lines: Vec<String>,
    pub start: usize,
    pub end: usize,
}

impl Rendered {
    /// The bytes printed, each line with its newline.
    pub fn bytes(&self) -> usize {
        self.lines.iter().map(|l| l.len() + 1).sum()
    }
}

/// The lines of part `k` of `n` of the slice `a` to `b`: runs cut at line boundaries into nearly equal bytes.
fn part_range(m: &Material, a: usize, b: usize, k: usize, n: usize) -> Option<(usize, usize)> {
    let count = b - a + 1;
    if count < n {
        return None;
    }
    let mut cum = vec![0];
    for line in a..=b {
        cum.push(cum[line - a] + m.line(line).len() + 1);
    }
    let total = cum[count];
    let mut ends = Vec::with_capacity(n);
    let mut taken = 0;
    for j in 1..=n {
        taken = if j == n {
            count
        } else {
            let target = total * j / n;
            (taken + 1..=count - (n - j))
                .find(|&e| cum[e] >= target)
                .unwrap_or(count - (n - j))
        };
        ends.push(taken);
    }
    let first = if k == 1 { 0 } else { ends[k - 2] };
    Some((a + first, a + ends[k - 1] - 1))
}

/// Slice `number`, from 1, of the plan, or its part `part` as `(k, n)`, under the source's current name `label`.
pub fn render(
    plan: &Plan,
    number: usize,
    part: Option<(usize, usize)>,
    label: &str,
    m: &Material,
) -> Result<Rendered, String> {
    let slice = &plan.slices[number - 1];
    let pick = &plan.picks[slice.pick];
    let (start, end) = match part {
        None => (slice.start, slice.end),
        Some((k, n)) => part_range(m, slice.start, slice.end, k, n).ok_or_else(|| {
            let count = slice.end - slice.start + 1;
            format!(
                "slice {number} has {count} {}, fewer than the {n} parts of --part",
                if count == 1 { "line" } else { "lines" },
            )
        })?,
    };
    let of = format!(
        "{number}/{}{}",
        plan.slices.len(),
        part.map_or(String::new(), |(k, n)| format!(" part {k}/{n}"))
    );
    let path = heading_path(m.sections, start);
    let mut lines = vec![
        format!("-- slice {of}: {label} {} lines {start}-{end} --", pick.id),
        format!("-- in: {} --", path.as_deref().unwrap_or("-")),
    ];
    for n in start..=end {
        lines.push(format!("{n}\t{}", m.line(n)));
    }
    lines.push(format!("-- end slice {of} --"));
    Ok(Rendered { lines, start, end })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01M3EZ8NVEC2KJQNGK5DTK349R";
    const ID_2: &str = "01M3EZ8NVEC2KJQNGK5DTK3400";
    const PLAN: &str = "01M3EZ8NVEC2KJQNGK5DTK3401";
    const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    /// `count` lines of `width` letters.
    fn block(count: usize, width: usize) -> String {
        (0..count).map(|_| "a".repeat(width) + "\n").collect()
    }

    fn pick(id: &str, name: &str, start: usize, end: usize) -> Pick {
        Pick {
            reference: format!("go/{name}"),
            id: id.into(),
            corpus: "go".into(),
            name: name.into(),
            start,
            end,
            digest: DIGEST.into(),
            body_start: 7,
        }
    }

    /// The plan of one whole source whose body is `text` from line 1.
    fn plan_of(text: &str, options: Options) -> Plan {
        plan_of_all(&[text], options)
    }

    fn plan_of_all(texts: &[&str], options: Options) -> Plan {
        let lines: Vec<Vec<&str>> = texts.iter().map(|t| markdown::lines(t)).collect();
        let sections: Vec<Vec<Section>> = lines.iter().map(|l| markdown::outline(l, 1)).collect();
        let materials: Vec<Material> = lines
            .iter()
            .zip(&sections)
            .map(|(lines, sections)| Material { lines, sections })
            .collect();
        let picks = (0..texts.len())
            .map(|i| pick([ID, ID_2][i], &format!("s{i}"), 1, lines[i].len()))
            .collect();
        build(
            PLAN.into(),
            "/r".into(),
            "t".into(),
            options,
            picks,
            &materials,
        )
    }

    fn ranges(plan: &Plan) -> Vec<(usize, usize)> {
        plan.slices.iter().map(|s| (s.start, s.end)).collect()
    }

    fn section(count: usize, name: &str) -> String {
        format!("## {name}\n{}", block(count, 99))
    }

    fn printed(plan: &Plan, text: &str, number: usize) -> usize {
        let lines = markdown::lines(text);
        let sections = markdown::outline(&lines, 1);
        let m = Material {
            lines: &lines,
            sections: &sections,
        };
        render(plan, number, None, "go/s0", &m).unwrap().bytes()
    }

    #[test]
    fn sections_pack_into_slices() {
        let text = format!(
            "# T\n{}{}{}",
            section(99, "A"),
            section(88, "B"),
            section(78, "C")
        );
        let plan = plan_of(&text, Options::default());
        assert_eq!(ranges(&plan), [(1, 190), (191, 269)]);
        assert!(plan.slices.iter().all(|s| s.bytes <= SLICE_BYTES));
        assert!(plan.slices[0].bytes > 19_000);
    }

    #[test]
    fn a_line_cap_bounds_every_slice() {
        let text = format!("# T\n{}", block(600, 10));
        let options = Options {
            slice_lines: Some(250),
            ..Options::default()
        };
        let plan = plan_of(&text, options);
        assert_eq!(plan.slices.first().map(|s| s.start), Some(1));
        assert_eq!(plan.slices.last().map(|s| s.end), Some(601));
        assert!(plan.slices.iter().all(|s| s.end - s.start < 250));
        assert!(plan.slices.len() >= 3);
    }

    #[test]
    fn a_headingless_source_is_one_slice() {
        let text = format!("# T\n{}", block(190, 99));
        let plan = plan_of(&text, Options::default());
        assert_eq!(ranges(&plan), [(1, 191)]);
    }

    #[test]
    fn picks_never_share_a_slice() {
        let small = format!("# T\n{}", block(18, 99));
        let plan = plan_of_all(&[&small, &small], Options::default());
        assert_eq!(plan.slices.len(), 2);
        assert_eq!((plan.slices[0].pick, plan.slices[1].pick), (0, 1));
    }

    #[test]
    fn a_long_section_is_cut_at_blank_lines() {
        let paragraph = format!("{}\n", block(19, 99));
        let text = format!("# T\n## Long\n{}", paragraph.repeat(30));
        let plan = plan_of(&text, Options::default());
        let lines = markdown::lines(&text);
        assert_eq!((plan.slices[0].start, plan.slices[0].end), (1, 1));
        let long: Vec<_> = plan.slices[1..].iter().collect();
        assert_eq!(long.len(), 3, "{:?}", ranges(&plan));
        for (i, s) in long.iter().enumerate() {
            assert!(s.bytes <= SLICE_BYTES);
            assert!(lines[s.end - 1].is_empty() || i == long.len() - 1);
        }
        assert_eq!(long[0].start, 2);
        assert_eq!(long.last().map(|s| s.end), Some(lines.len()));
    }

    #[test]
    fn a_long_section_with_no_blank_line_is_cut_at_line_boundaries() {
        let text = format!("# T\n## Long\n{}", block(480, 99));
        let plan = plan_of(&text, Options::default());
        assert!(plan.slices.len() >= 3);
        let mut next = 2;
        for s in &plan.slices[1..] {
            assert_eq!(s.start, next);
            assert!(s.bytes <= SLICE_BYTES);
            next = s.end + 1;
        }
        assert_eq!(next, markdown::lines(&text).len() + 1);
    }

    #[test]
    fn a_section_under_the_limit_is_not_cut_at_its_blank_lines() {
        let paragraph = format!("{}\n", block(9, 99));
        let text = format!("# T\n## A\n{}", paragraph.repeat(20));
        let plan = plan_of(&text, Options::default());
        assert_eq!(plan.slices.len(), 1);
    }

    #[test]
    fn one_huge_line_is_a_slice_by_itself() {
        let text = format!("# T\n{}\n", "a".repeat(30_000));
        let plan = plan_of(&text, Options::default());
        assert_eq!(ranges(&plan), [(1, 1), (2, 2)]);
        assert!(plan.slices[1].bytes > SLICE_BYTES);
    }

    #[test]
    fn a_slice_estimate_never_undercounts_what_read_prints() {
        let text = format!(
            "# T\n{}{}{}",
            section(99, "A"),
            section(88, "B"),
            section(78, "C")
        );
        let plan = plan_of(&text, Options::default());
        for (i, slice) in plan.slices.iter().enumerate() {
            let actual = printed(&plan, &text, i + 1);
            assert!(
                actual <= slice.bytes && slice.bytes - actual <= 6,
                "{actual} {}",
                slice.bytes
            );
        }
    }

    #[test]
    fn a_slice_that_starts_inside_a_section_names_its_path() {
        let text = format!("# T\n## Concurrency\n### Goroutines\n{}", block(300, 99));
        let lines = markdown::lines(&text);
        let sections = markdown::outline(&lines, 1);
        assert_eq!(heading_path(&sections, 1), None);
        assert_eq!(
            heading_path(&sections, 3).as_deref(),
            Some("Concurrency > Goroutines")
        );
        let plan = plan_of(&text, Options::default());
        let second = &plan.slices[1];
        assert_eq!(
            heading_path(&sections, second.start).as_deref(),
            Some("Concurrency > Goroutines")
        );
        let m = Material {
            lines: &lines,
            sections: &sections,
        };
        let out = render(&plan, 2, None, "go/s0", &m).unwrap();
        assert_eq!(out.lines[1], "-- in: Concurrency > Goroutines --");
    }

    fn tokens_of(tokens: &[usize]) -> Vec<Slice> {
        tokens
            .iter()
            .map(|&t| Slice {
                pick: 0,
                start: 1,
                end: 1,
                bytes: 0,
                tokens: t,
                partition: 0,
            })
            .collect()
    }

    fn partitions_of(tokens: &[usize], budget: usize) -> Vec<usize> {
        let mut slices = tokens_of(tokens);
        partition(&mut slices, budget);
        slices.iter().map(|s| s.partition).collect()
    }

    #[test]
    fn partitions_are_runs_of_consecutive_slices() {
        assert_eq!(partitions_of(&[9000; 7], 60_000), [1, 1, 1, 1, 1, 1, 2]);
    }

    #[test]
    fn a_slice_over_the_budget_is_a_partition_alone() {
        assert_eq!(partitions_of(&[1000, 9000, 1000], 5000), [1, 2, 3]);
        assert_eq!(partitions_of(&[9000], 5000), [1]);
    }

    #[test]
    fn a_slice_that_fits_opens_no_partition() {
        assert_eq!(partitions_of(&[50_000, 9000], 60_000), [1, 1]);
    }

    #[test]
    fn an_exact_fit_fills_a_partition() {
        assert_eq!(partitions_of(&[30_000, 30_000, 1], 60_000), [1, 1, 2]);
    }

    #[test]
    fn a_late_small_slice_never_goes_back() {
        assert_eq!(partitions_of(&[40_000, 40_000, 100], 60_000), [1, 2, 2]);
    }

    #[test]
    fn order_is_kept_and_nothing_is_dropped() {
        let mut slices = tokens_of(&[10, 20, 30, 40, 50]);
        partition(&mut slices, 60);
        let held: Vec<usize> = slices.iter().map(|s| s.tokens).collect();
        assert_eq!(held, [10, 20, 30, 40, 50]);
        let numbers: Vec<usize> = slices.iter().map(|s| s.partition).collect();
        assert_eq!(numbers, [1, 1, 1, 2, 3]);
    }

    #[test]
    fn partitions_report_their_slice_runs_and_tokens() {
        let text = format!(
            "# T\n{}{}{}",
            section(99, "A"),
            section(88, "B"),
            section(78, "C")
        );
        let options = Options {
            budget_tokens: MIN_BUDGET_TOKENS,
            ..Options::default()
        };
        let plan = plan_of(&text, options);
        let parts = plan.partitions();
        assert_eq!(parts.len(), 2);
        assert_eq!((parts[0].first, parts[0].last), (1, 1));
        assert_eq!((parts[1].first, parts[1].last), (2, 2));
        assert_eq!(parts.iter().map(|p| p.tokens).sum::<usize>(), plan.tokens());
    }

    #[test]
    fn slice_tokens_follow_the_derived_sizes() {
        let text = "# T\nabc\n";
        let plan = plan_of(text, Options::default());
        assert_eq!(plan.slices[0].tokens, markdown::tokens(8));
    }

    fn material_of(text: &str) -> (Vec<String>, Vec<Section>) {
        let lines = markdown::lines(text);
        let sections = markdown::outline(&lines, 1);
        (lines.iter().map(|l| l.to_string()).collect(), sections)
    }

    #[test]
    fn parts_cover_every_line_once() {
        let text = format!("# T\n{}", block(100, 30));
        let (owned, sections) = material_of(&text);
        let lines: Vec<&str> = owned.iter().map(String::as_str).collect();
        let m = Material {
            lines: &lines,
            sections: &sections,
        };
        for n in 2..=8 {
            let mut next = 1;
            for k in 1..=n {
                let (a, b) = part_range(&m, 1, 101, k, n).unwrap();
                assert_eq!(a, next, "{k}/{n}");
                assert!(b >= a);
                next = b + 1;
            }
            assert_eq!(next, 102);
        }
        assert_eq!(part_range(&m, 1, 3, 1, 4), None);
        assert!(part_range(&m, 1, 4, 4, 4).is_some());
    }

    #[test]
    fn parts_have_nearly_equal_bytes() {
        let text = format!("# T\n{}{}", block(50, 10), block(50, 90));
        let (owned, sections) = material_of(&text);
        let lines: Vec<&str> = owned.iter().map(String::as_str).collect();
        let m = Material {
            lines: &lines,
            sections: &sections,
        };
        let (a, b) = part_range(&m, 1, 101, 1, 2).unwrap();
        assert_eq!(a, 1);
        let bytes: usize = (a..=b).map(|n| lines[n - 1].len() + 1).sum();
        let total: usize = lines.iter().map(|l| l.len() + 1).sum();
        assert!(
            bytes >= total / 2 && bytes < total / 2 + 91,
            "{bytes} of {total}"
        );
    }

    #[test]
    fn a_part_prints_its_own_header_and_marker() {
        let text = format!("# T\n## A\n{}", block(40, 20));
        let plan = plan_of(&text, Options::default());
        let (owned, sections) = material_of(&text);
        let lines: Vec<&str> = owned.iter().map(String::as_str).collect();
        let m = Material {
            lines: &lines,
            sections: &sections,
        };
        let one = render(&plan, 1, Some((1, 2)), "go/s0", &m).unwrap();
        let two = render(&plan, 1, Some((2, 2)), "go/s0", &m).unwrap();
        assert_eq!(
            one.lines[0],
            format!("-- slice 1/1 part 1/2: go/s0 {ID} lines 1-{} --", one.end)
        );
        assert_eq!(one.lines.last().unwrap(), "-- end slice 1/1 part 1/2 --");
        assert_eq!(two.start, one.end + 1);
        assert_eq!(two.end, 42);
        let err = render(&plan, 1, Some((1, 8)), "go/s0", &m).map(|_| ());
        assert!(err.is_ok());
        let short = Plan {
            slices: vec![Slice {
                end: 3,
                ..plan.slices[0].clone()
            }],
            ..plan.clone()
        };
        assert!(
            render(&short, 1, Some((1, 4)), "go/s0", &m)
                .unwrap_err()
                .contains("--part")
        );
    }

    #[test]
    fn a_full_render_has_header_in_numbered_lines_and_marker() {
        let text = "# T\n\n## Concurrency\n\tbody \nmore\n";
        let (owned, sections) = material_of(text);
        let lines: Vec<&str> = owned.iter().map(String::as_str).collect();
        let m = Material {
            lines: &lines,
            sections: &sections,
        };
        let plan = plan_of(text, Options::default());
        let out = render(&plan, 1, None, "go/moved", &m).unwrap();
        let id = &plan.picks[0].id;
        assert_eq!(
            out.lines[0],
            format!("-- slice 1/1: go/moved {id} lines 1-5 --")
        );
        assert_eq!(out.lines[1], "-- in: - --");
        assert_eq!(
            &out.lines[2..],
            [
                "1\t# T",
                "2\t",
                "3\t## Concurrency",
                "4\t\tbody ",
                "5\tmore",
                "-- end slice 1/1 --"
            ]
        );
        assert_eq!(
            out.bytes(),
            out.lines.iter().map(|l| l.len() + 1).sum::<usize>()
        );
    }

    fn log_entry(slice: usize, id: &str, start: usize, end: usize) -> LogEntry {
        LogEntry {
            slice,
            id: id.into(),
            digest: DIGEST.into(),
            start,
            end,
            time: "2026-10-04T00:00:00Z".into(),
        }
    }

    fn two_slice_plan() -> Plan {
        Plan {
            id: PLAN.into(),
            root: "/r".into(),
            created: "t".into(),
            options: Options::default(),
            picks: vec![pick(ID, "a", 1, 100), pick(ID_2, "b", 5, 40)],
            slices: vec![
                Slice {
                    pick: 0,
                    start: 1,
                    end: 50,
                    bytes: 0,
                    tokens: 100,
                    partition: 1,
                },
                Slice {
                    pick: 0,
                    start: 51,
                    end: 100,
                    bytes: 0,
                    tokens: 100,
                    partition: 1,
                },
                Slice {
                    pick: 1,
                    start: 5,
                    end: 40,
                    bytes: 0,
                    tokens: 50,
                    partition: 1,
                },
            ],
        }
    }

    #[test]
    fn ranges_merge_and_filter_by_id_and_digest() {
        let mut other = log_entry(1, ID, 1, 5);
        other.digest = "sha256:x".into();
        let log = vec![
            log_entry(2, ID, 51, 60),
            log_entry(1, ID, 1, 50),
            log_entry(1, ID_2, 1, 3),
            other,
            log_entry(2, ID, 61, 70),
        ];
        assert_eq!(read_ranges(&log, ID, DIGEST), [(1, 70)]);
        assert_eq!(read_ranges(&log, ID_2, DIGEST), [(1, 3)]);
        assert!(read_ranges(&log, ID, "sha256:none").is_empty());
        assert!(covers(&[(1, 70)], 5, 70));
        assert!(!covers(&[(1, 70)], 5, 71));
        assert!(!covers(&[(1, 10), (12, 20)], 5, 15));
    }

    #[test]
    fn a_slice_is_read_when_every_line_of_it_was_printed() {
        let plan = two_slice_plan();
        let half = [log_entry(1, ID, 1, 25)];
        let cov = plan.coverage(&half);
        assert_eq!(cov.read_slices, 0);
        let both = [log_entry(1, ID, 1, 25), log_entry(1, ID, 26, 50)];
        let cov = plan.coverage(&both);
        assert_eq!((cov.read_slices, cov.read_tokens), (1, 100));
        assert_eq!(cov.unread.len(), 2);
    }

    #[test]
    fn coverage_merges_consecutive_unread_slices_of_a_pick() {
        let plan = two_slice_plan();
        let cov = plan.coverage(&[]);
        assert_eq!(
            cov.line(PLAN),
            format!(
                "coverage: plan {PLAN}: read 0 of 3 slices (0 of 250 tokens); not read: go/a lines 1-100 (slices 1-2), go/b lines 5-40 (slice 3)"
            )
        );
        let cov = plan.coverage(&[log_entry(1, ID, 1, 50)]);
        assert_eq!(
            cov.unread
                .iter()
                .map(|u| (u.first, u.last))
                .collect::<Vec<_>>(),
            [(2, 2), (3, 3)]
        );
    }

    #[test]
    fn coverage_with_everything_read_says_none() {
        let plan = two_slice_plan();
        let log = [log_entry(1, ID, 1, 100), log_entry(3, ID_2, 5, 40)];
        let cov = plan.coverage(&log);
        assert_eq!(
            cov.line(PLAN),
            format!(
                "coverage: plan {PLAN}: read 3 of 3 slices (250 of 250 tokens); not read: none"
            )
        );
    }

    #[test]
    fn coverage_ignores_a_read_of_other_text() {
        let plan = two_slice_plan();
        let mut stale = log_entry(1, ID, 1, 100);
        stale.digest = "sha256:old".into();
        assert_eq!(plan.coverage(&[stale]).read_slices, 0);
    }

    #[test]
    fn adjacent_picks_of_one_source_merge_in_the_coverage_line() {
        let mut plan = two_slice_plan();
        plan.picks = vec![pick(ID, "a", 3, 6), pick(ID, "a", 7, 20)];
        plan.slices = vec![
            Slice {
                pick: 0,
                start: 3,
                end: 6,
                bytes: 0,
                tokens: 10,
                partition: 1,
            },
            Slice {
                pick: 1,
                start: 7,
                end: 20,
                bytes: 0,
                tokens: 10,
                partition: 1,
            },
        ];
        let line = plan.coverage(&[]).line(PLAN);
        assert!(
            line.ends_with("not read: go/a lines 3-20 (slices 1-2)"),
            "{line}"
        );
        plan.picks[1].start = 8;
        plan.slices[1].start = 8;
        let line = plan.coverage(&[]).line(PLAN);
        assert!(
            line.contains("(slice 1), go/a lines 8-20 (slice 2)"),
            "{line}"
        );
    }

    #[test]
    fn a_label_can_be_replaced_in_the_coverage_line() {
        let plan = two_slice_plan();
        let line = plan
            .coverage(&[])
            .line_with(PLAN, |u| format!("go/new-{}", &u.label[3..]));
        assert!(line.contains("go/new-a lines 1-100"), "{line}");
    }

    #[test]
    fn picks_are_counted_per_corpus() {
        let mut plan = two_slice_plan();
        plan.picks[0].reference = format!("{ID}#Wrapping");
        plan.picks.push(pick(ID, "a", 200, 300));
        plan.picks[1].corpus = "rust".into();
        let groups = plan.picked(Pick::place);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].corpus, "go");
        assert_eq!(groups[0].sources, 1);
        assert_eq!(groups[0].picks, ["a#Wrapping", "a"]);
        assert_eq!(
            plan.picked_line(Pick::place, |corpus| if corpus == "go" { 14 } else { 3 }),
            format!(
                "picked: plan {PLAN}: go 1 of 14 sources (a#Wrapping, a); rust 1 of 3 sources (b)"
            )
        );
    }

    #[test]
    fn a_moved_source_counts_under_its_current_corpus() {
        let plan = two_slice_plan();
        let line = plan.picked_line(|p| ("rust".to_string(), format!("new-{}", p.name)), |_| 7);
        assert_eq!(
            line,
            format!("picked: plan {PLAN}: rust 2 of 7 sources (new-a, new-b)")
        );
    }

    #[test]
    fn overlapping_picks_of_one_source_are_found() {
        let a = pick(ID, "a", 1, 100);
        let inside = pick(ID, "a", 50, 60);
        let after = pick(ID, "a", 101, 120);
        let other = pick(ID_2, "b", 1, 100);
        assert_eq!(overlap(&[a.clone(), inside]), Some((0, 1)));
        assert_eq!(overlap(&[a.clone(), after, other]), None);
    }

    #[test]
    fn slices_are_found_by_line() {
        let plan = two_slice_plan();
        assert_eq!(plan.slice_at(ID, 51), Some(2));
        assert_eq!(plan.slice_at(ID_2, 5), Some(3));
        assert_eq!(plan.slice_at(ID_2, 4), None);
        assert!(plan.has_source(ID));
        assert!(!plan.has_source("x"));
        assert!(plan.is_current(ID_2, Some(DIGEST), 7));
        assert!(!plan.is_current(ID_2, Some(DIGEST), 8));
        assert!(!plan.is_current(ID_2, Some("sha256:x"), 7));
        assert!(!plan.is_current(ID_2, None, 7));
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir =
                std::env::temp_dir().join(format!("bilbo-plan-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_plan_is_saved_once_and_loaded() {
        let scratch = Scratch::new("save");
        let dir = scratch.0.join("plans");
        let plan = two_slice_plan();
        assert_eq!(load(&dir, PLAN), Ok(None));
        save(&dir, &plan).unwrap();
        assert_eq!(load(&dir, PLAN), Ok(Some(plan.clone())));
        assert!(save(&dir, &plan).is_err());
        fs::write(plan_path(&dir, ID), "{").unwrap();
        assert!(load(&dir, ID).unwrap_err().contains(ID));
    }

    #[test]
    fn the_log_appends_and_loads() {
        let scratch = Scratch::new("log");
        let dir = &scratch.0;
        assert_eq!(load_log(dir, PLAN).unwrap(), []);
        append_log(
            dir,
            PLAN,
            &[log_entry(1, ID, 1, 50), log_entry(2, ID, 51, 60)],
        )
        .unwrap();
        append_log(dir, PLAN, &[log_entry(3, ID_2, 5, 40)]).unwrap();
        let mut text = fs::read_to_string(log_path(dir, PLAN)).unwrap();
        assert_eq!(
            text.lines().next().unwrap(),
            format!("1\t{ID}\t{DIGEST}\t1-50\t2026-10-04T00:00:00Z")
        );
        text.push_str("junk\n");
        fs::write(log_path(dir, PLAN), text).unwrap();
        let log = load_log(dir, PLAN).unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[2], log_entry(3, ID_2, 5, 40));
    }

    #[test]
    fn a_stamped_entry_names_the_pick_and_the_time() {
        let plan = two_slice_plan();
        let e = entry(3, &plan.picks[1], 5, 40);
        assert_eq!(
            (e.slice, e.id.as_str(), e.digest.as_str()),
            (3, ID_2, DIGEST)
        );
        assert_eq!(LogEntry::parse(e.line().trim_end()), Some(e));
    }

    #[test]
    fn old_plans_go_with_their_logs_and_recent_ones_stay() {
        let scratch = Scratch::new("prune");
        let dir = &scratch.0;
        let old = SystemTime::now() - Duration::from_secs(31 * 24 * 60 * 60);
        let set = |path: PathBuf, time: SystemTime| {
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        };
        let mut plan = two_slice_plan();
        for id in [ID, ID_2, PLAN] {
            plan.id = id.into();
            save(dir, &plan).unwrap();
            append_log(dir, id, &[log_entry(1, ID, 1, 2)]).unwrap();
        }
        fs::write(dir.join("notes.txt"), "x").unwrap();
        set(plan_path(dir, ID), old);
        set(log_path(dir, ID), old);
        set(plan_path(dir, ID_2), old);
        assert_eq!(prune(dir, SystemTime::now()), 1);
        assert!(!plan_path(dir, ID).exists() && !log_path(dir, ID).exists());
        assert!(plan_path(dir, ID_2).exists() && log_path(dir, ID_2).exists());
        assert!(plan_path(dir, PLAN).exists() && dir.join("notes.txt").exists());
        assert_eq!(prune(&dir.join("missing"), SystemTime::now()), 0);
    }
}
