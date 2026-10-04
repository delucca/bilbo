# Design

## Context

- Agents write notes in `<root>/notes/` with their own tools (Claude Code's Edit, Codex patches, shell redirects), and with `bilbo new`. bilbo sees none of these writes, and no verb keeps a past version.
- bilbo has no long-running process of its own. `src/host/timer.rs` installs two kinds of job: the periodic `bilbo index` (launchd `StartInterval`, systemd timer) and the local embedder, a keep-alive service (launchd `KeepAlive` + `RunAtLoad`, systemd `Restart=on-failure`). `bilbo watch` is a third job of the second kind.
- `src/note/mod.rs` already reads a note's frontmatter and parses note names (`note::read`, `note::parse_name`), and `src/shared/frontmatter.rs` validates ULIDs (`frontmatter::is_ulid`). `src/shared/store.rs` resolves the root and lists entries, skipping hidden ones (`store::entries`).
- `<root>/.bilbo/` already exists: the library keeps its captures in `<root>/.bilbo/captures/`, which `bilbo check` and library recall do not read. History goes beside them, in `<root>/.bilbo/history/`.
- `src/` is one folder per domain, and AGENTS.md's Architecture rules, checked by `tests/layout.rs`, say where each piece goes: verbs in their domain, reached only from `main`; a module in `src/shared/` only when two domains use it; each listed crate in its own files.
- The sync design (planning notebook, `design-bilbo-remote-sync.md`, and the decisions the user took on 2026-10-03) needs, per note:
  - a graph of full-snapshot versions keyed by the ULID;
  - version ids derived from the parents and the content, never from the device or the time, so two devices that merge the same heads into the same text get the same id;
  - an atomic swap for every write into `notes/`.
  This change lays down the graph and the swap. It has one parent per version, because nothing local can fork a note.
- Corpus size: all of `~/Notebooks`, 440 notes and 1,181 sources, is 13.7 MB. One store's notes are a few MB.

## Goals / Non-Goals

**Goals:**
- Record every settled change to a note with no help from the agent, including changes made while the watcher was down.
- A history format that the sync change extends with more parents, a device and a signature, without rewriting existing ids.
- A restore that cannot lose an edit, even one that races it or a crash in the middle.

**Non-Goals:**
- Recording every keystroke-level save. Saves are debounced: a burst collapses into one version.
- Watching anything but `<root>/notes/`.
- A history index for search. `bilbo history` reads one note's log once it knows the note. Naming a deleted note by topic reads the last line of every log, which is a few hundred small reads.

## Decisions

### Events trigger a scan; the scan decides

`bilbo watch` uses `notify` only as a trigger. Every decision about what changed comes from a scan of `<root>/notes/` against history:

- **What triggers a scan:** a notify event on `notes/`, once the folder has been quiet for 2 seconds (or 10 seconds after the first event, if events keep coming). The debounce is per folder, not per file, in the spec and here: an agent editing note B without pause delays recording note A by up to the 10-second cap. A per-file timer would need per-path event attribution, which FSEvents' coalescing does not give reliably. A scan also runs on start, on any event notify flags with `need_rescan()`, on a watcher error (after re-creating the watcher), and every 10 minutes as a backstop.
- **What a scan does, in two passes:**
  - *Unlocked pass.* It lists `notes/` and keeps an in-memory map of `file name → (len, mtime, ctime, inode, content hash)`. It reads and hashes only the entries whose stat changed. This pass does the slow work, so the lock is held briefly.
  - *Locked pass.* It takes `history/lock`, sweeps restore leftovers (see the restore section), lists `notes/` again, re-stats every entry, and re-reads and re-hashes each one whose stat differs from the unlocked pass. Only then does it compare each id's current file with that id's latest version and record the differences. A restore that ran while the scan waited for the lock has therefore already been seen: the scan compares the file as the restore left it with the `restored` head, and records nothing.
  - The start scan works the same way.
- **Why ctime is in the stat key:** `cp -p`, `rsync -a` and cloud tools keep the mtime, so a rewrite of the same length could look unchanged. No user tool can set the ctime, and any write moves it. `touch` moves it too, so the file is re-hashed, the bytes match, and nothing is recorded.
- **Why a scan, not the events themselves:** notify's documentation says a `Rescan` flag means "events received so far can no longer be relied on", and FSEvents coalesces. So a full rescan has to exist anyway, and making it the only path means one piece of logic, tested without timing. The events only make recording quick.
- **Cost:** a scan of 440 notes whose stat did not change is two `read_dir`s plus 880 `stat` calls, a few milliseconds. A first start hashes every file once, outside the lock.

Alternatives considered:
- **Polling every 2 s with no dependency.** It was offered first, and the user picked events.
- **`notify-debouncer-full`.** It is another crate, and its rename tracking duplicates what the id-based scan already gets right, since a rename is the same id under a new name.
- **Holding `history/lock` for the whole scan.** Simpler, but a first start on a large store would block `restore` for the whole hashing pass.

### When `notes/` goes away

- **A listing that fails records nothing.** When either pass's `read_dir` of `notes/` fails (not found, not a folder, permission), the scan records nothing and prints `bilbo: cannot read <root>/notes: <reason>; waiting` once. Watch then checks for the folder every 10 seconds. Once it lists again, watch re-creates the notify watch, prints `bilbo: watching <root>/notes`, and scans.
- **The same at start.** A root that exists without `notes/`, such as a store that holds only `library/`, takes the same path: watch starts, takes `watch.lock`, prints the cannot-read line and waits. Only a missing root is `no store` and exit 1. Exiting on a missing `notes/` would make the login service restart every 10 seconds and log each failure.
- **Why:** a cloud-sync tool replacing the folder, `mv notes notes.bak && mv notes.bak notes`, or an unmount all look like every note vanished. Without the guard, each note gets a tombstone. In this change the content survives in history, but change 4 would propagate the mass deletion to every device.
- **An empty folder records no deletions.** When a scan lists `notes/` and finds no file that holds a note id, while history holds at least one note whose latest version is not `deleted`, it records nothing and prints `bilbo: <root>/notes holds no notes; not recording deletions` once. Deletions are recorded again as soon as any note file is back.
- Alternatives for the empty folder: recording every deletion (a mass tombstone from a folder swap in progress), or refusing any scan that would delete more than half the notes (an arbitrary threshold that also blocks a real cleanup). Refusing only the all-gone case costs one thing: a user who deletes every note sees those deletions recorded only when the next note is written. Nothing is lost meanwhile: restore and history work on the last recorded versions.

### Only note names are recorded

Watch records a file only when its name is a note name, `<kind>-<topic>.md` under `note::parse_name`. Any other name with a valid id is skipped with the reason `name is not <kind>-<topic>.md`.

- **Why:** a note under another name cannot be named by topic, and `restore` would write the bad name back. `bilbo check` already reports such a file.
- **What it buys the id rule:** a note name holds only lowercase letters, digits, hyphens and `.md`, never a newline. The version id's preimage below can then separate its fields with newlines without ambiguity.
- Alternative: record any name and escape the file name in the preimage. It keeps files that the store already calls invalid.

### `notify` 8.2.0, in `src/note/watch.rs` only

- **Why a crate:** std has no file-event API. Writing FSEvents and inotify bindings by hand would mean more `unsafe` FFI than the whole crate's surface.
- **Which release:** 8.2.0 is the newest stable release on 2026-10-04; 9.0.0 is still a release candidate. A scratch crate built it with Rust 1.95.0 on macOS and received a create event from a non-recursive watch.
- **What it brings:** on macOS, `notify-types`, `bitflags`, `fsevent-sys`, `walkdir`, `same-file`, `log` and `libc`. On Linux, `inotify`, `inotify-sys` and `mio` replace `fsevent-sys`. Measured with `cargo tree` on 2026-10-03 and again on 2026-10-04.
- **How it is used:** non-recursive, on `notes/` only (subfolders are not notes), with the recommended backend for each platform. `fsevent-sys` links CoreServices, which the nixpkgs darwin stdenv provides. Task 1.2 checks that the Nix package builds it.

### `sha2` 0.11.0, through `src/shared/hash.rs`

- **What it is for:** content and version ids. `src/note/versions.rs` calls `hash::sha256_hex` and never uses `sha2` directly.
- **Where it lives:** `add-library-store` added `sha2` with `src/library/hash.rs`, `pub fn sha256_hex(bytes: &[u8]) -> String`, as its one user. Now that the note domain hashes content too, the module moves unchanged to `src/shared/hash.rs`, since the Shared Kernel admits a module two domains use. `PLACEMENT` in `tests/layout.rs` moves `sha2` with it.
- **Why not `ring`:** ring already has SHA-256, but `PLACEMENT` keeps `ring` in `src/host/model.rs`.
- **What it adds:** nothing. `sha2` 0.11.0 and its RustCrypto dependencies are already in `Cargo.lock`, and the sync changes need them anyway, since `ed25519-dalek` depends on `sha2`.
- **Follow-up, out of scope:** moving `src/host/model.rs`'s download hash to `sha2` would drop `ring`.

### Storage layout

```
<root>/.bilbo/
  captures/                  the library's captures, from add-library-store
  watch.lock                 held by the running watcher for its lifetime
  history/
    lock                     held while anything writes history
    blobs/ab/cdef…           one file per distinct content, named by its SHA-256
    notes/<ULID>.jsonl       one line per version, oldest first
```

A version line:

```json
{"version":"<64 hex>","parents":["<64 hex>"],"file":"decision-release.md","blob":"<64 hex>","event":"edited","at":"2026-10-03T14:23:05-03:00"}
```

- **How the id is built:** `version` = SHA-256 over these lines, each ending in a newline:
  1. `bilbo-version-1`;
  2. the note's ULID;
  3. the parent ids, sorted, one per line (none for a note's first version);
  4. an empty line;
  5. the file name;
  6. the blob hash, or `deleted` for a tombstone.
- **Why the ULID is in it:** without it, two notes with the same parents, file name and bytes would share a version id. Today a note's bytes hold its id in the frontmatter, so that needs bytes that differ from the note's own id, but the rule should not lean on what a file happens to contain: a conflict copy, a hand-made file or a later format could break it. Logs are per note, so a shared id is harmless locally. But change 4 keys sets of applied versions and conflict parents by version id, and two notes would merge there. The id can never change after archive without rewriting every id, so it goes in now.
- **Why the fields cannot run together:** the ULID is 26 characters, ids and blobs are 64 hex characters, `deleted` is a fixed word, and a file name holds no newline (only note names are recorded). The empty line ends the parents.
- `parents` is a list from the start, so the sync change's merge versions use the same id rule and the existing ids stay valid. `event` and `at` sit outside the id: they describe a version without identifying it.
- **Readers ignore fields they do not know.** The sync change adds `device` and `sig` outside the id, as the panel's merge-storm fix requires, and may add `note`, a `conflict` field or new events. A reader of this change's format skips unknown fields and lists an unknown event by its name, so those additions need no format bump. `bilbo-version-1` stays the only format marker.
- **Why one log per note:** a note's history is read without touching the others, and pruning rewrites one small file.
- **Why JSON lines:** `serde_json` is already a dependency, and a log can be read with `cat` when something goes wrong.
- **Why separate blobs:** an `A → B → A` edit stores two blobs, not three, and a rename stores no new blob.

### Writing safely without a database

- A writer (the watcher, `restore` or a prune) holds `history/lock` exclusively, with std's `File::lock`, stable since Rust 1.89. Before each write it re-creates `history/` and its folders when they are missing, so a deleted `.bilbo/` does not fail every later append. It writes a blob to a hidden temporary name, `.tmp-<random>` in the blob's folder, and renames it into place, then appends one complete line to the note's log with a single `write` on an `O_APPEND` file.
- **The last line of a log.** A crash or a read during an append can leave a last line without its newline. When that line parses as a complete record, the next writer appends the missing newline and keeps it. When it does not parse, the next writer cuts it off before appending. Readers (`bilbo history`) take no lock and treat such a line the same way, in memory only.
- Pruning writes the new log to `notes/.tmp-<random>` under `history/` and renames it over the old one, so a reader sees either the old log or the new one.
- **Leftover temporaries.** A crash can leave a `.tmp-*` file under `history/`. Any one present while a process holds `history/lock` is a leftover, since every writer holds the lock from creating its temporary to renaming it. The watcher deletes them under the lock when it starts, before its first scan. They hold nothing a version needs: a blob is named only once its rename lands, and a pruned log's old copy stays in place until then. `history` and `restore` ignore them.
- **A reader can race a prune.** `bilbo history` loads a log, then opens a blob. When a prune removed the blob in between, it prints `bilbo: version <v> of <note> was pruned` and exits 1. Taking `history/lock` shared would close the window, but `bilbo history` would then wait behind a first start's scan.

### One watcher, and a standby instead of a restart loop

- `watch.lock` is held for the watcher's lifetime. A starting watcher tries it with `try_lock` up to 10 times, 100 ms apart.
- When it is still held after a second, another watcher runs. The new one prints `bilbo: bilbo watch is already running for <root>; waiting` and blocks on `File::lock` until the lock frees. It then prints `bilbo: watching <root>/notes`, scans and runs as usual.
- After taking the lock, watch checks that the file it locked is still `<root>/.bilbo/watch.lock` (same device and inode). When it is not, because `.bilbo/` was deleted under it, it opens the new file and takes the lock again. A running watcher makes the same check before each scan and exits 1 when its lock file is gone, so a deleted `.bilbo/` never leaves two watchers running. The service restarts it once, 10 seconds later.
- `bilbo history` uses one `try_lock` to tell whether a watcher runs: if it gets the lock, it releases it at once and prints the warning. It never creates `watch.lock`, so it stays read-only.
- **Why a standby:** the review found two ways to make the service exit and restart every 10 seconds, filling `watch.log`: `bilbo history` holding the lock for an instant while the service starts, and a user running `bilbo watch` by hand while the service is installed. The retries cover the first, since a probe holds the lock for microseconds. The standby covers the second: the service waits for the manual watcher instead of failing, and takes over the moment it stops, with no gap in recording.
- Alternatives:
  - **Exit 0 when another watcher runs, with launchd's `KeepAlive` set to restart only on failure.** No loop, but the service then stays down after the manual watcher stops, until the next login. Notes go unrecorded with nothing to say so but the `bilbo history` warning.
  - **Exit 1 and accept the loop.** One log line every 10 seconds for as long as the manual watcher runs, which is the noise the review flagged.

### Restore uses an atomic exchange

`src/host/swap.rs` is a new host adapter. It wraps two system calls that std does not expose:

- `exchange(a, b)` swaps two paths atomically. macOS: `renamex_np(a, b, RENAME_SWAP)`. Linux: `renameat2(AT_FDCWD, a, AT_FDCWD, b, RENAME_EXCHANGE)`. Both are in `libc` 0.2.190, for both release OSes, and glibc has `renameat2` since 2.28.
- `rename_new(a, b)` renames only if `b` does not exist. macOS: `RENAME_EXCL`. Linux: `RENAME_NOREPLACE`.
- When the filesystem refuses (`EINVAL`, `ENOTSUP`), it returns a message, and restore refuses. Silently falling back to a plain rename would bring back the race this module exists to close.

`libc` stays the dependency. `rustix` would wrap the same calls more safely, but it is a second crate for two calls. `PLACEMENT` in `tests/layout.rs` gives `libc` to `host/prompt.rs` and `host/swap.rs`.

The restore sequence, all under `history/lock`, so a running watcher's locked pass waits and then finds the head already matching the file:

1. Sweep leftovers from an interrupted restore (below).
2. Resolve the note and the version. Find the note's current file with the watcher's id scan of `notes/`, not from history, so a file the watcher has not recorded yet still counts. Refuse the cases in the spec (a deletion, a name taken by another note, already matching).
3. When the current file's bytes are not the latest version, record them as `edited` (or `renamed`, under the watcher's rules).
4. Write the version's bytes to `notes/.bilbo-restore-<ulid>`. That is a hidden name, so the watcher does not record it.
5. Call the restore's hook with the step about to run. The hook is also called before the `rename_new` of step 6. In the binary it does nothing. A unit test passes a closure that writes the note's file before the swap, which tests the race without a timing-dependent child process. A closure that returns an error stops restore where it is, without cleanup, as a kill would, which tests each crash point below.
6. Then one of:
   - **A current file:** `exchange` the temporary file with it. The note's file now holds the version's bytes, and the temporary name holds what came out. When the version's name differs, `rename_new` the note's file to the version's name.
   - **No current file:** `rename_new` the temporary file to the version's name.
7. Read what came out. When its bytes differ from what step 3 saw, something wrote during the restore, so record them as `edited`.
8. Delete the temporary file, then record `restored`, whose parent is the latest version.

When the `rename_new` of step 6 finds the version's name taken, because another note's file appeared there after step 2, the restored text stays under the current name. Restore still runs steps 7 and 8, recording `restored` under that name, then prints the taken message and exits 1.

**Why the exchange comes before the rename.** The exchange puts the restored text at the current name in one step. The rename then moves that one file. At every moment exactly one file holds the id, so a crash never leaves two files that the watcher refuses to record. The earlier draft renamed the new file in first and the old one out second, which left both in place between the two calls.

**What a crash leaves, and who cleans it:**
- **Before step 6:** `notes/.bilbo-restore-<ulid>` holding the version's bytes. The note is untouched.
- **Between the exchange and the rename:** the note's file holds the restored text under its old name, and the temporary name holds what came out. The watcher records the note as `edited`. Nothing needs a human.
- **Before step 7 or 8:** the temporary name holds what came out, which may be an agent's write that no version holds yet.
- **The sweep.** Any `notes/.bilbo-restore-<ulid>` present while a process holds `history/lock` is a leftover, since a live restore holds the lock from step 4 to step 8. Restore sweeps at its start, and the watcher at each locked pass, so a leftover lasts until the next scan at most. For each one, when no version of that note names its bytes, the sweep records them as an `edited` version under the note's latest file name, then deletes it and prints `bilbo: recorded notes/.bilbo-restore-<ulid> from an interrupted restore`. When a version already names them, it deletes it silently. `bilbo check` keeps ignoring hidden entries.
- The watcher's sweep is the one change it makes inside `notes/`. The `note-watch` spec says so.

**What remains:**
- An agent that writes the old file name after the rename of step 6 creates a second file holding the id. The watcher prints a `shared id` line naming both files and records neither. The fix is to delete the file under the old name. Nothing is lost, and the problem is reported.
- A process that opened the note's file before the exchange and writes after it writes into the swapped-out inode, now at the temporary name. When the write lands before step 7, it is recorded. When it lands after step 8, it goes to a deleted file. This is a window of microseconds, real only for a long-running writer such as a shell `>>` in a slow script, and accepted. Moving the swapped-out file into `blobs/` instead of deleting it would not help: the same late write would then corrupt a blob.

### A hand-written Myers diff in `src/note/diff.rs`

- **What it is:** `bilbo history --diff` needs a line diff. Myers' O((N+M)·D) algorithm is about 100 lines, and fast when the two versions are close, which is the usual case. The output is a unified diff with 3 lines of context.
- **Why not a plain LCS table:** an O(N·M) table on a 1 MiB file of 10,000 lines needs 100 million cells.
- **Why not the `similar` crate:** it is well made, but the sync change needs the same longest-common-subsequence core over passages, not lines. One generic function over slices serves both.

### Retention

- **When it runs:** pruning runs under `history/lock` when the watcher starts and every 24 hours after.
- **What it keeps, per note:**
  - every version at or after the cutoff;
  - the newest version before the cutoff;
  - for a deleted note, its tombstone and the version before it.
- **What it removes:** blobs that no kept version names, in a mark-and-sweep pass over all logs.
- **An unreadable log stops the sweep.** When any log holds a line that does not parse, other than a partial last line, pruning leaves that log as it is, removes no blob at all, and prints `bilbo: history/notes/<id>.jsonl line <n> is unreadable; no content removed`. Otherwise the blob of the unreadable version would look unnamed and be deleted.
- **Why the newest version before the cutoff:** without it, a note untouched for a year would lose its only version, and `--diff` against the oldest listed version would have nothing to compare with.
- **What the sync change adds:** it must also keep every version a peer may still need as a merge base. That rule belongs with the protocol, and change 4 modifies the `Retention` requirement to add it.

### The verbs and the output rule

- `watch::run` prints nothing itself. Like `setup::run`, it takes a `&mut dyn FnMut(&str)` for its progress lines (watching, skipped files, pruned, waiting), and `main` prints each with the `bilbo: ` prefix. It returns a `Failure` only for a start error.
- `restore` exposes its sequence as `restore::apply(root, note, version, hook: &mut dyn FnMut(Step) -> Result<(), String>)`, which `run` calls with a closure that returns `Ok`. The behavior is the same in every build profile, so `nix flake check`, which runs the tests in release, exercises it too.
- Naming a note, the id scan of `notes/`, the sweep, the version store, pruning and the probe of `watch.lock` that `history` makes live in `src/note/versions.rs`, so `watch`, `history` and `restore` share them without depending on each other. AGENTS.md lets only `main` use a verb's module.
- `bilbo history <note> <version>` prints a version's bytes exactly. `history::run` returns them as bytes, and `main` writes them to stdout unchanged: its line printer adds a newline, which would change a version that does not end in one.
- A note file's id alone is read by a new function in `src/note/mod.rs`, beside `note::read`, and follows the same rule (the first `id:` value of a closed frontmatter, when it is a canonical ULID), so `watch` and `bilbo check` agree on which id a file holds. It stays in the note domain because `src/shared/frontmatter.rs` leaves splitting the block to each kind of file, and only the note domain uses it.

### Setup and the service

- **How the service is built:** `src/host/timer.rs` gets `Name::Watch` and a `Kind::Watch`, since today `Job::name()` derives the job from its kind and `service()` hardcodes the embedder's description and the `llama-server` path in its messages. Task 5.1 moves the per-job text (description, program name in messages) behind the kind. The watch job writes `ExecStart` or `ProgramArguments` as `<bilbo> watch`, with the keep-alive shape of the embedder service.
- **Its environment:** the same six locations the index timer carries. The index timer's key rule does not apply: the watcher reads no key, so `embedder.token_env` does not fail the watch step.
- **Where the step goes:** setup adds the `watch` step after `timer`. Adding it at the end leaves every existing line's position alone. The flag goes in `src/setup/flags.rs`, the installed state in `facts.rs`, the plan and summary in `plan.rs`, the step in `apply.rs`, the remove line in `remove.rs` and the question in `wizard.rs`.
- **What the wizard asks:** one yes/no question, "Record note history in the background?", defaulting to yes, after the timer's question.
- **The home-manager module:** it passes `--no-watch` when `watch.enable` is false. `history.keep_days` is a string in `settings`, like every other key there (`"30"`), so the module's `nullOr str` type stays as it is.
- **Why the watcher is installed by default:** it needs no embedder and no network, and it is what makes history exist.

## Risks / Trade-offs

- [A missed event without a `Rescan` flag] → The change is recorded by the next event's scan or by the 10-minute backstop, so history can lag by up to 10 minutes but loses no content: a scan records the file as it then is. What a missed event can lose is an intermediate state, overwritten before the backstop scan.
- [The watcher reads a file mid-write] → It records a partial version, and the next settle records the full one. This is harmless, and the debounce makes it rare.
- [A rewrite that keeps length, mtime, ctime and inode] → Not possible from user space: any write moves the ctime. A filesystem without a ctime would need the backstop to hash every file, which this change does not do.
- [History grows] → Bounded by `history.keep_days`: at 90 days and a heavy 0.5 MB a day, about 45 MB.
- [A pasted secret stays in history] → Until pruning drops it. Manual removal: stop the watcher, delete `<root>/.bilbo/history/notes/<id>.jsonl`, fix the note, and start the watcher, which records the note as `added`. The orphaned blobs go at the next prune. The README says so. A `bilbo redact` belongs with sync, where the purge must also reach other devices.
- [The filesystem cannot exchange files] → `restore` refuses with a clear message. APFS, ext4, btrfs and xfs support the swap, and so do tmpfs since Linux 3.17 and overlayfs since 4.9. NFS does not, but a store on NFS is not a supported layout.
- [launchd restarts a watcher that keeps failing, for example with a missing root or a bad config] → launchd throttles restarts to one every 10 s by default, and systemd's `RestartSec=10` does the same. The log says why it failed. A second watcher no longer fails: it waits.
- [An old `bilbo watch` keeps running after an upgrade] → `setup` rewrites the service file with the new binary path and reloads it (`updated`), as it does for the timer. The lock stops two watchers from running together.

## Migration Plan

Nothing to migrate. After an upgrade, `bilbo setup` installs the watcher, and its first start records every note as `added`. To roll back, run `bilbo setup --yes --no-watch` and delete `<root>/.bilbo/`. The notes are never touched.
