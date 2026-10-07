# Tasks

## 1. Shared ground

- [x] 1.1 `shared::store::Env` reads `BILBO_HYPERLINKS` into `bilbo_hyperlinks`; `host::terminal::Term` gains `redraw`, `stamp` and `links` as design.md says (`decide` pure: `redraw` = human and `TERM` set, not empty, not `dumb`; `links` = `Some(String::new())` when paint and the value is exactly `1`; `stamp` false), `open` fills the host into `links`; `fixed` sets `redraw: true`, `stamp: false`, `links: None`; the `decide` table test gains the new columns. Verify: `nix develop -c cargo test --locked --bin bilbo host::terminal`
- [x] 1.2 Move the raw `gethostname` read to `host::terminal::machine_name`, have `identity::keys::host_name` sanitize its result, and add `host/terminal.rs` to `libc` in `PLACEMENT` (`tests/layout.rs`). Verify: `nix develop -c cargo test --locked --test layout` and `nix develop -c cargo test --locked --bin bilbo identity::keys`
- [x] 1.3 Add `terminal::stamped`, `terminal::file_uri` and `terminal::link`, and the `{l:<uri>}`/`{/l}` notation in `terminal::styled`, with unit tests for the exact strings in PLAN.md. Verify: `nix develop -c cargo test --locked --bin bilbo host::terminal`

## 2. Usage lines wrap on a terminal

- [x] 2.1 `usage_report` folds each usage line under its 7-column label on a terminal (design.md), the plain view unchanged; unit tests at widths 100, 80, 60 and 40 with the strings in PLAN.md, and the existing usage tests unchanged. Verify: `nix develop -c cargo test --locked --bin bilbo tests::a_usage_error`
- [x] 2.2 A pseudo-terminal test in `tests/cli.rs`: `bilbo recal x` at 60 columns gives the three verbs lines and no stderr line wider than 60; piped, the verbs line is one line. Verify: `nix develop -c cargo test --locked --test cli usage`

## 3. Times in service logs

