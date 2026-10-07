# Design

## Context

See proposal.md for why. Today `src/main.rs` holds one `USAGE` constant; `run` prints it for any `-h` or `--help` before `--` (`wants_help`, with the `--title` and `--scope` value exceptions), and `report` prints it after every `Failure::Usage`. Each verb parses its own arguments by hand and returns `Failure::Usage(String)` from about 100 places, through `src/main.rs`'s dispatch, which already knows the verb it ran. `tests/layout.rs` allows only `main` to print and only `main` to reach a verb's module.

## Goals / Non-Goals

**Goals:**
- Each verb's page lives in the file that parses its arguments, so a new option and its help line change in one diff.
- Tests catch a verb without a page, a page line over 80 columns, a parser option missing from its page, an escape byte in piped help, and a docs link to a missing section.

**Non-Goals:**
- Changing how any verb parses or what it reports.

## Decisions

- **Pages are `pub const HELP: &str` in each verb's parser file**, written as raw strings at the end of the file's production code, above `#[cfg(test)] mod tests`: `src/library/cli/mod.rs`, `src/relay/mod.rs`, `src/identity/pair/mod.rs`, and the single verb files. `setup`'s parser, `src/setup/flags.rs`, is a private module, so its page sits in `src/setup/mod.rs` beside `setup::run`, rather than making `flags` public for one constant. Only `main` reads them, which `tests/layout.rs` already allows. Alternative: one help module in `src/`. Rejected: it fits no domain, and it splits each option from its line of help.
- **`main.rs` keeps the overview and the order.** `OVERVIEW` is a raw string; `PAGES` lists `(verb, page)` in overview order, and that order is also the `verbs:` line. Lookups (`help <verb>`, `<verb> --help`, the usage error) go through `PAGES`. Dispatch stays the `match`; a test compares its arms with the overview.
- **The synopsis is read from the page.** A page's `Usage:` block holds one form per line, wrapped by hand with a deeper indent; `forms` joins a wrapped form back into one line for the usage error. One source, no second list to drift.
- **`Failure::Usage` stays a `String`; `report` gets the arguments.** Every verb's usage error belongs to the verb `main` dispatched, which is `args[0]`; its subcommand, when it has one, is `args[1]` for `scope`, `sync`, `device` and nearly always `library`. `report(failure, args)` narrows the forms to those whose third word is `args[1]`, when `args[1]` is such a word, and shows all of the verb's forms otherwise (`bilbo library --depth 2 show` shows all seven). Alternative: carry the verb and subcommand in `Failure::Usage`. Rejected: it touches every one of the ~100 construction sites to say what `main` already knows. A usage error from collecting the arguments (not UTF-8) has no arguments and gets `bilbo`'s synopsis.
- **`bilbo help` is handled before `wants_help`.** `help` takes at most one verb, ignores `-h` and `--help`, and an unknown name in it is the unknown-verb error. `<unknown> --help` is the same error instead of the overview, so a typo is never answered with success.
- **Suggestions:** a name that prefixes exactly one verb suggests it; one that prefixes several suggests nothing; otherwise the single verb at the smallest Levenshtein distance, when that distance is at most 2 and below the name's length (so `re` or `x` suggest nothing). Ties suggest nothing.
- **Bold is two escape codes in `main.rs`.** `print_help` asks `styled(stdout is a terminal, NO_COLOR, TERM)` and wraps each line with `bold`, which marks a heading (a line opening with a capital letter up to its first `:` over letters, spaces and commas, when that ends the line or takes at most three words) or a left column (after exactly two spaces, up to the first run of two or more spaces). Both are pure functions with unit tests, since binary tests have no terminal. Alternative: the `console` crate, already in the tree through `cliclack`. Rejected: a direct dependency for two escape codes, and `tests/layout.rs` keeps `cliclack` to `host/prompt.rs`. No new dependency.
- **Docs links point to `Commands#<verb>`** on the wiki. Every verb has a `## <verb>` section in `docs/reference/commands.md`, whose title the wiki generator turns into the page `Commands` and whose headings GitHub turns into these anchors; guide links such as `What-your-agent-does-with-bilbo#the-digest` would pass 80 columns. A test reads the reference, so `docs/` joins the `lib.fileset` in `flake.nix`.
- **Exit lines say "usage or config error"** wherever a verb reads the store root or the config, since `Failure::Config` also exits 2; `relay` reads neither and says "usage error".
- **Synopsis fixes in the pages:** `library stage` gets two forms instead of `<url> | <file> --origin ...`, `library show` takes `<ref>`, defined once; `history` gets three forms; `relay` writes `--owner <fingerprint> [--owner <fingerprint>]...` as its spec does, since `--owner <fingerprint>...` reads as several values after one flag; `setup`'s placeholder becomes `<option>`. The commands reference takes the same forms.

## Risks / Trade-offs

- [A script parsed the old usage text from stderr] → only the first line was ever stable, and it is unchanged.
- [A page drifts from its parser's prose] → tests check the options, the shape and the widths, not the prose; the page sits in the parser's file so a reviewer sees both.
- [`library`'s options may come before its subcommand] → the usage error then shows all of `library`'s forms, still on the right page.
- [`docs/` in the fileset rebuilds the Nix package on a docs-only change] → accepted; the Nix job runs on every push already.
