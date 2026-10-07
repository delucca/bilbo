# Design

## Context

See proposal.md for why. Today `src/main.rs` is the only printer: it writes each verb's `lines` to stdout and every stderr line through `print_stderr`, which prefixes `bilbo: `. Only help is styled: `print_help` bolds headings when `styled(stdout is a terminal, NO_COLOR, TERM)` allows. The wizard and the phrase ceremony draw through cliclack, which styles with `console` and decides colour by console's own global flags. Three files judge "an agent runs this" with a private `marked` over `CLAUDECODE` and `CODEX_THREAD_ID` (`src/identity/device.rs`, `src/identity/pair/mod.rs`, `src/setup/facts.rs`). Skills, specs and binary tests read the plain bytes of almost every verb, and every binary test asserts the `bilbo: ` prefix on each stderr line.

## Goals / Non-Goals

**Goals:**
- A person at a terminal gets readable output from the verbs they run by hand; a pipe, a file, a hook or an agent gets today's bytes, apart from the listed plurals and the `cite` hint.
- One rule decides the view, the escapes and the agent check, and it is a pure function with a table test.
- Each view is a pure function of the verb's result, a terminal description and the time, tested by whole strings.

**Non-Goals:**
- Changing what any verb computes, reads or writes, or any exit code.
- Styling `pair`, the service logs, or the protocol verbs (`digest`, `cite`, `library plan`, `read`, `stage`, `land`).

## Decisions

### Two views, decided per stream

- **The rule.** `agent` is true when any of `AI_AGENT`, `CLAUDE_CODE_CHILD_SESSION`, `CODEX_THREAD_ID` and `CODEX_CI` is set and not empty. A stream is human when it is a terminal and not `agent`. A human stream paints (uses escapes) when `NO_COLOR` is unset or empty, `CLICOLOR` is not `0`, and `TERM` is set and not `dumb`. Marks are Unicode on macOS, and elsewhere when `LANG` ends in `UTF-8` (case-insensitive), console's own rule, so bilbo's marks match the wizard's. Width is the stream's terminal columns, else `COLUMNS` when it parses as a number above 0, else 80; the content width is that clamped to 40..=100. `CLICOLOR_FORCE` and `FORCE_COLOR` are never read.
- **stdout and stderr are judged apart**, as cargo does: `bilbo recall x | less` gets plain stdout and human stderr.
- **Why a terminal check is not enough.** Claude Code's Bash tool gives the command no terminal on fds 0 to 2, but passes on `TERM=xterm-256color` and `COLORTERM=truecolor`, so a check on `TERM` alone would colour the agent's output. Codex runs commands over pipes by default, but its unified exec tool takes `tty: true` (`unified_exec_tty` is stable and on), so the model can ask for a pseudo-terminal; every such command gets `NO_COLOR=1`, `TERM=dumb`, `CODEX_CI=1` and, with a thread, `CODEX_THREAD_ID`. Under a terminal-only gate Codex would get the human layout without colour, and the recall skill would fail to parse it. So the agent check stands beside the terminal check.
- **Why `CLAUDECODE` leaves the set.** Claude Code documents that IDE extensions set it in their integrated terminals, where a person types. `CLAUDE_CODE_CHILD_SESSION` is documented as set only by Claude Code for its own subprocesses. The Bash tool already has no terminal. The same reasoning moves `device init`, `recover`, `revoke`, `pair` and the wizard's sync step to the shared set: today a person in a VS Code terminal is told `bilbo device init needs a terminal ... not through an agent`.
- **`AI_AGENT`** is a young convention (`@vercel/detect-agent` and the `is-ai-agent` crate read it first; Claude Code sets it). bilbo reads four variables, so no crate is taken for it.
- **Why the force variables are ignored.** Claude Code passes the user's environment to the Bash tool; a user who exports `CLICOLOR_FORCE=1` for `ls | less` would otherwise put escapes into the bytes the skills parse. The plain view is a protocol, as `git status --porcelain` is.
- **`NO_COLOR` drops bold and dim too**, as help already does, and keeps the layout and marks, which carry the meaning.
- **An unset `TERM` paints nothing.** console's `is_dumb` and anstyle-query treat it so. This changes help's rule (P5): today help is bold with `TERM` unset.

