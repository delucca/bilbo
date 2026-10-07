# Design

## Context

See proposal.md for why. The terminal views already exist: `host::terminal` decides a `Term` per stream (`human`, `paint`, `unicode`, `width`, `home`) from the environment that `shared::store::Env` reads, and `main` prints every line, choosing a verb's `view` or its plain `lines`. cliclack draws the wizard and the phrase ceremony on stderr through `host::prompt::Terminal`, the one writer besides `main`, behind the `Prompter` trait; `identity/script.rs` is the scripted `Prompter` the unit tests use.

What each item touches today:

- `usage_report` in `src/main.rs` marks the reason and wraps it, then pushes each usage line dim and indented, never wrapped. The `verbs:` line is 121 columns.
- `search::recall::meaning` calls `embed::query` with a 5 second timeout; nothing shows while it waits.
- `Output::snippet` in `src/search/recall.rs` runs every passage through `markdown::plain`, which drops fence lines and joins the rest into one line.
- `pair` prints through two callbacks (`out`, `err`) and reads A's answer as one line from an injected `BufRead`. A refuses to show a code unless stdin and stderr are terminals and no agent marker is set, so A always has a person. B prints `fingerprint ...` and is otherwise silent while it polls (up to 2 minutes for the mailbox, 10 for the reply, 2 for the manifests). The show tests learn the code from the `pairing code` line on the `err` callback, through a channel, while A waits.
- `watch` and `relay` print progress through `main`'s `print_stderr` (`bilbo: ` off a terminal); `index` prints its result on stdout and warnings on stderr. `setup` installs launchd agents with `StandardOutPath` and `StandardErrorPath` set to `watch.log` or `index.log`, and systemd user units with `StandardOutput=append:<log>` and `StandardError=append:<log>`. The relay runs under the NixOS module or a hand-written unit with the default output, the journal. Nothing in the code, tests or docs parses those logs.
- recall's dim meta line holds `~/…:<line>` for a note and `<corpus>/<name>` for a source; `meta_lines` breaks it with `terminal::tail_lines`, never cuts it.

## Goals / Non-Goals

**Goals:**
- Every change but the log times lives in the human view; plain stdout bytes do not move.
- The log times touch only `watch`, `relay` and `index`, and never a line the journal or a person's terminal shows.
- `pair` keeps one code path for its exchange; only the drawing differs between a person and a pipe, and the exchange stays testable in one process.

**Non-Goals:**
- A progress bar for `index`, links outside recall, link detection, windowing code passages to their match.

## Decisions

### Usage lines wrap under their label

On a terminal each usage line is folded (`terminal::fold`: break at spaces, split a word wider than the line, never cut) to `width - 10` columns. The first piece keeps its 7-column label (`usage: `, `verbs: ` or the 7 spaces before a later form); later pieces get 7 spaces, so with the 3-column indent the text lines up at column 10. Each piece is painted dim on its own, so no escape spans a line break. The `see` line is at most 24 columns (`see bilbo library --help`) and never wraps at the 40-column minimum, so it is left alone.

- Why a hanging indent, not the 3-column indent every other diagnostic uses: a wrapped form indented to the same column as `usage:` reads as a new form. Help pages wrap a form the same way, its continuation indented past `bilbo`.
- The plain view keeps one line per form: the `cli` spec pins it, and scripts match it.

### The recall spinner

- **Where:** `host::prompt` gains `Spinner { on: bool }` with `fn wait<T>(&self, message: &str, work: impl FnOnce() -> T) -> T`. `recall::run` takes `&prompt::Spinner` and wraps the `embed::query` call in `meaning`. `main` builds it with `on = io.err.human && io.err.redraw`. `prompt.rs` is already the file where cliclack draws on stderr, so `main` stays the only other writer.
- **Drawing:** cliclack's own spinner (`cliclack::spinner()`, an indicatif bar with the wizard's theme), stopped with `clear()`, which calls indicatif's `finish_and_clear` and prints nothing for an empty message. That leaves no line behind, matches the wizard's spinner glyphs and colours, and follows `terminal::init`'s colour switch. indicatif draws only when stderr is a terminal.
- **Delay of 500 ms:** a warm local embedder answers in tens of milliseconds; a spinner drawn and erased that fast is a flicker on every recall. A cold model or a remote embedder takes seconds, and the timeout is 5. git delays its progress meters for the same reason.
- **No `Send` on `work`:** the spinner runs on a scoped thread, not the work. The thread waits on a channel with `recv_timeout(500 ms)`; on a timeout it starts the spinner, waits for the done message, then clears it. The calling thread runs `work`, sends done and joins. `embed::query` borrows the config and builds a `ureq` client, and nothing needs to cross threads.
- **`Term.redraw`:** new, `human` and `TERM` set and not `dumb`. A spinner redraws its line with `\r` and erase sequences, which a dumb terminal prints as text. `NO_COLOR` does not stop it: the spinner then draws without colour, as the wizard does. `paint` could not serve, since it is false under `NO_COLOR`.
- **Alternatives:** indicatif directly would need a new `PLACEMENT` entry and its own theme; a hand-written spinner in `main` would need a thread that writes stderr while `main` waits, and the same erase logic indicatif has.

