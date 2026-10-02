# Proposal

## Why

Search, the digest, citations, the library and the skills all depend on knowing what a note is and where it lives. Nothing in bilbo defines that yet. The legacy notes, written before bilbo existed, live in a folder on the maintainer's laptop. That folder is tied to how the maintainer manages their own tasks and notes: each note sits inside a per-task folder, and its frontmatter encodes choices of that setup (`kind`, `supersedes`, a date-only `created`). bilbo needs its own store and note format before any other change can build on them.

## What Changes

- A note store at `~/.local/share/bilbo/notes/` (`BILBO_HOME` overrides the root). It is one flat folder of `<kind>-<topic>.md` files, and a topic is unique across the store whatever its kind.
- A note format: frontmatter with `id` (a ULID), `created` (ISO 8601 to the minute, with a UTC offset) and an optional `sources` list, followed by exactly one `#` title. There is no `kind` key (the filename carries it), no `supersedes` and no `project`.
- Agents read and edit note files directly. The CLI never stands between an agent and a note.
- `bilbo new <kind> <topic>`: creates a note with a fresh id and timestamp at the right path, prints the path, and refuses a topic that already has a note.
- `bilbo check`: lints the whole store against the format, prints every problem it finds, and changes nothing.
- The first code in the repo: a Rust crate that builds the `bilbo` binary. This fills in `Stack` and `Verification` in `openspec/config.yaml`.

## Capabilities

### New Capabilities

- `cli`: how `bilbo` takes a verb, prints usage and help, and what its exit codes and output streams mean, for every verb.
- `note-store`: where the store lives, which entries are notes, and the exact shape of a note file. Agents, `bilbo new` and `bilbo check` all share this contract.
- `note-create`: `bilbo new`, which starts a correctly named and correctly formed note and refuses a taken topic.
- `store-check`: `bilbo check`, a read-only lint that reports every way the store breaks the `note-store` contract.

### Modified Capabilities

None. `openspec/specs/` is empty.

## Non-goals

- Scopes. The store has no `<scope>/` level for now. Adding one later only moves files, because citations will use ids.
- Sources and the library (`sources/`, `corpus`, ingest), id-based citations and `cite-check`. These belong to the library change.
- Search, the index, the digest, the daemon, hooks, skills and `bilbo init`.
- A config file. The kinds and store defaults are built in.
- Commands to read, edit, move or delete notes. Agents use their own file tools.
- Moving the legacy notes into the store. A later cutover change does that.
- Windows support.

## Impact

- New files in the repo: `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` and `.gitignore`. Once the code exists, `AGENTS.md` gains a description of the repo's structure and module design, so later agents extend the architecture instead of guessing at it. `openspec/config.yaml` gets its `Stack` and `Verification` lines.
- One runtime dependency, `jiff`, for the local UTC offset and calendar validation. Its justification is in design.md.
- The legacy notes (about 420) will fail `bilbo check` as they stand, because they carry `kind`, a date-only `created` and sometimes `supersedes`. The cutover change converts them. This change does not.
