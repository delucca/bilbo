//! The three-way passage merge of two versions of a note: units, the rules, conflict blocks and dropped lines.

use std::collections::{BTreeMap, BTreeSet};

use crate::note::versions::{Conflict, Dropped, Version};
use crate::shared::{frontmatter, markdown};

const OPEN: &str = "<<<<<<< bilbo ";
const SEP: &str = "======= bilbo ";
const CLOSE: &str = ">>>>>>> bilbo";

/// One side of a merge: the version and the bytes of its file.
pub struct Side<'a> {
    pub version: &'a Version,
    pub bytes: &'a [u8],
}

/// What a merge produced. `flags` are sorted and the `conflict` entries follow the file's order.
#[derive(Debug, PartialEq, Eq)]
pub struct Merged {
    pub file: String,
    pub bytes: Vec<u8>,
    pub flags: Vec<String>,
    pub conflict: Vec<Conflict>,
}

/// A full-form conflict block in a note's body.
#[derive(Debug, PartialEq, Eq)]
pub struct Block {
    /// The heading path of the passage it holds, joined with ` > `; empty before the first heading.
    pub passage: String,
    /// The physical lines of the block's first and last marker, counted from 1.
    pub line: usize,
    pub end: usize,
    pub sides: Vec<BlockSide>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockSide {
    /// The first 12 characters of the side's version id.
    pub version: String,
    pub time: String,
    pub lines: Vec<String>,
}

/// Merges two concurrent versions of a note against the common version `bases` hold (none: an empty base; several:
/// only what all hold identically). `base_file` is the file name every base holds, none when they differ or there
/// is no base. `syncs` tells whether this device syncs a scope. Two merges of the same inputs give the same result
/// whatever the order of the sides or of `bases`. Line endings are normalized to LF, the byte order mark dropped and
/// invalid UTF-8 replaced. `conflict` names every block in `bytes`, those carried over from a side included.
pub fn merge(
    bases: &[&[u8]],
    base_file: Option<&str>,
    a: &Side,
    b: &Side,
    syncs: &dyn Fn(&str) -> bool,
) -> Merged {
    let mut bases = bases.to_vec();
    bases.sort_unstable();
    let (lo, hi) = if a.version.version <= b.version.version {
        (a, b)
    } else {
        (b, a)
    };
    let file = file_name(base_file, lo, hi);
    if lo.bytes == hi.bytes {
        return Merged {
            file,
            bytes: lo.bytes.to_vec(),
            flags: Vec::new(),
            conflict: recorded(&Doc::from_bytes(lo.bytes).units),
        };
    }
    let base = combine(
        &bases
            .iter()
            .map(|bytes| Doc::from_bytes(bytes))
            .collect::<Vec<_>>(),
    );
    let (dlo, dhi) = (Doc::from_bytes(lo.bytes), Doc::from_bytes(hi.bytes));
    let mut flags = BTreeSet::new();

    let mut out: Vec<String> = Vec::new();
    if dlo.front.is_some() || dhi.front.is_some() {
        let empty = Vec::new();
        let groups = merge_front(
            base.front.as_ref().unwrap_or(&empty),
            dlo.front.as_ref().unwrap_or(&empty),
            dhi.front.as_ref().unwrap_or(&empty),
            syncs,
            &mut flags,
        );
        out.push("---".into());
        out.extend(groups.into_iter().flat_map(|g| g.lines));
        out.push("---".into());
    }
    let (body, conflict) = merge_units(&base.units, &dlo.units, &dhi.units, lo, hi, &mut flags);
    out.extend(body);
    let mut bytes = Vec::new();
    for line in out {
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    Merged {
        file,
        bytes,
        flags: flags.into_iter().map(String::from).collect(),
        conflict,
    }
}

/// A `Conflict` for each block among `units`.
fn recorded(units: &[Unit]) -> Vec<Conflict> {
    let mut trail = Trail::default();
    let mut found = Vec::new();
    for unit in units {
        let passage = trail.visit(unit.head().as_ref(), &unit.lines);
        if let Some(sides) = &unit.sides {
            found.push(Conflict {
                passage,
                sides: sides.iter().map(|p| p.id.clone()).collect(),
            });
        }
    }
    found
}

fn file_name(base: Option<&str>, lo: &Side, hi: &Side) -> String {
    let (l, h) = (&lo.version.file, &hi.version.file);
    let name = match base {
        Some(base) if l == base => h,
        _ => l,
    };
    name.clone()
}

/// The conflict blocks of a note's body outside fenced code, in order. Nothing above the closing `---` of the
/// frontmatter counts.
pub fn blocks(text: &str) -> Vec<Block> {
    let doc = Doc::parse(text);
    let mut trail = Trail::default();
    let mut found = Vec::new();
    for unit in &doc.units {
        let passage = trail.visit(unit.head().as_ref(), &unit.lines);
        if let Some(sides) = &unit.sides {
            let line = doc.offset + unit.start + 1;
            found.push(Block {
                passage,
                line,
                end: line + unit.lines.len() - 1,
                sides: sides
                    .iter()
                    .map(|piece| BlockSide {
                        version: piece.id.clone(),
                        time: piece.time.clone(),
                        lines: piece.lines.clone(),
                    })
                    .collect(),
            });
        }
    }
    found
}

/// The lines, counted from 1, of full-form marker lines in a note's body outside fenced code: an opening or separator
/// line with a version label and a time, or a closing line. Nothing above the closing `---` of the frontmatter counts.
pub fn marker_lines(text: &str) -> Vec<usize> {
    let all = markdown::lines(text);
    let close = (all.first() == Some(&"---"))
        .then(|| all[1..].iter().position(|l| *l == "---").map(|i| i + 1))
        .flatten();
    let from = close.map_or(0, |close| close + 1);
    markdown::outside_fences(&all[from..])
        .into_iter()
        .filter(|&i| {
            let line = all[from + i];
            line == CLOSE || label(line, OPEN).is_some() || label(line, SEP).is_some()
        })
        .map(|i| from + i + 1)
        .collect()
}

/// The non-blank lines of the blocks' sides, whitespace collapsed, that `now` holds nowhere, per passage.
pub fn dropped(blocks: &[Block], now: &str) -> Vec<Dropped> {
    let held: BTreeSet<String> = markdown::lines(now).iter().map(|l| collapse(l)).collect();
    let mut found = Vec::new();
    for block in blocks {
        let mut seen = BTreeSet::new();
        let mut lines = Vec::new();
        for line in block.sides.iter().flat_map(|side| &side.lines) {
            let key = collapse(line);
            if !key.is_empty() && !held.contains(&key) && seen.insert(key) {
                lines.push(line.trim().to_string());
            }
        }
        if !lines.is_empty() {
            found.push(Dropped {
                passage: block.passage.clone(),
                lines,
            });
        }
    }
    found
}

fn collapse(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug)]
struct Group {
    key: String,
    lines: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    level: usize,
    text: String,
    rank: usize,
}

#[derive(Clone, Debug)]
struct Piece {
    id: String,
    time: String,
    lines: Vec<String>,
}

/// The text before the first heading, one heading's passage up to the next heading of any level, or one conflict
/// block. `key` is the heading line with its rank among the same headings; units without one share level 0.
#[derive(Clone, Debug)]
struct Unit {
    key: Key,
    /// The keys of a block's other sides' headings.
    alts: Vec<Key>,
    start: usize,
    lines: Vec<String>,
    sides: Option<Vec<Piece>>,
}

impl Unit {
    fn head(&self) -> Option<(usize, String)> {
        (self.key.level > 0).then(|| (self.key.level, self.key.text.clone()))
    }

    fn keys(&self) -> impl Iterator<Item = &Key> {
        std::iter::once(&self.key).chain(&self.alts)
    }

    fn headed(&self) -> bool {
        self.sides.is_none() && self.key.level > 0
    }
}

struct Doc {
    front: Option<Vec<Group>>,
    /// Lines above the body, the closing `---` included.
    offset: usize,
    units: Vec<Unit>,
}

impl Doc {
    fn from_bytes(bytes: &[u8]) -> Doc {
        Doc::parse(&String::from_utf8_lossy(bytes))
    }

    fn parse(text: &str) -> Doc {
        let all = markdown::lines(text);
        let close = (all.first() == Some(&"---"))
            .then(|| all[1..].iter().position(|l| *l == "---").map(|i| i + 1))
            .flatten();
        let (front, offset) = match close {
            Some(close) => (Some(groups(&all[1..close])), close + 1),
            None => (None, 0),
        };
        Doc {
            front,
            offset,
            units: units(&all[offset..]),
        }
    }
}

fn groups(lines: &[&str]) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for line in lines {
        if line.starts_with([' ', '-'])
            && let Some(last) = out.last_mut()
        {
            last.lines.push(line.to_string());
            continue;
        }
        let key = frontmatter::split_key(line).map_or(*line, |(key, _)| key);
        out.push(Group {
            key: key.to_string(),
            lines: vec![line.to_string()],
        });
    }
    out
}

