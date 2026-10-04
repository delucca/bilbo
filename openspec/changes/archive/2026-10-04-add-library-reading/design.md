# Design

## Context

See proposal.md for why. What exists, or exists once `add-library-store` is archived, and constrains the approach:

- **Sources and the outline.** `add-library-store` adds `src/source.rs` (frontmatter, digest, `outline`, `tokens`, `is_catalog`, `resolve` for anchors), `src/corpus.rs` (guides, the corpus listing) and `src/library.rs`, the verb with `bilbo library`, `library <corpus>`, `show`, `stage` and `land`. Its `library-store` spec fixes sections, heading paths, anchors (a trailing part of the path, whitespace collapsed, case kept), physical line numbers and derived sizes (2.5 bytes a token). Its `library-browse` spec fixes source references and the `library` verb's option rules. `land` holds `<root>/library/.lock` for its whole run.
- **State.** `store::state_dir` (`src/store.rs`) resolves `$XDG_STATE_HOME`, else `$HOME/.local/state`. `stage` already writes `<state>/bilbo/staging/`.
- **Notes.** `src/note.rs` gives `lines`, `fence_run`, `is_ulid` and `mint_ulid`; `rank::heading` is `pub(crate)` after `add-library-store`. A note's body uses the same heading rules as a source's.
- **Verbs and output.** Verbs return values and `crate::Failure`; only `src/main.rs` prints, prefixing stderr lines with `bilbo: ` (AGENTS.md, Architecture rules). A verb's stdout is its result; diagnostics go to stderr (`cli` spec, Output streams).
- **The scripts to port.** `check_citations.py` (dnix `skills/reference/scripts/`) extracts `note: <path>#<heading> "<quote>"`, normalizes both sides (NFKC, curly quotes straightened, inline Markdown stripped, whitespace collapsed; on the quote side, Read's line prefixes stripped and `...` splitting fragments), searches every section with the cited heading, and gives `ok`, `quote_elsewhere` (with `found_under`, the deepest sections holding the quote), `quote_missing` (with a fuzzy `hint`), `heading_missing`, `path_missing` or `too_short` (under six words). `plan_reads.py` estimates tokens at 2.5 bytes each and packs whole files first-fit into partitions of `--budget-tokens`, an oversize file alone.
- **The skill to port.** dnix's `reference` skill finds corpora through nbrecall or `ls`, reads the index, posts a picks message before any read, plans with `plan_reads.py`, reads with Read in slices under the Read cap or spawns `source-reader` agents (at most six), checks with `check_citations.py` until every verdict is `ok`, and ends with the picks, a `citations:` line and a `coverage:` line it writes itself. `reader-brief.md` holds a common block (reading rules, citation rules, output contract) that `review`'s conformance lenses paste in part.
- **The main consumer.** dnix's `review` Step 4 routes each concern to a corpus, picks entries, runs `plan_reads.py --budget-tokens 60000`, and gives one conformance lens per partition the reader brief's Reading and Citation rules (`lens-briefs.md`, variant A), or falls back to nbrecall hits (variant B). Its refuter runs `check_citations.py` over the candidates file; a failed check downgrades a finding's grounding to `judgment`, and it does not judge whether a quote supports its claim.
- **Output caps.** Claude Code passes a Bash result up to 30,000 characters inline by default (`bashOutputMaxChars`, cited in the design panel's `evaluation.md`). Codex's shell output cap was never measured (same file).
- **Dependencies.** std has no Unicode normalization. `Cargo.lock` lists `icu_normalizer` 2.3.0 because `url`'s `idna` can use it, but `cargo tree -i icu_normalizer -e all --target all` prints nothing on 2026-10-03: bilbo does not build it today.

## Goals / Non-Goals

**Goals:**
- An agent cannot report a read it did not get: bilbo prints the slices, logs what it printed, and counts coverage from the log.
- A slice fits one Claude Code Bash call by default, and an agent can tell when its tool cut one.
- Citations survive moves and renames, resolve for notes and sources alike, and fail loudly when the text moved.
- A re-ingest cannot silently break a note's citation.
- `review` gets the same verbs, so its lenses and its refuter stop depending on notebook paths and Python scripts.

**Non-Goals:**
- Proving that the agent looked at what bilbo printed. The log counts what reached the agent's tool. An agent that ignores its output is out of reach for any CLI.
- Entailment in code. Whether a quote supports a claim stays a judgment, written into the skill.
- Coverage of guides and notes. Notes are short and read with Read; the plan covers sources.

## Decisions

### Modules

- `src/plan.rs`, a new library module: pick ranges, slicing, partitions, the plan file, the read log, pruning, and the read lines and coverage that `cite` needs. It builds on `source::outline`, `source::resolve` and `source::tokens`.
- `src/text.rs`, a new library module: the `library-store` Text normalization, with the physical line of each normalized character for the body side. The one user of `unicode-normalization`. `source::resolve` calls it for anchors, so `library show`, `library plan` and `cite` share one anchor rule.
- `src/citation.rs`, a new library module: the citation grammar, the quote-side steps (line prefixes, fragments), id resolution over `notes/` and `library/`, verdicts, the nearest-passage hint, and the lines a match spans. It builds on `text` and `source`.
- `src/cite.rs`, the new verb: arguments, input, rows, the summary lines, the exit code. It builds on `citation` and `plan`.
- Extended: `src/library.rs` gains `plan` and `read`, and the pre-check and `--force` in `land`, through `plan` and `citation`. `src/main.rs` gains the `cite` arm and the USAGE lines. `src/store.rs` gains `plans_dir(env)`.

Why not one module: `land` needs citations without plans, and `cite` needs plans without slicing; two library modules keep each verb on what it uses, as AGENTS.md asks of verbs ("build on them, never on each other"). Why not in `source.rs`: it is a format module that `check` uses, and it should not grow plan files or citation grammar. Why `text.rs` apart from `citation.rs`: `source::resolve` needs the normalization for anchors, and a format module should not depend on citation grammar.

### `unicode-normalization` 0.1.25, in `src/text.rs` only

- **Why NFKC at all:** the contract keeps `check_citations.py`'s normalization, and agents copy quotes through tools that change code points. NFKC maps `…` to `...`, so an ellipsis character still splits fragments; it maps ligatures (`ﬁ`), fullwidth forms and no-break spaces; and its canonical composition makes `e` plus a combining accent equal `é`. A scratch run on 2026-10-03 printed `"ﬁle … café" -> "file ... café"`, `"cafe\u{301}" -> "café"` and `"Ａｂ" -> "Ab"`.
- **Against std:** std has no normalization.
- **Against a hand table:** a table can map `…`, ligatures and fullwidth ASCII, but canonical composition needs the Unicode decomposition data. A partial fold turns copied quotes into false `quote_missing`.
- **Against `icu_normalizer` 2.3.0:** it is already in `Cargo.lock`, so it adds no new version, but bilbo does not build it today. With `default-features = false, features = ["compiled_data"]` a scratch crate built about twenty crates for it, proc-macro crates (`displaydoc`, `yoke-derive`, `zerofrom-derive`, `zerovec-derive`, `synstructure`, `syn` 3) among them.
- **The pick:** `unicode-normalization` 0.1.25, the latest release (2025-10-30), MIT or Apache-2.0, `rust-version` 1.36, from the `unicode-rs` project. It brings only `tinyvec` 1.13.3 (Zlib, Apache-2.0 or MIT), with no optional feature on. `cargo tree -p unicode-normalization` in the scratch crate printed those two crates and nothing else; `Cargo.lock` gained exactly those two packages.

### The citation grammar, without a regex crate

`citation::parse` scans for `bilbo:` followed by 26 ASCII letters and digits. A lowercase or otherwise non-canonical id is still a citation, and resolves to `id_missing`; text like `bilbo: no store` is not one. An optional `#` and anchor run to the last run of spaces or tabs before the opening quote. A straight quote takes paired inner quotes, as `check_citations.py`'s pattern does; a curly quote runs to `”`. A blank line ends the search, and a closing quote followed by a letter or digit is no close. A `bilbo:<id>` with no quote gets a stderr line instead of a row, and so does `note:` followed by `/`, `~` or `.`, the old form, as `check_citations.py` warned. A hand-written scanner is about as long as the Python pattern and keeps bilbo off a regex dependency.

### Normalization and line mapping

Normalization follows `check_citations.py`'s `normalize` and `fragments` step for step, with one addition: NFKC, then the entities `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#39;` and `&#124;` decoded, the same quote table, links and images to their text, reference links, autolinks, backslash escapes, `*`, `_`, backticks and `~~` dropped, whitespace collapsed. The entities are there because `add-library-fetch`'s table handler writes a `|` inside a cell as `&#124;`, and an agent retypes it as `|`; the alternative, a `\|` escape that the normalization already drops, would put the fix in another change's converter. The body side is normalized line by line, and `text.rs` keeps, for each normalized character, the physical line it came from. A match then knows the lines it spans, which `unread` and `ok`'s `lines <a>-<b>` need. A construct that spans lines (a link split across two) loses its stripping on the body side only; the Python did the same on the joined text, so a quote copied across such a break still matches its words with the brackets dropped on both sides.

### One anchor rule, after normalization

This change modifies `library-store`'s Anchors so every verb compares anchor parts and headings after the Text normalization, a new `library-store` requirement that the citations spec also uses for quotes. Agents copy headings from `library read` output, where `` ## The `Option` type `` keeps its backticks, but a model restating the anchor in prose often drops them, and a `recall --library` hit prints the heading path raw. One rule means an anchor `cite` accepts also works for `library show` and `library plan`. Case still counts, as change 1 decided. Alternatives: change 1's rule as it is, which turns a harmless markup difference into `anchor_missing`; or a looser rule in `cite` only, which gives two anchor grammars.

The title is not a section (`library-store` Outline), so an anchor equal to the title gets `anchor_missing`, with the sections that hold the quote in the detail. `check_citations.py` accepted the title as a heading spanning the file. bilbo's form has a place for "the whole body": no anchor.

### No anchor in a file with sections

A citation without an anchor resolves `ok` only when a match lies in no section: in the title line or the text before the first heading below it. A match under a heading gets `quote_elsewhere`, naming it. The old skill told agents that a file with headings is always cited with one; this makes that rule mechanical, and the detail gives the anchor to add. Alternative: accept any match in the body, as the Python did. A reader would then get no locator in a 900 KB catalog.

### Verdict order

One verdict per citation, decided in this order: `id_missing`; `anchor_missing`; then the quote: with no fragment, `quote_missing`; a match in the anchored section, `ok` (or `ambiguous` when the anchor matched several sections); elsewhere in the body, `quote_elsewhere`; nowhere, `quote_missing`. A resolving quote under six words becomes `too_short`. Last, with `--plan`, a resolving citation of a source whose matches all lie outside read lines becomes `unread`, the only verdict that turns a warning into a failure. `found under` names the deepest sections holding the quote, as `found_under` did. The `quote_missing` hint is the window of the quote's word count, over the cited section or else the body, that shares the most words with the quote, first best, at most 300 characters. It replaces `difflib`, which has no std counterpart, and it only points the agent at where to look.

### Slices count printed bytes, partitions count source tokens

- A slice's size is what `library read` prints for it: two header lines, one `<line>\t` prefix per line, and the end marker. The Claude Code cap counts characters of output, so bytes of output bound it. Counting source bytes instead would let a slice of 2,000 short lines print 14,000 bytes of line numbers on top of 24,000 bytes of text, over the 30,000 cap.
- A slice's tokens follow `library-store`'s Derived sizes over its source lines, so `plan`, `library show` and the guide's facts lines agree on what a pick costs.
- The default 24,000 sits under Claude Code's 30,000-character Bash default with room for multi-byte text counted as characters. `--slice-lines` exists because a shell tool may cap lines as well as bytes; Codex's caps are unmeasured, so the probe in tasks.md sets the values the skill passes when it has no Agent tool. Until then the skill passes `--slice-bytes 10000 --slice-lines 250`, a guess the probe replaces.
- The floor of 1,000 bytes (and 10 lines) rejects values that would cut every line into a slice of its own. The ceiling of 30,000 bytes is Claude Code's inline cap: a larger slice could never arrive in one tool result, and the Codex probe can only lower the value the skill passes.

### The cut rule

Cut points are the start lines of every section inside a pick, at any level, so a slice ends where a section begins. The greedy pass keeps the slicing a single forward walk that any reader can predict from `library show`'s rows. An oversized run closes the slice before it, so a long section starts a fresh slice and keeps its own heading at the top. Within it, blank lines come first because they end paragraphs; a run with none, such as a long code block or a table, is cut at line boundaries. Cutting inside a fenced block is accepted: the reader sees the line numbers and the fence lines, and `in:` gives the section.

### Partitions are runs of consecutive slices

`plan_reads.py` placed whole files first-fit, each into the first partition with room. With slices, first-fit would put a late, small slice into an early partition, so one reader would get the end of a source without its middle. Partitions here are next-fit: a slice joins the current partition or opens the next. A reader then gets `slices 7-12`, one range of text in reading order, and its prompt names one range. The cost is at most one slice of slack per partition, about 9,600 tokens of 60,000 at the defaults.

Alternative: first-fit, as the contract words it. It saves a partition now and then and costs every reader its reading order.

### The read output

```
-- slice 3/9: go/effective-go 01M3EZ8NVEC2KJQNGK5DTK349R lines 820-1104 --
-- in: Concurrency --
820	## Concurrency
...
1104	...
-- end slice 3/9 --
```

- The header gives the reader what a citation needs: the id, and through `in:` the heading path at the first line, since a slice that starts mid-section shows no heading above it.
- The `<line>\t<text>` form is the one `check_citations.py` already strips from quotes, so a quote copied with its number still matches.
- The end marker says the output reached its end. A gap in the line numbers says the middle was cut, which a head-and-tail truncation would otherwise hide behind an intact end marker.
- `--part <k>/<n>` replaces the contract's `--half`. "The first half" leaves the second half without a name, and a half that is still cut needs quarters. `--part 1/2` and `--part 2/2` name both, and `n` up to 8 covers a tool with a cap an eighth of the default.

### The read log counts lines

Each `read` appends one line per printed run to `<state>/bilbo/plans/<plan>.log`: the slice, the source id and digest, the line range, and the time. It opens the file for append and holds `File::lock` on it for the write, so six readers on one plan never lose a line. Coverage is per line: a slice is read when every line of it was printed, by a whole read or by parts, and `unread` asks whether a quote's lines were printed. A slice counter could not credit a quote from the first half of a slice read in parts.

Two rules keep one call's log entry equal to one tool result. `plan` refuses a `--slice-bytes` above 30,000, so no single slice outgrows the inline cap the design targets. `read` refuses, with nothing printed or logged, a call that names several slices whose output together exceeds the plan's slice size, so `read <plan> 1 2 3 4 5 6 7 8 9` cannot print 200 KB of which the tool shows 30 KB while the log credits all of it. Several small slices in one call stay allowed, and a read of one slice always prints, even a slice that is one oversized line.

The plan itself is `<plan>.json`, written once by `plan` and never changed. The contract put the log in the same file; a separate append-only file needs no rewrite under a lock and cannot corrupt the plan.

### The plan file

`<plan>.json` holds the root, the creation time, the options, each pick (the reference as given, the id, the corpus and name at plan time, the line range, the digest) and each slice (its pick, line range, printed bytes, tokens, partition). It is bilbo's bookkeeping and no part of any spec beyond its folder.

- `read` finds a slice's source by id, so a source moved or renamed after the plan still reads, and the header shows its current name. A changed digest refuses the read: the slice's line numbers would point at other text.
- A plan records its root. A plan read against another root is refused, since its ids may resolve to other files.
- `plan` removes plan and log files older than 30 days, judged by modification time. A plan is used within one task; a month leaves room for a long review without letting the folder grow without bound.
- Known limit: a plan is not tied to a session. Any plan in the folder, including one another agent read in full last week, satisfies `cite --plan`. That sits inside the Non-Goals (careless, not adversarial use), and the skill always plans fresh. Tying a plan to a session would need a session id that neither tool hands to a shell command in the same way.

### `cite --plan`, once or more

`review` plans once per concern, so its refuter needs every plan of the run in one check. `--plan` may repeat: a line is read when any of the plans' logs printed it for the source's current digest, and each plan gets its own `coverage:` and `picked:` lines. A citation of a source in no plan is `unread` with that reason. Citations of notes are never `unread`: notes are read with Read, and the plan does not cover them.

`picked:` counts the corpus's sources when `cite` runs, from `bilbo library`'s rule, so the skill no longer counts entries with `grep -c`.

### The citation pre-check in `land --replace`

Under the `land` lock, after the target checks and the stage hash check and before the capture is written, `land` reads every note in `<root>/notes/`, parses its citations, and keeps those whose id is the replaced source's. It checks each against the old body and the new one, with no plan. A citation degrades when its new verdict differs from its old one and is not `ok`. That rule:

- blocks `ok -> quote_missing`, `ok -> anchor_missing`, and `ok -> quote_elsewhere` (the quote moved sections);
- lets a citation that already failed stay failed without blocking, since the re-ingest did not cause it;
- lets an improvement through (`quote_missing -> ok`).

The refusal goes to stderr with exit 1, and nothing is written, the stage included, so the agent can show the user and run again with `--force`. With `--force` the same lines are warnings and `land` goes on. Guides are not checked: their entries carry the stale line already, and `bilbo check` fails until an agent revises them.

Cost: one pass over the notes' text, about 5 MB for this user, only when a digest changes.

### The reference skill

`plugins/bilbo/skills/reference/SKILL.md`, frontmatter in `recall`'s order:

```yaml
---
name: reference
description: Answers a question from the library's sources in the bilbo store, read in full through a plan and cited by id. Use for "what does the Go book say", "check the sources on X", "what do the docs say about Y". NOT for notes (recall) or adding a source.
license: Apache-2.0
allowed-tools: Bash(command -v bilbo), Bash(bilbo library *), Bash(bilbo cite *), Read, Agent(general-purpose), SendMessage
---
```

`Read` is there for `references/reader.md` only; the body forbids it on sources. `Agent(general-purpose)` and `SendMessage` serve the readers in Claude Code; Codex ignores them. The body, in the note skill's shape (numbered steps, a table per command, a `Never` list):

1. **Missing binary.** `command -v bilbo`; nothing printed: stop with `reference: bilbo is not on PATH; install the bilbo CLI first`.
2. **Corpora.** `bilbo library`. A corpus the user named is used as given; an unknown one gets the list and a stop. Otherwise pick the corpora whose name and title fit, at most three. An empty listing or no fit: say so, name the corpora, stop. Exit 1 with `bilbo: no store at <root>` or exit 2: print bilbo's first stderr line and stop.
3. **Picks.** `bilbo library <corpus>` for each, read whole. Pick by the entry prose, only the sources that answer this question, even when the whole corpus would fit. A source whose facts line says `catalog` is picked by section only: `bilbo library show '<corpus>/<name>#<the name from the question>'`, and the section becomes the pick; with no name to look up, it is listed as `lookup only, not searched`. Post the picks message, then go on:

   ```
   picks from <corpus> (<k> of <n> sources):
   - <name or name#anchor>: <one clause on why>
   ```

4. **Plan.** `bilbo library plan '<ref>'...`, with `--slice-bytes` and `--slice-lines` from the probe when the Agent tool is not available. Then by `partitions:`: one, read it yourself; two to six with the Agent tool, one reader each; more than six with the Agent tool, show the partition lines and ask the user to narrow the question or accept a sample (the first six); without the Agent tool, read in order while the tokens read stay within 100,000.
5. **Read.** `bilbo library read <plan> <slice>`, one slice per call. A result without its end marker, with a gap in its line numbers, or with a truncation notice is read again as `--part 1/2` and `--part 2/2`, then quarters. Exit 1 (the source changed, the plan is gone): name the slice and the message in the answer. For readers: Read `references/reader.md`, fill its prompt per partition (question, plan, `slices <a>-<b>`, why each pick), and spawn `reader-<i>` in one turn. A reader without a `read:` line gets one follow-up by name.
6. **Cite.** Draft from what was read, one `bilbo:` citation per claim, then check the whole draft:

   ```bash
   bilbo cite --plan <plan> <<'EOF'
   <the draft>
   EOF
   ```

   | Verdict | What to do |
   |---|---|
   | `ok` | keep |
   | `quote_elsewhere`, `ambiguous` | take the heading path from the detail as the anchor |
   | `too_short` | lengthen the quote from the same section to six words or more |
   | `quote_missing`, `anchor_missing` | read the slice again, copy the quote again; still failing: drop the claim |
   | `unread` | read the slice the detail names, then check again; in no plan: drop the claim |
   | `id_missing` | drop the claim |

   Run it again after any change until the exit code is 0 and every verdict is `ok`.
7. **Support.** Apply reader.md's "Support judgment" to every claim, rewrite or drop, and run `bilbo cite` once more if anything changed.
8. **Answer.** The picks block, the answer, then cite's `citations:`, `coverage:` and `picked:` lines from its last run, word for word, then one line naming catalogs left as `lookup only` and readers that did not report.

`Never`: read a source with Read, `cat`, `grep` or any tool but `bilbo library read`; answer from memory; cite a passage nobody read in this run; write any file; set an environment variable in a command; chain commands.

The heredoc in step 6 is one command whose body is data. `tests/plugin.rs`'s `skills_allow_every_command_they_run` treats each line of a `bash` fence as a command, so it learns to skip a heredoc's body (from a line ending in `<<'EOF'` to the line `EOF`) and checks the command line alone. Whether Claude Code's permission rule `Bash(bilbo cite *)` lets the heredoc through without a prompt is checked in the smoke test.

### The reader brief

`references/reader.md` keeps `reader-brief.md`'s layout: how to fill it, a common block, the reader prompt, follow-ups. The common block has five sections, so other skills can paste the ones they need:

- **Reading rules.** Your slices are all you read, through `bilbo library read <plan> <i>`, one call per slice, in order. Check the header, the unbroken line numbers and the end marker; re-read a cut slice in parts. No other command but `bilbo cite`, no file reads, no search, no spawning. A slice bilbo refuses goes in `not read:` with bilbo's message.
- **Citation rules.** At most ten claims, each `bilbo:<id>#<anchor> "<quote>"`. The id from the slice header. The anchor is the heading path of the deepest section holding the quote: the `-- in:` line gives it at the slice's first line, and each heading line in the slice opens a section below or beside it; a trailing part is enough when it names one section. A source with no heading below its title is cited without an anchor. The quote is copied from the slice without its line number and tab, the clause that carries the claim, normally ten to twenty-five words, never fewer than six; `...` joins two fragments of one section in order. Silence is said, not filled; disagreement is given with both citations.
- **Support judgment.** Below.
- **Output contract.** Numbered claims, each with its citation; one or two sentences on what the slices do not answer; then `read: slices <list>` and `not read: slices <list> (<why>)` or `not read: none`. Before reporting, run `bilbo cite --plan <plan>` on the claims and fix or drop every failing one.
- **Boundaries.** Read-only: no edits, no writes, no command beyond `bilbo library read` and `bilbo cite`.

### Support judgment

`bilbo cite` proves that the words are in the source under that heading, and with `--plan` that someone printed them. It cannot prove that they say what the claim says (the panel's `design-fable.md`, hard problem 2; `design-sol.md` and `design-astra.md` say the same). The section reads, in substance:

> For each claim, read its quote, and the lines around it when the quote is short or hedged. Decide one of three:
> - **supports**: the quote states the claim, or the claim is a faithful narrowing of it;
> - **supports in part**: the quote states a weaker or narrower claim. Rewrite the claim to what the quote says;
> - **does not support**: drop the claim.
>
> A quote does not support a claim when it is on the topic but does not state it; when the claim drops a condition, a version or a hedge the quote carries ("usually", "before Go 1.22", "in this example"); when the claim turns an example into a rule; when the quote states the opposite, or a position the source sets out in order to reject; or when the source marks it as deprecated or superseded. Judge from the quote and its section, never from what you know of the subject.

The skill applies it after `cite` passes. A reader applies it to its own claims before reporting. `review`'s refuter pastes it and reports `citation: unsupported`, which downgrades a finding's grounding to `judgment` as a failed check does.

### How the verbs serve `review`

- **Routing:** `bilbo library` rows give each corpus's name, size and title, so a concern routes by name with no notebook path. `bilbo library <corpus>` is the index to pick from, and its row gives `n`.
- **Planning:** one `bilbo library plan` per concern, with the default budget of 60,000 tokens that Step 4 already pins. Its `partition <k>: slices <a>-<b>` lines map one to one onto conformance lenses.
- **Lenses:** variant A gives each lens the plan id, its slice range and reader.md's Reading and Citation rules. Lenses read with `bilbo library read`, so a lens that skips a slice shows up as `unread` at the refuter, which `check_citations.py` could not see.
- **Refuter:** `bilbo cite --plan <p1> --plan <p2> <candidates file>` checks every candidate in one run. `quote_elsewhere` stays a wrong heading; `quote_missing`, `anchor_missing`, `id_missing` and `unread` fail the check; Support judgment adds `unsupported`.
- **The brief's path:** `review` cannot call the skill, so it pastes from the installed package, `share/bilbo/plugins/bilbo/skills/reference/references/reader.md`, which `flake.nix`'s `postInstall` already copies.

## Risks / Trade-offs

- [Reading through Bash costs about twice the tool calls of Read] → It is the price of coverage counted in code, and one read path for Claude Code and Codex (the planning note's pushback). The default slice is as large as one Bash result allows.
- [Codex's caps are unknown] → The probe task measures them before the skill text is final, and `--slice-bytes`, `--slice-lines` and `--part` let the skill adapt without a code change.
- [A head-and-tail truncation keeps the end marker] → The line numbers show the gap, and the skill and the brief treat a gap as a cut read.
- [The read log credits output the agent never looked at] → Accepted (Non-Goals). The log is still stricter than self-reported coverage, and `unread` catches citations of slices never printed.
- [A plan goes stale when a source is re-ingested] → `read` refuses with a message to re-plan; `cite --plan` gives `unread` with the reason.
- [Cutting inside a fenced block] → The reader sees the fence lines and the numbers; the next slice continues the block.
- [The pre-check reads every note on each replace] → Only when the digest changes, and notes are a few MB.
- [Agents drop anchors' markup] → Anchors compare after the Text normalization, for every verb.
- [`Bash(bilbo cite *)` may not cover a heredoc in Claude Code's permission matcher] → The smoke test checks it. The fallback is a draft file the agent writes, which needs `Write` in `allowed-tools`.
- [The `INFO` from `openspec validate`: the `library-store`, `library-ingest` and `library-browse` deltas need those specs to exist] → They exist once `add-library-store` is archived, which ships first.

## Migration Plan

- The product: new verbs and a new skill; nothing on an existing store changes. `land --replace` gains a refusal that only fires when a note's citation would degrade.
- The cutover, after archive, is listed in proposal.md's Impact. Tasks rehearse its citation step against a migrated copy in a temporary `BILBO_HOME`: the 3 citations rewritten from `map.tsv` must give 3 `ok`.
- Rollback: remove the `reference` skill folder and `<state>/bilbo/plans/`. A store is unchanged by this change.

## Open Questions

- Codex's shell output caps in bytes and lines. The probe in tasks.md answers it and sets two numbers in the skill text; the specs do not depend on it.

## Decisions to confirm

1. Two capabilities, `library-reading` and `citations`, and the reference skill in `agent-plugin`, as the note skill is.
2. `unicode-normalization` 0.1.25 for NFKC, in a new `src/text.rs`, over `icu_normalizer` (already locked, unbuilt, about twenty crates) and a hand table.
3. A slice's size counts printed bytes; its tokens count source bytes.
4. `--slice-lines` added beside `--slice-bytes`.
5. Partitions are runs of consecutive slices (next-fit), not first-fit as the contract says.
6. `--part <k>/<n>` instead of `--half`.
7. The read header's `in:` line, and a second header line beside the contract's end marker.
8. The read log is `<plan>.log`, append-only and per line, beside an immutable `<plan>.json`.
9. Plans older than 30 days are removed by the next `plan`.
10. Overlapping picks of one source are a usage error, not merged.
11. One anchor rule for every verb, after the Text normalization (a MODIFIED `library-store` Anchors); the title is not an anchor.
12. A citation without an anchor in a file with sections is `quote_elsewhere` unless it matches outside every section.
13. An id two files share is `id_missing`.
14. `--plan` may repeat; coverage lines name their plan; `picked:` counts sources at check time.
15. A draft with no citation exits 0; a missing draft file exits 1.
16. The pre-check checks notes only, not guides, and a citation degrades when its verdict changes to anything but `ok`.
17. Catalogs are reached by anchor until `add-library-recall`; the skill names one it could not look up as `lookup only, not searched`.
18. The skill passes `--slice-bytes 10000 --slice-lines 250` without the Agent tool until the probe sets the values.
19. `review` pastes `reader.md` from the installed package path.
20. At the cutover, the 2 `doc:` sources become `"doc: bilbo:<id>"`.
21. `bilbo check` does not check citations.
22. The manifests' descriptions name the library (shared with `add-library-fetch`, which adds `ingest`).
23. `--slice-bytes` has a ceiling of 30,000, and one `read` of several slices may not print more than the plan's slice size.
24. The Text normalization decodes six HTML entities, `&#124;` among them, instead of change 3 writing `\|`.
