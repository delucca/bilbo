# Devices and the owner

A device is one machine's copy of bilbo. The owner is you: one identity that
every device of yours shares. This page shows how to create that identity, add a
second device, pair devices, and revoke one. [Sync](sync.md) signs and encrypts
with these identities. [Security](../security.md) says what they protect and
what they do not.

## Commands

| Command | What it does |
| --- | --- |
| `bilbo device` | Prints this device, the owner fingerprint and one line per syncing scope. Exits 1 when it prints a problem line for a scope. |
| `bilbo device list` | Lists the enrolled devices: name, id, and `this` for this one. |
| `bilbo device init [--name <name>]` | Creates the recovery phrase, the owner key and this device's key, then a manifest for each scope with a sync URL. Run again, it brings the manifests in line with the config. |
| `bilbo device recover [--name <name>]` | Rebuilds the owner key from the typed phrase on a new or wiped device and adds the device to the scopes the store holds. |
| `bilbo device revoke <device>` | Removes a device, by name or id, from every scope that lists both it and this device, and starts a new epoch key. |

The device name is the host name up to its first `.`, lowercased, unless
`--name` gives one: lowercase words joined by single hyphens, at most 32
characters. A device's id is derived from its key.

## The recovery phrase

`bilbo device init` on a device with no keys shows 12 words, the recovery
phrase, and the owner fingerprint, six groups like
`yb4b-5aju-v6zb-x2nm-nc5x-ompf`. Run it in a terminal.

1. Write down the phrase, and the fingerprint beside it.
2. Type 3 of the words back when bilbo asks. It writes nothing until they match.

