# Proposal

## Why

The user works on several machines, and a note written on one is invisible on the others. Changes 1 to 3 laid the ground: per-note history (`add-note-history`), a `scope:` key that says which notes may leave the machine (`add-note-scope`), and device keys with signed per-scope manifests (`add-device-keys`). Nothing moves a note yet. This change makes `bilbo watch` sync every note whose scope syncs, through a folder the user already syncs with a cloud tool, without ever dropping either side of a concurrent edit. The design is the planning notebook's `design-bilbo-remote-sync.md`, as the user decided it on 2026-10-03.

## What Changes

- `bilbo watch` syncs. For every scope whose `scope.<name>.sync` is a URL, it pushes the versions of the notes whose `scope:` names that scope, and pulls the other devices' versions. A note with no scope, an undeclared scope or a scope that does not sync never leaves the machine, so the first sync uploads nothing until notes are assigned.
- The `file://` transport: a folder, usually inside Dropbox, iCloud Drive or Syncthing, holding a tree of encrypted, signed, create-only files. Each device writes only its own segment files, so the folder tool never sees two writers on one file. The tree is the one changes 5 and 6 serve over the relay.
- Timing: a version is pushed as soon as watch records it; other devices' segments are polled every `sync.poll_seconds` (30 by default).
- The merge engine. Two concurrent versions of a note merge three-way against their lowest common version, passage by passage, where a passage is recall's, at any heading level. Frontmatter merges key by key: `id` and `created` never change, `sources` merges as a set, and a `scope` clash goes to the side that shares less, flagged. The same passage changed on both sides is a conflict that keeps both: it is written into the file between markers and recorded on the merge version. Merge versions are labelled `merged`.
- Merges settle. A merge's id comes from its parents and its content, so two devices that merge the same versions get the same version. Heads with identical content are not merged.
- The stale-base rule. The first local save after sync wrote a note is merged against the version from just before that write, so an agent that saves over text it never read cannot revert it in silence.
- Every inbound write goes through change 1's atomic exchange. Whatever it swaps out is inspected, and an unexpected save is recorded after the version it was made from and merged three-way, on the stale-base rule's path.
- Edit beats delete, flagged. Two notes created offline with one topic: the younger id gets a `-<4 lowercase id chars>` suffix, flagged.
- Scope moves. Moving a note out of a scope pushes no content into the old scope, only a `left` marker, applied only as a fast-forward, and devices outside the new scope stop holding the note, with a visible line and a `check` warning on the moving device.
- Conflicts are resolved by an agent that removes the markers. Text it drops must be declared with `bilbo sync declare`; until then `bilbo check` reports it. `bilbo check` reports open conflicts, the digest labels conflicted and auto-merged notes, and a session's first digest names open conflicts even when nothing else matches, so an untouched note does not stay conflicted forever.
- Acks travel in segments. History pruning keeps every version another device may still need as a merge base; a device that leaves this device's segments unacknowledged for `sync.stale_days` (180 by default) stops holding pruning back.
- Manifests travel too. The watcher publishes this device's pending manifest versions, confirms them by reading them back (clearing `.pending`), calls change 3's `lose()` when another version wins, seals only under the usable epoch, never seals a key or enrolls a device on its own, and reports devices and epochs it did not add.
- New verb `bilbo sync`: per scope, the transport, the devices and their state, the notes that sync and the open conflicts. `bilbo sync declare <note> <reason>` declares dropped text.
- `bilbo setup` gains a `sync` step after `watch`. The wizard can turn sync on: it asks for the scope and the folder, runs change 3's recovery phrase flow when the device has no key, and writes `scope.<name>.sync`.

## Capabilities

### New Capabilities

- `note-sync`: what syncs and when, inbound writes, the stale-base rule, deletes, topic collisions, scope moves, joining a scope and convergence.
- `note-merge`: the three-way passage merge, frontmatter rules, conflicts and their markers, resolution and dropped text.
- `sync-transport`: the transport URL, the tree of objects, create-only writes, the segment envelope, ordering, acks and manifests on the transport.
- `sync-status`: the `bilbo sync` verb, its status report and `declare`.

### Modified Capabilities

