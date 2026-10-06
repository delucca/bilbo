# Security

Read this page before you turn sync on. It says what bilbo protects, what it
assumes, and what it does not protect, so you can decide what to sync and where.

In short: notes stay on your machine unless you give them a scope that syncs.
Synced notes leave the machine encrypted, under a key only your devices hold.
Anyone who reaches your devices or your keys can read your notes. Lose the
recovery phrase and every device, and the synced notes are gone.

## What bilbo protects

bilbo protects the text of the notes you sync from whoever stores or carries
them: the tool that syncs your folder, anyone with access to that folder, and
the operator and proxy of a [relay](guides/relay.md).

- A note's text is encrypted under a key only your devices hold. bilbo writes
  into the folder or relay only encrypted, signed files, and each device writes
  only its own.
- A note with no `scope`, a scope the config does not declare, or a scope whose
  `sync` is `off` never leaves the device, in any form. Nothing outside
  `<root>/notes/` syncs: not the library, the vector cache, the config or the
  keys. See [What syncs](guides/sync.md#what-syncs).
- A relay serves only the owners you name with `--owner`. A request that is not
  signed by a key of an owner's scope is refused: strangers can read nothing and
  store nothing in a scope. The one write a stranger gets is a single sealed
  answer into an open pairing mailbox; see [Pairing](#pairing). See also [Public
  exposure](guides/relay.md#public-exposure).
- After a confirmed revocation, a revoked device cannot read anything written
  under later epochs. Until the revoking version is confirmed, writers keep
  using the old epoch (up to 10 minutes on a `file://` folder). See [What
  revoking guarantees](#what-revoking-guarantees).
- Devices are added by a code you compare on two screens, or by the recovery
  phrase. See [Pairing](#pairing).

## What it assumes

- You trust the machine and every program that runs as you. The keys are as safe
  as `~/.ssh/id_ed25519`: any program that runs as you can read them. See [The
  recovery phrase and your keys](#the-recovery-phrase-and-your-keys).
- You trust the terminal while the recovery phrase is on screen. A recording of
  the terminal, `tmux capture-pane` while the words are up, or a person behind
  you still sees them, as they would a sheet of paper.
- You compare the fingerprints and ids on the two screens when you pair a
  device, and the fingerprint you wrote down when you recover one, and you
  answer `y` only when they match.
- You keep the recovery phrase somewhere safe and apart from the machines.

## What sync does not protect

- A stolen or lost device keeps every note and key it held; see [What revoking
  guarantees](#what-revoking-guarantees) for what it can still do.
- A device that holds the owner key can write a second scope named like one of
  yours. When `bilbo device recover` on an empty store names two scope ids, run
  `bilbo device list` afterwards and revoke a device you do not recognize.
- A relay that loses data stays that way: devices never re-upload what it loses.
  See [Back up the data folder](guides/relay.md#back-up-the-data-folder).

### What the folder sees

The folder's tool and anyone with access to the folder see the file names
(device ids and sequence numbers), their sizes and times: not note names, topics
or text, and they cannot read a note.

## What the relay sees

A relay sees the same as a `file://` folder does, and a little more: scope ids,
device ids, each device's public key, the owner's fingerprint, the sizes and
times of the objects, and the address of whoever connects (the proxy's, behind
one).

It never holds a key, and it cannot read a note, a topic, a scope's name or a
file name: those are sealed under keys only your devices hold. The operator of
the machine, which is you, and the proxy see exactly this.

A relay cannot tell a stolen device that signs as the owner from you. [What
revoking guarantees](#what-revoking-guarantees) says what that still allows.

The relay logs one line per object created and per refusal of a device it knows,
and a count of the other refusals once a minute, never an address, a header, a
body or a pairing code. Whether the proxy in front of it logs addresses is up to
the proxy; see [TLS](guides/relay.md#tls).

A relay is also exposed to connection exhaustion. See [Public
exposure](guides/relay.md#public-exposure).

## What revoking guarantees

After a confirmed revocation a revoked device, even one using the owner signing
seed, cannot read anything written under later epochs; it can still disrupt by
signing versions that members reject or that change the device list, which watch
announces.

This holds for the revoked device's own keys, and it works the same through a
relay. What it does not do:

- Until the revocation is confirmed, the revoked device still holds the current
  epoch key and can add a device of its own, which the new epoch is then sealed
  to. Run `bilbo device list` after revoking and revoke any device you do not
  recognize. Writers keep using the old epoch until the revoking version is
  confirmed; on a `file://` folder that takes up to 10 minutes.
- The revoked device keeps every note and key it already had.
- A scope that syncs through a `file://` folder leaves the revoked device with
  write access to the folder. `revoke` prints a line telling you to remove it
  from the cloud account that syncs the folder. Its sync client can still write
  there, and a version it signs that changes the device list shows up as a
  `change` line in `bilbo sync`.
- Holding the owner seed, it can create scopes of its own, one of them named
  like a scope you have not added yet. Check the devices `bilbo device` shows
  for a new scope.
- A revoked device can still write to a relay until you give it a new owner:
  restart the relay with the new fingerprint at the end of the new-owner steps.
  See [Replace the owner](guides/devices.md#replace-the-owner).

When a revoked device keeps disrupting, replace the owner. The steps are in
[Replace the owner](guides/devices.md#replace-the-owner).

## The recovery phrase and your keys

### The recovery phrase

The recovery phrase, 12 words, is the only way back into your scopes when every
device is lost. Lose it and every device, and the encrypted scopes are gone.
bilbo keeps it in no file and never prints it to stdout, and no one can recover
it for you.

Write it down, with the owner fingerprint beside it, and keep it apart from the
machines. `bilbo device recover` rebuilds the owner key from it, so whoever
holds it can act as the owner.

The words are drawn on the terminal's alternate screen, and when the ceremony
ends or you cancel, they are gone from the screen and the scrollback. Before
drawing them, bilbo stops the process from writing a core file. See [The
recovery phrase](guides/devices.md#the-recovery-phrase) for the steps.

### Where keys live

The keys are as safe as `~/.ssh/id_ed25519`: any program that runs as you can
read them. A command that reads the keys refuses when the folder or a file
grants anything to group or others, and names the `chmod` that fixes it. The
layout, modes and removal are in [Where keys
live](guides/devices.md#where-keys-live).

Secrets are zeroed in memory as far as bilbo can. The terminal emulator, swap
and the prompt library's line buffers are not covered.

### What the terminal rule does not stop

The [terminal rule](guides/devices.md#commands-that-need-a-terminal) stops
accidental and low-effort misuse, such as an agent that reaches for a one-line
verb.

It does not stop a hostile agent, which can unset the variables, fake a
terminal, or copy the key files. The key files are the real boundary.

## Pairing

`--scope` decides what the new device can read: a stolen paired device reads
only the scopes it was paired into. What a revoked or stolen device can still do
is in [What revoking guarantees](#what-revoking-guarantees).

Showing a code needs the same terminal rule, with the same limit: the
confirmation is yours to give, not an agent's, and the key files' modes are the
last line.

Whoever can write the shared folder can stop a pairing, by removing the mailbox
or answering first, but cannot read it. The code never crosses the folder, and
the mailbox holds no key, scope name or URL in the clear. A wrong guess gets one
attempt, which uses the code up and shows on both screens.

Through a relay, a stranger gets at most one write into a mailbox, and behind a
proxy the limits on unsigned requests are shared by everyone using it, so an
abuser can stall a pairing for up to 10 minutes. They cannot read it or store
anything. See [Pair and recover through a
relay](guides/relay.md#pair-and-recover-through-a-relay).

## Two local privacy controls

Sync is not the only way a note can leave your machine.

- **The embedder.** A remote embedder receives the passages of the notes it
  indexes. With a remote embedder, every `recall` query (once the index holds
  a vector) and the first 1,000 bytes of every prompt the digest sees go to it
  too, so pick the embedder with that in mind. A note whose scope sets `embedder = local` must not reach one:
  `bilbo index` sends none of its passages to an `embedder.url` that is not on
  `localhost`, `127.0.0.1` or `::1`, unless a note whose rule is `any` holds an
  identical passage, which is then sent once. The rule trusts the URL's host as
  written, so an ssh tunnel on `localhost` counts as local while the text leaves
  the machine through it. See [The embedder
  rule](guides/embedders.md#the-embedder-rule).
- **The digest log.** With `digest.log = on`, each digest run writes the first
  500 characters of the prompt, in plain text, to `digest.jsonl` under bilbo's
  state folder. The file is mode 0600 and the setting is off by default. See
  [The digest](guides/agents.md#the-digest).

## See also

- [Devices and the owner](guides/devices.md)
- [Sync](guides/sync.md)
- [Relay](guides/relay.md)
