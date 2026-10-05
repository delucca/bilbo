# Design

## Context

- Change 1 (`add-note-history`) records only valid `<kind>-<topic>.md` names, re-hashes under `history/lock` before recording, records nothing when `notes/` cannot be listed or lists no note, and lets a second watcher wait on the lock. Inbound writes take the same lock, so they never race a scan. It gives every note a log of full-snapshot versions under `<root>/.bilbo/history/`, ids from parents, file name and blob, a `bilbo watch` that records saves after a 2-second settle (`note/watch.rs`), `host/swap.rs` (`exchange`, `rename_new`) and `note/diff.rs` (a Myers LCS over any `Eq` slice), with the version store in `note/versions.rs`. Everything below extends those.
- Change 2 (`add-note-scope`) adds `scope: <name>` to the frontmatter and `scope.<name>.*` to the config (`shared/config.rs`, where `Scope.sync` is the URL or `off` as written), and `bilbo scope set` (`note/scope.rs`). A note with no key, or an undeclared name, is unassigned and local.
- Change 3 (`add-device-keys`) adds the recovery phrase, the owner key, device keys under `<state>/bilbo/keys/`, and per-scope signed manifests with epoch keys, kept locally under `<root>/.bilbo/scopes/<scope id>/`, in the `identity` domain: `identity/keys.rs` (`encrypt`, `decrypt`, `verify`, `random`, `SignKey::sign`), `identity/manifest.rs` (`verify_scope`, `read_scope`, `survey`, `usable_epoch`, `adopt`, `confirm`, `lose`, and the per-scope steps of `init` and `recover`), `identity/ceremony.rs` (the phrase screens) and the verb `identity/device.rs`. A new scope lists the devices every other opened scope of the owner lists, minus any device one of them dropped; a version's sealed name is bound to its number `n`. It has no transport: `usable_epoch`, `adopt`, `confirm` and `lose` carry `cfg_attr(not(test), expect(dead_code))` until this change calls them.
- The binding decisions are in the notebook's `design-bilbo-remote-sync.md`, "Decisions taken (2026-10-03)", and in `work/sync-spec/CONTRACT.md`. This design does not reopen them.
- Passages: `rank::passages` (`search/rank.rs`) splits a body at every ATX heading outside fences, at any level, then cuts passages over 4,000 bytes into parts. The heading scan, built on `shared::markdown`'s `fence_run`, `outside_fences` and `heading`, is the part the merge reuses.
- Size: all of `~/Notebooks` is 13.7 MB, but 1,181 of its files are sources, which bilbo does not hold yet. The notes are 440 files of a few MB. Change 1 estimates history at about 0.5 MB a day for a heavy user.
- `bilbo check` is read-only and prints `<path>: <message>` lines (`src/check.rs`). `bilbo digest` (`search/digest.rs`) must finish in 1.5 s and never touch the store (`note-digest` spec).
- The layout rules (AGENTS.md, checked by `tests/layout.rs`): one folder per domain with its verbs inside; verbs never use each other and only `main` uses a verb; domain code forms no cycle (today `search` uses `note` and `library`, `identity` uses `host`); `shared/` holds only what two domains use; `foo/foo.rs` is refused (`module_inception`), and each crate stays in the files `PLACEMENT` lists.

## Goals / Non-Goals

**Goals:**
- No silent loss: every byte any device recorded stays in some version, and every merge that drops nothing it should keep is labelled; every conflict is visible until resolved, and every drop until declared.
- Convergence without coordination: devices that hold the same versions compute the same merges, byte for byte.
- A transport the user already has: a folder their cloud tool syncs, with no segment ever written by two devices; manifests are handled apart.
- Nothing leaves a device unless its note is assigned to a scope that syncs.

**Non-Goals:**
- Real-time sync. Seconds to a minute is fine; the hook reads local files.
- Merging inside a passage. A passage edited on both sides is a conflict, by the user's decision.
- Checkpoints and transport compaction (see "No checkpoints in v1").

## Decisions

### Module layout

Verbs build on library modules, never on each other (AGENTS.md), so the sync engine is library code that `watch` drives. It is a new domain, `src/sync/`, because it joins `note` (history, merge) with `identity` (keys, manifests) and neither may hold it: `identity` must never use a domain that uses it, and `note` already sits below `search`.

