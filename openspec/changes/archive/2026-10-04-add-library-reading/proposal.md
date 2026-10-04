# Proposal

## Why

After `add-library-store`, bilbo holds sources but gives an agent no way to show that an answer rests on them. Today's notebook library leans on trust at three points: the reader splits its own reads under one tool's Read cap, it reports its own coverage, and a script checks citations that name absolute paths (`design-bilbo-library.md` in the planning notebook, "The library today"). This change moves all three into bilbo: reads go through a plan and a log, citations name ids, and coverage is counted from the log. It also ships the `reference` skill that drives them, so dnix's `review`, `rubber-duck` and `reference` can stop depending on the notebook layout.

## What Changes

- `bilbo library plan <ref>... [--budget-tokens <n>] [--slice-bytes <n>] [--slice-lines <n>]`: cuts each pick (a whole source, or a section by anchor) into slices at section starts, sized for one shell call (24,000 printed bytes by default), and groups them into partitions sized for one reader (60,000 tokens by default). A catalog is refused whole. It prints the plan id, the partitions and one row per slice, and writes the plan to `<state>/bilbo/plans/`.
- `bilbo library read <plan> <slice>... [--part <k>/<n>]`: prints each slice as `<line>\t<text>` lines between a header and an end marker, and logs the lines it printed. One call never prints more than one slice's size, so a log entry is one tool result. A missing end marker or a gap in the line numbers tells the agent its tool cut the output; `--part` reads the slice in smaller runs.
- A new verb, `bilbo cite [--plan <plan>]... [<file> | -]`: finds every `bilbo:<id>[#<anchor>] "<quote>"` in a draft, resolves the id across notes and sources, and gives each citation a verdict: `ok`, `quote_elsewhere`, `ambiguous`, `too_short`, `quote_missing`, `anchor_missing`, `id_missing`, and with `--plan`, `unread`. The normalization is `check_citations.py`'s, plus six HTML entities, and `library show` and `library plan` now match anchors by the same rule. With `--plan` it prints a `coverage:` line and a `picked:` line counted from the plans and their read logs. It exits 1 on a failing verdict.
- `bilbo library land --replace` checks the citations in `<root>/notes/` that name the source, against the old and the new body, before it writes. When any would degrade, it lists them and exits 1; `--force` replaces anyway.
- A `reference` skill in the plugin, with `references/reader.md`: list corpora, read the guide, post the picks, plan, read (itself, or one `general-purpose` reader per partition in Claude Code, at most six; in its own context up to 100,000 tokens without the Agent tool), cite, judge whether each quote supports its claim, and end with cite's coverage lines. The plugin still ships no subagent.
- `unicode-normalization` 0.1.25 as a new dependency, for NFKC, kept in one module.

## Capabilities

### New Capabilities

- `library-reading`: `bilbo library plan` and `bilbo library read`: picks, slices, partitions, the plan file, read output, parts, the read log and the refusals.
- `citations`: the citation form, id and anchor resolution, the normalization, the verdicts, `bilbo cite` and its output, `unread`, and the coverage lines.

### Modified Capabilities

- `library-ingest` (from `add-library-store`): Replace a source runs the citation pre-check and takes `--force`; new requirements Citation pre-check and Degraded citations block a replace.
- `library-store` (from `add-library-store`): Anchors compares after a new Text normalization requirement, for every verb, the rule `cite` uses for quotes too.
- `library-browse` (from `add-library-store`): Corpus argument errors no longer calls `plan` and `read` reserved words; they are subcommands now.
- `cli`: Verb dispatch adds `cite`.
- `config`: Config location adds `cite` to the verbs that run whatever the config holds.
- `agent-plugin`:
  - One plugin for Claude Code and Codex: the skills include `reference`.
  - A missing binary: the reference skill's line.
  - New requirements: The reference skill, Reference picks, Reference reads, Reference answer, Support judgment and The reader brief.

## Non-goals