### Code passages

- **The rule:** a passage is a code passage when its first line (`Passage.text` starts at its first non-blank line) is a fence line by `markdown::fence_run`. Its lines are those after that fence up to the closing fence (same character, a run at least as long, no info string), or to the end of the passage when the block is cut there. This is the same fence rule the outline and `markdown::plain` use.
- **Display:** the first 3 lines (2 under 60 columns, as prose), each with tabs as 4 spaces and every other control character written with `char::escape_default` as `check` writes them, then cut to the passage width with `terminal::cut` and painted dim. When more lines follow in the block or anything follows the block, the last shown line ends in ` …` (dim), cut first to leave room. Blank lines inside the block are left out, so no shown line is only indent; a block with no other line shows no passage lines.
- **Why dim and no bold:** dim and bold close with the same code (22), so a bold query word would end the dim early. Code is shown for its shape; the title line already says why it matched.
- **Why only a passage that opens with a fence:** a passage of prose with a block inside reads well flattened, and the prose usually says what the code does. A passage that is all code turns into one unreadable line when flattened, which is the case to fix.
- **Why the first lines, not the match:** the top of a block carries its context (the function, the command); this change keeps to the first lines, and windowing can come later without a spec change to the plain view.
- **Escapes:** `markdown::plain` keeps an ESC byte in prose today; code lines are written as typed, so they must escape control characters, or a note could write escapes to the terminal.

### pair on cliclack