- `note/merge.rs` (new): splits a note into units, three-way merges them, writes and parses conflict blocks, finds dropped lines. Pure functions over bytes. It uses `note/diff.rs`'s LCS and the heading scan, which moves out of `rank::passages` into `shared::markdown::headings`, so recall and merge cut passages identically outside conflict blocks. Inside a block they differ: merge keeps the block as one unit, recall still splits at the headings of its sides, which lets recall find either side's text. It lives in `note` because it is about notes and both `sync` and `conflicts` use it; `search` already uses `note`, so `note` cannot reach into `search/rank.rs`, which is why the scan goes to `shared/markdown.rs` beside `heading` and `outside_fences`, now used by `search` and `note`.
- `note/conflicts.rs` (new): reads and writes the open-conflict summary, and judges open conflicts, drops, declarations and stray markers from it, the note's log and the file now. `check`, `digest`, `bilbo sync` and `integrate` call it.
- `sync/segment.rs` (new): the envelope, sealing and opening, the plaintext format. The only user of `base64`; the AEAD and signatures stay in `identity/keys.rs`.
- `sync/transport.rs` (new): the transport interface and its `file://` implementation: `list_devices(scope)`, `list_after(scope, device, seq)`, `get(path)`, `create(path, bytes)` returning created, already-exists or full (its own failure, with the transport's message, such as change 6's `relay <url> is full for scope <name>` or a full disk), `highest_manifest(scope)`, and `remove_mailbox(nameplate)` for change 5, which removes `pair/<nameplate>/` on `file://` (the one deletion in the transport, and only under `pair/`) and does nothing on `https://`. Change 6's `sync/remote.rs` implements the same interface over `https://`, picked by the URL's scheme, and pages its listings internally. Nothing in the interface deletes under `scopes/`.
- `sync/scopes.rs` (new): the owner's scopes on a transport: list `scopes/`, verify each chain with `manifest::verify_scope`, open names with the phrase's box key or this device's key, pick one per name, and tell whether the transport holds a scope of this owner that this device cannot open. `bilbo device recover` and `init` (`identity/device.rs`), the setup wizard and the watcher call it. The verbs join it with `identity::manifest`; `identity` itself never uses `sync`.
- `sync/replica.rs` (new): one scope's sync state, its pull and push of segments over the transport, acks and the outbox. It returns events (lines to print) and never prints.
- `sync/manifests.rs` (new): one scope's manifests on the transport: publishing and confirming this device's pending versions, adopting others' through `identity::manifest`, losing a fork, the pin and the stop lines, and `changes.jsonl`. It returns events and never prints. Apart from `replica.rs` because it moves manifests, not notes, and `replica.rs` only reads the local manifests it leaves. It also computes, from `seen.jsonl` and the acks, the known heads the pruning guard keeps, which `watch` passes to `versions::prune`: `note` never uses `sync`.
- `sync/integrate.rs` (new): applying verified versions to the notes: the inbox, heads, merges, inbound writes, `record_local` and the stale-base entries, deletes, topic collisions and scope moves. It returns events and never prints. `bilbo restore` and `bilbo scope set` call its stale-base and staged-version functions; `versions::sweep_restore_leftovers` takes the staged versions as an argument, so a staged inbound write is left for the watcher without `note` using `sync`.
- Extended: `note/versions.rs` (record fields, events, declarations, heads, the lowest common versions, the pruning guard), `note/marks.rs` (`first_mark`, moved from `check.rs` so the watcher's mark line uses it too), `note/watch.rs` (the loop), `note/history.rs`, `note/restore.rs`, `note/scope.rs`, `src/check.rs`, `search/digest.rs`, `shared/config.rs`, `shared/store.rs` (`sync_dir`), `identity/device.rs`, `identity/manifest.rs`, `setup/`.
- New verb: `sync/cli.rs`, `bilbo sync`. A verb cannot share its domain's name (`sync/sync.rs` fails `module_inception`), so it follows `library/cli/`.

### Version records

Change 1's id rule stands: SHA-256 over `bilbo-version-1`, the note's ULID, the sorted parent ids, an empty line, the file name and the blob hash or `deleted`, one per line. Readers skip fields they do not know. New fields sit outside the id:

```json
{"version":"…","parents":["…","…"],"file":"plan-release.md","blob":"…","event":"merged","at":"2026-10-03T14:23:05-03:00",
 "device":"<device id>","outside":["…"],"flags":["stale-base"],
 "conflict":[{"passage":"Release > Rollout","sides":["…","…"]}],"dropped":[{"passage":"Release > Rollout","lines":["…"]}]}
```

- `device`: the device that recorded it, written on records that arrive through sync. A version recorded here carries none: it is this device's.
- **No per-record signature in v1.** The contract allowed one; the segment is already signed by its writer, and two devices computing one merge would sign it differently, leaving the receiver to pick. A later checkpoint format that carries other devices' records can add one under `format` 2.
- **Where a version was seen** is not on the record, because change 1 appends a record once and never rewrites it. `<root>/.bilbo/scopes/<scope id>/seen.jsonl` holds one line per version and blob as it is pushed or applied: `{"version":…,"device":…,"seq":…}`. The pruning guard and the closure rule read it; Resuming after lost state rebuilds it.
- `outside`: parents the receiver must not wait for (see "What a scope's log holds").
- `flags`: `edit-beat-delete`, `created-kept`, `key-kept`, `stale-base`, `topic-taken`, `scope-clash`. `bilbo history` also shows `conflict` for a version with a `conflict` field and `dropped` for one with a `dropped` field.
- `conflict`: per conflicting passage, its heading path and the side versions. `dropped`: on the first version without a conflict's blocks, the dropped lines per passage. Both are a function of the parents and the bytes, so devices agree on them; they stay out of the id because the bytes already fix them.
- New events: `merged` (two or more parents) and `left` (see "Scope moves"). The contract also allowed `synced`; it is not needed, because a version that arrives keeps its author's event and gains `device`.
- A second line type in a note's log, `{"declare":"<conflict version>","reason":"…","at":…,"device":…}`, records `bilbo sync declare`. Pruning drops it with its conflict version.

### The merge unit

- **Units:** the frontmatter keys, the text before the first heading, and one unit per heading outside fences, running to the next heading of any level. This is recall's passage before the 4,000-byte split. The split is left out because it moves with byte counts, and a cut would turn one edit into two.
- **Matching across versions:** a unit's key is its own heading line (level and text) plus its rank among units with that heading; the heading path only breaks ties between equal keys. So renaming a parent `##` changes no child's key. Keys match across base and sides regardless of order.
- **Renames:** a base unit with no match on a side is that side's unit renamed when exactly one new unit of the same level sits between the same matched neighbours there, or, failing that, when a new unit holds the identical body. Both tests are positional or exact, so no threshold decides. A rename with an edit on one side and an edit on the other is therefore one unit changed on both sides: a single conflict block, not two copies.
- **Order:** when only one side reordered units, its order wins. When both did, the side whose head id sorts first wins, and the other side's new units follow the unit that preceded them on their side.
- **Rules:** as in the `note-merge` spec. A conflict block keeps every side; a side that deleted the unit is an empty side.
- **Marker blocks are units.** While splitting, a line `<<<<<<< bilbo <12 hex> <time>` outside a fence opens a block that runs to the line `>>>>>>> bilbo`, and headings inside it do not split. Only the full marker forms count, and never inside a fence, so a quoted example is text. A later merge treats the block as one unit, so a side that resolved it on one device and a device that left it alone merge cleanly, and a third edit joins the block as another side instead of nesting.
- **The file name:** the base's, unless one side renamed (that side's) or both did (the side whose id sorts first), as the `note-merge` spec's The file name says. The name is part of the id, so this rule is what lets two devices agree on a merge of a rename.
- **More than two heads:** merged two at a time, in id order, each step recorded. Every device runs the same steps on the same heads and gets the same ids.
- **Several lowest common versions** (criss-cross merges): the base is built from them, holding only the units and keys all of them hold identically. Picking one of them could drop text: if the chosen one holds a passage the true common version had deleted, and one side re-added it identically while the other never had it, the passage reads as deleted by the other side. The built base turns that into an addition, which is kept.
- **No common version held:** an empty base, as the spec says.