- `ok_earlier` and any check against older source versions. It needs source history (`add-library-sync`).
- Keyword lookup in a catalog or across sources. `add-library-recall` adds `recall --library` and then extends the `reference` skill's catalog step. Here a catalog is reached by anchor from `bilbo library show`.
- Fetching and the `ingest` skill (`add-library-fetch`). The pre-check lands here because it guards `land`, which exists now.
- `bilbo check` verifying citations. It stays a format check; `cite` and the pre-check cover citations.
- Checking citations in guides or outside the store, such as review evidence under a notebook's `work/`.
- A `--json` output for `plan`, `read` or `cite`.
- Shipping a `source-reader` subagent. Readers are `general-purpose` agents given `references/reader.md`.
- The cutover itself: running the one-off migration tool in the planning notebook's `work/library-migrate/` against the real notebooks, rewriting the 3 note citations and 2 `doc:` sources, and the dnix edits below. They happen outside this repository, after this change is archived.

## Impact

- New verb `src/cite.rs`, with its `mod` line, dispatch arm and USAGE line in `src/main.rs`, and `tests/cite.rs`.
- New library modules: `src/text.rs` (the Text normalization; the one user of `unicode-normalization`), `src/citation.rs` (parsing, resolution, verdicts) and `src/plan.rs` (slicing, partitions, the plan file, the read log, coverage). `cite` and `library` both build on them.
- `src/library.rs` gains `plan`, `read`, and the pre-check and `--force` in `land`. `src/source.rs` from `add-library-store` provides the outline and anchors. `tests/library.rs` gains the plan, read and pre-check scenarios.
- `Cargo.toml` and `Cargo.lock`: `unicode-normalization` 0.1.25 and `tinyvec` 1.13.3.
- `plugins/bilbo/skills/reference/SKILL.md` and `references/reader.md` (new); the two manifests and the Claude Code marketplace entry get a description that names the library; `tests/plugin.rs`.
- `README.md`, `AGENTS.md`, and `openspec/config.yaml`'s Stack line.
- On disk: `<state>/bilbo/plans/<plan>.json` and `<plan>.log`.
- **The cutover** (after archive, outside this repo, the user's run):
  1. Run the migration tool in the planning notebook's `work/library-migrate/` into the real `BILBO_HOME` and check it with `bilbo check`.
  2. Rewrite the 3 citations in `estate__shared/notes/gotcha-claude-code-skill-frontmatter.md` to `bilbo:<id>#<anchor> "<quote>"` from `map.tsv`, and the 2 `doc:` sources that point into a notebook library to `"doc: bilbo:<id>"`. `bilbo cite` on the note gives 3 `ok`.
  3. The dnix edits below, in one commit, then `just switch` on each host.
- **dnix edits** (`modules/ai/`, `modules/notebooks/`):
  - `skills/review/SKILL.md` Step 4: corpora from `bilbo library`, guides from `bilbo library <corpus>`, the source count from its row, one `bilbo library plan` per concern instead of `plan_reads.py`, partitions from its output. Terms and trust labels: `bilbo:<id>#<anchor> "<quote>"` instead of `note:`.
  - `skills/review/references/lens-briefs.md`: variant A gives a conformance lens the plan id and its slice range, and pastes reader.md's "Reading rules" and "Citation rules"; variant B uses `bilbo library show` and anchor plans until `add-library-recall` brings keyword lookup. The refuter runs `bilbo cite --plan <plan>... <candidates file>` and pastes reader.md's "Support judgment": a failed check or an unsupported quote downgrades the grounding to `judgment`, and `unread` is a failed check. The brief comes from the installed package, `share/bilbo/plugins/bilbo/skills/reference/references/reader.md`.
  - `skills/rubber-duck/SKILL.md`: the `ls ~/Notebooks/*--<language>__*/library/index.md` probe becomes a `bilbo library` row, and it keeps the reference skill's `bilbo:` citations and coverage lines verbatim.
  - `agents/developer.md:94`: a standards question goes through `bilbo library`, `bilbo library <corpus>`, `library plan`, `library read` and `bilbo cite`, not `nbrecall --kind index` and Read.
  - Remove `skills/reference/` (its scripts and their tests in `tests/default.nix`) and `agents/source-reader.md`; the plugin's `bilbo:reference` replaces them.
  - `notebooks/AGENTS.md`: the library section and the citation lines point at bilbo and the `bilbo:` form.
  - `skills/recall/SKILL.md` (`--include-library`) and nbrecall's library globs move with `add-library-recall`; `skills/ingest/` with `add-library-fetch`.