### Where the code lives

- **`src/host/terminal.rs`, new, in the existing `host` domain.** `host` holds the adapters to what bilbo runs beside, and its `mod.rs` already names the terminal. `host` imports no other domain, so `search`, `note`, `library`, `identity`, `setup`, `sync`, `citation` and `check.rs` can call it from their views without a cycle in `tests/layout.rs`. It holds `Term`, `decide`, the painting, the marks, width, cutting, wrapping, padding, ages and number grouping. Nothing in `shared/` fits: the Shared Kernel holds what two domains use about the store, and terminal styling is an adapter.
- **`shared::store::Env` reads the new variables** (`AI_AGENT`, `CLAUDE_CODE_CHILD_SESSION`, `CODEX_CI`, `NO_COLOR`, `CLICOLOR`, `TERM`, `COLUMNS`, `LANG`) and drops `CLAUDECODE`. It stays the one reader of the environment, so tests inject it. `Env::agent()` is the one agent rule; the three private `marked` functions go.
- **Each restyled verb's `Output` keeps `lines`, its plain view, byte for byte, and gains `pub fn view(&self, term: &terminal::Term, now: jiff::Timestamp) -> Vec<String>` in its own verb file.** Where the plain lines drop what the view needs, `Output` gains fields: recall the hits (title, headings, full passage text, kind, scope, created, path, line, whether a query word is in the passage, the total before `--limit`); check the problems by file with a warning flag and the counts of notes and corpora; setup and device the steps as `(step, status, detail)`; library the corpora, guide entries and the source's sections; restore its outcome. Where the plain lines are a fixed tab or `<step> <status>: <detail>` form that a spec pins (scope, sync, history, device show and list), the view reads them back, which keeps one source for the data.
- **`main` stays the only printer.** It decides a `Term` for stdout and one for stderr at the start, calls `terminal::init`, and per verb prints `view` when stdout is human, else `lines`. It prints stderr through `print_stderr(&Term, Level, &str)`: the plain view prefixes each line with `bilbo: `; the human view writes the level's mark, two spaces, the message wrapped to the width, and indents later lines three columns. When stdout is human, warnings print after the result; otherwise before, as now.
- **A `Failure::Unmatched` variant** carries recall's "nothing matched": the pending warnings, the message (`no notes match`), the query and a hint. Plain stderr is the warnings and the message, each prefixed, as today; human stderr is `○  <message> '<query>'`, the hint, then the warnings after `▲`. It exits 1. Keeping `Refused` would print `■`, an error, for a search that worked and found nothing.

### console 0.16.6 as a direct dependency