- `cli`: Verb dispatch adds `sync`. Output streams lets setup's sync step draw the phrase ceremony on stderr.
- `device-identity` (from `add-device-keys`): Recovery phrase lets `bilbo setup`'s sync step create the phrase through the same ceremony; recover fetches scopes from a folder and heals a pending, unpublished fork.
- `scope-manifest` (from `add-device-keys`): Versions that init writes reads the transport and mints no version 1 while it holds this owner's scope and this device is in none there. A new requirement, Recover fetches scopes from the transport, has `bilbo device recover` copy the owner's chains from a `file://` folder before enrolling, so the phrase alone recovers a folder scope with every device lost; What recover writes then reports `unsealed` naming `bilbo device init` only when the folder holds no such scope. `add-relay` extends both to `https://`.
- `config`: Config location adds `sync` to the verbs that read settings. A new requirement, Sync settings, adds `sync.poll_seconds` and `sync.stale_days`.
- `setup`: Step report gains a `sync` line after `watch`. Plan before writing names sync in the summary. Existing config file lets the wizard write a scope's sync URL and keeps the scope and sync settings on a rewrite. Home-manager module accepts the sync keys. A new requirement, Sync step.
- `store-check`: new requirements, Sync conflicts (conflict, dropped-text and stray-marker lines) and A note that left its scope (a 30-day warning).
- `note-digest`: a new requirement, Sync state in the digest, labels conflicted and auto-merged notes and names open conflicts in a session's first digest. How many notes and The digest block make room for that line and the labels, and Session memory remembers that it did.
- `agent-plugin`: a new requirement, Resolving a sync conflict, tells the note skill how to resolve a conflict in the note it touched and when to declare dropped text.
- `note-watch` (from `add-note-history`): Watch leaves the notes alone allows sync writes, beside the restore-leftover sweep it keeps.
- `note-restore` (from `add-note-history`): Restore a version prints a line when the version leaves the current scope; Nothing is lost on restore keeps a pending stale-base entry; An interrupted restore leaves an interrupted sync write to the watcher.
- `note-scope` (from `add-note-scope`): Assigning never loses a write records the version `scope set` wrote and keeps a pending stale-base entry.
- `note-history` (from `add-note-history`): Where history lives adds `merged` and `left` versions, several parents and the recording device. Retention keeps merge bases other devices need, also for a device that has acknowledged nothing yet. Listing versions names the device and the flags.

## Non-goals

- The relay and the `https://` transport (change 6) and pairing (change 5). A scope whose URL is `https://` waits, reported, until change 6.
- Checkpoints and transport compaction. Nothing is deleted from the transport in v1, and a new device replays every segment. design.md sizes this.
- Syncing anything outside `<root>/notes/`. bilbo has no library yet.
- Purging a pasted secret from other devices and the transport.
- Sharing a scope with another person. Members are one owner's devices.
- Recall isolation by scope.
- Padding segments against size and timing analysis.
- Devices on different sync formats. A segment whose `format` is unknown is reported and skipped.

## Impact

- A new domain, `src/sync/`: `transport.rs` (the transport interface, picked by URL scheme, and its `file://` folder, including the pairing mailbox removal change 5 needs; change 6's `src/sync/remote.rs` implements it over `https://`), `segment.rs` (envelope, sealing, signatures, the plaintext format), `scopes.rs` (the owner's scopes on a transport: chains verified, names opened, one picked by name, for `bilbo device`, the wizard and the watcher), `replica.rs` (one scope's state, pull, push, acks, the outbox and its manifests), `integrate.rs` (applying other devices' versions: the inbox, heads, merges, inbound writes, the stale-base rule, deletes, topic collisions, scope moves) and the verb `cli.rs`, `bilbo sync`.
- The `note` domain gains `merge.rs` (the passage merge, markers, dropped text) and `conflicts.rs` (the open-conflict summary and the judging of open conflicts, drops and stray markers, read by `check`, `digest`, `bilbo sync` and `integrate`). `note/versions.rs` gains several parents, the `merged` and `left` events, the device, flag, conflict and drop fields, declaration lines, heads and the lowest common versions, a sweep that leaves staged inbound writes, and the pruning guard. `note/marks.rs` takes `check`'s `first_mark`, which the watcher's mark line needs too.
- `note/watch.rs` runs the sync loop. `note/history.rs`, `note/restore.rs`, `note/scope.rs`, `src/check.rs`, `search/digest.rs`, `shared/config.rs`, `shared/store.rs`, `identity/device.rs`, `identity/manifest.rs` and `setup/` (`plan.rs`, `apply.rs`, `wizard.rs`) grow as the deltas say. The heading scan moves out of `search/rank.rs` into `shared::markdown::headings`, so recall and merge cut passages alike. `flake.nix`'s module accepts the sync keys. `plugins/bilbo/skills/note/SKILL.md` gains the conflict steps and `Bash(bilbo sync declare *)`.
- `tests/layout.rs`: `sync` joins `DOMAINS`, `sync/cli.rs` joins `VERBS`, and `PLACEMENT` gains `base64` in `sync/segment.rs`.
- Dependencies: `base64` 0.23.1, already in the tree through `ureq`, becomes direct in `src/sync/segment.rs`. Encryption, signatures and randomness go through change 3's `src/identity/keys.rs`, which owns `chacha20poly1305` and `ed25519-dalek`.
- Tests: `tests/sync.rs` runs two stores, each with its own `BILBO_HOME`, state folder and `bilbo watch` child, against one temporary `file://` folder, with `sync.poll_seconds = 1`, and covers `bilbo sync`. The merge is unit-tested in `src/note/merge.rs`, the engine in `src/sync/`. `tests/check.rs`, `tests/digest.rs`, `tests/history.rs`, `tests/restore.rs`, `tests/scope.rs`, `tests/watch.rs`, `tests/device.rs`, `tests/cli.rs` and `tests/setup.rs` gain the deltas' scenarios.
- Migration: none. Existing notes have no scope, so nothing is pushed until the user assigns them.