Alternatives considered:
- **A line merge** (the Opus design). Agents write one line per paragraph, so line adjacency is arbitrary, and two contradicting edits of one passage would merge in silence. The user picked the passage.
- **Never merge** (astra, sol). Every concurrent append to a busy note becomes agent work, and every agent resolution can drop text.
- **Delete against edit of a passage as a conflict** (Fable). The note-level rule is "edit beats delete"; applying it to passages too means fewer resolutions. The cost: a deliberate deletion of a passage comes back when the other device edited it, flagged.

### Frontmatter

- `id` never changes; a version holds the id it was recorded under.
- `created` keeps the base's value. With no base, the value of the side whose id sorts first.
- `sources` is a three-way set merge. The lead's brief and the panel say "set union"; a plain union would bring back a source one side deliberately removed, so additions are unioned and a removal sticks when the other side kept the item. This is a decision for the user to confirm.
- `scope`, by the spec's clash rule. Determinism holds for a simpler reason than it looks: a side naming a scope that does not sync exists only on the device that wrote it, so no two devices ever merge it, and when both sides name syncing scopes the result is no key on every device. Alternative: rank by each manifest's device count. Manifests change over time, so two devices could rank differently.
- Other keys (invalid ones; `note-store` allows none) take the side whose id sorts first, flagged `key-kept`. Markers are never written in the frontmatter: `note::read` would reject the note, and the watcher would stop recording it.

### Conflict markers

```
<<<<<<< bilbo 3f9a2c1b0d4e 2026-10-03T14:23-03:00
## Rollout

Ship on Monday.
======= bilbo 9c8d7e6f5a4b 2026-10-03T14:25-03:00
## Rollout

Ship on Friday.
>>>>>>> bilbo
```

- Git-like, because agents already read git markers. Each side is labelled where it starts, so a third side fits without nesting.
- The labels hold the version id and the time it was recorded, both fixed by the record. Device names are left out because a renamed device would change the bytes of the merge on one device and not another, and two merges of the same versions must be identical. `bilbo history` names the devices.
- No base section (diff3 style): it doubles the block, and the base is one `bilbo history --diff` away.

### Resolution and dropped text

- An agent resolves by editing the file. The watcher records the save. When a version has none of a conflict's blocks, `note/merge.rs` compares the conflict's sides with the whole new body. A side's non-blank line, whitespace-collapsed, that appears nowhere in the new body is dropped. The version records the dropped lines.
- Line granularity, exact match: a paraphrase counts as a drop. That is noisy on purpose: the panel's finding is that a tidy-up by an agent is where content is lost, and a declaration costs one command.
- `bilbo sync declare <note> <reason>` appends a declaration that names the conflict, not the resolving version, so it works before the watcher records the save, and survives a second edit of the resolution.
- `note/conflicts.rs` judges a note from three inputs: the open-conflict summary, the note's log, and the file now. A file without the blocks is judged as if recorded, so `check` right after an edit already reports the drop instead of the conflict. That is what the note skill needs, since it runs `check` right after it writes.

Alternatives considered:
- **Declaring in the file**, as a comment where the text was. It needs no command, but leaves bilbo's bookkeeping in the note forever.
- **A `bilbo resolve` that writes the file.** Agents edit files with their own tools everywhere else; a second path invites them to bypass it.
- **A fuzzy match** (most words of a line kept). It hides real drops behind a threshold.

### The open-conflict summary

The watcher keeps `<root>/.bilbo/sync/open.json`: per note, open conflicts, undeclared drops, whether the latest version is an auto-merge, and the flags recorded in the last 7 days, which `bilbo sync` prints as notices. It rewrites it under the history lock, through a temporary file and a rename, whenever a merge, a resolution or a declaration is recorded. `digest` reads only this file, so it stays within its budget. `check` and `sync` read it to find candidates and then judge each candidate's file as it is now. A stale summary can only list a note that is already resolved, which the file check corrects; a missing entry cannot happen, because the entry is written under the same lock as the conflict version.

### The stale-base rule and inbound inspection: one path

- Every sync write to a note stores, in `<root>/.bilbo/sync/stale-base.json`, the version the file held just before the write, unless an entry already exists. So after two sync writes, the base is the version from before the first.
- **`record_local(note, bytes)`** is the one way a local change enters history when such an entry exists. It merges the bytes against the stored base, with the latest sync-written version as the other side, and deletes the entry. When the merge equals the bytes, they are recorded as `edited` after the sync-written version: the agent had read it. Otherwise they are recorded after the stored base, which is where they came from, and a `merged` version flagged `stale-base` follows both; its bytes are written as an inbound write.
- The scan calls it for the first local save after a sync write. The inbound write calls it for bytes the exchange takes out that differ from what the file held before the write (below): those bytes were saved against that version, which the entry, set before the swap, names.
- A local delete counts as a save. Against a sync write that edited the note, edit beats delete, and the note comes back once, flagged. A second delete sticks.
- **Verb writes are not saves.** `bilbo restore` and `bilbo scope set` record their own version under the lock, so the scan sees the file equal the head. When an entry exists, they keep its base and make their version its written version, so an agent's later save from its stale read is still merged against the base it read, with the verb's result as the other side. Restore records an unrecorded save through `record_local` when an entry exists. This MODIFIES change 1's `note-restore` Nothing is lost on restore and change 2's `note-scope` Assigning never loses a write. Before, `scope set` spent the entry and `restore` cleared it, and the agent's next stale save reverted the synced edit in silence.
- **Restore across a scope:** when the restored version's `scope` differs from the current file's, restore prints a stderr line naming the scope it leaves and how to keep it, and still writes the version, per the user's "freely editable" decision.
- The entry survives a restart of the watcher, so a save made while it was down is still caught.

Alternative: the evaluator's time window (merge any save within N seconds of a sync write). Any N is wrong for some agent; the rule above needs no clock.

### Inbound writes

All under `history/lock`, so scans and `restore` wait.