- `console` is already compiled into bilbo through `cliclack 0.5.6`, with the same default features (`unicode-width`, `ansi-parsing`, `std`), so the direct dependency adds no crate and no build time. `Cargo.toml` takes `console = "0.16.6"` and `Cargo.lock` keeps the pin. `tests/layout.rs` adds `("console", &["host/terminal.rs"])` to `PLACEMENT`, and `openspec/config.yaml` lists it under Stack.
- It gives what bilbo would otherwise hand-write: the terminal size by `TIOCGWINSZ` (`Term::stdout().size_checked()`), ANSI-aware display width with East Asian widths (`measure_text_width`), cutting that keeps escape codes intact (`truncate_str`), and `strip_ansi_codes` for tests. Above all it holds the colour flag cliclack reads; no other crate can make the wizard follow bilbo's rule.
- **Against the standard library and the existing dependencies:** the standard library has `IsTerminal` but no terminal size and no display width; `libc` gives the ioctl but not widths; `unicode-width` would be a new direct dependency and still not parse escapes. A hand-rolled version is about 120 lines and still leaves cliclack on console's own rules (it honours `CLICOLOR_FORCE` and treats an empty `NO_COLOR` as set). `anstyle` with `anstream` is the cargo and clap standard but brings about six crates, no width, and an auto choice that honours `CLICOLOR_FORCE` over the terminal check. `owo-colors`, `yansi`, `supports-color`, `terminal_size`, `comfy-table`, `tabled` and `is-ai-agent` each cover a slice and add crates.
- **When help gained bold, `console` was turned down** as "a direct dependency for two escape codes". This change needs width, display width, ANSI-safe cutting and the switch cliclack reads, so that reason no longer holds.
- **bilbo writes its own SGR codes** in `terminal::paint`, as help does now: bold `1`/`22`, dim `2`/`22`, colours `31`, `32`, `33`, `34`, `36` closed by `39`. Closing with the matching off-code lets a cyan word sit inside a dim line. console's `Style` closes everything with `0` and follows console's global flags, so bilbo does not use it for its own lines.
- **The switch.** `terminal::init(&stderr_term)` calls both `console::set_colors_enabled` and `console::set_colors_enabled_stderr` with the stderr decision. cliclack builds its styles with `console::style` and `Style::new()` and never calls `for_stderr`, so its colours follow console's stdout flag even though it draws on stderr; setting both flags keeps every console user on stderr's rule. bilbo prints nothing to stdout through console, so the stdout flag has no other reader. The wizard keeps its own palette, which includes a 256-colour grey for its bars; the Human view escapes requirement scopes the basic-colour rule to bilbo's own lines for that reason.

### The look

- **Palette and marks** are cliclack's: `◆` green done or changed, `◇` dim kept, `○` dim skipped or nothing, `▲` yellow warning, `■` red error, `●` blue progress, with cliclack's ASCII fallbacks `*`, `o`, `-`, `!`, `x`, `•`; `›`/`>` joins headings and `…`/`...` marks a cut. A mark is followed by two spaces. Bold for titles, counts and matched words; dim for ranks, paths, ids, times, headers; cyan for words a reader types back (kinds, versions, commands in hints). At most one colour per line besides dim.
- **Recall.** A title line of the rank (right-aligned in at least 2 columns), the note's title in bold and each heading after `›`, cut to the width. Up to 3 passage lines (2 under 60 columns) wrapped by words: the passage text is flattened for display (`markdown::plain`: emphasis, code and link markup, images, escapes, fences and list markers removed, whitespace collapsed) and, when the first query word would fall beyond what the lines can show, starts at the sentence holding it after `… `; a cut tail ends in ` …`. Query words are bolded where a whole word folds, as `text::words` folds it, to a query word. A dim meta line: kind (cyan), scope when two or more scopes are declared, age, `by meaning` when no query word is in the heading path or text, and `~/…:<line>`; under 60 columns it wraps instead of cutting, since the path is what a person copies. Library hits use the source's `#` title, or `<corpus> guide`, and a meta of `source|guide · <ref> · lines <a>-<b>`. A count line closes the list.
- **`markdown::plain` is new and does not reuse `text::normalize`**, which drops every `_` and `*` so that `_emphasis_` matches; display must keep `busy_timeout` and `SQLITE_BUSY`. It removes `_` only as emphasis delimiters, not between two word characters.
- **Times** come from the `created` form (`frontmatter::created_time` reads it back): under a minute `just now`, then `N min ago`, `1 hour ago`/`N hours ago`, `yesterday` (24 to 48 hours), `N days ago` up to 30 days, then the date as written; a time in the future also prints the date.
- **Numbers** group thousands with commas; sizes print `KB` under 1024 KB and `MB` with one decimal above, 1024-based as the plain `KB` is.
- **Paths** under `HOME` print from `~/`; a report about the store (`check`) keeps `notes/...`.

### Plain-byte changes

