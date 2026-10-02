# Design

## Context

bilbo has two verbs, `new` and `check`, on top of `store.rs` (root and entries of `notes/`) and `note.rs` (names, the strict reader, rendering). `note::read` returns the note's id and its problems, nothing about the body. Arguments are parsed by hand, and `jiff` is the only dependency.

The legacy `nbrecall` (dnix `modules/notebooks/nbrecall.py`) is the behavior reference, not code to port. It splits files into heading sections, cuts long ones at 4,000 bytes on paragraph boundaries, indexes them in SQLite FTS5 (`unicode61 remove_diacritics 2`), ORs the query words, ranks with BM25, keeps one hit per note and fuses that with embeddings through reciprocal rank fusion.

Measured on 2026-10-02 over the 424 legacy notes (5.6 MiB, 4,827 heading sections), in Python on rivendell: building an in-memory FTS5 table took 200 to 300 ms per run and a query 3 to 4 ms; tokenizing every note with a regex took about 350 ms and BM25 scoring 4 to 7 ms. Evidence: `work/recall-arms/bench.py` and `bench2.py` in the bilbo-initial-launch notebook. Rust tokenizing by `char` is expected to be an order of magnitude faster than the Python regex, which is why the index can wait.

Measured on 2026-10-02 on rivendell with `cargo test --release --test recall -- --ignored --nocapture`: `bilbo recall` over the generated store (450 notes, 6.3 MiB) took 67, 64 and 90 ms over three runs, process start included.

## Goals / Non-Goals

**Goals:**
- `recall` over a store the size of the legacy one (about 6 MiB) answers in under 250 ms on rivendell, with a cold start and no cache.
- One definition of a passage and of a query word, in a module the add-embeddings and add-note-digest changes reuse instead of redefining.
- An output contract that survives the ranking changing underneath it.

**Non-Goals:**
- An index, a cache or anything written to disk.
- Ranking parity with `nbrecall`. The spec fixes what a hit looks like and the ordering rules, not scores.

## Decisions

### No index: read the store on every run

`recall` lists `notes/`, reads every note, splits it into passages, tokenizes them and ranks, on every run. Nothing goes stale, there is no cache to invalidate, and the read-only rule stays as simple as `check`'s.

- Why not a persistent FTS5 index now: it must be kept current, which needs a watcher, a timer or a freshness check on every query, and that is the add-embeddings change's problem anyway. At 6 MiB the scan is cheap.
- When this stops holding: the cost is linear in store size. A store ten times the legacy one would take the scan to roughly a second. add-embeddings brings an index for its own reasons, and keyword ranking can move into it then without a spec change.

### BM25 by hand, no SQLite yet

Score each passage with Okapi BM25 over all passages in the store (`k1 = 1.2`, `b = 0.75`, IDF `ln(1 + (N - df + 0.5) / (df + 0.5))`), where a passage's terms are its heading path plus its text. Query words are deduplicated, and a passage that holds none of them is not a hit. A note's score is its best passage's score.

| Need | Choice | Why not the alternative |
|---|---|---|
| Ranking | BM25 by hand over word counts, standard library only | `rusqlite` with bundled SQLite gives FTS5's BM25, but compiles a C library, adds the first dependency beyond `jiff`, and would be rebuilt in memory on every run anyway. It earns its place in add-embeddings, which needs sqlite-vec and a persistent index. `tantivy` is a full search engine with dozens of crates for a 6 MiB corpus. |
| Case folding | `char::to_lowercase` | Standard library, full Unicode. |
| Accent folding | A fixed table over Latin-1 Supplement and Latin Extended-A (U+00C0 to U+017F), mapping each letter to its base: `ã` to `a`, `ç` to `c`, `ø` to `o`, `ß` to `ss`, `æ` to `ae` | The `unicode-normalization` crate decomposes everything, but the notes are English and Portuguese, which this range covers completely. The table holds the 97 lowercase letters of the range, 17 of them with no decomposition (`ð` to `d`, `þ` to `th`, `ı` to `i`, `ł` to `l`, `ŋ` to `n`, `ſ` to `s` and the like), and a unit test. Combining marks U+0300 to U+036F join the word they follow and are dropped, so decomposed text folds like composed text, and `İ`, which lowercases to `i` plus a mark, folds to `i`. Letters outside the range pass through unchanged. |
| Words | Runs of `char::is_alphanumeric`, folded, at least 2 characters | Matches `nbrecall`'s `[^\W_]+` with its 2-character floor. `_` splits words in both. The floor counts folded characters, so `ß` alone is the word `ss`. |

The score is never printed. add-embeddings replaces it with a fusion of keyword and meaning ranks, and the output stays the same.

### Passages

A passage starts at each ATX heading (`#` to `######` followed by a space) outside fenced code blocks, and runs to the next one. Text before the first heading forms a passage whose heading path is the title alone. The heading path is the stack of enclosing heading texts, starting with the note's title. A note with no `#` title takes its filename stem as the title, so every heading path still starts with something readable.

A heading is a line that starts with one to six `#` and a space, and still has text once a closing run of `#` is dropped; indented lines are not headings. The title is the first level-1 heading; later level-1 headings nest under it. Blank text before the first heading forms no passage: the line after the frontmatter is usually blank and would otherwise win every query on a title word. A heading with no text under it is still a passage. The first part of a split passage starts at the passage's line, later parts at their own first line.

A passage over 4,000 bytes is cut at blank lines into parts of at most 4,000 bytes, and a single paragraph over the limit is cut at the last character boundary under 4,000 bytes. Each part keeps the passage's heading path and starts at its own line. That limit is the embedder's, from the legacy setup: one llama-server slot of 4,096 tokens holds at most 4,000 bytes. Fixing it here means the line numbers `recall` prints today stay the same when add-embeddings starts embedding the same parts.