- **Staged, then recorded.** A cycle verifies pulled records and stores their blobs, but keeps the records in `<root>/.bilbo/sync/inbox.jsonl`, not in the notes' logs. Heads for writing are computed over the logs plus the inbox, but a local save's parent is always the log head, the latest version the file held, never a staged one; a staged version then merges with the save as any concurrent head does, so a failed or refused write can never make a save revert it. Each touched note's file is written once, from its heads, and only after its write succeeds are the note's staged records appended to its log. So history never names an inbound version as the head of a file that does not hold it, and a crash between the hidden write and the swap cannot read as a deletion. A device joining with a year of versions writes each file once.
- **An interrupted inbound write:** a `notes/.bilbo-restore-<id>` whose bytes equal a staged version is completed by the watcher's sweep (swapped or renamed into place, then recorded), never recorded as `edited`. `bilbo restore`'s sweep leaves such a file for the watcher. This MODIFIES change 1's `note-restore` An interrupted restore.
- **No writes without `notes/`:** while change 1's missing-folder guard is active, inbound writes wait in the inbox; `rename_new` would otherwise recreate `notes/`.

- **Before the swap:** re-hash the file. When its bytes are not the head's, an agent saved since the last scan: `record_local` records them first, so the head is now the agent's save. Then compute what to write from the heads (fast-forward or merge), and set the stale-base entry to the version the file holds now, `H`.
- **The note has a file:** write the new bytes to `notes/.bilbo-restore-<id>`, the name change 1's restore uses, so change 1's sweep finds it after a crash. `swap::exchange` it with the file at its current name, then, for a rename, `rename_new` the file to the new name, the order restore uses, so two visible files never hold the id. Read what came out.
- **What came out:** bytes equal to `H` are expected. Other bytes are a save that landed during the swap, made against `H`: `record_local` records them after `H` and merges them three-way with what was written, base `H`. Nothing can be recorded after the inbound version by mistake, because its parent is `H`, never the written version. The loop is bounded at 5 rounds; past that, the watcher leaves the agent's file, records it, and retries at the next cycle.
- **No file yet:** `rename_new`. When it fails because a file appeared, read it: the same id is an agent's write (`record_local`), another id is a topic collision.
- **A deletion:** rename the file to `notes/.bilbo-restore-<id>`, then read it. Unexpected bytes go through `record_local` and win over the deletion.
- The scan that the swap's own events trigger waits on the lock the writer holds, and by then the written version is recorded, so it sees the file equal the head and records nothing.
- A filesystem that refuses the swap stops inbound writes for the store (the spec's line) and leaves pushing alone. The unit tests reach it through an injected exchange that fails, since no test filesystem refuses.
- The race test ("An agent writes during the swap") runs as a unit test of `sync/integrate.rs`, with an injected hook between the read and the exchange, as change 1 tests restore. No environment variable reaches the binary.

### Topic collisions

- The suffixed name is still a valid `<kind>-<topic>.md`, so change 1's watcher records it.

- The younger id is the one that sorts later. The suffix is the id's last 4 characters, lowercased. The first 4 encode the creation time and are often the same for two notes made the same week; the last 4 are random. Lowercase Crockford base32 fits the topic grammar `[a-z0-9]+`.
- The rename is a `renamed` version with the note's head as parent and the new name, so every device where the suffixed name is free computes the same id. A device where a local note already holds that name adds characters and records a different rename; the next sync merges the two renames (both renamed: the id that sorts first wins), so they converge after one `merged` version. It is pushed like any version.
- The same rule applies when the older note is local only: the incoming younger note is renamed. When the younger is the local-only note, only this device renames it.

### What a scope's log holds

- A version goes into scope S's log when its own bytes say `scope: S` and S syncs on this device. It is pushed with the versions it needs: its ancestors whose bytes also say S and that are not yet in S's log, as far as this device knows. Parents whose bytes say anything else, and parents this device can no longer push because pruning dropped them, are listed in `outside`, so no receiver waits for them.
- So assigning an old note pushes one version, its current text. The unassigned versions before it never leave, and neither do versions from another scope.
- A receiver applies a version once every parent not in `outside` is known. A version whose parent has not arrived waits, for instance while a cloud folder is still downloading another device's segment. `bilbo sync` lists waiting versions. After `sync.stale_days` a waiting version is applied with what is known, falling back to an empty base if needed.
- Blobs are pushed once per scope log, as far as this device knows.

### Scope moves

- When a version M moves a note out of S, S's log gets a `left` record: M's id, its parents and `at`, with no file name, blob or new scope. M itself goes to the new scope's log, when that scope syncs. A later version whose parent is in S's log but whose own scope is not S also sends a `left` to S.
- **A `left` is applied only as a fast-forward:** when it follows the device's head for the note. Then the device removes the file (an inbound deletion, so an agent's unexpected write still wins), records the `left`, keeps the history, and prints the spec's line. A `left` concurrent with a local head is recorded and never merged, so it can never start an edit-beats-delete round. Edit beats delete applies to `deleted` only.
- **Every case of the spec still ends with the file gone where it should be:**
  - A device in S that did not edit concurrently holds M's parent as its head, so `left`(M) follows it.
  - A device in S that edited concurrently (E) keeps the file. E reaches a device holding S and the new scope, whose merge M2 has parents M and E, lands in the new scope, and sends `left`(M2) to S. `left`(M2) follows E, so the editor then removes the file. E's text lives on in M2.
  - The B2 sequence of the review (A moves the note to `shared`, B to `work`, which does not sync): B's merge C2 = merge(C1, M) keeps `work`, never leaves B, and sends `left`(C2) to `shared` because its parent M is there. On A, `left`(C2) follows M, so A removes the file and stops. B's `left`(C1) to `personal` is an ancestor of C2 on any device that sees both. Nothing is merged again, so nothing loops.
- **A note that comes back:** a device whose only head for the note is a `left` applies any later version as a fast-forward, even when its parents are `outside`, because it holds no text that could be lost.
- A device that holds both S and the new scope gets the full M under the same id through the other log and ignores the `left`.
- Dropping the `scope` key by accident therefore removes the note from the other devices. The other options were to keep a stale copy on every other device (scope-assignment design B), which leaves copies the owner meant to withdraw, or to ignore a dropped key, which breaks "sync obeys what the key says now". The user confirmed this decision. To make it visible, the moving device's `check` warns that the note left the scope for 30 days, besides change 2's missing-scope line, and the other devices print a line when they remove the file. Every device keeps the history.

