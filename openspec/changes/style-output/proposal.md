# Proposal

## Why

Every verb but help prints the same plain bytes on a terminal as down a pipe. `recall`, the verb a person runs most, shows a wall: an absolute path first, the note's title repeated in every heading path, a raw-Markdown snippet cut mid-word at 300 characters, no rank or count, and warnings printed before the hits with the same `bilbo: ` prefix as errors. Tables drift at 8-column tabs, `check` is silent when the store is clean, and the setup wizard ends in an unaligned report. Agents and skills parse the plain bytes, so a better look has to leave them alone.

## What Changes

- Two views per stream. stdout and stderr each get the **human view** only when the stream is an interactive terminal and no agent marker (`AI_AGENT`, `CLAUDE_CODE_CHILD_SESSION`, `CODEX_THREAD_ID`, `CODEX_CI`) is set; everything else, pipes, files, hooks and agents, gets the **plain view**, today's bytes. `CLICOLOR_FORCE` and `FORCE_COLOR` never change a view.
- The human view uses escapes only when `NO_COLOR` is unset or empty, `CLICOLOR` is not `0` and `TERM` is set and not `dumb`; otherwise it keeps its layout and marks without escapes. It uses bold, dim and the five basic colours, the setup wizard's marks (`◆ ◇ ○ ▲ ■ ●`, ASCII without a UTF-8 locale outside macOS), and fits lines to the terminal, between 40 and 100 columns.
- On a terminal, stderr drops the `bilbo: ` prefix for a level mark: `■` error, `▲` warning, `●` progress, `○` nothing found. Warnings print after the result when stdout is a terminal. The setup wizard's colours follow the same rule as stderr's human view.
- Human views for `recall` (ranked hits, bold title and sections, a cleaned and wrapped snippet that starts near the first query word with the query words in bold, a dim line with kind, age and `~` path, a count line, and a hint when nothing matches), `check` (problems grouped by file, a summary, an all-clear), `setup`'s step report (a mark per step and a summary, after the wizard too), `scope`, `library`, `library <corpus>`, `library show`, `device`, `device list`, `device init` and `recover`, `sync`, `history` (a version list and a coloured diff), `restore`, `new` and `index`.
- `digest`, `cite`, `library plan`, `read`, `stage` and `land`, `scope set`, `sync declare`, `pair`, `relay` and `watch` keep the plain view on stdout everywhere; only their stderr levels change on a terminal.
- **BREAKING** (plain bytes): counts of one take the singular: `1 passage not indexed`; `1 note` in `scope`'s listing, `sync`'s scope line, `local: 1 note syncs nowhere` and `setup`'s sync line; and `1 heading` in a guide's facts line.
- `cite`'s `nearest passage:` hint keeps the source's own words, so `SQLITE_BUSY` no longer reads `SQLITEBUSY`.
- Help is bold under the same rule as the human view, so an agent marker, an unset `TERM` or `CLICOLOR=0` now turn bold off.
- The phrase ceremony of `device init`, `device recover` and `device revoke`, `pair`, and the wizard's sync step judge "an agent runs this" by the same marker set. `CLAUDECODE` leaves the set: IDE extensions set it in the terminals people type in, and the agent's own shell has no terminal.

## Non-goals

- No change to any plain-view byte beyond the plurals and the `cite` hint above. `digest`'s `1 of 1 notes`, `device`'s `1 devices` and `sync`'s `waiting <scope> <name>: 1 versions` stay as they are.
- No `--plain`, `--color` flag or `BILBO_PLAIN` variable; `| cat` gives the plain view.
- No styling of `pair`, which stays on its own prompts; no timestamps in service logs; no OSC 8 links; no spinner for recall's embedder; no verbatim code blocks in snippets; no next-step hints after `library land`; no new wording in the wizard's summary box or outro.
- No bright, 256 or true colours, no backgrounds, no pager. The wizard's own cliclack palette is left as it is.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `cli`: Output streams gains the two views, stderr levels and the order of warnings; new Terminal views and Human view escapes requirements; Help styling and Usage errors follow the views.
- `note-recall`: the singular `1 passage not indexed`; a human view of note hits.
- `library-recall`: a human view of library hits.
- `store-check`: a human view of the problems.
- `setup`: a human view of the step report; `(1 note)` in the sync line; the wizard's sync step uses the shared agent markers.
- `note-scope`: `1 note` in the listing; a human view of it.
- `library-browse`: `1 heading` in the facts line; human views of the corpus list, a guide and a source.
- `device-identity`: the terminal-only forms use the shared agent markers; human views of `device`, `device list` and the init and recover step report.
- `device-pairing`: showing a code uses the shared agent markers.
- `sync-status`: `1 note` in the status lines; a human view of the report.
- `note-history`: human views of the version list and the diff.
- `note-restore`: a human view of the result.
- `note-create`: a human view of the created note.
- `note-index`: a human view of the summary.
- `citations`: the `quote_missing` hint keeps the source's own words.

## Impact

- Code: a new `src/host/terminal.rs` (the gate, the palette, width, wrapping, ages and number grouping); `src/main.rs` (picks each verb's view or its plain lines, prints stderr levels, help through the shared gate, the colour switch cliclack reads, a `Failure` for "nothing matched"); `src/shared/store.rs` (`Env` reads the terminal and agent variables and holds the one agent rule); a `view` beside each restyled verb's `Output` in its own verb file; `src/shared/markdown.rs` (Markdown flattened for display); `src/shared/frontmatter.rs` (a `created` time read back); `src/identity/{device.rs,pair/mod.rs}` and `src/setup/facts.rs` (the agent rule); `src/shared/text.rs` and `src/citation/mod.rs` (the hint); `src/search/recall.rs`, `src/note/scope.rs`, `src/sync/cli.rs`, `src/setup/syncing.rs`, `src/library/cli/list.rs` (plurals).
- Dependencies: `console 0.16.6` becomes direct; it is already in `Cargo.lock` through `cliclack`. `tests/layout.rs` places it in `host/terminal.rs`; `openspec/config.yaml` lists it.
- Tests: unit tests for every view and the gate; a pseudo-terminal helper in `tests/common/` and terminal tests in `tests/cli.rs`; `tests/recall.rs`, `tests/scope.rs`, `tests/sync.rs`, `tests/setup.rs`, `tests/library.rs`, `tests/device.rs` and `tests/pair.rs` for the plain-byte and marker changes. The other binary tests keep proving the plain bytes.
- Plugin: `plugins/bilbo/skills/recall/SKILL.md` quotes the `not indexed` warning.
- Docs: `docs/reference/commands.md`, `docs/reference/configuration.md`, `docs/troubleshooting.md`, `docs/guides/{devices,scopes,sync,library}.md`, `docs/manual-tests.md` (a terminal pass); `AGENTS.md`'s rule on the `bilbo: ` prefix.