### Modules

- `src/note.rs` (extended): `Note` gains `created: Option<String>`, set only when the value is valid, and `body_start`, the line after the closing `---` or line 1 when the frontmatter is missing or never closes. The strict reader already walks those lines, so recall and check share one parse. It also gives `lines`, the splitter `read` numbers lines with, so recall slices the body at `body_start` with the same numbering; `parse_name` returns the kind; and `fence_run` is shared with `rank`, so passages and the title rule agree on fences.
- `src/rank.rs` (new): `words(text)`, `passages(lines, first_line, fallback_title)`, where the fallback (the filename stem) is used only when the body has no title and `rank(query_words, documents)`, returning plain values. It knows nothing about the store or the CLI. add-embeddings extends it with fusion, and add-note-digest calls it.
- `src/recall.rs` (new): argument parsing, listing through `store::entries` and `note::parse_name`, reading files, calling `rank`, rendering blocks. It returns `Vec<String>` or `crate::Failure`, and never prints.
- `src/main.rs`: the dispatch arm and USAGE line. "Nothing matches" is `Failure::Refused("no notes match")`, which already exits 1 with the `bilbo: ` prefix.

A separate `rank.rs` instead of putting everything in `recall.rs`: the AGENTS.md rule says verbs never build on each other, and the digest needs the same ranking.

### Reading files

Files are read as bytes and decoded with `String::from_utf8_lossy`, then given to `note::read`, which already strips a byte order mark and CRLF. A file that cannot be read is skipped. `check` is where such problems surface, and one unreadable note should not hide every other hit.

### Arguments

Still by hand: two options, a `--` terminator and free words. The `--title` help-scan rule in `main.rs` does not apply to `recall`. `bilbo recall --help` prints help like every verb, and `bilbo recall -- --help` searches for the word `help`. `--kind=<kind>` and `--limit=<n>` are the same as the two-argument forms, as `new` takes `--title=<text>`. An argument is an option when it starts with `-` followed by a character other than whitespace, so `-` and `- ?` are query text.

### Snippets

The snippet turns every run of whitespace into one space and trims both ends, then keeps the first 300 characters, not bytes. Without the trim almost every snippet would start with a space, from the blank line under its heading.

An empty snippet (a match through the title or a heading with no text under it) prints as `-`, like an invalid `created`, so no block contains a blank line and one blank line between blocks stays unambiguous.

### Absolute paths in hits

`check` prints paths relative to the root because they label problems. `recall` prints absolute paths because an agent's next step is to open the file, and an absolute path needs no root resolution. The root is already absolute (`BILBO_HOME` must be, and `HOME` and `XDG_DATA_HOME` are taken only when absolute).

### Plugin

```
.claude-plugin/marketplace.json   Claude Code marketplace: one entry, source ./plugins/bilbo
.agents/plugins/marketplace.json  Codex marketplace: one entry, local source ./plugins/bilbo
plugins/bilbo/
  .claude-plugin/plugin.json      name, description, author, repository, license; no version
  .codex-plugin/plugin.json       the same, plus version, skills and interface
  skills/recall/SKILL.md          frontmatter: name, description, license, allowed-tools
```

- Why a plugin, not copies in `~/.claude/skills` and `~/.agents/skills`: both tools name plugins as the way to distribute skills. A plugin installs and updates with one command, and the later hooks and skills join the same unit. The cost is about 20 lines of manifest per tool.
- Why `plugins/bilbo/`, not the repo root: the Rust root stays clean, and a Codex plugin at a repo root is unverified. steveyegge/beads uses the same layout.
- Versions: the Claude manifest and its marketplace entry set no `version`, so Claude Code follows commits; a set version pins users to the cached copy until it changes. `claude plugin validate` warns about it, and that warning is accepted. Codex requires a semver `version` and caches by it, so it equals `Cargo.toml`'s `version`, and `tests/plugin.rs` checks that in CI.
- The skill runs `bilbo` from PATH. Neither plugin system installs binaries; when `bilbo` is missing the skill says `install the bilbo CLI first` and stops.
- The skill sends the user's words verbatim first, and after a miss retries at most twice in the note's likely wording, as the first risk below asks, since there is no stemming. It always says which query produced the hits.
- Checks: CI runs `tests/plugin.rs`. Locally, `claude plugin validate`, the validator bundled with Codex's `plugin-creator` skill, and installs into a throwaway `CLAUDE_CONFIG_DIR` and `CODEX_HOME`.

## Risks / Trade-offs

- [No stemming or synonyms: `note` misses `notes`, and a Portuguese query misses an English note] → Expected, and measured: 23 of 37 against 29 of 37 for the legacy hybrid search (`work/recall-arms/arms.py`, `arms.json`). add-embeddings is the fix. The `recall` skill retries up to twice in the note's likely wording when nothing matches.
- [The scan grows linearly with the store] → The goal is checked with a generated 6 MiB store in an ignored test. add-embeddings brings the index.
- [The accent table misses letters outside Latin-1 and Latin Extended-A] → They pass through unfolded, so an exact-accent query still matches. The table can grow without a spec change.
- [`title_problem` in `note.rs` counts `# `, `# #` and `# ###` as a title, while rank's heading rule treats them as text, so `check` passes such a note while `recall` falls back to the filename stem] → Accepted for now.
- [Common words like `the` or `de` add noise] → BM25's IDF makes them nearly worthless, and a passage needs to rank, not just match. A stopword list is not worth its language-specific maintenance.

## Migration Plan

None in this repo. The legacy `recall` skill in dnix keeps using `nbrecall` until the user installs this plugin and moves the notes into the store; that switch happens in dnix.