fn is_hex12(id: &str) -> bool {
    id.len() == 12 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The version and time of a marker line that opens with `prefix`.
fn label(line: &str, prefix: &str) -> Option<(String, String)> {
    let (id, time) = line.strip_prefix(prefix)?.split_once(' ')?;
    (is_hex12(id) && !time.is_empty()).then(|| (id.to_string(), time.to_string()))
}

/// The block that opens at line `at`: the index of its closing line and its sides.
fn block_at(lines: &[&str], outside: &[bool], at: usize) -> Option<(usize, Vec<Piece>)> {
    let (id, time) = label(lines[at], OPEN)?;
    let end = (at + 1..lines.len()).find(|&i| outside[i] && lines[i] == CLOSE)?;
    let mut sides = Vec::new();
    let mut open = Piece {
        id,
        time,
        lines: Vec::new(),
    };
    for i in at + 1..end {
        match label(lines[i], SEP).filter(|_| outside[i]) {
            Some((id, time)) => {
                let next = Piece {
                    id,
                    time,
                    lines: Vec::new(),
                };
                sides.push(std::mem::replace(&mut open, next));
            }
            None => open.lines.push(lines[i].to_string()),
        }
    }
    sides.push(open);
    Some((end, sides))
}

fn units(lines: &[&str]) -> Vec<Unit> {
    struct Raw {
        start: usize,
        lines: Vec<String>,
        sides: Option<Vec<Piece>>,
    }
    let mut outside = vec![false; lines.len()];
    for i in markdown::outside_fences(lines) {
        outside[i] = true;
    }
    let raw = |start| Raw {
        start,
        lines: Vec::new(),
        sides: None,
    };
    let mut raws = vec![raw(0)];
    let mut i = 0;
    while i < lines.len() {
        if outside[i]
            && let Some((end, sides)) = block_at(lines, &outside, i)
        {
            if raws.len() == 1 && raws[0].lines.is_empty() {
                raws.pop();
            }
            raws.push(Raw {
                start: i,
                lines: lines[i..=end].iter().map(|l| l.to_string()).collect(),
                sides: Some(sides),
            });
            raws.push(raw(end + 1));
            i = end + 1;
            continue;
        }
        if outside[i] && markdown::heading(lines[i]).is_some() {
            raws.push(raw(i));
        }
        raws.last_mut()
            .expect("raws is never empty")
            .lines
            .push(lines[i].to_string());
        i += 1;
    }
    let mut ranks: BTreeMap<(usize, String), usize> = BTreeMap::new();
    let mut out = Vec::new();
    for (n, raw) in raws.into_iter().enumerate() {
        if n > 0 && raw.lines.is_empty() {
            continue;
        }
        let mut heads: Vec<(usize, String)> = Vec::new();
        let firsts: Vec<Option<(usize, String)>> = match &raw.sides {
            Some(sides) => sides
                .iter()
                .map(|piece| piece.lines.first().and_then(|l| markdown::heading(l)))
                .collect(),
            None => vec![raw.lines.first().and_then(|l| markdown::heading(l))],
        };
        for head in firsts.into_iter().flatten() {
            if !heads.contains(&head) {
                heads.push(head);
            }
        }
        if heads.is_empty() {
            heads.push((0, String::new()));
        }
        let mut keys = Vec::new();
        for (level, text) in heads {
            let rank = ranks.entry((level, text.clone())).or_insert(0);
            keys.push(Key {
                level,
                text,
                rank: *rank,
            });
            *rank += 1;
        }
        let key = keys.remove(0);
        out.push(Unit {
            key,
            alts: keys,
            start: raw.start,
            lines: raw.lines,
            sides: raw.sides,
        });
    }
    out
}

/// The base of several lowest common versions: what all of them hold identically.
fn combine(docs: &[Doc]) -> Doc {
    let Some((first, rest)) = docs.split_first() else {
        return Doc {
            front: None,
            offset: 0,
            units: Vec::new(),
        };
    };
    let front = docs.iter().any(|d| d.front.is_some()).then(|| {
        let mut kept = Vec::new();
        for group in first.front.iter().flatten() {
            if group.key == "sources" {
                let held = |d: &Doc| -> BTreeSet<String> {
                    held_group(d, &group.key)
                        .map(|g| g.lines[1..].iter().cloned().collect())
                        .unwrap_or_default()
                };
                let all = rest.iter().fold(held(first), |acc, d| {
                    acc.intersection(&held(d)).cloned().collect()
                });
                let lines: Vec<String> = group.lines[1..]
                    .iter()
                    .filter(|l| all.contains(*l))
                    .cloned()
                    .collect();
                if !lines.is_empty() {
                    kept.push(Group {
                        key: group.key.clone(),
                        lines: [vec![group.lines[0].clone()], lines].concat(),
                    });
                }
            } else if rest
                .iter()
                .all(|d| held_group(d, &group.key).is_some_and(|g| g.lines == group.lines))
            {
                kept.push(group.clone());
            }
        }
        kept
    });
    let units = first
        .units
        .iter()
        .filter(|u| {
            rest.iter()
                .all(|d| d.units.iter().any(|o| o.key == u.key && o.lines == u.lines))
        })
        .cloned()
        .collect();
    Doc {
        front,
        offset: 0,
        units,
    }
}

fn held_group<'a>(doc: &'a Doc, key: &str) -> Option<&'a Group> {
    doc.front.iter().flatten().find(|g| g.key == key)
}

fn get<'a>(groups: &'a [Group], key: &str) -> Option<&'a Vec<String>> {
    groups.iter().find(|g| g.key == key).map(|g| &g.lines)
}

fn merge_front(
    base: &[Group],
    lo: &[Group],
    hi: &[Group],
    syncs: &dyn Fn(&str) -> bool,
    flags: &mut BTreeSet<&'static str>,
) -> Vec<Group> {
    let mut keys: Vec<&str> = Vec::new();
    for group in base.iter().chain(lo).chain(hi) {
        if !keys.contains(&group.key.as_str()) {
            keys.push(&group.key);
        }
    }
    let mut out = Vec::new();
    for key in keys {
        let (b, l, h) = (get(base, key), get(lo, key), get(hi, key));
        let picked = match key {
            "id" => b.or(l).or(h).cloned(),
            "created" => {
                if b.is_some() {
                    if l != b || h != b {
                        flags.insert("created-kept");
                    }
                    b.cloned()
                } else {
                    if l != h {
                        flags.insert("created-kept");
                    }
                    l.or(h).cloned()
                }
            }
            _ if l == h => l.cloned(),
            "sources" => merge_sources(b, l, h),
            _ if l == b => h.cloned(),
            _ if h == b => l.cloned(),
            "scope" => {
                flags.insert("scope-clash");
                clash(l, h, syncs)
            }
            _ => {
                flags.insert("key-kept");
                l.cloned()
            }
        };
        if let Some(lines) = picked {
            out.push(Group {
                key: key.to_string(),
                lines,
            });
        }
    }
    out
}

/// The value that shares less: no key when a side has none, else the side that this device does not sync.
fn clash(
    l: Option<&Vec<String>>,
    h: Option<&Vec<String>>,
    syncs: &dyn Fn(&str) -> bool,
) -> Option<Vec<String>> {
    let (l, h) = (l?, h?);
    let value = |lines: &Vec<String>| {
        frontmatter::split_key(&lines[0]).map_or(String::new(), |(_, rest)| rest.trim().to_string())
    };
    match (syncs(&value(l)), syncs(&value(h))) {
        (true, false) => Some(h.clone()),
        (false, true) => Some(l.clone()),
        _ => None,
    }
}

