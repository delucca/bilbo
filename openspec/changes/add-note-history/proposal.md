# Proposal

## Why

Agents edit notes in place with their own tools, and bilbo keeps no record of what a note said before. A careless rewrite loses a paragraph, and nobody notices or can get it back. Remote sync (the planning notebook's `design-bilbo-remote-sync.md`) also needs every note's past versions: a merge needs the version both sides started from. This change records that history locally, before any sync exists, so it is useful on its own and is the base the sync changes build on.

## What Changes

- `bilbo watch`: a long-running verb that watches `<root>/notes/` and records a version of a note each time it settles after a change. It records creations, edits, renames and deletions, keyed by the note's `id`, so a rename is the same note. Changes made while it was not running are recorded once it starts again. A second watcher for the same store waits until the first stops, and a missing or emptied `notes/` folder records no deletions.
- History lives under `<root>/.bilbo/history/`, beside the notes, because later changes treat it as data rather than cache. Each version is a full copy of the note. Identical content is stored once.
- History is pruned: versions older than `history.keep_days` (90 by default) are dropped. Kept are each note's newest version from before the cutoff, so the state as of the cutoff stays readable, and a deleted note's last content.
- `bilbo history <note>` lists a note's versions, newest first. `bilbo history <note> <version>` prints one version's exact text. `bilbo history <note> --diff <version> [<version>]` prints a unified diff between two versions, or between a version and the file now on disk. A note is named by its topic or its id, and a deleted note can still be named by its last topic.
- `bilbo restore <note> <version>`: writes a past version back into `<root>/notes/` as a new version. Nothing is lost on the way: whatever the file held, including an edit an agent makes while the restore runs, is recorded first. bilbo swaps the file in with an atomic exchange, then checks what came out. A restore that is killed leaves one hidden file, which the next restore or watcher scan records and removes.
- `bilbo setup` installs `bilbo watch` as a login service (`io.github.delucca.bilbo.watch` under launchd, `bilbo-watch.service` under systemd), reported on a new `watch` line. `--no-watch` and the wizard can skip it, and `--remove` removes it. The home-manager module gains `watch.enable`.
- New config key `history.keep_days`. `setup` keeps it on a rewrite, and the home-manager module accepts it.

## Capabilities

### New Capabilities

- `note-watch`: `bilbo watch`, covering what it records and when, what it skips and reports, a missing `notes/` folder, one watcher per store, and its pruning schedule.
- `note-history`: where history lives, what a version holds, what pruning keeps, and the `bilbo history` verb (list, print, diff).
- `note-restore`: the `bilbo restore` verb, including what an interrupted restore leaves and how it is cleaned up.

### Modified Capabilities

- `cli`: Verb dispatch adds `watch`, `history` and `restore`.
- `config`: Config location adds `watch` to the verbs that read settings. A new requirement, History settings, adds `history.keep_days`.
- `setup`: Setup flags and Modes add `--no-watch`. Step report and Remove gain a `watch` line after `timer`. Plan before writing names the watcher in the summary. Existing config file keeps the history settings on a rewrite. Home-manager module adds `watch.enable` and accepts the history key. A new requirement, Watch service, installs the service.

## Non-goals

- Sync, merging and conflicts. Without sync, a note has only one line of versions. The merge engine and the stale-base rule belong to the sync change, which extends `bilbo watch`.
- Reindexing on save. `bilbo index` keeps its timer.
- History for anything outside `<root>/notes/`. bilbo has no library yet.
- A history browser, blame, or a diff between two different notes.
- Redacting a secret from history. A pasted token stays in history until pruning drops it, and design.md says how to remove it by hand.
- Encrypting history at rest. It holds the same text as `notes/`, under the same disk protection.
- `bilbo check` reporting on history or on the watcher.

## Impact

- New verbs `src/watch.rs`, `src/history.rs` and `src/restore.rs`. The version store and pruning are a new library module, `src/versions.rs`. The line diff is another, `src/diff.rs`, which the sync change reuses for the section merge. `store.rs` gains the `.bilbo/history` path, and `note.rs` reads the id out of a file.
- An atomic file exchange, used by `restore` and later by sync's inbound writes, goes in a new library module, `src/swap.rs`.
- `src/timer.rs` gains a third job, `watch`, built like the local embedder's keep-alive service. `src/setup.rs` and `src/wizard.rs` gain the `watch` step, its flag and its prompt. `flake.nix`'s module gains `watch.enable` and the history key.
- `src/config.rs` gains `history.keep_days`.
- A version's id is a hash of the note's id, its parents, its file name and its content, so the sync changes can add fields without rewriting ids.
- New dependencies: `notify` 8.2 (file events) and `sha2` 0.11 (content hashes). `libc` gains a second user, `src/swap.rs`. The AGENTS.md dependency rule changes to match.
- Tests: `tests/watch.rs`, `tests/history.rs` and `tests/restore.rs`, which run `bilbo watch` as a child process against a temporary store. The fake `launchctl` and `systemctl` in `tests/common/fakes.rs` know only the index timer and the embedder service, so both gain a branch for the watcher.
- Migration: none. The first `bilbo watch` run records every note as `added`.