- [x] 3.1 `main` stamps every line of `watch`, `relay` and `index` (result, warnings, info, failures, the relay's panic line; not help) written to a stream whose `Term.stamp` `main` set (the verb is one of the three and the stream is a regular file), through `terminal::stamped` with `jiff::Zoned::now()`. Verify: `nix develop -c cargo test --locked --bin bilbo`
- [x] 3.2 `tests/common/mod.rs` gains `bilbo_logged(cwd, env, args) -> Run`, which runs the binary with stdout and stderr redirected to files and reads them back. Existing binary tests use pipes and stay unchanged. New tests: `bilbo watch` with no store, `bilbo index` with a fake embedder and with none, and `bilbo relay` on a taken port, each through `bilbo_logged`, write lines matching `^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d ` before the plain text; the same runs piped carry no time; `bilbo recall` and `bilbo watch --help` through `bilbo_logged` carry none; on a pseudo-terminal `bilbo relay` on a taken port carries none. Verify: `nix develop -c cargo test --locked`
- [x] 3.3 Docs: `docs/reference/commands.md` (`watch`, `index`, `relay`: each line written to a file starts with the time), `docs/guides/setup.md` (Index timer and Watch service logs), `docs/guides/relay.md` (what it logs; only lines written to a file carry a time; the journal keeps its own), `AGENTS.md` (the `bilbo: ` rule: service lines written to a file start with the time; tests that need it use `common::bilbo_logged`), `docs/troubleshooting.md` (a `Read the service logs` entry with an example line). Verify: `grep -n '2026-10-07T01:02:03' docs/reference/commands.md docs/troubleshooting.md`

## 4. Recall's spinner

- [x] 4.1 `host::prompt`: a private `spinning(message, after, work)` (the spinner on a scoped thread, cleared with `clear()`), `Spinner { on }` with `wait` at 500 ms, and `Prompter::wait` (`Terminal`: zero delay; `identity::script::Script`: logs `wait: <message>` and runs `work`). Verify: `nix develop -c cargo clippy --locked --all-targets -- -D warnings`
- [x] 4.2 `search::recall::run` takes `&prompt::Spinner` and wraps `embed::query` in `meaning` with `Waiting for the embedder`; `main` passes `Spinner { on: io.err.human && io.err.redraw }`. Verify: `nix develop -c cargo test --locked --test recall`
- [x] 4.3 Pseudo-terminal tests in `tests/recall.rs` with `Fake::delay`: 1.5 s on a terminal shows `Waiting for the embedder` on stderr and ends with the erase; a quick fake, `CLAUDE_CODE_CHILD_SESSION=1` and `TERM=dumb` leave stderr empty. Verify: `nix develop -c cargo test --locked --test recall spinner`
- [x] 4.4 Docs: `docs/reference/commands.md` (`recall`, Terminal output: the spinner), `docs/manual-tests.md` (Terminal views: a slow embedder's spinner is erased), `AGENTS.md` (the stderr writers in `src/host/prompt.rs` are `Terminal` and `Spinner`). Verify: `grep -n 'Waiting for the embedder' docs/reference/commands.md docs/manual-tests.md`

## 5. Code passages

- [x] 5.1 recall's `snippet` shows a passage that opens with a fence as its code lines (design.md: count, tabs, escapes, cut, ` …`, dim, no bold), unit tests for the cases in PLAN.md at widths 100 and 50, painted and not. Verify: `nix develop -c cargo test --locked --bin bilbo search::recall`
- [x] 5.2 A binary pseudo-terminal test: a note whose section is one fenced block shows its first lines as written; piped, the snippet is unchanged. Verify: `nix develop -c cargo test --locked --test recall code`
- [x] 5.3 Docs: `docs/reference/commands.md` `recall` (a code passage shows its first lines as written). Verify: `grep -n 'code block' docs/reference/commands.md`
- [x] 5.4 In recall's human view, each control character but a tab in a title, a heading, a snippet, a path or a code line is written as an escape (`\u{1b}`); the plain view is unchanged; unit tests with an ESC sequence and a BEL in a passage, a title and a block. Verify: `nix develop -c cargo test --locked --bin bilbo search::recall`

## 6. Links

- [x] 6.1 recall's meta lines link a note's path and a library hit's reference when `term.links` is set (each piece of a broken path), `Meta::Library` gains `path`; unit tests with the notation for a note, a broken path at 50 columns, a library hit, and `links: None`. Verify: `nix develop -c cargo test --locked --bin bilbo search::recall`
- [x] 6.2 Pseudo-terminal tests: `BILBO_HYPERLINKS=1` gives `\x1b]8;;file://` on stdout; unset, `yes`, with `NO_COLOR=1`, or piped gives none. Verify: `nix develop -c cargo test --locked --test recall link`
- [x] 6.3 Docs: `docs/reference/commands.md` Terminal output (a `BILBO_HYPERLINKS` row and a sentence on links), `docs/reference/configuration.md` (the variable), `docs/troubleshooting.md` (links do not open: the terminal lacks OSC 8, or tmux needs its hyperlinks feature), `docs/manual-tests.md` (links in a terminal that supports them and one that does not). Verify: `grep -n 'BILBO_HYPERLINKS' docs/reference/commands.md docs/reference/configuration.md docs/troubleshooting.md docs/manual-tests.md`

## 7. pair on the wizard's prompts

- [x] 7.1 `pair::run` takes `&mut impl Prompter` in place of the `BufRead`; `Cx` is generic over it; `main` passes `host::prompt::Terminal`. Verify: `nix develop -c cargo build --locked`
- [x] 7.2 The showing device's flow and the joining device's two paths as design.md says (intro, box, waits, info, confirm defaulting to no, outro, `Not paired` on a refusal after the intro; a cancelled question sends the decline). Verify: `nix develop -c cargo test --locked --bin bilbo identity::pair`
- [x] 7.3 `identity::script::Script` gains the tap and `Answer::Late`; the show tests read the code from the tapped `note:` line and answer through the script; join tests cover both paths, including that a plain B's `err` lines are byte for byte today's. Verify: `nix develop -c cargo test --locked --bin bilbo identity::pair`
- [x] 7.4 Docs: `docs/guides/devices.md` (Pair a device: the box, the question, B's box), `docs/reference/commands.md` (`pair`; Terminal output: `pair` draws on stderr like the wizard), `docs/manual-tests.md` (a Pairing section: two terminals, yes, no, Esc), `AGENTS.md` (pairing tests use `pair::run`'s injected prompter). Verify: `grep -n 'On the new device, run' docs/guides/devices.md docs/manual-tests.md`

## 8. Integration

- [x] 8.1 Run the full gate: `nix develop -c cargo fmt --check && nix develop -c cargo clippy --locked --all-targets -- -D warnings && nix develop -c cargo test --locked && nix flake check -L && openspec validate polish-output --strict`