The phrase is the only way back into your scopes when every device is lost.
bilbo keeps it in no file and never prints it to stdout. The words are drawn on
the terminal's alternate screen, and they are gone from the screen and the
scrollback when the ceremony ends or you cancel.
[Security](../security.md#the-recovery-phrase-and-your-keys) covers what that
does and does not protect.

### Commands that need a terminal

`recover`, `revoke` and any `init` that creates a phrase or changes a scope's
pinned URL run only in a terminal. A folder's path is not pinned. Both stdin and
stderr must be a terminal, and no agent marker may be set: `AI_AGENT`,
`CLAUDE_CODE_CHILD_SESSION`, `CODEX_THREAD_ID` or `CODEX_CI`, each unset or
empty. `CLAUDECODE` alone, as in an IDE's terminal, does not count. An agent
that runs them gets one line asking you to run the form yourself.

Running `bilbo device init` again on an enrolled device needs no terminal, so an
agent can create the manifest of a scope whose URL you added.
[Security](../security.md#what-the-terminal-rule-does-not-stop) says how far
this rule goes.

## Where keys live

```
<state>/bilbo/keys/
  owner.key     0600
  device.key    0600
```

`<state>` is `$XDG_STATE_HOME`, else `~/.local/state`. The folder is 0700. The
keys live outside the store, so copying or backing up the store never clones an
identity.

- `owner.key` holds the owner's signing seed and public key, never the phrase
  and never the owner's private box key.
- `device.key` holds this device's own keys and name.

A command that reads the keys refuses when the folder or a file grants anything
to group or others, and names the `chmod` that fixes it.

`bilbo setup --remove` leaves the keys. To forget an identity, delete
`<state>/bilbo/keys/` and `<root>/.bilbo/scopes/` by hand.

## Add a device with the phrase

Use the phrase when no enrolled device is left. While one is,
[pair](#pair-a-device) instead, so the phrase stays put away.

`bilbo device init` on a second machine would make a second owner, so it refuses
once the store holds another owner's manifests.

When the scope's `sync` is a `file://` folder or a [relay](relay.md) URL, set it
in the config and run `bilbo device recover`. It copies the owner's scope from
there, as [Sync](sync.md#add-a-second-device) describes.

Otherwise:

1. Copy the store to the new machine, with the sync tool you already use or a
   plain copy.
2. Run `bilbo device recover` and type the phrase.

`recover` shows the fingerprint it derived. When a manifest in the store vouches
for the phrase, it asks nothing more. Otherwise it asks you to compare the
fingerprint with the one you wrote down. It then writes this device's keys and
adds the device to every scope of yours the store holds.

A syncing scope with no manifest in the copied store or the folder is reported
`unsealed`. When another device holds that scope, bring its manifest over (copy
the store again, or let the folder finish syncing) and run `recover` again.
`bilbo device init` would create a second scope of the same name beside it. Run
`init` only for a scope that no device holds yet, which it then creates.

## Pair a device

`bilbo pair` adds a device with a short code typed from one you already have, so
the recovery phrase stays put away. On the enrolled device, in a terminal:

```console
$ bilbo pair
┌  bilbo pair
│
◇  On the new device, run ────────────────────────────────────────────────╮
│                                                                         │
│  bilbo pair 42-orbit-tunnel-velvet --via file:///Users/me/Dropbox/bilbo  │
│                                                                         │
│  The code is 42-orbit-tunnel-velvet. It works once, for 10 minutes.     │
├─────────────────────────────────────────────────────────────────────────╯
```

A spinner, `Waiting for the new device`, runs under the box until the new device
answers and is erased then. The box holds the command the way you copy it. When
the command is too wide for the terminal, it prints on its own line above the
box, which is titled `Pairing code` and holds only the code, so a copy never
carries a border. An agent never gets this view, or the code: showing one needs
a person at a terminal.

On the new device, install bilbo, run `bilbo setup` so the store exists, and
type the code with the folder as that machine sees it:

```sh
bilbo pair 42-orbit-tunnel-velvet --via file:///home/me/Dropbox/bilbo
```

The code is a number and three words. Case, spaces for hyphens and the first
four letters of a word are all accepted: `"42 ORBI tunn velvet"` is the same
code.

`--via` is the folder's path on the new device, which differs from the path on
the first one. bilbo writes it to the new device's config as the scope's `sync`.
With a relay it is the relay's URL, as the first device shows it; see [Pair and
recover through a relay](relay.md#pair-and-recover-through-a-relay).

The first device pairs every syncing scope, or only the ones `--scope <name>`
names, up to 12. `--scope` picks which scopes the new device can read. Scopes on
different folders need one `bilbo pair --scope <name>...` each, naming the
scopes of one folder. bilbo refuses and names both URLs when it cannot tell.

Both devices then show the same fingerprint, twelve digits in three groups of
four, and the new one adds its name and device id. The first device names the
new one and the scopes it will join, and asks:

```console
│
●  bywater q4n7rj2dxwmk5ta3hz6pyce4lu asks to join personal
│
◆  Fingerprint 5812 0934 7761: does bywater show the same?
│  ○ Yes / ● No
└
```

The new device shows a box titled `Fingerprint` with the same digits, its name and
its id, and the line `Confirm on the device that showed the code`, each wait
under a spinner (`Looking for pairing 42`, `Waiting for the other device to
confirm`, `Fetching the scopes`). When an agent runs `bilbo pair <code>` it gets
the plain line instead: `bilbo: fingerprint 5812 0934 7761 for bywater
q4n7rj2dxwmk5ta3hz6pyce4lu; confirm on the device that showed the code`.

Compare the fingerprint, the name and the id on the two screens, and answer yes
only when they match. The question starts on no, so Enter alone declines. No,
Esc, Ctrl-C and the end of input send no secret, close the drawing with `Not
paired`, and both devices exit 1; Esc ends the new device at once instead of
leaving it to wait out the code.

The new device then waits up to 2 minutes for the manifests to reach it through
the folder, checks the whole chain, and only then writes its config and keys.
The first device closes with `Paired` and prints `paired bywater <id>:
personal`. The new one closes with `Paired with rhosgobel` and prints `paired
with rhosgobel: personal`, and `bilbo watch` starts syncing the scope within one
cycle.

### Codes and terminals

A code works for one answer and for 10 minutes. A wrong word uses it up, and
both devices say so; run `bilbo pair` again for a new code. A mailbox nobody
answered is removed when the code expires, and any `bilbo pair` removes one
whose first message is more than 30 minutes old.

Showing a code needs a terminal, under [the same
rule](#commands-that-need-a-terminal) as the recovery phrase. The confirmation
is yours to give, not an agent's.

### Join a scope from an enrolled device

An enrolled device pairs too, to join a scope it is not in. Run
`bilbo pair --scope shared` on a device in `shared` and answer on the other with
`bilbo pair <code> --via <url>`. Its keys do not change, and it must belong to
the same owner.

### Finish an interrupted pairing

A new device can stop after it checks the manifests, for example on a config it
cannot write. It keeps those manifests but no keys. Pairing it again with the
same owner finishes. Pairing it with another owner is refused until its store is
cleared.

When no enrolled device is left, use [the phrase](#add-a-device-with-the-phrase)
with `bilbo device recover`.

[Security](../security.md#pairing) says what a paired, stolen or hostile party
can and cannot do.

## Revoke a device

Run `bilbo device revoke bywater`, in a terminal on another device. It writes a
new version of each scope that lists both `bywater` and this device, under a new
epoch key sealed only to the remaining devices and to you. What that guarantees,
and what it does not, is in [Security](../security.md#what-revoking-guarantees).

After you revoke:

1. Run `bilbo device list` and revoke any device you do not recognize. Until the
   revocation is confirmed, the revoked device still holds the current epoch key
   and can add a device of its own, which the new epoch is then sealed to.
2. When the scope syncs through a `file://` folder, remove the device from the
   cloud account that syncs the folder. `revoke` prints a line telling you to.
3. Check the devices `bilbo device` shows for a new scope. Holding the owner
   seed, a revoked device can create scopes of its own, one of them named like a
   scope you have not added yet.

### Replace the owner

When a revoked device keeps disrupting, the remedy is a new owner, done by hand:

1. On a trusted device, move `<state>/bilbo/keys/` and `<root>/.bilbo/scopes/`
   aside.
2. Run `bilbo device init`, which makes a new phrase, a new owner and new scope
   ids.
3. Recover the other devices from the new phrase, and give any relay the new
   fingerprint.

## See also

- [Sync](sync.md) for scopes, folders and conflicts.
- [Relay](relay.md) to sync without a shared folder.
- [Security](../security.md) before you turn sync on.
