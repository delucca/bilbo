# Proposal

## Why

`bilbo <verb> --help` prints the same 54-line text as `bilbo --help`, with one synopsis line and one run-on sentence per verb: no option meanings, defaults, output formats or exit codes. Every usage error prints that whole text again on stderr, about 1,200 tokens that start with `bilbo new` whatever verb failed, and push the reason off a 24-row screen. Agents read help as the tool's contract and pay for every line of it.

## What Changes

- `bilbo --help`, `bilbo -h` and the new `bilbo help` print a short overview: what bilbo is, the verbs in four groups with a one-line summary each, examples, the exit codes, the path rules and a docs link.
- `bilbo <verb> --help`, `-h` and `bilbo help <verb>` print that verb's own page: summary, synopsis, options with their defaults, what the output looks like, exit codes, examples and a link to the verb's section of the commands reference. A subcommand's help is its verb's page.
- **BREAKING** A usage error no longer prints the whole help. It prints the reason, the synopsis of the verb that was run (only its subcommand's forms when one was named), and the page to read. With no verb or an unknown one it prints `bilbo`'s synopsis, the list of verbs and a pointer to `bilbo --help`. The reason stays the first stderr line.
- An unknown verb near a real one gets `; did you mean '<verb>'?`.
- `bilbo help <unknown>` and `bilbo <unknown> --help` are usage errors.
- Every help line fits 80 columns. On a terminal, with `NO_COLOR` unset or empty and `TERM` not `dumb`, headings and the left column are bold; pipes and agents get plain text.
- The commands reference and the README describe the new help, and the reference's synopses match the pages.

## Non-goals

- No change to any verb's behavior, options, reasons or exit codes.
- No help parser library, no runtime wrapping to the terminal width, no pager, no man pages, no machine-readable help.
- No suggestion for an unknown option of a verb; only verbs get one.
- No `--color` flag, no `CLICOLOR_FORCE`, no color on stderr.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `cli`: Help prints an overview or a verb's page, accepts `bilbo help [<verb>]`, and is bold only on a terminal; Verb dispatch names the verbs through a short usage message and treats `help` as Help; Version's extra-arguments scenario no longer names "the usage message"; new requirements for the shape of usage errors and for verb suggestions.
- `library-ingest`: Stage a file names the state folder as the one `bilbo --help` names, since usage errors no longer print the path rules.

## Impact

- Code: `src/main.rs` (overview, page table, help and usage rendering, suggestions), and one `HELP` page beside each verb's parser: `src/note/{new,history,restore,scope,watch}.rs`, `src/search/{recall,index,digest}.rs`, `src/check.rs`, `src/library/cli/mod.rs`, `src/citation/cite.rs`, `src/sync/cli.rs`, `src/identity/device.rs`, `src/identity/pair/mod.rs`, `src/relay/mod.rs`, `src/setup/mod.rs`.
- Tests: `tests/cli.rs`, `tests/recall.rs`, `tests/library.rs`, `tests/setup.rs`.
- Build: `flake.nix` adds `docs/` to the source set, since a test reads the commands reference.
- Docs: `docs/reference/commands.md`, `README.md`, `AGENTS.md`.
- Callers that parsed the old usage text out of stderr: only the reason line is stable, as before.
