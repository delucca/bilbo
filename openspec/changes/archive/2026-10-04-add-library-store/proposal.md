# Proposal

## Why

bilbo's vocabulary has sources and a library, but the store holds only notes. The library lives today in eight notebook `library/` folders, run by five Python scripts and nbrecall. Its citations name absolute paths, its split rule follows one tool's Read cap, and its "verbatim" text rests on a model retyping pages through Write (`design-bilbo-library.md` in the planning notebook, "The library today"). This change gives sources a home in the store, a file format that makes edits visible, and a way to land text that the agent never types. The reading, citation, fetch and search changes build on it.

## What Changes

- A library in the store: `<root>/library/<corpus>/<name>.md`, one file per source whatever its size, and one authored `guide.md` per corpus. There are no subfolders and no split files.
- A source file format. The frontmatter holds `id`, `fetched`, `origin`, `digest`, and optionally `kept` and `capture`. The body opens with one `# ` title. `digest` is the SHA-256 of the body, so any later edit shows up in `bilbo check`.
- A guide format. The frontmatter holds `id` and `created`. Then a `# ` title, a lead, and one `## <name>` entry per source with authored prose. Sizes, token counts, dates and the catalog mark are derived when printed, never written.
- An outline rule shared by every library verb: a source's sections are its headings below the title, outside code fences, as `recall` reads them. Sections are addressed by heading-path anchors (`needless_return > What it does`). A source over 55,000 bytes with more than 40 sections at or above its cut level (the shallowest heading level that holds at least two sections) is a catalog.
- A new verb, `bilbo library`:
  - `bilbo library` lists the corpora with source counts and sizes.
  - `bilbo library <corpus>` prints the guide with a facts line under each entry (file, id, size, tokens, fetched, headings, catalog).
  - `bilbo library show <ref>` prints a source's header and its outline (heading path, line range, tokens). `<ref>` is `<corpus>/<name>` or an id, optionally with `#<anchor>`.
  - `bilbo library stage <file> --origin <item> [--fetched <date>]` copies text another tool produced into a staging folder and prints its line count, a suggested keep range and its top headings.
  - `bilbo library land <stage> <corpus>/<name> --keep <ranges> [--title <text>] [--replace]` builds the body from the kept lines, mints the id, writes the digest, keeps the staged text as a local capture, and adds or marks the guide entry.
- `bilbo check` also checks `<root>/library/`: names, frontmatter, digests, titles, guides, entries against sources, `TODO` stubs, stale lines, and ids shared across notes and library. A store with a library and no `notes/` folder is a store.
- `sha2` 0.11.0 as a new dependency, kept in one module.

## Capabilities

### New Capabilities

- `library-store`: the layout, corpus and source names, the source file (frontmatter, title, digest), the guide (frontmatter, entries, stub and stale lines), ids unique across notes and library, the outline, derived sizes, catalogs, anchors, and captures.
- `library-browse`: `bilbo library`, `bilbo library <corpus>` with its facts lines, `bilbo library show`, source references, and their errors.
- `library-ingest`: `bilbo library stage <file>` and `bilbo library land`, with `--replace`, the body it builds, the guide entry it writes, the capture it keeps, and its refusals.

### Modified Capabilities

- `cli`: Verb dispatch adds `library`.
- `config`: Config location adds `library` to the verbs that run whatever the config holds.
- `store-check`:
  - Report problems: check covers `<root>/library/` against `library-store`.
  - Report every problem in one run: a shared id between a note and a source.
  - A missing store is a problem: only when neither `notes/` nor `library/` exists.

## Non-goals

- Fetching a URL, converting HTML, gate warnings and the `ingest` skill (`add-library-fetch`). Here `stage` takes only a file, and every source it lands is labelled `capture: external`.
- `library plan`, `library read`, the `cite` verb, the citation pre-check on `land --replace` and the `reference` skill (`add-library-reading`).
- Searching the library with `recall` (`add-library-recall`). Plain `recall`, `index` and `digest` never read `<root>/library/`, and nothing embeds library text.
- Scope, history, watch and sync for the library (`add-library-sync`, after the sync series). Nothing here depends on those changes.
- Any plugin change. No skill, hook or subagent is added.
- A `--json` output for the `library` verb.
- Moving a source between corpora or deleting one through a verb. Both are a `mv` or `rm` and a guide edit, which `bilbo check` then verifies.
- Rewriting the 3 note citations and 2 `doc:` sources that point into notebook libraries, switching the dnix skills, and dropping nbrecall's library globs. They belong to the cutover, which `add-library-reading` names.
- Migrating the eight notebook libraries. A one-off Rust tool outside this repository, in the planning notebook's `work/library-migrate/`, writes them in this change's format (old ids kept, `capture: legacy`, split sources rejoined), and the cutover runs it. It is not a product verb, and nothing in this change runs it.

## Impact

- New verb `src/library.rs`, with its `mod` line, dispatch arm and USAGE lines in `src/main.rs`. `wants_help` also skips the value of `library land --title`.
- New library modules: `src/source.rs` (source frontmatter, digest, outline, sizes, catalog, anchors), `src/corpus.rs` (guide parsing and updates, the corpus listing, the library check problems) and `src/hash.rs` (SHA-256, the one user of `sha2`). `openspec/config.yaml`'s Stack line names `sha2`.
- `src/store.rs` gains the library, captures and staging paths. `src/rank.rs` makes its heading parser public to the crate. `src/check.rs` adds the library scan and the cross-folder id rule.
- `Cargo.toml` and `Cargo.lock`: `sha2` 0.11.0, which brings `digest`, `block-buffer`, `crypto-common`, `hybrid-array`, `typenum`, `const-oid`, `cpufeatures` and `cfg-if`.
- Tests: `tests/library.rs` (new), `tests/check.rs` extended, unit tests in the new modules.
- `README.md` gains a Library section. `AGENTS.md` names the new modules and the `sha2` rule.
- On disk: `<root>/library/`, `<root>/library/.lock`, `<root>/.bilbo/captures/`, and `<state>/bilbo/staging/`.
- Cross-series dependency: `add-note-history` uses `sha2` too. Its design now has `versions.rs` call `hash::sha256_hex`, because `src/hash.rs` is the one home of `sha2` (design.md).