### The `file://` transport

- **Create:** write `.<device id>-<16 lowercase hexadecimal characters>.tmp` in the target folder, `fsync`, then `swap::rename_new` onto the final name. When the name exists, `create` reports already-exists; the caller then `get`s the object and compares bytes (the crash-retry case in "The segment format"). Readers ignore names outside the layout, which covers these temporary files and cloud tools' conflict copies.
- Folders are created as needed below the transport root, mode 0700; files mode 0600. The root itself is never created by `watch`: an unmounted volume must not become an empty local folder.
- **List:** `read_dir` of `scopes/<id>/devices/` finds new devices. For a known device, a poll probes `<cursor+1>.seg`, then the next, until one is missing, instead of listing a folder that grows by tens of files a day. `list_after` lists the folder only on a first read, after a resume, or when a probe finds a gap.
- **Single writer:** only `devices/<device id>/` is written by one device. Manifests are not: any device holding the owner signing seed writes them (see "Manifests on the transport").
- **Temporary names:** every temporary file a device writes on a transport is `.<device id>-<16 lowercase hexadecimal characters>.tmp` in the target folder, so in a multi-writer folder such as `manifest/` each device sweeps only its own, older than an hour, at the watcher's start. Local temporaries outside `history/` (`open.json`, `state.json`, `seen.jsonl`, `inbox.jsonl`) use change 1's form, `.tmp-<random>` beside the file, and the watcher's start-up sweep, which change 1 runs for `history/**/.tmp-<random>`, covers `<root>/.bilbo/sync/` and `<root>/.bilbo/scopes/` too.
- **Damaged own segments:** a cloud tool can truncate a file or put a conflict copy's bytes under the original name. Create-only forbids fixing that on a relay, which verifies on upload and keeps the first; on `file://`, a device replaces a file in its own folder that does not verify as a segment it signed, with its outbox copy, through a hidden temporary name and `rename`. Readers stuck on that seq pick it up at their next poll.
- **Cloud tools:** iCloud may evict a file to a placeholder that downloads on read. A read that fails or times out is a failed poll, retried with backoff. Syncthing and Dropbox may deliver a device's segment 5 before segment 4; reading stops at the gap and resumes when it closes.

### The segment format

The envelope is in the `sync-transport` spec, because the relay (change 6) reads its plaintext fields. Here is the rest:

