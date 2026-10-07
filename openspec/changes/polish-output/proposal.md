# Proposal

## Why

The terminal view left six rough edges. A usage error's `verbs:` and `usage:` lines run past a narrow terminal while every other diagnostic wraps. `recall` sits silent for up to 5 seconds while the embedder answers. A passage that is a code block is flattened into one line of prose. `pair` still asks a person to read five `bilbo:` lines and type `y`, and the joining device is silent for up to two minutes. The watcher, the index timer and the relay write logs with no times in them, so a person cannot tell when a line was written. And recall's paths and references cannot be clicked in terminals that support links.

## What Changes

- On a terminal, a usage error's `usage:` and `verbs:` lines wrap to the width, later lines indented under the text after the label. Plain bytes are unchanged.
- `recall` shows a spinner, `Waiting for the embedder`, on stderr while it waits for the embedder's answer to the query, only when stderr gets the human view, `TERM` is set and not `dumb`, and the wait has lasted half a second. The spinner is erased when the wait ends.
- In recall's human view, a passage that opens with a fenced code block shows the block's first lines verbatim and dim (two under 60 columns, three otherwise), each line cut to the width, instead of the flattened text.
- `pair` on a terminal draws with the setup wizard's prompts. The device that shows the code puts the command to run on the new device, then the code, in a box. It shows a spinner while it waits, names the device that answered, and asks a yes/no question with the fingerprint, defaulting to no. The joining device shows spinners while it looks for the mailbox, waits for the confirmation and fetches the manifests, and puts the fingerprint in a box. A joining device without a person at a terminal (stdin or stderr not a terminal, or an agent marker set) prints today's lines. stdout is unchanged on both sides.
- **BREAKING** (log bytes): every line that `watch`, `relay` and `index` write starts with an RFC 3339 time with its offset and a space, `2026-10-07T01:02:03-03:00 bilbo: watching /Users/a/.local/share/bilbo/notes`, when the stream is a regular file. That covers `watch.log` and `index.log` on macOS and Linux; a terminal, a pipe and systemd's journal (a socket, which keeps its own times) get no time.
- With `BILBO_HYPERLINKS=1`, recall's human view makes each note's path and each library reference an OSC 8 link to the file. The plain view never carries one.

## Non-goals

- No next-step hints after `library stage` or `land`: those verbs are agent protocols and stay plain.
- No change to any plain byte apart from the service-log times of `watch`, `relay` and `index`.
- No spinner or progress bar for a hand-run `index`, and no spinner for library recall, which uses no embedder.
- No link detection: bilbo does not guess whether a terminal supports OSC 8, and does not read `FORCE_HYPERLINK`.
- No windowing of a code passage to its first match; it shows its first lines.
- No change to how `pair` decides who may show a code: showing still needs a person at a terminal.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `cli`: Usage errors wrap their usage lines on a terminal; Output streams lists `pair`'s drawing as an exception and leaves the time before `bilbo: `; a new requirement gives the times in service logs.
- `note-recall`: new requirements for the embedder spinner, code passages, and links in the human view.
- `library-recall`: the human view of library hits takes code passages and links, the reference linking to the source's file.
- `device-pairing`: showing a code and confirming the fingerprint draw with prompts on a terminal; a new requirement covers both sides on a terminal.
- `note-index`: the timer's log line carries its time.

## Impact

- Code: `src/main.rs` (usage lines wrap; times on service lines; the spinner switch for recall; `pair` gets the cliclack prompter); `src/host/terminal.rs` (`Term` gains `redraw`, `stamp` and `links`; the hostname for links; OSC 8 and time helpers); `src/host/prompt.rs` (`Prompter::wait`, a delayed `Spinner`); `src/search/recall.rs` (spinner around the query, code passages, links); `src/shared/store.rs` (`Env` reads `BILBO_HYPERLINKS`); `src/identity/pair/{mod,show,join}.rs` (the prompter in place of the stdin reader); `src/identity/script.rs` (a tap and a late answer for the tests); `src/identity/keys.rs` (the hostname read moves to `host::terminal`).
- Dependencies: none new. `libc` gains `host/terminal.rs` in `tests/layout.rs`'s `PLACEMENT`.
- Tests: unit tests for the views and helpers; pseudo-terminal binary tests in `tests/cli.rs` and `tests/recall.rs`; binary tests of the times in `watch`, `index` and `relay` output, and the existing tests of those verbs read through a helper that drops the time; `src/identity/pair/` tests on the scripted prompter.
- Docs: `docs/reference/commands.md`, `docs/reference/configuration.md`, `docs/troubleshooting.md`, `docs/guides/devices.md`, `docs/guides/relay.md`, `docs/manual-tests.md`; `AGENTS.md` (the stderr writers in `src/host/prompt.rs`, pairing tests on the scripted prompter).