- **P1–P3, plurals.** `1 passage not indexed` (`src/search/recall.rs`); `1 note` in `scope`'s rows and `(unassigned)`, `sync`'s scope line and `local: 1 note syncs nowhere` (`src/note/scope.rs`, `src/sync/cli.rs`) and the setup sync line's `(1 note)` (`src/setup/syncing.rs`); `1 heading` in the facts line (`src/library/cli/list.rs`). Every count of notes in a plain line takes the singular, since the specs these lines live in change anyway. No skill reads the changed words: the recall skill quotes `<n> passages not indexed` and takes `<n> passage(s)`, the note skill reads only `scope`'s first field and `(unassigned)`. Counts of other things (`1 devices`, `1 versions`, digest's `1 of 1 notes`) stay: they are pinned by other tests or feed the model, and nothing here needs them.
- **P4, the cite hint.** `nearest` cuts its window from the body's normalized text, and `text::normalize` drops `_` on purpose so that markup does not break quote matching. The fix keeps matching as it is and builds the hint's words from a readable form of the same lines that keeps an `_` between two word characters; both forms split into the same words, so the window maps across by word index. When the counts differ the hint falls back to today's words.
- **P5, help.** `styled` goes; help paints when `decide` says stdout is human and paints. New: an agent marker, an unset `TERM` or `CLICOLOR=0` turn bold off.
- **The agent rule.** The terminal-only forms, pairing and the wizard's sync step use `Env::agent()`.

### Edits to archived scenarios

- `cli` Output streams: "A failure leaves stdout empty" and "Non-interactive setup keeps the prefix" now say that stderr is piped, since a terminal gets level marks. Both are kept.
- `device-identity` Terminal-only forms, `device-pairing` Who can pair and `setup` Turning sync on in the wizard: the scenarios that used `CLAUDECODE=1` as the Claude Code marker use `CLAUDE_CODE_CHILD_SESSION=1`, and a new scenario says `CLAUDECODE` alone is a person.
- `note-scope` List the scopes and `note-recall` Keyword fallback: `1 notes` and `1 passages` become singular in their scenarios.
- No archived scenario is dropped.

### Tests

- `decide` gets a table test over the whole environment matrix; `paint`, `cut`, `wrap`, `ago`, `group` and `tilde` get unit tests; every `view` gets whole-string tests with a fixed `Term` (paint on and off, widths 100 and 50, Unicode on and off) and a fixed `now`. Expected strings write escapes through a test notation (`{b}`, `{d}`, `{c}`, `{g}`, `{y}`, `{r}`, `{u}`, `{/}`) expanded by one helper in `host::terminal`.
- Binary tests keep running without a terminal and keep proving the plain bytes and the `bilbo: ` prefix. A pseudo-terminal helper in `tests/common/` (`libc::openpty` for stdout and stderr, a fixed window size, the child's output drained while it runs) lets `tests/cli.rs` check the gate end to end on Linux and macOS: the human view on a terminal, the plain view under each agent marker, no escapes under `NO_COLOR`, `TERM=dumb` and unset `TERM`, the width cap, and stderr marks. `script(1)` differs between macOS and Linux, so the tests do not use it; `docs/manual-tests.md` does, for a person's pass.

## Risks / Trade-offs

- [A harness runs bilbo in a pseudo-terminal and sets none of the four markers] → it gets the human view and a skill fails to parse it. Claude Code gives no terminal and Codex sets `CODEX_CI`; a Claude Code tmux session gets the human view, which its model reads like a person. Mitigation: the markers are in one function, and adding one is a one-line change.
- [A person in a Claude Code IDE terminal now gets colour and the phrase ceremony] → intended; the documented marker for Claude Code's own processes is `CLAUDE_CODE_CHILD_SESSION`.
- [Dim (SGR 2) is faint on some themes] → uv and cargo use it for secondary text; the manual pass checks Terminal.app, iTerm2, Ghostty and a light theme.
- [A script matched `1 notes`, `1 passages` or `1 headings`] → no skill, spec or docs page reads them after this change; the release notes say so.
- [cliclack moves to a console release cargo cannot unify with 0.16] → bump both together; `tests/layout.rs` keeps console in one file.
- [Wrapping or cutting splits a grapheme cluster] → widths come from `unicode-width` through console, by `char`; a cluster cut at the edge shows one stray mark at worst.
- [The wizard loses colour where console used to force it] → `CLICOLOR_FORCE` no longer colours the wizard, and an empty `NO_COLOR` no longer turns it off; both now match the rest of bilbo.
