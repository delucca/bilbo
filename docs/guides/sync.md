# Sync notes between devices

`bilbo watch` keeps the notes of a scope the same on all your devices. This page
shows how to turn sync on, add a device, read the status, resolve conflicts, and
move or end sync. Read [Security](../security.md) first: it says what sync
protects and what it does not.

Sync needs a folder that a tool you already use syncs between your devices, such
as Dropbox, iCloud Drive or Syncthing. bilbo writes into that folder only
encrypted, signed files, and each device writes only its own, so the tool never
sees two writers on one file. The folder is the whole transport: bilbo runs no
server, unless you run a [relay](relay.md) and sync through its `https://` URL.

## Turn sync on

In a terminal, the `bilbo setup` wizard does steps 1 to 3 in one go: it asks for
the scope, the folder or [relay](relay.md) URL and the keys, and writes the
config line. See [Set up](setup.md). By hand:

1. Declare the scope's folder in the [config](../reference/configuration.md):

   ```
   scope.personal.sync = file:///Users/me/Dropbox/bilbo
   ```

2. Create the keys with `bilbo device init`, in a terminal, and write down the
   phrase; see [The recovery phrase](devices.md#the-recovery-phrase). It writes
   the scope's manifest.
3. Make sure `bilbo watch` runs, which `bilbo setup` installs. It syncs the
   scope from then on.
4. Give the notes the scope. A note syncs only when its `scope:` line names a
   scope whose `sync` is a URL, so turning sync on uploads nothing until you
   assign notes. Triage the store as in [Scopes](scopes.md).

`bilbo setup` creates the folder's last component when it is missing and its
parent exists. Watch never creates a folder: one that vanishes is reported as
not reachable.

`bilbo sync` then shows how it goes.

## Sync URLs

`scope.<name>.sync` is `off`, the default, or a URL that says where the scope
will sync:

```
scope.personal.sync = file:///Users/me/Library/Mobile Documents/bilbo
scope.work.sync = https://relay.example.net:8443/bilbo
scope.dev.sync = http://127.0.0.1:8740
```

- `file://` and an absolute path, taken literally: a space stays a space, and
  `%20` is a folder named with `%20`. Put the folder a cloud service syncs here.
- `https://<host>[:<port>][/<prefix>]`, or `http://` only to `localhost`,
  `127.0.0.1` or `::1`.
- No user name, password, query or fragment. A bad value is a config error that
  names the key and exits 2.

## Add a second device

Copy nothing. Install bilbo, point the same scope at the same folder once the
folder's sync tool has synced it, run `bilbo device recover`, type the phrase,
and start `bilbo watch`:

```
scope.personal.sync = file:///Users/me/Dropbox/bilbo
```

`recover` reads the owner's scopes in the folder, finds the one named `personal`
and adds this device to it, so the phrase alone is enough, even when every other
device is lost. Watch then reads every segment of every device from the first
and writes each note once. A device joins a scope only through `recover` or a
version another device wrote, never through watch alone.

To add a device without typing the phrase, [pair it](devices.md#pair-a-device).

## What syncs

Only the notes in a scope that syncs, and only their text and frontmatter:

- A note with no `scope`, a scope the config does not declare, or a scope whose
  `sync` is `off` never leaves the device, in any form. `bilbo sync` counts
  them: `local: 17 notes sync nowhere`.
- Nothing outside `<root>/notes/` syncs: not the library, the vector cache, the
  config or the keys.
- A deletion syncs. A note deleted on one device while another edits it comes
  back with the edit, flagged `edit-beat-delete`.
- Two notes created offline with one topic keep both: the one with the later id
  is renamed to `<kind>-<topic>-<4 id characters>.md` on every device, flagged
  `topic-taken`.
- Moving a note out of a scope pushes none of its text into the old scope, only
  a marker. The other devices of that scope remove their copy, keep its history
  and print `notes/<file> left the scope; its history stays`. Until 30 days
  pass, `bilbo check` warns on the device that moved it.

[Security](../security.md#what-the-folder-sees) says what the folder's tool can
still see.

A version is pushed when watch records it. Other devices' files are polled every
`sync.poll_seconds`, 30 by default. A device that is offline, or whose folder is
not reachable or full, keeps recording history and retries: watch says so once
on stderr, and `bilbo sync` shows it.

## Conflicts

Two devices that edit one note concurrently are merged three-way against the
last version they share, passage by passage, a passage being what
[recall](../reference/commands.md) cuts at a heading. A passage changed on one
side takes that side.

Frontmatter merges key by key:

- `id` and `created` never change.
- `sources` merges as a set.
- When the two sides gave the note different scopes, the one that shares less
  wins, flagged `scope-clash`.

Each merge is recorded in [history](history.md) as `merged`, so an automatic one
is always labelled, and two devices that merge the same versions write the same
bytes.

When one passage was changed differently on both sides, bilbo keeps both in the
file:

```
<<<<<<< bilbo 9b46cd969c2b 2026-10-05T09:12-03:00
## Rollout

Ship on Monday.

======= bilbo b1fb38126e5c 2026-10-05T09:40-03:00
## Rollout

Ship on Friday.

>>>>>>> bilbo
```

Watch prints `notes/<file>: conflict in <n> passages; run bilbo check`. `bilbo
check` keeps reporting the conflict until the markers are gone. The digest
labels a conflicted note `conflict` and an auto-merged one `auto-merged`, and
names open conflicts in a session's first digest.

The `note` skill resolves a conflict in the note it touches: one passage that
keeps every fact of both sides, and when you said which side holds, only that
side.

### Declare a dropped line

A resolution that leaves out a line one side held is reported by `bilbo check`
as `conflict: dropped <n> lines of '<heading path>', first "<line>"; restore
them or run bilbo sync declare <topic> "<why>"`. It stays reported until the
line is put back or declared dropped on purpose. A marker line that belongs to
no open conflict is `line <n>: stray conflict marker`.

```sh
bilbo sync declare release "Monday was superseded"
```

`<note>` is named as in `bilbo history`, and the reason is one line of up to 500
characters. The command prints `declared plan-release.md: 3 lines dropped on
purpose`, writes only under `<root>/.bilbo/`, and syncs with the note. With
nothing to declare it exits 1.

### Stale saves

An agent that saves a note it read long ago cannot revert a synced change in
silence either. The first save after watch wrote a note from another device is
merged against the version from before that write, and a merge that keeps both
is flagged `stale-base`.

## Check the status

`bilbo sync` prints, per syncing scope, where it syncs and which devices keep
up, then the notes that sync nowhere, the open conflicts and undeclared drops,
recent flags and changes of the device list:

```console
$ bilbo sync
scope personal file:///Users/me/Dropbox/bilbo: 42 notes, pushed 2026-10-05T09:12-03:00, pulled 2026-10-05T09:13-03:00
device personal rhosgobel: this device
device personal bywater: up to date
local: 17 notes sync nowhere
conflict notes/gotcha-nix.md: 1 passage
notice 2026-10-04T18:02-03:00 notes/plan-release.md: edit-beat-delete
change 2026-10-02T11:30-03:00 personal: device morthond added by owner key (manifest 4)
```

A device's state is `up to date`, `behind by <n> segments`, or
`stale since <time>` once it left a segment of this device unacknowledged for
`sync.stale_days`, 180 by default.

A `change` line is a device or an epoch another device added. If you do not know
the device, run `bilbo device revoke`; see [Revoke a
device](devices.md#revoke-a-device).

`bilbo sync` reads only local state, so it works offline and changes nothing. It
exits 1 when:

- a conflict or undeclared drop is open,
- a scope's folder was unreachable or full,
- a scope or a device is stopped (its line goes to stderr, and a segment that
  fails to verify names the copy to restore), or
- no `bilbo watch` runs.

Without a syncing scope it says
`no scope syncs; set scope.<name>.sync in <config path>` and exits 1.

## The folder only grows

bilbo deletes nothing from the folder, and a new device reads every segment from
the first, so the folder and a new device's first sync grow with your history.
Trimming `history.keep_days` shortens only the local history. A device that has
left the scope's segments unacknowledged for `sync.stale_days` stops holding
back pruning of the versions it might still need as a merge base.

### Lose the local state

Removing `<root>/.bilbo/` loses the local history and any manifest version not
yet on the folder (one this device wrote and no folder holds yet). Watch then
copies back the one scope of that name whose latest version lists this device,
reads the folder from the first segment and resumes where this device's own
files end.

When the folder holds several scopes of that name, watch copies none and says
`run bilbo device recover on this device`.

Two machines that share one device key are not supported: watch stops pushing
when it finds a segment of this device that its store did not write.

## Move or end sync

A scope's manifest pins the scheme of its URL, not a folder's path. To move a
synced folder, let the tool finish moving it, then change `scope.<name>.sync` on
every device at once. A device whose config still names the old path finds no
manifest of its scope there. Watch syncs nothing for that scope and says `holds
none of this device's scopes; if the folder moved, change scope.<name>.sync on
every device`.

To move a folder scope to a relay, see [Move a folder scope to a
relay](relay.md#move-a-folder-scope-to-a-relay).

To turn sync off for a scope, set `scope.<name>.sync = off` on each device. Each
keeps its notes and history. The folder keeps what it holds, which you can
delete once no device syncs through it. Delete the folder first, and the other
devices report it as not reachable.

## See also

- [Devices and the owner](devices.md)
- [Relay](relay.md)
- [Security](../security.md)