- **The port:** `Prompter` gains `fn wait<T>(&mut self, message: &str, work: impl FnOnce() -> T) -> T`: a spinner shown at once and erased when `work` ends. `Terminal` implements it with the same helper as `Spinner` at zero delay; `Script` logs `wait: <message>` and runs `work`. pairing's waits last seconds to minutes, so the delay buys nothing there.
- **`pair::run` takes `prompter: &mut impl Prompter` in place of `answer: &mut dyn BufRead`.** `Cx` becomes generic over the prompter. `main` passes `&mut host::prompt::Terminal`. A always draws (it refuses without a person, unchanged). B draws when `cx.human`, the same test A uses; otherwise it prints today's lines through `err`.
- **A's flow:** `intro("bilbo pair")`; `note("On the new device, run", "<command>\n\nThe code is <code>. It works once, for 10 minutes.")`; `wait("Waiting for the new device", poll)`; `info("<name> <id> asks to join <scopes>")`; `confirm("Fingerprint <F>: does <name> show the same?", false)`; after the reply is sent, `outro("Paired")`, then the `paired` line on stdout. A refusal after the intro calls `cancel("Not paired")`, then `main` prints the reason with `■`. A `confirm` error (Esc, Ctrl-C, end of input) is a no: A sends the `Declined` reply, so B ends at once with `the other device declined` instead of waiting out 10 minutes as it does when Ctrl-C kills A today.
- **B's flow, on a terminal:** after its local checks pass, `intro("bilbo pair")`; `wait("Looking for pairing <nameplate>", …)`; `note("Fingerprint", "<F>\n\nThis device is <name> <id>. Confirm on the device that showed the code.")`; `wait("Waiting for the other device to confirm", …)`; `wait("Fetching the scopes", …)` around the manifest polls in `fetch`; `outro("Paired with <A's name>")`; the two stdout lines. A refusal after the intro: `cancel("Not paired")`. The later notices (`n notes already carry scope …`, a pending-key removal error) stay on `err`, which `main` marks with `●` on a terminal.
- **A command too wide for the box:** cliclack wraps a note's text to the terminal (`textwrap::fill`, breaking at spaces and inside long words), so a long `--via` URL would be cut over two lines with the box's border between them, and a copy would carry the border. When `width_of(command) + 6` exceeds the terminal's columns, A shows `On the new device, run:` and the command with `info` (cliclack does not wrap log lines; the terminal soft-wraps them, which copies whole), then a box titled `Pairing code` with the code line. `pair::run`'s `terminal: bool` becomes `terminal: Option<usize>`, the stderr width when stdin and stderr are terminals, so the argument count stays at seven (clippy's `too_many_arguments`).
- **Order in the box:** the command first, then the code. magic-wormhole prints the code first (`Wormhole code is: …`, then `On the other computer, please run:` and the command, in `cli/cmd_send.py`), but the command is what a person copies and acts on, and the code on its own line is what they read out or type by hand when they cannot paste; the box's title already says "run". The URL stays absolute, not `~/`: it is pasted on another machine.
- **Default no:** a stray Enter must not enroll a device; the ceremony's `Written down?` defaults to no for the same reason.
- **Tests:** `Script` gains a tap, an `mpsc::Sender<String>` that receives every logged line, so the show tests' player thread reads the code from the `note:` line while A waits, as it read the `err` line before; and `Answer::Late(Duration, bool)`, an answer given after a pause, for the confirmed-too-late test that used a slow reader. The join tests drive both paths: `human: false` with the `err` lines as today, and a `Script` for the drawing. `AGENTS.md`'s line on pairing tests changes from "injected terminal" to "injected prompter".

### Times in service logs

- **When:** a line of `watch`, `relay` or `index` (their result, diagnostics, failures and the relay's panic line; never help) gets the time when the stream it is written to is a regular file. Once the verb is known, `main` sets `Term.stamp` on each stream from the file type of its descriptor (`std::fs::File::metadata` on a clone of the fd, `is_file()`); no environment variable is read.
- **Why a regular file:** the place no one else stamps a line is a log file. The units `setup` installs send both streams to files (`StandardOutput=append:` on systemd, `StandardOutPath` on launchd), so `watch.log` and `index.log` get times on both systems. systemd's journal receives output through a socket and keeps its own times, so the relay under the NixOS module or the documented hand-written unit gets none. A terminal is read live, where the human view marks lines with `●`; a pipe goes to an agent, a test or a log collector, and keeps the exact plain bytes, the line every other verb holds.
- **Why not `JOURNAL_STREAM`:** the first rule considered was "stamp unless `JOURNAL_STREAM` is set". systemd.exec(5) warns that "it is generally not sufficient to only check whether $JOURNAL_STREAM is set at all", since a service may redirect its children's output without unsetting it, and the variable says nothing about pipes, which would then gain times in every agent run and test. The file type of the stream answers the question directly.
- **Form:** `<time> <line>`, where `<line>` is exactly what the plain view prints: `2026-10-07T01:02:03-03:00 bilbo: watching /Users/a/.local/share/bilbo/notes`. The time leads because `index.log` holds both streams: stdout's `embedded …` has no `bilbo: ` and stderr's warnings do, so a time after `bilbo: ` would sit in two columns. A leading time also sorts and merges, as `journalctl -o short-iso` and syslog files read. The form is RFC 3339 to the second with the local offset (`%Y-%m-%dT%H:%M:%S%:z`), the `created` form of notes plus seconds, since RFC 3339 requires them; UTC prints `+00:00`. Each line takes the time it is written.
- **Code:** `terminal::stamped(now: &jiff::Zoned, line: &str) -> String` is pure and unit-tested; `main` calls it with `jiff::Zoned::now()` on each line that `print_stderr`, `report` and `emit`'s stdout write when that stream's `stamp` is set. Help, `--version` and the other verbs' lines go through paths that never stamp.

### OSC 8 links, opt-in

Evidence, read for this change:
- The OSC 8 proposal (the "Hyperlinks in terminal emulators" gist that VTE and iTerm2 implement): open `OSC 8 ; params ; URI ST`, close `OSC 8 ; ; ST`, ST being `ESC \` (BEL is common, ST encouraged); params and URI only bytes 32-126, the rest percent-encoded; "Utilities that print hyperlinks are requested to fill out the hostname", terminals "must accept the string localhost or the empty string as local"; "Currently there's no way of detecting whether the terminal emulator supports hyperlinks"; a terminal that parses OSC per ECMA-48 ignores the sequence and shows the text.
- ripgrep: "At present, ripgrep does not enable hyperlinks by default. Users must opt into them" (`--hyperlink-format`, default `file://{host}{path}` with the `gethostname` host); `NO_COLOR` and `TERM=dumb` turn hyperlinks off with colours. GNU `ls --hyperlink[=WHEN]` defaults to never. delta: `hyperlinks = true` in its git config, off by default, with `hyperlinks-file-link-format`. The `supports-hyperlinks` crate guesses from an allowlist (`VTE_VERSION` ≥ 5000, `TERM_PROGRAM` of iTerm.app, WezTerm, vscode, ghostty and others, `xterm-kitty`, `WT_SESSION`, `KONSOLE_VERSION`) and `FORCE_HYPERLINK`. gh was not checked.

Decisions:
- **`BILBO_HYPERLINKS=1` turns links on; unset or any other value leaves them off.** Support is a property of the terminal emulator, not of the store or the device: one machine can run iTerm2 and Terminal.app, and the variable goes in the profile of the one that supports links, beside `TERM` and `COLORTERM`. A config key would apply to every terminal of the device, and `setup` rewrites the config keeping only the key families it knows, so a new family needs plumbing. The value is exactly `1` so that `BILBO_HYPERLINKS=0` cannot turn links on. `Env` reads it, as it reads every variable.
- **No detection and no `FORCE_HYPERLINK`:** the proposal says detection is impossible; allowlists miss tmux, ssh and new terminals; bilbo ignores force variables because they leak into agents' environments, and `FORCE_HYPERLINK` means "into pipes too".
- **Only when the human view paints:** an OSC is an escape; `NO_COLOR`, `CLICOLOR=0` and `TERM=dumb` asked for none, as ripgrep treats them. The plain view never links, so the variable leaking into an agent changes nothing.
- **Target:** `file://<host><path>`, the path absolute and percent-encoded except `A-Z a-z 0-9 - . _ ~ /`; no line number, since `file://` URIs have no standard for one. The host is the raw `gethostname`, percent-encoded the same way, empty when it cannot be read. `identity::keys::host_name` reads the same value and sanitizes it into a device name; the raw read moves to `host::terminal::machine_name`, and `keys` calls it, so `libc` adds `host/terminal.rs` to its `PLACEMENT` (`host` imports no other domain, so the terminal code cannot call `keys`).
- **Shape:** `Term.links: Option<String>`, the host when links are on. `decide` stays pure and sets `Some(String::new())`; `open` fills in the host. `terminal::link(term, uri, text)` wraps `text` and returns it unchanged when `links` is `None`. A note hit links the `~/…:<line>` text, a library hit links the reference; a path that `tail_lines` broke over lines links each piece to the same URI, so no line ends inside an open link. `Meta::Library` gains the file's path.
- **Width:** console 0.16.6 parses OSC sequences ending in `ESC \` or BEL as zero-width escapes (`strip_osc8_hyperlink_st` in its `ansi.rs` tests), so `width_of`, `wrap` and `cut` stay right; the meta line is folded, never cut, so no link loses its close.

### Term and Env

`Term` gains `redraw: bool`, `stamp: bool` and `links: Option<String>`; `decide` sets `stamp: false` and only `main` sets it; `fixed` sets `redraw: true`, `stamp: false`, `links: None`. `Env` gains `bilbo_hyperlinks`. No new crate; `jiff` already formats `created`.

### Edits to archived scenarios

- `cli` Output streams: the requirement names `pair`'s prompts and recall's spinner among the exceptions, and the time before `bilbo: `; a scenario is added. Usage errors: three scenarios added. No scenario dropped.
- `device-pairing` Show a pairing code: "A code is shown" now finds the command and the code in the box, since A's `pairing code <code>` line goes. Confirm the fingerprint: "Matching fingerprints" pipes B's stderr, A asks a question instead of reading a line, "Declined" answers no, "End of input" keeps its meaning for a terminal that closes; two scenarios added. Expiry: "Confirmed too late" answers yes instead of typing `y`. None dropped.
- `note-index` Human view of index: the timer's log line carries its time; a journal scenario is added. The other specs that state lines of `watch`, `index` and `relay` are untouched: the `cli` requirement says they state the text after the time.
- `library-recall` Human view of library hits: two scenarios added.

## Risks / Trade-offs

- [A terminal that does not parse OSC (an old multiplexer) prints the URI] → links are off unless the person turns them on for that terminal.
- [tmux before 3.4, or without its hyperlinks feature, drops links] → the text still shows; nothing breaks.
- [`bilbo recall x | less`: a slow embedder's spinner draws on the terminal under `less`] → it appears only after 500 ms and is erased; git's progress does the same. The scope asked for stderr's view alone to decide.
- [A log reader that matched lines starting with `bilbo: `] → none in bilbo, its docs or its plugin; the release notes say the logs gained times.
- [A script that redirects `bilbo index` to a file and matches whole lines] → the times lead each line, so a match on the text after the first space still works; the release notes say so. Pipes keep their bytes.
- [A log collector reading bilbo through a pipe that adds no time of its own] → no times; redirect to a file, as the units `setup` installs do.
- [cliclack draws pair's prompts in a dumb terminal] → the same as the wizard today; `pair` needs a person at a terminal.
- [Esc at A's question now sends a decline] → intended: B learns at once instead of after 10 minutes.
- [The spinner thread clears the line after `work` has returned] → `wait` joins the thread before it returns, so the clear lands before any line `main` prints.
- [indicatif ticks every 100 ms on its own thread while `work` runs] → it stops at `clear()`; the spinner thread owns the bar, so no tick follows the join.
