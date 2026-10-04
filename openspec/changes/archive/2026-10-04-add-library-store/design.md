# Design

## Context

See proposal.md for why. What exists today and constrains the approach:

- **The store.** `store::root` resolves the root, `store::state_dir` the state folder, and `store::entries` lists a folder sorted by name, skipping names that start with `.` (`src/store.rs`). Only `<root>/notes/` exists as a store folder.
- **Note parsing.** `src/note.rs` holds the pieces a library needs: `is_topic`, `is_ulid`, `mint_ulid`, `is_created`, `now_created`, `lines`, `fence_run`, and the `Problem` type whose `Display` appends ` (line N)`. `note::read` is strict about note keys, so it cannot read a source or a guide as it is.
- **Headings.** `rank::passages` (`src/rank.rs`) finds headings outside fences with a private `heading` function: one to six `#` at column 0, a space, and text with its closing `#` run removed and whitespace collapsed. The `note-recall` spec's Passages requirement fixes that behavior. The outline reuses it, so a section and a recall passage start on the same lines.
- **Check.** `check::run` (`src/check.rs`) scans `notes/`, collects `(path, message)` pairs, reports shared ids and topics on every file involved with the `shared` helper, sorts, and exits 1 on any line. It refuses with `no store at <root>` when `notes/` is missing.
- **Writes.** `bilbo new` writes a temp file, fsyncs it and hard-links it into place, so a name race has one winner (`src/new.rs`, `write_new`). `src/swap.rs` exists only in the unshipped `add-note-history` draft.
- **Dependencies.** `Cargo.toml` has no hash crate except `ring`, which AGENTS.md keeps in `src/model.rs`. The `add-note-history` draft also uses `sha2` 0.11.0, through `src/hash.rs` (its design.md, "`sha2` 0.11.0, through `src/hash.rs`").
- **Rust.** The dev shell runs Rust 1.95.0, so `std::fs::File::lock` (stable since 1.89) is available. A scratch crate locked a file with it on 2026-10-03.
- **The data to migrate.** 8 notebook libraries, 441 sources, 1,172 files (`libaudit.json` in the planning notebook). A scan on 2026-10-03 found: every file has frontmatter and one `sources` item of type `url`; every body opens with its `# ` title; every body has exactly one level-1 heading outside fences (the audit's "multi_h1" file counted fences naively); 5 files hold CR characters; 5 bodies end inside an open fence, and only one of them is a chapter (the last chapter of its source); no name breaks the topic grammar and none is `guide`; no corpus name is reserved; 26 cognition bodies open with a provenance header in one of two shapes.

## Goals / Non-Goals

**Goals:**
- One file per source, one authored guide per corpus, and nothing derived written to disk.
- A body the agent never types: the CLI builds it from a staged capture and line ranges, and `bilbo check` sees any later edit.
- Formats, outline rules and output shapes exact enough that `add-library-reading`, `add-library-fetch` and `add-library-recall` can build on them without reopening this change.
- The eight notebook libraries convertible into this format by a script, with `bilbo check` clean afterwards.

**Non-Goals:**
- Proving that `capture: external` text is what its origin served. The label says that it rests on trust.
- Stopping a determined forger: an agent could write a file and hash it by hand. The digest catches accidental and careless edits.
- Cleaning up abandoned stage folders. They sit in the state folder, outside the store, and are small.

## Decisions

### Three capabilities

`library-store` holds the formats and the rules every verb shares, as `note-store` does for notes. `library-browse` holds the read-only forms of the verb, and `library-ingest` holds `stage` and `land`. The later changes then modify narrow requirements: `add-library-fetch` changes Stage a file and Stage output, `add-library-reading` changes Replace a source and adds its own capability for `plan` and `read`.

Alternative: one `library` capability for the whole verb. It would grow to the plan, read and fetch requirements of three more changes, and every delta would touch one large spec.

### Modules

- `src/library.rs`, the verb: argument parsing and the five forms. It builds on the library modules below and on `store`, never on another verb.
- `src/source.rs`, a new library module: the source frontmatter (parse, validate, render), the digest, the outline, sizes, the catalog rule and anchor resolution. `add-library-reading` reuses the outline and anchors for `plan` and `cite`, and `add-library-recall` the outline for passages.
- `src/corpus.rs`, a new library module: guide parsing, entry insertion and the stale line, the corpus listing, and the library problems `check` reports.
- `src/hash.rs`, a new library module: `sha256_hex(&[u8]) -> String`, the one user of `sha2`.
- Extended: `src/store.rs` gains `library_dir(root)`, `captures_dir(root)` and `staging_dir(env)`. `src/rank.rs` makes `heading` `pub(crate)`. `src/note.rs` is reused through its public helpers, with no change. `src/check.rs` calls `corpus::problems` and runs `shared` over note and library ids together.

Why new modules instead of extending `note.rs`: a note and a source share the frontmatter delimiters and the ULID rule but no key, and `note.rs` is already 855 lines. Sharing goes through the existing public helpers.

### `sha2` 0.11.0, in `src/hash.rs` only

- **Why a crate:** std has no SHA-256. `ring` has one, but AGENTS.md keeps `ring` in `src/model.rs`, and `add-note-history` already chose `sha2` for the same reason.
- **The version:** 0.11.0, the latest stable release (2026-03-25 on crates.io), MIT or Apache-2.0, `rust-version` 1.85. It is the version `add-note-history` pins, so both changes resolve to one copy.
- **What it adds** (`cargo tree` on 2026-10-03, in a scratch crate): `digest` 0.11.3, `block-buffer`, `crypto-common`, `hybrid-array`, `typenum`, `const-oid`, `cpufeatures` (which uses the `libc` 0.2.190 already in the tree) and `cfg-if`.
- **Why `src/hash.rs` and not `src/source.rs`:** the history change needs the same function. A module of its own lets both call it while the dependency keeps one user, as AGENTS.md asks.
- **`src/hash.rs` is the home of `sha2`, whatever the archive order.** This is a cross-series dependency: `add-note-history`'s design has `versions.rs` call `hash::sha256_hex` and never use `sha2` directly. If `add-note-history` ships first, its implementation creates `src/hash.rs` with that one function. AGENTS.md's dependency rule says "`sha2` in `src/hash.rs`", and `openspec/config.yaml`'s Stack line names `sha2`.

### The source file as written

`land` writes the keys in the contract's order, `id`, `fetched`, `origin`, `digest`, `kept`, `capture`, with the closing `---` directly followed by `# <title>`, the shape today's notebook sources already have. `check` accepts any key order, since only `land` writes the file.

- The digest covers every byte after the newline that ends the closing `---` line, so a changed title, a trailing newline or a CR all change it.
- Line endings are normalized once, when `stage` writes `capture.md`. Every line number `stage` prints and `--keep` takes then counts the same lines, and `land` copies lines without touching their ends.
- The demotion rule adds one `#` in front of every heading line outside fences in the kept lines. A `######` line becomes seven `#`, which is plain text. The rule stays one deterministic step that any reader can undo.

### The guide

Every `##` heading in a guide is an entry, and its text is the source's file name. A name is the join key: titles change on a re-ingest, names do not. An author who wants other headings uses `###` inside an entry. The lead is everything between the title and the first entry.

`land` writes two kinds of marker lines, and `check` fails on both until an agent removes them:

| Line | Written by `land` when |
|---|---|
| `TODO: describe this corpus.` | it creates a guide |
| `TODO: describe this source.` | it adds an entry |
| `stale: re-ingested <YYYY-MM-DD>; re-read the source and revise this entry.` | `--replace` changes the digest, or a new source meets an entry that already exists |

The stale line goes directly under the heading, so `bilbo library <corpus>` prints it right after the facts line. A second re-ingest replaces it instead of stacking a second one. The prose stays: a reviewed paragraph is worth more than a reset to `TODO` (the panel's evaluation, "Corpus index").

### Output shapes

- **`bilbo library`:** one tab-separated row per corpus. The panel asked for scope too, which waits for `add-library-sync`.
- **`bilbo library <corpus>`:** the guide's absolute path first, so an agent can edit it. Then the guide as written, frontmatter left out, with a facts line under each entry. The facts line keeps the shape of today's script line (`` `<file>` · <KB> · ~<tokens> tokens · fetched <date> · <n> headings``), which agents already read, adds the id that citations need, and drops the sha comment.
- **`library show`, `stage` and `land`:** `<key>: <value>` header lines, then tab-separated rows where there are rows. A header line names itself, so a later change can add one without breaking a reader.
- Line numbers in `show` are the file's physical lines, the numbers Read and `recall` show. `add-library-reading`'s `library read` prints the same numbers.

Rejected: `--json` for every form. Agents read the text forms, and nothing parses the output yet. A later change can add it.

### Staging

A stage is `<state>/bilbo/staging/<stage>/`, holding `capture.md` and `stage.json`. `stage.json` holds the origin, the fetched date, the capture label (`external` for a file) and the SHA-256 of `capture.md`. It is bilbo's bookkeeping and no part of any spec.

- `land` refuses a stage whose `capture.md` no longer hashes to the recorded value. Without that, an agent could Edit the staged text and then keep it, which brings back typed text by another door.
- `land` copies every file of the stage folder except `stage.json` into the capture folder. `add-library-fetch` adds `raw` and `fetch.json` to the stage, and they reach the capture with no change to `land`.
- `stage` prints only level-1 and level-2 headings. A full outline of a lint catalog is hundreds of rows, more than a Bash call shows. The agent reads `capture.md` itself to choose its ranges.
- The suggested `keep` starts after the title line, because `land` writes the title, or at the first non-blank line when there is no title, and ends at the last non-blank line. It is a starting point: the agent still cuts navigation and footers.

### Captures and the `landed` file

A capture folder is named by the SHA-256 of its `capture.md`, so the same text staged twice is kept once. The contract gives a source no key that names its capture, and `kept` is meaningless without one. `land` therefore appends `<id>\t<digest>\t<date>` to the capture folder's `landed` file, and `library show` prints `capture folder:` when one records the source's current digest. `show` scans the `landed` files, which is one small read per capture.

Alternatives:
- A `captured: sha256:<hex>` key in the source. It is the simplest link, but the contract forbids other keys, and a key that points at local-only data would sync to devices where it resolves to nothing.
- No link at all. The evidence would then exist without a way to find it.

### Land: order of writes and concurrency

`land` takes an exclusive `File::lock` on `<root>/library/.lock`, a hidden file every listing and check ignores, and holds it for the whole run. Under the lock:

1. Check the target: a new name must not exist, and a replaced one must exist and have a valid `id`.
2. Write the capture folder if it is missing: a hidden temp folder `<root>/.bilbo/captures/.tmp-<random>`, which every listing ignores, renamed into place. Append the `landed` line.
3. Write the source: a temp file `.land-<id>.tmp` in the corpus folder, fsynced. A new source is hard-linked into place, as `bilbo new` does, so a name taken in the meantime fails with exit 1. A replaced source is renamed over the old file.
4. Write the guide the same way, through a temp file and a rename.
5. Remove the stage folder.

The lock makes two `land` runs take turns, so neither loses the other's guide entry. It does not stop an agent's Edit of the guide between steps; that window is milliseconds long.

If step 4 fails, the source exists with no entry. `land` exits 1 naming what it wrote, and `check` reports the missing entry. Re-running needs `--replace`.

Rejected: `src/swap.rs` from `add-note-history`. A source has one writer, so a rename is enough until history lands.

### The catalog rule counts at the cut level

The contract words the catalog test as "the most common heading level below the H1 holds more than 40 sections". Measured on 2026-10-03 against the notebook libraries, with the 54 split sources rejoined as the migration does, that wording marks 25 sources as catalogs: the 4 that are catalogs today and 21 rejoined books, among them `effective-go` (102 KB, 43 level-3 sections) and the Go spec. `add-library-reading`'s `plan` would then refuse those books whole.

The rule here is `split_source.py`'s: the cut level is the shallowest level from 2 to 6 at which at least two sections sit at that level or above, and a source over 55,000 bytes with more than 40 sections there is a catalog. On the same data it marks exactly today's 4 catalogs (`clippy-lints`, `revive-rules-descriptions`, `clj-kondo-linters`, `tutorial-on-good-lisp-programming-style`) and no rejoined book. A book's chapters sit at its cut level, and a reference list's entries do.

The old 2,000-line cap is dropped on purpose. `split_source.py` applied the catalog test to a source over 2,000 lines or 55,000 bytes, and the line cap was a Read tool constant (the design note's problem 4), not a property of the text. A 2,100-line, 50 KB list with 50 sections was a catalog and is not one now: at that size `plan` slices it like any other source.

### Anchors compare with case

An anchor's parts compare with whitespace collapsed and case kept. Headings like `Option` and `option` both occur in API docs, and a case-folded match would make more anchors ambiguous. Agents copy anchors from `library show`, so they get the case right.

### The library check problems

Paths are relative to the root. A `(line N)` suffix follows the existing `Problem` display.

| Where | Message |
|---|---|
| a file in `library/` | `entry: library/ holds only corpus folders` |
| a corpus with a bad name | `corpus: invalid name '<c>': use segments of a-z and 0-9 joined by single hyphens` |
| a reserved corpus | `corpus: '<c>' is reserved for a library subcommand` |
| a folder in a corpus | `folder: a corpus holds only guide.md and sources` |
| a bad file name in a corpus | `name: must be <name>.md, with segments of a-z and 0-9 joined by single hyphens` |
| no guide | `library/<c>/guide.md: guide: missing; every corpus needs one` |
| source frontmatter | the `note-store` messages for delimiters, unknown keys, repeats and `id`, plus `<key>: missing` for each required key |
| `fetched` | `fetched: '<v>' is not YYYY-MM-DD, a real date` |
| `origin` | `origin: write it as origin: "<url or doc>: <value>"` |
| `digest` form | `digest: '<v>' is not sha256: and 64 lowercase hex digits` |
| `digest` mismatch | `digest: does not match the body; only bilbo library land writes a source` |
| `kept` | `kept: '<v>' is not ascending, non-overlapping <a>-<b> ranges from 1` |
| `capture` | `capture: '<v>' is not external or legacy` |
| title not first | `title: the body must open with a '# <title>' line` |
| two titles | `title: found <n> '# ' headings outside code fences, expected one` |
| a source with no entry | `guide: no '## <name>' entry in guide.md` |
| guide frontmatter and title | the `note-store` messages, with `sources` reported as an unknown key |
| an entry with no source | `entry '<name>': no source <name>.md in this corpus` |
| a repeated entry | `entry '<name>': given more than once` |
| a stub entry | `entry '<name>': TODO stub; write the entry and remove the TODO line` |
| a stale entry | `entry '<name>': stale; re-read the source, revise the entry and remove the stale line` |
| a stub or stale line in the lead | `lead: TODO stub; describe the corpus and remove the TODO line`, or `lead: stale line; remove it` |
| a shared id | `id: <id> is also the id of <paths>`, the existing message, now across both folders |

The duplicate-origin rule is a `land` warning, not a check problem. Two sources cut from one page can be deliberate.

### A store is `notes/` or `library/`

`check` refuses only when both folders are missing. A user whose first act is `library land` gets a store `check` accepts, and the migration does not need to create an empty `notes/`. `recall`, `index` and `digest` keep their own `no store` rule on `notes/`, since they never read the library.

### The migration is outside this change

The eight notebook libraries move into this format through a one-off Rust tool in the planning notebook's `work/library-migrate/`, never through a product verb. It writes the files itself instead of calling `stage` and `land`, because `land` mints new ids and labels everything `external`, while migration must keep every old id and say `legacy`. A `land --id` flag for one run would stay in the product forever. Its own `DESIGN.md` holds the steps: corpora named by notebook topic, the 54 split sources rejoined with every chapter at `##` under their folder ids, `capture: legacy`, the entry prose copied, the 26 cognition provenance headers moved word for word into their guide entries, and an old-path to id map. The cutover runs it.

Rejected:
- A product import verb (`library import`). The notebook layout is this user's, not the product's.
- A script in this change folder. Nothing in the repository runs it after the cutover, and bilbo's tooling is Rust.

## Risks / Trade-offs

- [An agent edits `guide.md` while `land` rewrites it] → The rename replaces the file whole, so the agent's Edit fails or applies to the new text. Claude Code's Edit fails when the file changed since its Read.
- [A forged source passes check] → Out of scope (Goals). The `capture` label and the capture folder make provenance visible, and `library show` names the capture that produced a body.
- [Abandoned stage folders pile up] → They live under `<state>`, outside the store and outside sync, and hold text only.
- [A big `library show` overflows a Bash call] → `--depth` and `#<anchor>` narrow the outline. `add-library-recall` adds keyword lookup for catalogs.
- [The migration flattens chapter levels] → Every chapter becomes `##`, a shape the old files never stored. The map records each chapter's new anchor, and the 3 note citations into a chapter cite headings inside it, which keep their text.
- [A `land` killed while writing a capture leaves `<root>/.bilbo/captures/.tmp-<random>`] → It is hidden, so no listing or check sees it, and no sync sweep touches `captures/`. It stays until a human removes it. Accepted: it holds only staged text.
- [Guides with long cognition entries] → 26 entries gain about 100 words each, in a corpus of 26 sources. The guide stays well under one read.
- [`add-note-history` and this change both add `sha2`] → One version and one module, and the merge rule above.

## Migration Plan

- The product: the library is new, so nothing on an existing store changes. A store with notes and no library checks as before.
- The user's library: the cutover runs the one-off tool in the planning notebook's `work/library-migrate/`, first into a scratch `BILBO_HOME`, then into the real one. That run, the rewrite of the 3 note citations and 2 `doc:` sources, and the dnix switch happen at the cutover that `add-library-reading` names. Until then, the notebook libraries and their skills keep working.
- Rollback: remove `<root>/library/` and `<root>/.bilbo/captures/`. No other file changes.

## Decisions to confirm

1. Three capabilities (`library-store`, `library-browse`, `library-ingest`) instead of one.
2. `sha2` lives in a new `src/hash.rs`, and `add-note-history`'s `versions.rs` calls `hash::sha256_hex` instead of using `sha2` itself.
3. A `landed` file in each capture folder links sources to captures, since the source has no key for it.
4. Every `##` heading in a guide is an entry; free headings go one level down.
5. A store exists when `notes/` or `library/` exists.
6. `land` takes a lock on `<root>/library/.lock`.
7. `land` refuses a stage whose `capture.md` was edited.
8. The duplicate-origin rule is a `land` warning, not a check problem.
9. No `--json` in this change.
10. `stage` lists only level-1 and level-2 headings, and suggests a keep range and a title.
11. The catalog test counts sections at the cut level, not at the most common level, which differs from the contract's wording.
12. Migration: a one-off Rust tool in the planning notebook, run at the cutover, not a script in this change; files written by the tool, not through `land`; chapters rejoined at level 2; the cognition headers moved verbatim into their guide entries; guide `created` set to the old date at `00:00` local time.
