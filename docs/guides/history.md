# Note history

See what a note said before, undo a careless rewrite, and control how long
versions are kept. Agents edit notes in place with their own tools, so bilbo
sees no write; the watcher records each version instead.

## Record history

The watcher, `bilbo watch`, runs in the background and records a version of a
note each time `<root>/notes/` has been quiet for 2 seconds after a change, or
10 seconds after the first change while edits keep coming. A burst of saves is
one version. It records a creation (`added`), an edit (`edited`), a rename
(`renamed`) and a deletion (`deleted`), keyed by the note's `id`, so a renamed
note is the same note. A change made while the watcher was not running is
recorded when it starts.

`bilbo setup` installs the watcher as a login service; see [Set up
bilbo](setup.md#watch-service). The watcher never touches `notes/`, apart from
sweeping the leftover of an interrupted restore (below) and writing what other
devices synced, when a scope [syncs](sync.md).

History lives in `<root>/.bilbo/history/`, beside the notes. A version is a full
copy of the note, and identical content is stored once. Deleting
`<root>/.bilbo/history/` loses the history and nothing else: the next `bilbo
watch` starts over with every note `added`. Leave the rest of `<root>/.bilbo/`
alone, since it also holds the library's captures and the state of
[sync](sync.md).

### What the watcher skips

The watcher records only a regular, non-hidden file directly in `notes/`, named
`<kind>-<topic>.md`, no larger than 1 MiB, whose frontmatter has an `id`, and
which no other file shares. For a file skipped for its name, id, size or a
shared id it prints one line to stderr,
`bilbo: notes/<name>: not recorded: <reason>`.

When `notes/` cannot be listed, as when a sync tool swaps the folder, it records
nothing and waits; when it holds no note at all, it records no deletions. A
second watcher for the same store waits for the first to stop.

## Read a note's history

`bilbo history <note>` lists a note's versions, newest first, one per line as
`<version> <time> <event> <file name>`. `<note>` is its topic or its id; a
deleted note is still named by its last topic. A version is named by 6 or more
characters of the start of its id, and the list shows 12:

```console
$ bilbo history release
8f3c2a91d0b7 2026-10-03T16:02-03:00 edited decision-release.md
a1b2c3d4e5f6 2026-10-03T14:23-03:00 added decision-release.md
```

Print a version, or compare:

```sh
bilbo history release a1b2c3          # print that version's exact text
bilbo history release --diff a1b2c3   # diff it against the file on disk now
bilbo history release --diff a1b2c3 8f3c2a   # diff two versions
```

A diff is unified, with 3 lines of context, and its header lines are `--- <file
name>@<version>` and `+++ <file name>@<version>`, or `+++ <file name>@now`
against the file on disk. `bilbo history` changes nothing, and when no watcher
runs it adds `bilbo: bilbo watch is not running; recent edits may not be
recorded` on stderr.

## Restore a version

`bilbo restore <note> <version>` writes a past version back to
`<root>/notes/<the version's file name>` and records it as a `restored` version:

```console
$ bilbo restore release a1b2c3
restored decision-release.md to a1b2c3d4e5f6
```

It works with or without a running watcher. Nothing is lost on the way. Restore
first records what the file holds now when history does not, swaps the file in
atomically, and records what came out of the swap too, in case an agent wrote to
the file meanwhile.

### When restore refuses

- If the version's file name differs from the current one, the current file is
  removed, and restore refuses with `<file name> is taken by another note` when
  another note holds that name.
- It refuses a `deleted` version, and writes nothing when the file already holds
  the version.
- It needs a filesystem that can swap two files atomically (APFS, ext4, btrfs
  and xfs can).

A restore that is killed leaves one hidden file, `notes/.bilbo-restore-<id>`;
the next restore or watcher scan records its bytes if no version holds them, and
deletes it.

## Retention

Versions older than `history.keep_days` (90 by default) are dropped when the
watcher starts and every 24 hours after. Kept are every newer version, each
note's newest version from before the cutoff, so the note as it stood then stays
readable, and for a deleted note its deletion and the version before it, however
old. Content that no kept version holds is removed from disk.

The watcher reads the setting when it starts, so after editing it, restart the
watcher: `launchctl kickstart -k gui/$(id -u)/io.github.delucca.bilbo.watch` on
macOS, `systemctl --user restart bilbo-watch.service` on Linux. Rerunning `bilbo
setup` does not restart a watcher it keeps.

## Remove a secret from history

A secret pasted into a note stays in its history until pruning drops it. To
remove it sooner by hand:

1. Stop the watcher. `bilbo setup --yes --no-watch` removes its service, and
   `bilbo setup --yes` installs it again.
2. Delete `<root>/.bilbo/history/notes/<id>.jsonl`, with the note's `id` from
   its frontmatter. This drops every past version of that note.
3. Remove the secret from the note itself.
4. Start the watcher. It records the note as `added`, and its next prune removes
   the old content that no version holds.

## See also

- [Configuration](../reference/configuration.md#keys) for `history.keep_days`.
- [Commands](../reference/commands.md#history)