- **Encodings:** `nonce`, `ciphertext` and `sig` in padded standard base64 (RFC 4648 section 4). `epoch` is the manifest's epoch number.
- **Nonce:** 24 random bytes from `identity::keys::random`. XChaCha20's 192-bit nonce makes random nonces safe at any segment count bilbo will reach.
- **Plaintext:** JSON, `{"format":1,"at":"<RFC 3339>","records":[…],"blobs":[{"hash":"…","text":"…"}],"acks":{"<device id>":<seq>},"declarations":[…]}`. Records are the version lines above plus `note`. A blob that is valid UTF-8, as every note is, travels as `text`; any other as `data` in base64. So the ciphertext's base64 is the only expansion, about a third, not base64 twice. `at` is the writer's time, shown in `bilbo sync` and never used to decide staleness.
- **Size:** at most 8 MiB per file, half the relay's 16 MiB object cap (change 6); a large push is split by records, never inside a blob. A note over 1 MiB is never recorded (`note-watch`), so a blob always fits.
- **Verification on read:** some confirmed version at the segment's epoch lists the device, and the latest confirmed version lists it (a device the latest drops is read only up to the seq applied when that version was adopted, so a revoked device that keeps writing under its old epoch is not applied). Several versions can share an epoch, since adding a device does not rotate it, so "the manifest of the epoch" would be ambiguous; the signature verifies; the plaintext decrypts; every record's id matches its parents, file and blob (except `left` records, which carry no content); every blob matches its hash; every record's scope matches the log's. A record that fails is reported and skipped; a segment that fails stops that device's reading.
- **Crash safety:** a segment is written to the store's outbox, `<root>/.bilbo/scopes/<scope id>/out/<seq>.seg`, before it is created on the transport. After a crash, the same bytes are created again; an existing object with the same bytes counts as done. An existing object that verifies as this device's segment but that this store neither wrote nor read means a second writer (the spec's "other content" line); one that does not verify is damaged (above).
- **Resuming after lost state:** with no `state.json` for a scope, as after `<root>/.bilbo/` was removed, the watcher lists every device folder, its own included, reads every segment from seq 1 (its own segments tell it which versions and blobs are already in the log), rebuilds `seen.jsonl`, copies the manifests back, and pushes from its highest seq plus one. A store that resumes from a folder whose listing still lags can later meet one of its own older segments it never read; the line then reads as a second writer. Waiting for two polls with an unchanged listing before the first push narrows that.

Alternatives considered:
- **A binary framing** instead of JSON plus base64. It saves the 33% base64 overhead, but JSON can be inspected and needs no new code.
- **Compression.** Notes are small text; it can be added under `format` 2.

### When watch pushes and polls

- **Push:** right after a scan records versions in a syncing scope, and after a merge. The first push of a note that holds a mark of another declared scope (change 2's Scope marks) prints one stderr line naming both scopes, so the warning `check` would give reaches the watch log even when nobody runs `check`. The scan already waits for 2 quiet seconds (at most 10), so a burst of saves is one segment.
- **Poll:** every `sync.poll_seconds`, 30 by default. A poll lists the device folders and reads past each cursor: a handful of `read_dir` calls, cheap even for a cloud folder. After a failure, the wait doubles, up to 10 minutes, and resets on success.
- **Each cycle starts fresh:** at the start of every sync cycle the watcher reads the config, the device keys and the local manifests again. They are small files, and this makes `bilbo device init`, `recover`, `revoke`, pairing (change 5) and config edits take effect within one poll, with no signal to the running process. A config that no longer parses keeps the last good settings and prints the error once.
- Alternative: notify events on the transport folder. Cloud tools write through placeholders and temporary names, and their events say nothing a 30-second poll does not. The relay (change 6) can add a long poll later.

### Acks, staleness and the outbox

- Every segment carries `acks`: the highest seq applied per other device.
- **Ack-only segments:** written at most once an hour, and only when a segment holding versions was applied since this device's last segment. An ack-only segment never causes another one, so two idle devices go quiet.
- **Stale device:** one that has left unacknowledged, for more than `sync.stale_days` by this device's clock, a segment holding versions that this device wrote. Only this device's own segments count, timed by when it wrote them, so another device's clock, however wrong, cannot make anyone stale. A device that is online acks within an hour, so only a device that is away goes stale. Its acks stop holding back pruning and the outbox. When it comes back it reads everything, since nothing is deleted, and its edits merge with an empty base wherever their base was pruned.
- **`seen.jsonl` is trimmed** at each prune: a line whose version no log holds and no device that is not stale still needs is dropped.
- **Outbox:** a device keeps its own segments until every device that is not stale acked them, and, on `file://` only, recreates one missing from the folder. A relay stores each segment durably and accepts only the next seq, so a stored segment is never created there again. If a segment is missing and the outbox no longer holds it, the scope's history on the transport is broken; the watcher stops pushing that scope and prints why. Recovering from that is out of scope.

### The pruning guard

- For each device D of a scope that is not stale, and each note, D is known to hold every version that `seen.jsonl` places in a segment D acknowledged, or in D's own. D's newest such version, by the DAG, is D's known head. A D that has acknowledged nothing yet, such as a device that joined, replayed and closed before its first ack, has no known head: it holds back every version of the note until it acks or goes stale.
- Retention then keeps D's known head and every version that follows it, on top of change 1's rules. `sync/replica.rs` computes the known heads per note and `watch` hands them to `versions::prune`, which keeps them and their descendants and knows nothing of devices. D's next edit follows something at or after its known head, so the common version stays in history.
- Keeping all descendants matters: dropping versions between D's known head and the current head would cut the parent chain the lowest-common-version search walks.
- Alternative: keep every pruned version's log line, without its blob, forever, so the chain never breaks. It keeps history files growing without bound, and the guard keeps exactly what a merge needs.

### No checkpoints in v1

- The transport only grows. A year of heavy use is about 0.5 MB a day of versions, roughly 180 MB, plus base64's third on the ciphertext: about 240 MB. A device that joins reads all of it once. XChaCha20-Poly1305 and JSON parsing run at hundreds of MB/s, so the replay takes seconds to a minute, after which the device prunes its history to `history.keep_days`.
- Checkpoints would need the panel's fix (each head's merge base) and a way to delete segments every device has applied, which in a cloud folder means a second writer deleting another device's files. That belongs in a change with `format` 2 and, for the relay, with change 6's storage limits.

### Manifests on the transport

- **The watcher moves manifests; it never decides membership.** Enrolled devices hold the owner signing seed but not the owner box secret (change 3), so no device can open `sealed.owner` on its own. A device gets an epoch key only when a member device seals it (pairing (change 5), change 5) or through `bilbo device recover`, which derives the box secret from the phrase for the length of the command. The watcher therefore seals nothing and writes no version of its own, with one exception: re-applying a version this device wrote (below). A listing written by another device is trusted for verification and for reading the epoch key sealed to this device, never as a reason to seal a key to the device it lists.
- **Publishing and copying:** the watcher publishes local versions the transport lacks, through create-only writes, and copies into `<root>/.bilbo/scopes/<scope id>/manifest/` every version of its scopes that verifies, through change 3's `manifest::{verify_scope, adopt}`.
- **Creating a scope** is change 3's `init`: version 1 lists this device and every device that the latest version of every other scope of the owner this device opens lists, minus any device one of those scopes dropped, whose box keys it holds from those manifests, sealed to each. So the owner's known devices get a new scope at once, through a member that already trusted them.
- **A second device** joins through pairing (change 5), or through the setup wizard, which opens the names with the phrase-derived keys and copies in only the manifest of the picked scope before `recover` writes the keys, so the device joins that scope and no other. `bilbo device recover` run by hand first copies in, from a `file://` folder, the owner's chain named like each syncing scope that has no local manifest (`device-identity` Recover fetches scopes from the transport), with the wizard's fetch, so the phrase alone recovers a folder scope even with every device lost. It still never creates a scope (change 3): when the folder holds no such scope it reports `unsealed` and names `bilbo device init`. A folder that has not reached the device yet leaves it enrolled into nothing; the watcher then finds no manifest listing this device, syncs nothing for the scope, and says to run `recover` again. `add-relay` extends the fetch to `https://`.
- **Two published scopes with one name** can still happen when two devices created them before knowing each other. Only a device that can open both sees it, after pairing. It keeps the scope listing more devices (on a tie, the lower id), pushes its notes there and stops syncing the other. The rule needs no agreement, because only devices holding both keys act on it.
- **Changes are shown, not hidden.** A version this device did not write is adopted, since pairing elsewhere depends on it. But a thief holding the owner signing seed can sign versions that change the device list; in change 3's words, "After a confirmed revocation a revoked device, even one using the owner signing seed, cannot read anything written under later epochs; it can still disrupt by signing versions that members reject or that change the device list, which watch announces." So each added device and each new epoch from a version this device did not write is printed once by the watcher and appended to `<root>/.bilbo/scopes/<scope id>/changes.jsonl`, which `bilbo sync` lists for 30 days and the watcher trims past that. Manifests carry no writer signature in v1, so the signer shows as `owner key`; a later format that names the writing device shows its name.
- **Pending versions:** a version this device writes (through `init`, `recover`, `revoke` or pairing, change 5) carries change 3's empty `manifest/<n>.pending` marker until a transport holds identical bytes. Confirming is this change's duty: the watcher reads the version back from the transport and, on identical bytes, clears `.pending` through `manifest::confirm`. On a relay a 201, or a 200 for identical bytes, is that read. In a cloud folder the device's own copy of the folder holds its bytes at once, and another device's write of the same number can still replace them as the folder settles, so a `file://` version after version 1 that changes the epoch is confirmed only when read back unchanged at least 10 minutes after the write; cloud tools settle a two-writer file within that in practice. Version 1 of a new scope, and versions that keep the epoch (adding a device, a URL change), are confirmed on the first identical read back: a version 1 has a random scope id nobody else writes, and a version that keeps the epoch introduces no key, so a lost fork costs only a re-applied change. A new scope therefore syncs at once.
- **No key under an unconfirmed epoch:** segments are sealed only under an epoch change 3's Pending epochs allows (`usable_epoch`), and while a pending version introduces a newer epoch, watch pushes nothing to that scope; versions wait in history. So a revocation that loses a fork never leaves segments under a key no manifest holds, and the revoked device reads nothing new while the revocation waits. The cost is a delay of up to 10 minutes after a revocation on a folder.
- **Forks of a version number:** the first `manifest/<n>.json` on the transport wins. When the transport holds a different valid version `n`, the watcher calls change 3's `manifest::lose`, which moves the pending version to `manifest/lost/<n>.json`, kept, copies the winner in, and applies the change again as a pending `n+1`. Conflict copies are ignored by name.
- A version confirmed and later replaced in a cloud folder (a tool that settles later, or a thief's device still linked to the cloud account) cannot move aside; the watcher then stops syncing that scope with the `sync-transport` spec's line, and the user runs `bilbo device` to see both. After revoking a device on a `file://` scope, remove it from the cloud account too: its sync client can still write the folder. This is the residual risk of a folder transport.
- **Validity includes the chain.** The watcher copies in and adopts only versions valid under change 3's `scope-manifest` rules, including Manifest validity's complete chain and its Chain check by a member. The guarantee, in change 3's words: "After a confirmed revocation a revoked device, even one using the owner signing seed, cannot read anything written under later epochs; it can still disrupt by signing versions that members reject or that change the device list, which watch announces."
- **The pinned transport:** a scope syncs only while the config URL matches the latest manifest's `transport` under change 3's Pinned transport (a folder pins only `file://`, since each machine maps it to its own path; a relay URL is compared whole). Otherwise watch syncs nothing for it and prints the pin line naming both URLs and the fix, and `bilbo sync` exits 1.
- **No scope id beside an unopenable scope, for an outsider:** neither the wizard nor watch lets a version 1 reach the transport when this device is in no scope on that transport and the transport holds a scope of this owner (the plaintext `owner` field) that it cannot open; both say to run `bilbo device recover` on this device. This is the every-device-lost case, where minting would hide the old scope. A device that is a member of at least one scope there may mint a new one, so a device kept out of one scope on purpose can still create scopes.
- This bends the contract's "no file ever has two writers" (see the report). Manifest writes are rare (pairing, recover, revoke, a URL change).

### `bilbo sync`

- Status reads the open-conflict summary, `<root>/.bilbo/scopes/*/state.json` (which the watcher rewrites after every poll: last push, last pull, last error, cursors, acks seen, waiting versions, and per device the seq it stopped at and why), `changes.jsonl` and the manifests. It reads the device key to open scope names, under change 3's rules for the keys folder. It reads nothing on the transport, so it is instant and works offline.
- It exits 1 when something needs attention, so a script or an agent can test it. The design considered `bilbo sync --now`: the watcher polls every 30 seconds, and a trigger would mean a second channel into a running process for little gain.
- `declare` appends the declaration to the note's log under the history lock; the watcher pushes it with the note's next push or on its own.

### `check` and the digest

- `check` gains the `store-check` delta's lines, including the 30-day warning that a note left its scope, read from `open.json`. It reads history but still writes nothing.
- The digest reads `open.json` once per run. Labels come from it. The `Sync conflicts wait in` line goes only in a session's first digest; the session file remembers it, so the line does not repeat on every prompt. This is the proactive raise: a conflicted note nobody recalls still reaches the next session's agent.

### Setup

- The `sync` step installs nothing: the watcher syncs. It only checks and reports, so `--remove` needs no `sync` line, as `embedder` has none today.
- Non-interactive setup gets no sync flags. Turning sync on needs a recovery phrase shown once and typed back, which a non-interactive run cannot do. A Nix user sets `scope.<name>.sync` in `settings` and runs `bilbo device init` or `recover` once by hand. Until then the step reports `skipped: no device key; run bilbo device init in a terminal`, not `failed`, so home-manager activation does not fail on a fresh machine.
- The wizard asks after the watcher's question. It runs the same ceremony as `bilbo device init` and `recover`, through `identity::ceremony` (`confirm_written`, `read_phrase`, `confirm_fingerprint`) on the `Prompter`'s own screen: only with terminals on stdin and stderr, `CLAUDECODE` and `CODEX_THREAD_ID` unset, on the alternate screen, cleared afterwards. The phrase is generated and confirmed before the summary, but the keys are written only after it (`Plan before writing`). The folder must be absolute (or start with `~/`) and its parent must exist; the wizard creates the last component. It writes the config through the existing rewrite path (`config.bak`).
- A scope that syncs with `--no-watch` fails the step, because nothing would sync.

### Config

`sync.poll_seconds` (1 to 3600, default 30) and `sync.stale_days` (1 to 3650, default 180). 180 days is twice `history.keep_days`'s default, so a laptop left in a drawer for a season still merges cleanly. Tests set `sync.poll_seconds = 1`.

### Dependencies

- **No new crypto crate, and no AEAD user here.** Change 3 puts `chacha20poly1305` 0.11.0 (and `ed25519-dalek`) in `src/identity/keys.rs` and exposes `keys::{verify, encrypt, decrypt, random}` and `SignKey::sign`. Segments are sealed with `keys::encrypt` (XChaCha20-Poly1305) and nonces come from `keys::random`. Checked on 2026-10-03 in a scratch crate: `chacha20poly1305` 0.11.0, `sha2` 0.11.0, `ed25519-dalek` 3.0.0 and `base64` 0.23.1 build together. In bilbo's own lock, change 3 measured two duplicates already present, `getrandom` 0.2 and 0.4 and `syn` 2 and 3; `base64` adds none. Re-checked on 2026-10-05 against release 0.12.0: adding the line changes only bilbo's own dependency list in `Cargo.lock`, the normal tree's duplicates stay `getrandom syn`, and 0.23.1 is the newest release.
- **`base64` 0.23.1**, in `src/sync/segment.rs`. It is already in `Cargo.lock` through `ureq`, so it adds no crate. A hand-written encoder would be 40 lines of code to test for nothing.
- **Signatures** go through `SignKey::sign` and `keys::verify`.
- **Why base64 here when manifests use hex:** change 3 chose hex for manifests of a few kilobytes and left the segment encoding to this change. Segments carry whole notes, and hex would double them where base64 adds a third.

## Risks / Trade-offs

- [A deliberate revert of a synced edit is undone once] → The stale-base rule reapplies the synced side. It loses an intent, never text, and the `stale-base` flag shows it.
- [A deliberate delete right after a sync write comes back once] → Same rule. A second delete sticks.
- [A paraphrasing resolution is reported as dropped text] → One `bilbo sync declare` clears it, with the reason kept in history.
- [Dropping the `scope` key by accident removes the note from the other devices] → It stays on the device that dropped it, `check` reports the missing scope there, and the other devices keep its history.
- [A stale device's edits come back as conflicts in every changed passage] → Nothing is lost; the agent resolves them. `sync.stale_days` defaults to 180 so that this is rare.
- [The transport grows without bound] → About 240 MB a year at heavy use. Checkpoints come later with `format` 2.
- [A stolen device holds the owner signing seed and can sign manifests] → Change 3's v1 trade-off: "After a confirmed revocation a revoked device, even one using the owner signing seed, cannot read anything written under later epochs; it can still disrupt by signing versions that members reject or that change the device list, which watch announces."
- [The transport shows sizes, times and device ids] → Names, topics and text are encrypted. Padding is a non-goal.
- [A cloud tool loses or reorders files] → Reading stops at gaps, the outbox recreates own segments, and manifests are re-read and re-applied.
- [Two bilbo versions disagree on a merge] → Two merges with the same parents are never merged with each other, so they cannot ping-pong. A newer `format` is reported, not misread.
- [Two stores on one machine share a device key] → The second writer's create fails with other content, and it stops pushing that scope with a clear line, instead of forking the device's sequence.
- [Clocks are wrong] → Clocks order nothing and show only in history. Staleness counts only this device's own segments by its own clock.
- [A revoked device's last edits] → Its segments after the seq applied when the revocation was adopted are never applied, so edits it made just before the revocation that no peer had read stay only in its own history. Segments a lagging legitimate device writes under the old epoch stay readable by the revoked device until that device adopts the new epoch.
- [A version confirmed in a folder and replaced later] → The watcher stops that scope and names the version (see "Manifests on the transport").
- [A pushed version whose ancestors were pruned] → They are listed in `outside`, so receivers do not wait; a receiver that needs them as a base merges with an empty base, keeping both sides.
- [A folder moved on some devices only] → The pin holds only `file://`, so a device whose config still names the old folder syncs there unseen. The README says to change `scope.<name>.sync` on every device at once, and watch warns when this device's folder holds none of the confirmed manifest versions it holds.
- [A store resuming against a folder whose listing lags] → It may report a second writer by mistake; it waits for two unchanged listings before its first push.

## Migration Plan

No data moves. After the upgrade, sync stays off until a scope gets a URL and a device key exists. The first sync pushes nothing until notes carry the scope. To roll back, set `scope.<name>.sync = off` on every device; notes and history stay, and the transport folder can be deleted by hand. Downgrading the binary below this change is not supported once a `merged` or `left` version exists, because change 1's reader does not know those events.

## Decisions taken while the user was away

Each is argued, with its alternatives, in the section named. They are for the user to confirm.

1. A passage deleted on one side and edited on the other keeps the edit, flagged, rather than a conflict ("The merge unit").
2. `sources` merges three-way, so a removal sticks, rather than as a plain union ("Frontmatter").
3. The concrete scope clash rule, which avoids the local config whenever the result would be pushed ("Frontmatter").
4. Conflict markers carry version ids and times, not device names, have no base section, and hold any number of sides ("Conflict markers").
5. Dropped text is found line by line, exactly, and declared with `bilbo sync declare`, attached to the conflict ("Resolution and dropped text").
6. A local delete right after a sync write is treated like any stale save: edit beats delete, once ("The stale-base rule and inbound inspection: one path").
7. The topic suffix uses the id's last 4 characters, the random part ("Topic collisions").
8. A note whose scope moves away, or whose `scope` key is dropped, disappears from the devices outside its new scope, only as a fast-forward and with a visible line ("Scope moves"). Confirmed by the user.
9. Polling every 30 seconds, pushing on every recorded version, no events on the transport, no `bilbo sync --now` ("When watch pushes and polls", "`bilbo sync`").
10. No checkpoints: the transport only grows in v1 ("No checkpoints in v1").
11. `sync.stale_days` defaults to 180, and ack-only segments are written at most hourly ("Acks, staleness and the outbox", "Config").
12. The watcher never seals or enrolls, and between two published scopes with one name it keeps the one listing more devices ("Manifests on the transport").
13. Setup has no sync flags; only the wizard turns sync on. The `sync` step fails without the watcher and is skipped without a key ("Setup").
14. `bilbo sync` exits 1 whenever something needs attention ("`bilbo sync`").
15. The digest names open conflicts only in a session's first digest ("`check` and the digest").
16. The note skill resolves a conflict by keeping every fact of both sides unless the user picks one (the `agent-plugin` delta).
17. No per-record signature in v1; the segment signature authenticates records ("Version records").
18. A `file://` manifest version is confirmed only after 10 minutes unchanged, and a scope pushes nothing while a pending version introduces a newer epoch than `usable_epoch` ("Manifests on the transport").
19. Version 1 of a new scope, and versions that keep the epoch, are confirmed on the first identical read back; only epoch changes wait 10 minutes on `file://` (lead decision, "Manifests on the transport").
20. A scope whose config URL differs from the manifest's pin (by scheme for `file://`) is refused with a line naming the fix (lead decision).
21. `bilbo restore` of a version with another `scope` prints a stderr line and still restores ("The stale-base rule and inbound inspection: one path").