/// The three-way set: the base's items both sides kept, then the items either side added, sorted.
fn merge_sources(
    b: Option<&Vec<String>>,
    l: Option<&Vec<String>>,
    h: Option<&Vec<String>>,
) -> Option<Vec<String>> {
    let items =
        |g: Option<&Vec<String>>| -> Vec<String> { g.map(|g| g[1..].to_vec()).unwrap_or_default() };
    let (base, lo, hi) = (items(b), items(l), items(h));
    let mut out: Vec<String> = base
        .iter()
        .filter(|i| lo.contains(i) && hi.contains(i))
        .cloned()
        .collect();
    let added: BTreeSet<&String> = lo.iter().chain(&hi).filter(|i| !base.contains(i)).collect();
    out.extend(added.into_iter().cloned());
    (!out.is_empty()).then(|| [vec!["sources:".to_string()], out].concat())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Id {
    Base(usize),
    New(usize),
}

struct Item {
    lines: Vec<String>,
    head: Option<(usize, String)>,
    /// The short versions of the sides, when the item is a conflict block.
    conflict: Option<Vec<String>>,
}

impl Item {
    fn of(unit: &Unit) -> Item {
        Item {
            lines: unit.lines.clone(),
            head: unit.head(),
            conflict: unit
                .sides
                .as_ref()
                .map(|sides| sides.iter().map(|p| p.id.clone()).collect()),
        }
    }

    fn text(lines: Vec<String>) -> Item {
        let head = lines.first().and_then(|l| markdown::heading(l));
        Item {
            lines,
            head,
            conflict: None,
        }
    }
}

/// Heading levels and texts seen so far, giving each passage its path. A first level-1 heading with no text
/// above it is the title and is left out, as recall's paths leave it out.
#[derive(Default)]
struct Trail {
    stack: Vec<(usize, String)>,
    started: bool,
    text_first: bool,
}

impl Trail {
    fn visit(&mut self, head: Option<&(usize, String)>, lines: &[String]) -> String {
        match head {
            Some((level, text)) => {
                let title = !self.started && *level == 1 && !self.text_first;
                self.started = true;
                if !title {
                    while self.stack.last().is_some_and(|(top, _)| top >= level) {
                        self.stack.pop();
                    }
                    self.stack.push((*level, text.clone()));
                }
            }
            None if !self.started => {
                self.text_first |= lines.iter().any(|l| !l.trim().is_empty());
            }
            None => {}
        }
        self.stack
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join(" > ")
    }
}

fn bind(at: &mut [Option<usize>], taken: &mut [bool], j: usize, i: usize) {
    at[j] = Some(i);
    taken[i] = true;
}

/// For each unit of `side`, the unit of `base` it is. In order: the same heading and the same text, the same key,
/// a rename to a unique identical body, a rename by position between the same matched neighbours.
fn align(base: &[Unit], side: &[Unit]) -> Vec<Option<usize>> {
    let mut at: Vec<Option<usize>> = vec![None; side.len()];
    let mut taken = vec![false; base.len()];
    for (j, unit) in side.iter().enumerate() {
        let same = (0..base.len()).find(|&i| {
            !taken[i]
                && base[i].key.level == unit.key.level
                && base[i].key.text == unit.key.text
                && base[i].lines == unit.lines
        });
        if let Some(i) = same {
            bind(&mut at, &mut taken, j, i);
        }
    }
    for (j, unit) in side.iter().enumerate() {
        if at[j].is_some() {
            continue;
        }
        let found = unit
            .keys()
            .find_map(|k| (0..base.len()).find(|&i| !taken[i] && base[i].keys().any(|o| o == k)));
        if let Some(i) = found {
            bind(&mut at, &mut taken, j, i);
        }
    }
    let renamable = |u: &Unit| u.headed();
    let blank = |body: &[String]| body.iter().all(|l| l.trim().is_empty());
    for i in 0..base.len() {
        if taken[i] || !renamable(&base[i]) || blank(&base[i].lines[1..]) {
            continue;
        }
        let body = &base[i].lines[1..];
        let cands: Vec<usize> = (0..side.len())
            .filter(|&j| at[j].is_none() && renamable(&side[j]) && side[j].lines[1..] == *body)
            .collect();
        let rivals = (0..base.len())
            .filter(|&k| !taken[k] && renamable(&base[k]) && base[k].lines[1..] == *body)
            .count();
        if let [j] = cands[..]
            && rivals == 1
        {
            bind(&mut at, &mut taken, j, i);
        }
    }
    let mut pairs: Vec<(usize, usize)> = at
        .iter()
        .enumerate()
        .filter_map(|(j, i)| i.map(|i| (i, j)))
        .collect();
    pairs.sort_unstable();
    for i in 0..base.len() {
        if taken[i] || !renamable(&base[i]) {
            continue;
        }
        let before = pairs.iter().rev().find(|(b, _)| *b < i);
        let after = pairs.iter().find(|(b, _)| *b > i);
        let from = before.map_or(0, |(_, s)| s + 1);
        let to = after.map_or(side.len(), |(_, s)| *s);
        let level = base[i].key.level;
        let cands: Vec<usize> = (from..to.max(from))
            .filter(|&j| at[j].is_none() && renamable(&side[j]) && side[j].key.level == level)
            .collect();
        let (low, high) = (
            before.map_or(0, |(b, _)| b + 1),
            after.map_or(base.len(), |(b, _)| *b),
        );
        let rivals = (low..high)
            .filter(|&k| !taken[k] && renamable(&base[k]) && base[k].key.level == level)
            .count();
        if let [j] = cands[..]
            && rivals == 1
        {
            bind(&mut at, &mut taken, j, i);
        }
    }
    at
}

fn merge_units(
    base: &[Unit],
    lo: &[Unit],
    hi: &[Unit],
    vlo: &Side,
    vhi: &Side,
    flags: &mut BTreeSet<&'static str>,
) -> (Vec<String>, Vec<Conflict>) {
    let (lo_at, hi_at) = (align(base, lo), align(base, hi));
    // Identities: a base unit, or one both sides added under the same key, or one side's addition.
    let mut news: Vec<(Option<usize>, Option<usize>)> = Vec::new();
    let mut lo_id: Vec<Id> = Vec::new();
    let mut hi_id: Vec<Id> = vec![Id::New(usize::MAX); hi.len()];
    for (j, at) in hi_at.iter().enumerate() {
        if let Some(i) = at {
            hi_id[j] = Id::Base(*i);
        }
    }
    for (j, at) in lo_at.iter().enumerate() {
        if let Some(i) = at {
            lo_id.push(Id::Base(*i));
            continue;
        }
        let twin = (0..hi.len()).find(|&k| {
            hi_at[k].is_none()
                && hi_id[k] == Id::New(usize::MAX)
                && hi[k].keys().any(|o| lo[j].keys().any(|p| p == o))
        });
        let id = Id::New(news.len());
        news.push((Some(j), twin));
        if let Some(k) = twin {
            hi_id[k] = id;
        }
        lo_id.push(id);
    }
    for (k, id) in hi_id.iter_mut().enumerate() {
        if *id == Id::New(usize::MAX) {
            *id = Id::New(news.len());
            news.push((None, Some(k)));
        }
    }
    let lo_of = |id: Id| match id {
        Id::Base(i) => lo_at.iter().position(|a| *a == Some(i)),
        Id::New(n) => news[n].0,
    };
    let hi_of = |id: Id| match id {
        Id::Base(i) => hi_at.iter().position(|a| *a == Some(i)),
        Id::New(n) => news[n].1,
    };
    let ids: Vec<Id> = (0..base.len())
        .map(Id::Base)
        .chain((0..news.len()).map(Id::New))
        .collect();
    let mut items: BTreeMap<Id, Item> = BTreeMap::new();
    for id in ids {
        let b = match id {
            Id::Base(i) => Some(&base[i]),
            Id::New(_) => None,
        };
        let (l, h) = (lo_of(id).map(|j| &lo[j]), hi_of(id).map(|j| &hi[j]));
        if let Some(item) = merge_unit(b, l, h, vlo, vhi, flags) {
            items.insert(id, item);
        }
    }

    let out = order(&lo_id, &hi_id, &items);
    let mut trail = Trail::default();
    let mut lines = Vec::new();
    let mut conflict = Vec::new();
    for id in out {
        let item = &items[&id];
        let passage = trail.visit(item.head.as_ref(), &item.lines);
        if let Some(sides) = &item.conflict {
            conflict.push(Conflict {
                passage,
                sides: sides.clone(),
            });
        }
        lines.extend(item.lines.iter().cloned());
    }
    (lines, conflict)
}

fn merge_unit(
    base: Option<&Unit>,
    lo: Option<&Unit>,
    hi: Option<&Unit>,
    vlo: &Side,
    vhi: &Side,
    flags: &mut BTreeSet<&'static str>,
) -> Option<Item> {
    match (base, lo, hi) {
        (_, Some(l), Some(h)) if l.lines == h.lines => Some(Item::of(l)),
        (Some(b), Some(l), Some(h)) if l.lines == b.lines => Some(Item::of(h)),
        (Some(b), Some(l), Some(h)) if h.lines == b.lines => Some(Item::of(l)),
        (Some(b), Some(l), Some(h)) => {
            let parts = [b, l, h].iter().all(|u| u.headed()).then(|| {
                let pick = |at: fn(&[String]) -> &[String]| {
                    let (b, l, h) = (at(&b.lines), at(&l.lines), at(&h.lines));
                    if l == b {
                        Some(h)
                    } else if h == b {
                        Some(l)
                    } else {
                        None
                    }
                };
                let head = pick(|lines| &lines[..1])?;
                let body = pick(|lines| &lines[1..])?;
                Some([head, body].concat())
            });
            match parts.flatten() {
                Some(lines) => Some(Item::text(lines)),
                None => Some(conflict_item(Some(b), l, h, vlo, vhi)),
            }
        }
        (None, Some(l), Some(h)) => Some(conflict_item(None, l, h, vlo, vhi)),
        (Some(b), Some(u), None) | (Some(b), None, Some(u)) => {
            if u.lines == b.lines {
                None
            } else {
                flags.insert("edit-beat-delete");
                Some(Item::of(u))
            }
        }
        (None, Some(u), None) | (None, None, Some(u)) => Some(Item::of(u)),
        (_, None, None) => None,
    }
}

/// The sides of both units, a block's own included, ordered by version id and written as one block.
fn conflict_item(base: Option<&Unit>, lo: &Unit, hi: &Unit, vlo: &Side, vhi: &Side) -> Item {
    // Each side's pieces, tagged with the base piece it changed (`Some(true)`), left alone (`Some(false)`) or none.
    let held = base.and_then(|b| b.sides.as_ref());
    let mut tagged: Vec<(Piece, Option<bool>)> = Vec::new();
    let mut edited: [BTreeSet<String>; 2] = Default::default();
    for (n, (unit, side)) in [(lo, vlo), (hi, vhi)].into_iter().enumerate() {
        let own = |lines: &[String]| Piece {
            id: side.version.short().to_string(),
            time: side.version.minute(),
            lines: lines.to_vec(),
        };
        match &unit.sides {
            Some(sides) => {
                for piece in sides {
                    let old = held.and_then(|held| held.iter().find(|h| h.id == piece.id));
                    match old {
                        Some(old) if old.lines != piece.lines => {
                            edited[n].insert(piece.id.clone());
                            tagged.push((own(&piece.lines), None));
                        }
                        Some(_) => tagged.push((piece.clone(), Some(false))),
                        None => tagged.push((piece.clone(), None)),
                    }
                }
            }
            None => tagged.push((own(&unit.lines), None)),
        }
    }
    let mut pieces: Vec<Piece> = Vec::new();
    for (piece, left) in tagged {
        let superseded =
            left.is_some() && (edited[0].contains(&piece.id) || edited[1].contains(&piece.id));
        if !superseded {
            pieces.push(piece);
        }
    }
    pieces.sort_by(|a, b| (&a.id, &a.lines, &a.time).cmp(&(&b.id, &b.lines, &b.time)));
    let mut sides: Vec<Piece> = Vec::new();
    for piece in pieces {
        if !sides.iter().any(|s| s.lines == piece.lines) {
            sides.push(piece);
        }
    }
    if let [only] = sides.as_slice() {
        return Item::text(only.lines.clone());
    }
    let mut lines = Vec::new();
    for (n, side) in sides.iter().enumerate() {
        let mark = if n == 0 { OPEN } else { SEP };
        lines.push(format!("{mark}{} {}", side.id, side.time));
        lines.extend(side.lines.iter().cloned());
    }
    lines.push(CLOSE.to_string());
    let head = sides
        .iter()
        .find_map(|s| s.lines.first().and_then(|l| markdown::heading(l)));
    Item {
        lines,
        head,
        conflict: Some(sides.into_iter().map(|s| s.id).collect()),
    }
}

/// The merged order. Base units keep the order of the one side that reordered them, the first by id when both did.
/// A unit one side added follows the unit that preceded it there, and at one place the first side's come first.
fn order(lo_id: &[Id], hi_id: &[Id], items: &BTreeMap<Id, Item>) -> Vec<Id> {
    let kept = |side: &[Id]| -> Vec<Id> {
        side.iter()
            .filter(|id| matches!(id, Id::Base(_)) && items.contains_key(id))
            .copied()
            .collect()
    };
    let moved = |ids: &[Id]| {
        let mut sorted = ids.to_vec();
        sorted.sort_unstable();
        sorted != ids
    };
    let (k_lo, k_hi) = (kept(lo_id), kept(hi_id));
    let mut out = if moved(&k_lo) || !moved(&k_hi) {
        k_lo
    } else {
        k_hi
    };
    let missing: Vec<Id> = items
        .keys()
        .filter(|id| matches!(id, Id::Base(_)) && !out.contains(id))
        .copied()
        .collect();
    for id in missing {
        let at = out
            .iter()
            .rposition(|other| *other < id)
            .map_or(0, |p| p + 1);
        out.insert(at, id);
    }
    let anchor = |out: &[Id], side: &[Id], p: usize| {
        side[..p]
            .iter()
            .rev()
            .find_map(|prev| out.iter().position(|id| id == prev))
    };
    let mut lo_added = BTreeSet::new();
    for (p, id) in lo_id.iter().enumerate() {
        if matches!(id, Id::New(_)) && items.contains_key(id) {
            let at = anchor(&out, lo_id, p).map_or(0, |q| q + 1);
            out.insert(at, *id);
            lo_added.insert(*id);
        }
    }
    let mut hi_added = BTreeSet::new();
    for (p, id) in hi_id.iter().enumerate() {
        if matches!(id, Id::New(_)) && items.contains_key(id) && !lo_added.contains(id) {
            let mut at = anchor(&out, hi_id, p).map_or(0, |q| q + 1);
            let after = at.checked_sub(1).map(|q| out[q]);
            if !after.is_some_and(|a| hi_added.contains(&a)) {
                while at < out.len() && lo_added.contains(&out[at]) {
                    at += 1;
                }
            }
            out.insert(at, *id);
            hi_added.insert(*id);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01M3YJ7R6HK6NQ30DCDB1P4DYB";
    const CREATED: &str = "2026-10-02T14:23-03:00";
    const A: &str = "3f9a2c1b0d4e";
    const B: &str = "9c8d7e6f5a4b";

    fn note(extra: &[&str], body: &[&str]) -> String {
        let mut all = vec![
            "---".to_string(),
            format!("id: {ID}"),
            format!("created: {CREATED}"),
        ];
        all.extend(extra.iter().map(|l| l.to_string()));
        all.push("---".into());
        all.push(String::new());
        all.extend(body.iter().map(|l| l.to_string()));
        all.iter().map(|l| format!("{l}\n")).collect()
    }

    fn ver(prefix: &str, file: &str) -> Version {
        Version {
            version: format!("{prefix:0<64}"),
            file: file.into(),
            at: "2026-10-03T14:23:05-03:00".into(),
            ..Version::default()
        }
    }

    fn run_files(
        bases: &[&str],
        base_file: Option<&str>,
        a: (&str, &str),
        b: (&str, &str),
        syncs: &dyn Fn(&str) -> bool,
    ) -> Merged {
        let (va, vb) = (ver(A, a.1), ver(B, b.1));
        let bases: Vec<&[u8]> = bases.iter().map(|s| s.as_bytes()).collect();
        merge(
            &bases,
            base_file,
            &Side {
                version: &va,
                bytes: a.0.as_bytes(),
            },
            &Side {
                version: &vb,
                bytes: b.0.as_bytes(),
            },
            syncs,
        )
    }

    fn run(base: &str, a: &str, b: &str) -> Merged {
        run_files(
            &[base],
            Some("plan-x.md"),
            (a, "plan-x.md"),
            (b, "plan-x.md"),
            &|_| true,
        )
    }

    /// The merge of `run` with the arguments in the other order and each version keeping its content.
    fn run_swapped(base: &str, a: &str, b: &str) -> Merged {
        let (va, vb) = (ver(A, "plan-x.md"), ver(B, "plan-x.md"));
        merge(
            &[base.as_bytes()],
            Some("plan-x.md"),
            &Side {
                version: &vb,
                bytes: b.as_bytes(),
            },
            &Side {
                version: &va,
                bytes: a.as_bytes(),
            },
            &|_| true,
        )
    }

    fn text(merged: &Merged) -> String {
        String::from_utf8(merged.bytes.clone()).unwrap()
    }

    fn lines_of(merged: &Merged) -> Vec<String> {
        text(merged).lines().map(String::from).collect()
    }

    fn base() -> String {
        note(
            &[],
            &[
                "# Plan",
                "",
                "## Setup",
                "",
                "Install it.",
                "",
                "## Rollout",
                "",
                "Ship on Monday.",
                "",
                "### Rollback",
                "",
                "Revert it.",
                "",
            ],
        )
    }

    fn sub(text: &str, from: &str, to: &str) -> String {
        assert!(text.contains(from), "{from}");
        text.replacen(from, to, 1)
    }

    fn position(lines: &[String], line: &str) -> usize {
        lines.iter().position(|l| l == line).expect(line)
    }

    #[test]
    fn two_passages_two_devices() {
        let a = sub(&base(), "Install it.", "Install it twice.");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let merged = run(&base(), &a, &b);
        let out = text(&merged);
        assert!(out.contains("Install it twice.") && out.contains("Ship on Friday."));
        assert!(merged.conflict.is_empty() && !out.contains("<<<<<<<"));
        assert!(merged.flags.is_empty());
    }

    #[test]
    fn a_nested_heading_is_its_own_passage() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(&base(), "Revert it.", "Revert it fast.");
        let merged = run(&base(), &a, &b);
        let out = text(&merged);
        assert!(out.contains("Ship on Tuesday.") && out.contains("Revert it fast."));
        assert!(merged.conflict.is_empty());
    }

    #[test]
    fn one_passage_two_edits() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let merged = run(&base(), &a, &b);
        let out = text(&merged);
        assert!(out.contains(&format!(
            "<<<<<<< bilbo {A} 2026-10-03T14:23-03:00\n## Rollout"
        )));
        assert!(out.contains(&format!(
            "======= bilbo {B} 2026-10-03T14:23-03:00\n## Rollout"
        )));
        assert!(out.contains("Ship on Tuesday.") && out.contains("Ship on Friday."));
        assert!(out.contains(">>>>>>> bilbo\n"));
        assert_eq!(merged.conflict.len(), 1);
        assert_eq!(merged.conflict[0].passage, "Rollout");
        assert_eq!(merged.conflict[0].sides, vec![A.to_string(), B.to_string()]);
        let found = blocks(&out);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].passage, "Rollout");
        assert_eq!(found[0].sides[0].version, A);
        assert_eq!(found[0].sides[1].version, B);
        assert_eq!(found[0].sides[1].time, "2026-10-03T14:23-03:00");
    }

    #[test]
    fn the_same_edit_on_both_sides() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let merged = run(&base(), &a, &a);
        assert_eq!(text(&merged), a);
        let b = sub(&a, "Install it.", "Install it twice.");
        let merged = run(&base(), &a, &b);
        assert!(merged.conflict.is_empty());
        assert_eq!(text(&merged).matches("Ship on Friday.").count(), 1);
    }

    #[test]
    fn both_sides_append() {
        let a = format!("{}\n## Notes A\n\nA's.\n", base());
        let b = format!("{}\n## Notes B\n\nB's.\n", base());
        let merged = run(&base(), &a, &b);
        let lines = lines_of(&merged);
        assert!(merged.conflict.is_empty());
        assert!(position(&lines, "## Notes A") < position(&lines, "## Notes B"));
        assert_eq!(text(&run_swapped(&base(), &a, &b)), text(&merged));
        assert!(text(&merged).ends_with("B's.\n"));
    }

    #[test]
    fn deleted_against_edited() {
        let a = sub(
            &sub(
                &base(),
                "## Rollout\n\nShip on Monday.\n\n### Rollback\n\nRevert it.\n",
                "",
            ),
            "",
            "",
        );
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let merged = run(&base(), &a, &b);
        let out = text(&merged);
        assert!(out.contains("## Rollout") && out.contains("Ship on Friday."));
        assert_eq!(merged.flags, vec!["edit-beat-delete".to_string()]);
        assert_eq!(text(&run_swapped(&base(), &a, &b)), out);
    }

    #[test]
    fn deleted_against_untouched() {
        let a = sub(&base(), "## Rollout\n\nShip on Monday.\n\n", "");
        let merged = run(&base(), &a, &base());
        assert!(!text(&merged).contains("Ship on Monday."));
        assert!(!text(&merged).contains("## Rollout"));
        assert!(merged.flags.is_empty() && merged.conflict.is_empty());
    }

    #[test]
    fn renamed_against_edited() {
        let a = sub(&base(), "## Rollout", "## Release");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let merged = run(&base(), &a, &b);
        let out = text(&merged);
        assert!(merged.conflict.is_empty());
        assert_eq!(out.matches("## Release").count(), 1);
        assert!(!out.contains("## Rollout\n") && out.contains("Ship on Friday."));
        assert_eq!(text(&run_swapped(&base(), &a, &b)), out);
    }

    #[test]
    fn renamed_and_edited_against_edited() {
        let a = sub(
            &sub(&base(), "## Rollout", "## Release"),
            "Ship on Monday.",
            "Ship on Tuesday.",
        );
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let merged = run(&base(), &a, &b);
        let out = text(&merged);
        assert_eq!(merged.conflict.len(), 1);
        assert_eq!(out.matches("<<<<<<<").count(), 1);
        assert_eq!(out.matches("## Release").count(), 1);
        assert_eq!(out.matches("## Rollout").count(), 1);
        assert_eq!(out.matches("Ship on").count(), 2);
        assert!(out.contains(&format!("{A} 2026-10-03T14:23-03:00\n## Release")));
        assert!(out.contains(&format!("{B} 2026-10-03T14:23-03:00\n## Rollout")));
    }

    #[test]
    fn a_parent_heading_renamed() {
        let a = sub(&base(), "## Rollout", "## Release");
        let b = sub(&base(), "Revert it.", "Revert it fast.");
        let merged = run(&base(), &a, &b);
        let out = text(&merged);
        assert!(merged.conflict.is_empty());
        assert_eq!(out.matches("## Release").count(), 1);
        assert_eq!(out.matches("### Rollback").count(), 1);
        let lines = lines_of(&merged);
        assert!(position(&lines, "## Release") < position(&lines, "### Rollback"));
        assert!(out.contains("Revert it fast."));
    }

    #[test]
    fn sources_from_both_sides() {
        let start = note(&["sources:", "  - \"doc: base\""], &["# T"]);
        let a = note(
            &[
                "sources:",
                "  - \"doc: base\"",
                "  - \"url: https://a.example\"",
            ],
            &["# T"],
        );
        let b = note(
            &[
                "sources:",
                "  - \"doc: base\"",
                "  - \"code: src/watch.rs:12\"",
            ],
            &["# T"],
        );
        let merged = run(&start, &a, &b);
        let out = text(&merged);
        assert!(out.contains(
            "sources:\n  - \"doc: base\"\n  - \"code: src/watch.rs:12\"\n  - \"url: https://a.example\"\n---"
        ));
        assert_eq!(text(&run_swapped(&start, &a, &b)), out);
    }

    #[test]
    fn a_removed_source() {
        let start = note(
            &[
                "sources:",
                "  - \"url: https://old.example\"",
                "  - \"doc: keep\"",
            ],
            &["# T"],
        );
        let a = note(&["sources:", "  - \"doc: keep\""], &["# T"]);
        let b = sub(&start, "# T", "# T\n\nMore.");
        let merged = run(&start, &a, &b);
        let out = text(&merged);
        assert!(!out.contains("old.example") && out.contains("doc: keep") && out.contains("More."));
        let a = note(&[], &["# T"]);
        assert!(!text(&run(&start, &a, &b)).contains("sources"));
    }

    #[test]
    fn an_unknown_key_changed_on_both_sides() {
        let start = note(&["project: x"], &["# T"]);
        let a = note(&["project: a"], &["# T"]);
        let b = note(&["project: b"], &["# T"]);
        let merged = run(&start, &a, &b);
        let out = text(&merged);
        assert!(out.contains("project: a\n---") && !out.contains("project: b"));
        assert!(!out.contains("<<<<<<<"));
        assert_eq!(merged.flags, vec!["key-kept".to_string()]);
        assert_eq!(text(&run_swapped(&start, &a, &b)), out);
    }

    #[test]
    fn a_changed_created() {
        let a = sub(&base(), CREATED, "2026-10-04T10:00-03:00");
        let b = sub(&base(), "Install it.", "Install it twice.");
        let merged = run(&base(), &a, &b);
        let out = text(&merged);
        assert!(out.contains(&format!("created: {CREATED}\n")));
        assert!(out.contains("Install it twice.") && !out.contains("2026-10-04"));
        assert_eq!(merged.flags, vec!["created-kept".to_string()]);
    }

    #[test]
    fn the_id_never_changes() {
        let a = sub(&base(), ID, "01M3YJ7R6HK6NQ30DCDB1P4DYC");
        let b = sub(&base(), "Install it.", "Install it twice.");
        assert!(text(&run(&base(), &a, &b)).contains(&format!("id: {ID}\n")));
    }

    #[test]
    fn a_scope_synced_against_local() {
        let start = note(&["scope: personal"], &["# T"]);
        let a = note(&["scope: shared"], &["# T"]);
        let b = note(&["scope: work"], &["# T"]);
        let syncs = |name: &str| name != "work";
        for (x, y) in [
            ((&a[..], "plan-x.md"), (&b[..], "plan-x.md")),
            ((&b[..], "plan-x.md"), (&a[..], "plan-x.md")),
        ] {
            let merged = run_files(&[&start], Some("plan-x.md"), x, y, &syncs);
            assert!(text(&merged).contains("scope: work\n"));
            assert_eq!(merged.flags, vec!["scope-clash".to_string()]);
        }
    }

    #[test]
    fn two_synced_scopes_leave_no_scope_key() {
        let start = note(&["scope: personal"], &["# T"]);
        let a = note(&["scope: shared"], &["# T"]);
        let b = note(&["scope: team"], &["# T"]);
        let merged = run(&start, &a, &b);
        assert!(!text(&merged).contains("scope"));
        assert_eq!(merged.flags, vec!["scope-clash".to_string()]);
        let none = note(&[], &["# T"]);
        let merged = run(&start, &a, &none);
        assert!(!text(&merged).contains("scope"));
    }

    #[test]
    fn markers_in_a_fence_are_not_blocks() {
        let fenced = note(
            &[],
            &[
                "# T",
                "",
                "```",
                "<<<<<<< bilbo 3f9a2c1b0d4e 2026-10-03T14:23-03:00",
                "one",
                "======= bilbo 9c8d7e6f5a4b 2026-10-03T14:25-03:00",
                "two",
                ">>>>>>> bilbo",
                "```",
            ],
        );
        assert!(blocks(&fenced).is_empty());
        let pasted = fenced.replace("```\n", "");
        assert_eq!(blocks(&pasted).len(), 1);
        let above = "---\nid: x\n<<<<<<< bilbo 3f9a2c1b0d4e t\n>>>>>>> bilbo\n---\n\n# T\n";
        assert!(blocks(above).is_empty());
    }

    #[test]
    fn a_fenced_example_merges_as_text() {
        let example = [
            "```",
            "<<<<<<< bilbo 3f9a2c1b0d4e 2026-10-03T14:23-03:00",
            "```",
        ];
        let mut body = vec!["# T", ""];
        body.extend(example);
        body.extend(["", "## Next", "", "x"]);
        let start = note(&[], &body);
        let a = sub(&start, "x", "y");
        let merged = run(&start, &a, &start);
        assert!(merged.conflict.is_empty());
        assert_eq!(text(&merged), a);
    }

    #[test]
    fn a_third_side_joins_the_block() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let blocked = text(&run(&base(), &a, &b));
        let c = sub(&base(), "Ship on Monday.", "Ship on Sunday.");
        let (vb, vc) = (
            ver("5a5a5a5a5a5a", "plan-x.md"),
            ver("c1c1c1c1c1c1", "plan-x.md"),
        );
        let merged = merge(
            &[base().as_bytes()],
            Some("plan-x.md"),
            &Side {
                version: &vb,
                bytes: blocked.as_bytes(),
            },
            &Side {
                version: &vc,
                bytes: c.as_bytes(),
            },
            &|_| true,
        );
        let out = text(&merged);
        assert_eq!(out.matches("<<<<<<<").count(), 1);
        assert_eq!(out.matches("=======").count(), 2);
        assert_eq!(out.matches(">>>>>>>").count(), 1);
        for line in ["Ship on Tuesday.", "Ship on Friday.", "Ship on Sunday."] {
            assert_eq!(out.matches(line).count(), 1, "{line}");
        }
        assert_eq!(merged.conflict[0].sides.len(), 3);
        let found = blocks(&out);
        let order: Vec<&str> = found[0].sides.iter().map(|s| s.version.as_str()).collect();
        assert_eq!(order, [A, B, "c1c1c1c1c1c1"]);
    }

    #[test]
    fn a_resolved_block_merges_with_an_untouched_one() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let blocked = text(&run(&base(), &a, &b));
        let resolved = sub(&base(), "Ship on Monday.", "Ship on Tuesday or Friday.");
        let (vx, vy) = (
            ver("5a5a5a5a5a5a", "plan-x.md"),
            ver("c1c1c1c1c1c1", "plan-x.md"),
        );
        let merged = merge(
            &[blocked.as_bytes()],
            Some("plan-x.md"),
            &Side {
                version: &vx,
                bytes: blocked.as_bytes(),
            },
            &Side {
                version: &vy,
                bytes: resolved.as_bytes(),
            },
            &|_| true,
        );
        assert!(merged.conflict.is_empty());
        assert_eq!(text(&merged), resolved);
    }

    #[test]
    fn keeping_both_sides_drops_nothing() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let blocked = text(&run(&base(), &a, &b));
        let found = blocks(&blocked);
        let kept = sub(
            &base(),
            "Ship on Monday.",
            "Ship on Tuesday.\n\nShip on Friday.",
        );
        assert!(dropped(&found, &kept).is_empty());
    }

    #[test]
    fn dropping_a_side_reports_its_lines() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(
            &base(),
            "Ship on Monday.",
            "Ship on  Friday.\n\nAfter lunch.",
        );
        let blocked = text(&run(&base(), &a, &b));
        let found = blocks(&blocked);
        let lost = dropped(&found, &a);
        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0].passage, "Rollout");
        assert_eq!(lost[0].lines, ["Ship on  Friday.", "After lunch."]);
        let spaced = sub(
            &base(),
            "Ship on Monday.",
            "Ship on Tuesday.\nShip on Friday.\n",
        );
        assert_eq!(dropped(&found, &spaced)[0].lines, ["After lunch."]);
    }

    #[test]
    fn markers_left_in_place_stay_open() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let blocked = text(&run(&base(), &a, &b));
        let edited = sub(&blocked, "Install it.", "Install it twice.");
        assert_eq!(blocks(&edited).len(), 1);
    }

    #[test]
    fn the_file_name_follows_the_renamer() {
        let start = base();
        let edited = sub(&start, "Install it.", "Install it twice.");
        let go = |a: (&str, &str), b: (&str, &str)| {
            run_files(&[&start], Some("plan-release.md"), a, b, &|_| true).file
        };
        assert_eq!(
            go(
                (&start, "decision-release.md"),
                (&edited, "plan-release.md")
            ),
            "decision-release.md"
        );
        assert_eq!(
            go(
                (&edited, "plan-release.md"),
                (&start, "decision-release.md")
            ),
            "decision-release.md"
        );
        assert_eq!(
            go((&start, "plan-release.md"), (&edited, "plan-release.md")),
            "plan-release.md"
        );
        assert_eq!(
            go(
                (&start, "spec-release.md"),
                (&edited, "decision-release.md")
            ),
            "spec-release.md"
        );
    }

    #[test]
    fn a_base_pruned_away() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(
            &sub(&base(), "Ship on Monday.", "Ship on Friday."),
            "Install it.",
            "Install it twice.",
        );
        let merged = run_files(&[], None, (&a, "plan-x.md"), (&b, "plan-x.md"), &|_| true);
        let out = text(&merged);
        assert_eq!(merged.conflict.len(), 2);
        assert_eq!(out.matches("<<<<<<<").count(), 2);
        for line in [
            "Install it.",
            "Install it twice.",
            "Ship on Tuesday.",
            "Ship on Friday.",
            "Revert it.",
        ] {
            assert!(out.contains(line), "{line}");
        }
        assert_eq!(out.matches("Revert it.").count(), 1);
    }

    #[test]
    fn identical_passages_without_a_base() {
        let a = sub(
            &base(),
            "Revert it.\n",
            "Revert it.\n\n## Extra\n\nA only.\n",
        );
        let merged = run_files(
            &[],
            None,
            (&a, "plan-x.md"),
            (&base(), "plan-x.md"),
            &|_| true,
        );
        let out = text(&merged);
        assert!(merged.conflict.is_empty());
        assert_eq!(out.matches("## Setup").count(), 1);
        assert!(out.contains("A only."));
    }

    #[test]
    fn two_bases_that_disagree() {
        let with = sub(
            &base(),
            "Revert it.\n\n",
            "Revert it.\n\n## Notes\n\nKept.\n\n",
        );
        let (a, b) = (with.clone(), base());
        let merged = run_files(
            &[&with, &base()],
            Some("plan-x.md"),
            (&a, "plan-x.md"),
            (&b, "plan-x.md"),
            &|_| true,
        );
        assert!(text(&merged).contains("## Notes") && text(&merged).contains("Kept."));
        assert!(merged.conflict.is_empty());
    }

    #[test]
    fn a_criss_cross_base_keeps_a_re_added_passage() {
        // B1 holds the passage, B2 never did. One side re-added it identically, the other never had it.
        let with = sub(
            &base(),
            "Revert it.\n\n",
            "Revert it.\n\n## Notes\n\nKept.\n\n",
        );
        let a = sub(&with, "Install it.", "Install it twice.");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let merged = run_files(
            &[&with, &base()],
            Some("plan-x.md"),
            (&a, "plan-x.md"),
            (&b, "plan-x.md"),
            &|_| true,
        );
        assert!(text(&merged).contains("Kept."));
        // Picking B1 alone reads the passage as deleted by the other side.
        let alone = run_files(
            &[&with],
            Some("plan-x.md"),
            (&a, "plan-x.md"),
            (&b, "plan-x.md"),
            &|_| true,
        );
        assert!(!text(&alone).contains("Kept."));
    }

    #[test]
    fn the_text_before_the_first_heading_merges() {
        let start = note(&[], &["Intro line.", "", "## A", "", "a"]);
        let a = sub(&start, "Intro line.", "Intro line, changed.");
        let b = sub(&start, "\na\n", "\nb\n");
        let merged = run(&start, &a, &b);
        assert!(merged.conflict.is_empty());
        assert!(text(&merged).contains("Intro line, changed.") && text(&merged).contains("\nb\n"));
        let c = sub(&start, "Intro line.", "Intro line, other.");
        let merged = run(&start, &a, &c);
        assert_eq!(merged.conflict.len(), 1);
        assert_eq!(merged.conflict[0].passage, "");
        assert_eq!(blocks(&text(&merged))[0].line, 5);
    }

    #[test]
    fn a_swap_gives_the_same_bytes() {
        let a = sub(
            &sub(&base(), "Ship on Monday.", "Ship on Tuesday."),
            "Install it.",
            "Install it twice.",
        );
        let b = sub(
            &sub(&base(), "Ship on Monday.", "Ship on Friday."),
            "# Plan",
            "# Plan B",
        );
        assert_eq!(run(&base(), &a, &b), run_swapped(&base(), &a, &b));
    }

    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    type Sections = Vec<(String, Vec<String>)>;

    fn random_base() -> Sections {
        let mut sections: Sections = vec![("# Plan".into(), vec![String::new()])];
        for s in 0..4 {
            let mut lines = vec![String::new()];
            for n in 0..3 {
                lines.push(format!("base line {s}.{n}"));
            }
            lines.push(String::new());
            sections.push((format!("## Part {s}"), lines));
            if s % 2 == 0 {
                sections.push((
                    format!("### Part {s} sub"),
                    vec![String::new(), format!("base sub {s}"), String::new()],
                ));
            }
        }
        sections
    }

    fn random_edit(sections: &mut Sections, rng: &mut Rng, tag: &str, n: usize, moved: &mut bool) {
        let s = rng.below(sections.len());
        match rng.below(6) {
            0 if !sections[s].1.is_empty() => {
                let l = rng.below(sections[s].1.len());
                sections[s].1[l] = format!("edit {tag}{n}");
            }
            1 if !sections[s].1.is_empty() => {
                let l = rng.below(sections[s].1.len());
                sections[s].1.remove(l);
            }
            2 => {
                let l = rng.below(sections[s].1.len() + 1);
                sections[s].1.insert(l, format!("added {tag}{n}"));
            }
            3 if s > 0 && !*moved => {
                *moved = true;
                sections.remove(s);
            }
            4 if !*moved => {
                *moved = true;
                let heading = format!("## New {tag}{n}");
                sections.insert(
                    s + 1,
                    (
                        heading,
                        vec![String::new(), format!("new body {tag}{n}"), String::new()],
                    ),
                );
            }
            5 if s > 0 && !*moved => {
                *moved = true;
                let level = sections[s].0.bytes().take_while(|b| *b == b'#').count();
                sections[s].0 = format!("{} Renamed {tag}{n}", "#".repeat(level));
            }
            _ => {}
        }
    }

    fn render(sections: &Sections) -> String {
        let body: Vec<&str> = sections
            .iter()
            .flat_map(|(h, lines)| std::iter::once(h).chain(lines).map(String::as_str))
            .collect();
        note(&[], &body)
    }

    fn non_blank(text: &str) -> BTreeSet<String> {
        text.lines()
            .map(collapse)
            .filter(|l| !l.is_empty())
            .collect()
    }

    fn mg(bases: &[&str], a: (&str, &str, &str), b: (&str, &str, &str)) -> Merged {
        let (va, vb) = (ver(a.2, a.1), ver(b.2, b.1));
        let bases: Vec<&[u8]> = bases.iter().map(|s| s.as_bytes()).collect();
        merge(
            &bases,
            Some("plan-x.md"),
            &Side {
                version: &va,
                bytes: a.0.as_bytes(),
            },
            &Side {
                version: &vb,
                bytes: b.0.as_bytes(),
            },
            &|_| true,
        )
    }

    #[test]
    fn a_third_side_joins_a_block_whose_first_side_renamed() {
        // A renames+edits, B edits: block keyed by A's `## Release`. C edits `## Rollout` against the base.
        let a = sub(
            &sub(&base(), "## Rollout", "## Release"),
            "Ship on Monday.",
            "Ship on Tuesday.",
        );
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let m = text(&run(&base(), &a, &b));
        let c = sub(&base(), "Ship on Monday.", "Ship on Sunday.");
        let merged = mg(
            &[&base()],
            (&m, "plan-x.md", "5a5a5a5a5a5a"),
            (&c, "plan-x.md", "c1c1c1c1c1c1"),
        );
        let out = text(&merged);
        assert_eq!(out.matches("<<<<<<<").count(), 1);
        assert_eq!(
            out.matches("=======").count(),
            2,
            "third side should join the block"
        );
        assert!(merged.flags.is_empty(), "spurious flags {:?}", merged.flags);
        assert_eq!(blocks(&out)[0].sides.len(), 3);
    }

    #[test]
    fn both_sides_edit_inside_the_same_block() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let m = text(&run(&base(), &a, &b));
        let x = sub(&m, "Ship on Tuesday.", "Ship on Tuesday, morning.");
        let y = sub(&m, "Ship on Tuesday.", "Ship on Tuesday, evening.");
        let merged = mg(
            &[&m],
            (&x, "plan-x.md", "5a5a5a5a5a5a"),
            (&y, "plan-x.md", "c1c1c1c1c1c1"),
        );
        let out = text(&merged);
        assert!(
            out.contains("Tuesday, morning.") && out.contains("Tuesday, evening."),
            "an edit inside a block was lost"
        );
    }

    #[test]
    fn duplicate_headings_delete_first_edit_second() {
        let start = note(
            &[],
            &[
                "# T", "", "## Notes", "", "first.", "", "## Notes", "", "second.", "",
            ],
        );
        let a = sub(&start, "## Notes\n\nfirst.\n\n", "");
        let b = sub(&start, "second.", "second, edited.");
        let merged = run(&start, &a, &b);
        let out = text(&merged);
        assert!(!out.contains("first."));
        assert_eq!(out.matches("## Notes").count(), 1, "duplicated passage");
        assert!(out.contains("second, edited."));
    }

    #[test]
    fn a_same_heading_inserted_before_shifts_no_rank() {
        // base: `## Notes`(x). A inserts another `## Notes` before it. B edits x.
        let start = note(&[], &["# T", "", "## Notes", "", "x", ""]);
        let a = note(
            &[],
            &[
                "# T", "", "## Notes", "", "new", "", "## Notes", "", "x", "",
            ],
        );
        let b = sub(&start, "\nx\n", "\nx2\n");
        let merged = run(&start, &a, &b);
        let out = text(&merged);
        assert!(out.contains("x2") && out.contains("new"));
        assert!(merged.conflict.is_empty(), "rank shift made a conflict");
    }

    #[test]
    fn a_carried_block_is_recorded() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let m = text(&run(&base(), &a, &b));
        // a device that never saw M merges M with its own unrelated edit, base = the original base
        let c = sub(&base(), "Install it.", "Install it twice.");
        let merged = mg(
            &[&base()],
            (&m, "plan-x.md", "5a5a5a5a5a5a"),
            (&c, "plan-x.md", "c1c1c1c1c1c1"),
        );
        let out = text(&merged);
        assert_eq!(
            merged.conflict.len(),
            blocks(&out).len(),
            "record disagrees with the file"
        );
    }

    #[test]
    fn a_carried_block_is_recorded_with_the_block_as_base() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let m = text(&run(&base(), &a, &b));
        let x = sub(&m, "Install it.", "Install it twice.");
        let y = sub(&m, "Revert it.", "Revert it fast.");
        let merged = mg(
            &[&m],
            (&x, "plan-x.md", "5a5a5a5a5a5a"),
            (&y, "plan-x.md", "c1c1c1c1c1c1"),
        );
        let out = text(&merged);
        assert_eq!(
            merged.conflict.len(),
            blocks(&out).len(),
            "record disagrees with the file"
        );
    }

    #[test]
    fn delete_rename_and_move_keep_each_identity() {
        let start = note(
            &[],
            &[
                "# T",
                "",
                "## P0",
                "",
                "p0",
                "",
                "### P0 sub",
                "",
                "s",
                "",
                "## P1",
                "",
                "p1",
                "",
                "## P2",
                "",
                "p2",
                "",
            ],
        );
        // X deletes `## P0`, renames `## P1` to `## R`, moves `### P0 sub` after `## P2`
        let x = note(
            &[],
            &[
                "# T",
                "",
                "## R",
                "",
                "p1",
                "",
                "## P2",
                "",
                "p2",
                "",
                "### P0 sub",
                "",
                "s",
                "",
            ],
        );
        let y = sub(&start, "\np1\n", "\np1 edited\n");
        let merged = run(&start, &x, &y);
        let out = text(&merged);
        assert!(out.contains("p1 edited"));
        assert_eq!(
            out.matches("\np1\n").count(),
            0,
            "the unedited body came back beside the edit"
        );
        assert!(
            !out.contains("## P1\n"),
            "the renamed passage came back under its old name"
        );
        assert!(merged.flags.is_empty(), "{:?}", merged.flags);
    }

    #[test]
    fn the_order_of_the_bases_changes_nothing() {
        let b1 = note(&[], &["# T", "", "## P", "", "p", "", "## Q", "", "q", ""]);
        let b2 = note(&[], &["# T", "", "## Q", "", "q", "", "## P", "", "p", ""]);
        let lo = sub(&b1, "\np\n", "\np2\n");
        let hi = sub(&b2, "\nq\n", "\nq2\n");
        let one = run_files(
            &[&b1, &b2],
            Some("plan-x.md"),
            (&lo, "plan-x.md"),
            (&hi, "plan-x.md"),
            &|_| true,
        );
        let two = run_files(
            &[&b2, &b1],
            Some("plan-x.md"),
            (&lo, "plan-x.md"),
            (&hi, "plan-x.md"),
            &|_| true,
        );
        assert_eq!(one, two, "the merge depends on the order of the bases");
    }

    #[test]
    fn a_block_renamed_side_keeps_its_base_passage() {
        // block keyed by A's `## Release`; C deletes the base `## Rollout`
        let a = sub(
            &sub(&base(), "## Rollout", "## Release"),
            "Ship on Monday.",
            "Ship on Tuesday.",
        );
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let m = text(&run(&base(), &a, &b));
        let c = sub(&base(), "## Rollout\n\nShip on Monday.\n\n", "");
        let merged = mg(
            &[&base()],
            (&m, "plan-x.md", "5a5a5a5a5a5a"),
            (&c, "plan-x.md", "c1c1c1c1c1c1"),
        );
        let out = text(&merged);
        assert_eq!(blocks(&out).len(), 1);
        assert_eq!(out.matches("## Rollout").count(), 1);
        assert_eq!(merged.conflict.len(), 1);
    }

    #[test]
    fn each_side_edits_one_side_of_a_block() {
        let a = sub(&base(), "Ship on Monday.", "Ship on Tuesday.");
        let b = sub(&base(), "Ship on Monday.", "Ship on Friday.");
        let m = text(&run(&base(), &a, &b));
        let x = sub(&m, "Ship on Tuesday.", "Ship on Tuesday. Early.");
        let y = sub(&m, "Ship on Friday.", "Ship on Friday. Late.");
        let merged = mg(
            &[&m],
            (&x, "plan-x.md", "5a5a5a5a5a5a"),
            (&y, "plan-x.md", "c1c1c1c1c1c1"),
        );
        let out = text(&merged);
        assert!(
            out.contains("Tuesday. Early."),
            "X's edit inside the block was lost"
        );
        assert!(
            out.contains("Friday. Late."),
            "Y's edit inside the block was lost"
        );
    }

    fn count_lines(text: &str) -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for l in text.lines().map(collapse).filter(|l| !l.is_empty()) {
            *m.entry(l).or_insert(0) += 1;
        }
        m
    }

    /// A side restructures at most once (a rename, move, duplicate, addition or deletion of a passage), so no merge
    /// has to guess which passage a renamed and moved one was.
    fn random_edit2(sections: &mut Sections, rng: &mut Rng, tag: &str, n: usize, moved: &mut bool) {
        let s = rng.below(sections.len());
        match rng.below(9) {
            0..=5 => random_edit(sections, rng, tag, n, moved),
            6 if sections.len() > 2 && !*moved => {
                *moved = true;
                // move a section
                let t = 1 + rng.below(sections.len() - 1);
                let sec = sections.remove(s.max(1));
                let t = t.min(sections.len());
                sections.insert(t, sec);
            }
            7 if !*moved => {
                *moved = true;
                // duplicate heading: add a section with an existing heading
                let h = sections[s].0.clone();
                sections.insert(
                    s + 1,
                    (
                        h,
                        vec![String::new(), format!("dup body {tag}{n}"), String::new()],
                    ),
                );
            }
            8 if s > 0 && !*moved => {
                *moved = true;
                // rename + edit
                let level = sections[s].0.bytes().take_while(|b| *b == b'#').count();
                sections[s].0 = format!("{} Renamed {tag}{n}", "#".repeat(level));
                if !sections[s].1.is_empty() {
                    let l = rng.below(sections[s].1.len());
                    sections[s].1[l] = format!("edit {tag}{n}");
                }
            }
            _ => {}
        }
    }

    #[test]
    fn random_edit_pairs_lose_and_duplicate_no_line() {
        let start = render(&random_base());
        let held = non_blank(&start);
        let base_counts = count_lines(&start);
        let mut failures = Vec::new();
        for seed in 1..=2000u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let (mut a, mut b) = (random_base(), random_base());
            let (mut moved_a, mut moved_b) = (false, false);
            for n in 0..1 + rng.below(4) {
                random_edit2(&mut a, &mut rng, "a", n, &mut moved_a);
            }
            for n in 0..1 + rng.below(4) {
                random_edit2(&mut b, &mut rng, "b", n, &mut moved_b);
            }
            let (ta, tb) = (render(&a), render(&b));
            let merged = run(&start, &ta, &tb);
            let out_text = text(&merged);
            let out = non_blank(&out_text);
            let (ca, cb) = (count_lines(&ta), count_lines(&tb));
            let co = count_lines(&out_text);
            let mut problems = Vec::new();
            for side in [&ta, &tb] {
                for line in non_blank(side) {
                    if !(held.contains(&line) || out.contains(&line)) {
                        problems.push(format!("new line lost: {line:?}"));
                    }
                }
            }
            // a base line both sides kept must survive
            for line in base_counts.keys() {
                if ca.contains_key(line) && cb.contains_key(line) && !co.contains_key(line) {
                    problems.push(format!("base line kept by both lost: {line:?}"));
                }
            }
            // without conflicts, no line should appear more often than on the side holding it most
            if merged.conflict.is_empty() {
                for (line, n) in &co {
                    let most = ca
                        .get(line)
                        .copied()
                        .unwrap_or(0)
                        .max(cb.get(line).copied().unwrap_or(0));
                    if *n > most {
                        problems.push(format!("duplicated: {line:?} x{n} (sides at most {most})"));
                    }
                }
            }
            if merged != run_swapped(&start, &ta, &tb) {
                problems.push("not symmetric".into());
            }
            if merged.conflict.len() != blocks(&out_text).len() {
                problems.push("conflicts != blocks".into());
            }
            if !problems.is_empty() {
                failures.push((
                    seed,
                    problems,
                    ta.clone(),
                    tb.clone(),
                    out_text.clone(),
                    merged.flags.clone(),
                ));
            }
        }
        if let Some((seed, problems, ta, tb, out, flags)) = failures.first() {
            panic!(
                "seed {seed}: {problems:?} flags={flags:?}\n--- a:\n{ta}--- b:\n{tb}--- merged:\n{out}"
            );
        }
    }

    #[test]
    fn marker_lines_are_full_form_outside_fences_below_the_frontmatter() {
        let text = "---\nid: x\n>>>>>>> bilbo\n---\n\n<<<<<<< bilbo 3f9a2c1b0d4e 2026-10-03T14:23-03:00\nx\n======= bilbo 9c8d7e6f5a4b t\n```\n>>>>>>> bilbo\n```\n>>>>>>> bilbo was here\n======= bilbo nothex t\n>>>>>>> bilbo\n";
        assert_eq!(marker_lines(text), [6, 8, 14]);
        assert_eq!(marker_lines(">>>>>>> bilbo\n"), [1]);
    }
}
