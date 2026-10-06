# device-pairing Specification

## Purpose
`bilbo pair` enrolls a new device, or adds an enrolled one, into the user's syncing scopes with a short one-time code typed from an enrolled device, so the recovery phrase stays put away. The two devices meet in a mailbox on the transport the scopes already use.

## Requirements

### Requirement: Show a pairing code
`bilbo pair [--scope <name>]... [--via <url>]` on an enrolled device SHALL create a mailbox on the transport and print on stderr a code `<nameplate>-<word>-<word>-<word>`, with a random nameplate from 1 to 999 and three random words from the BIP39 English list, the command to run on the new device, and that the code works once for 10 minutes. It SHALL then wait for the new device. stdout SHALL carry only the final `paired` line.

#### Scenario: A code is shown
- **WHEN** a device enrolled with scope `personal` syncing through `file:///srv/sync` runs `bilbo pair`
- **THEN** stderr holds `pairing code <code>`, where `<code>` matches `^[1-9][0-9]{0,2}(-[a-z]+){3}$` with every word in the BIP39 English list, and `on the new device, run: bilbo pair <code> --via file:///srv/sync`; stdout is empty; and `/srv/sync/pair/<nameplate>/a.msg` exists

#### Scenario: Two pairings at once
- **WHEN** a second `bilbo pair` runs while the first still waits on the same transport
- **THEN** the two codes have different nameplates

#### Scenario: No free nameplate
- **WHEN** A picks 20 nameplates in a row whose mailboxes already exist
- **THEN** stderr says `no free pairing number at <url>; try again later`, the exit code is 1, and no mailbox is created

#### Scenario: A stray argument
- **WHEN** a user runs `bilbo pair --scope`
- **THEN** bilbo prints a usage message to stderr, exits 2, and creates no mailbox

### Requirement: Join with a code
`bilbo pair <code> --via <url> [--name <name>]` on a new device, or an enrolled one, SHALL find the mailbox the code names on that transport and join the device to the scopes paired, under its name or `--name`. The code SHALL be read in any case, with hyphens or spaces between its parts, and with any word given by its first four or more letters when they name one word of the list. A code that is not a number and three list words SHALL be a usage error that touches no mailbox.

#### Scenario: Joining
- **WHEN** device A shows `42-orbit-tunnel-velvet`, B runs `bilbo pair 42-orbit-tunnel-velvet --via file:///srv/sync`, and the user confirms on A
- **THEN** both exit 0, A's stdout is `paired <B's name> <B's device id>: personal`, and B's stdout is `paired with <A's name>: personal` and then `bilbo watch starts syncing them within one cycle`

#### Scenario: A loosely typed code
- **WHEN** B runs `bilbo pair "42 ORBI tunn velvet" --via file:///srv/sync` for the code `42-orbit-tunnel-velvet`
- **THEN** pairing goes ahead as with the exact code

#### Scenario: Not a pairing word
- **WHEN** B runs `bilbo pair 42-orbit-tunel-velvet --via file:///srv/sync`
- **THEN** stderr says `'tunel' is not a pairing word`, the exit code is 2, nothing is written to `/srv/sync`, and the code still works

#### Scenario: No transport given
- **WHEN** B runs `bilbo pair 42-orbit-tunnel-velvet` without `--via`
- **THEN** bilbo prints a usage message to stderr and exits 2

#### Scenario: No such mailbox
- **WHEN** B runs `bilbo pair 43-orbit-tunnel-velvet --via file:///srv/sync`, no mailbox 43 appears there within 2 minutes, and A's code is `42-orbit-tunnel-velvet`
- **THEN** stderr says `no pairing 43 at file:///srv/sync`, the exit code is 1, and A's code still works

### Requirement: The transport URL on the new device
The new device SHALL take its transport only from `--via`: a `file:///<absolute path>` URL, or `https://` or loopback `http://` as the config spec's sync URLs allow, the last two reaching a relay as the `relay-transport` spec says. For each scope paired, it SHALL use the `--via` URL when the scope's URL on A is literally the URL A pairs over, and the scope's own URL when that is a relay URL (`https://`, or `http://` to a loopback host). It SHALL write `--via` without a trailing slash. A `--via` folder that does not exist SHALL be refused.

#### Scenario: The folder has another path
- **WHEN** A syncs `personal` through `file:///Users/a/Dropbox/bilbo` and B runs `bilbo pair <code> --via file:///home/a/Dropbox/bilbo`, the same synced folder
- **THEN** B's config holds `scope.personal.sync = file:///home/a/Dropbox/bilbo`, and no manifest version is written for the path, since a manifest pins only `file://` for a folder

#### Scenario: A missing folder
- **WHEN** B runs `bilbo pair <code> --via file:///nope`
- **THEN** stderr says `no folder at /nope`, the exit code is 1, and the code still works

#### Scenario: A remote plain-HTTP URL
- **WHEN** B runs `bilbo pair <code> --via http://relay.example:8090`
- **THEN** bilbo prints a message naming the URL to stderr, exits 2, and touches no mailbox

#### Scenario: A relay URL
- **WHEN** A syncs `personal` through `https://relay.example`, shows a code over it, and B runs `bilbo pair <code> --via https://relay.example`
- **THEN** the two devices pair through the relay's mailbox, and B's config holds `scope.personal.sync = https://relay.example`

#### Scenario: Two relays
- **WHEN** A syncs `personal` through one relay and `work` through another, and B pairs with `--via` naming the first
- **THEN** B's config holds each scope's own relay URL, B reads `work`'s manifests from the second relay, and only the first relay's mailbox carries the pairing

### Requirement: Confirm the fingerprint
Once the new device has answered, both devices SHALL print the same fingerprint, twelve digits in three groups of four, derived from the session, and B SHALL print its own name and device id with it. A SHALL name the new device, its device id and the scopes it will join, read one line from stdin, and enroll it only when that line is `y` or `yes`, given before the code expires. Otherwise A SHALL send no secret, and both devices SHALL exit 1.

#### Scenario: Matching fingerprints
- **WHEN** B answers A's code
- **THEN** A's stderr holds `fingerprint <F>` and the question naming B's name and id, and B's stderr holds `fingerprint <F> for <B's name> <B's id>; confirm on the device that showed the code`, with the same `<F>`

#### Scenario: Declined
- **WHEN** the user types `n` on A
- **THEN** A says `not confirmed; nothing was sent`, B says `the other device declined; nothing was received`, both exit 1, no manifest changes, and B holds no keys

#### Scenario: End of input
- **WHEN** the user ends input at A's question without typing a line
- **THEN** A says `not confirmed; nothing was sent`, and both devices exit 1

### Requirement: One attempt per code
A code SHALL accept one answer. When the answer was made with a wrong code, A SHALL print `the other device used a wrong code; this code is used up, run bilbo pair again`, send no secret and exit 1, and B SHALL print `wrong code; run bilbo pair again on the other device for a new one` and exit 1. A second answer to the same mailbox SHALL be refused at once, even with the right code.

#### Scenario: A wrong word
- **WHEN** A shows `42-orbit-tunnel-velvet` and B runs `bilbo pair 42-orbit-tunnel-vessel --via file:///srv/sync`
- **THEN** both exit 1 with those messages, no manifest changes, and B holds no keys

#### Scenario: Retrying the right code after a wrong one
- **WHEN** B then runs `bilbo pair 42-orbit-tunnel-velvet --via file:///srv/sync`
- **THEN** stderr says `code 42 was already used; run bilbo pair again on the other device` without waiting, and the exit code is 1

#### Scenario: A second answer while the first is pending
- **WHEN** a device has answered code 42 and A has not replied yet, and another device runs `bilbo pair 42-orbit-tunnel-velvet --via file:///srv/sync`
- **THEN** that device prints `code 42 was already used; run bilbo pair again on the other device`, exits 1, and A still handles only the first answer

### Requirement: Expiry
A SHALL send its reply within 10 minutes of showing the code; past that it SHALL print `the code expired; nothing was sent`, send no secret and exit 1, removing an unanswered mailbox. B SHALL wait up to 2 minutes for the mailbox to appear and up to 10 minutes from its start for A's reply, then exit 1 with `no answer from the other device`. After the reply, B SHALL wait up to 2 more minutes for the manifests to reach it.

#### Scenario: Nobody answers
- **WHEN** A shows a code and no device answers within 10 minutes
- **THEN** A exits 1 with the expiry message and `pair/<nameplate>/` is gone from the folder

#### Scenario: Too late
- **WHEN** B types the code after A has expired it
- **THEN** stderr says `no pairing <nameplate> at <url>` and the exit code is 1

#### Scenario: Confirmed too late
- **WHEN** B answered in time but the user types `y` on A after the 10 minutes
- **THEN** A says `the code expired; nothing was sent`, no manifest changes, and both exit 1

#### Scenario: A late manifest
- **WHEN** the user confirms at minute 9 and the synced folder delivers A's new manifest to B 90 seconds after A's reply
- **THEN** pairing succeeds

#### Scenario: A manifest that never arrives
- **WHEN** A's new manifest has not reached B 2 minutes after A's reply
- **THEN** B says `the personal manifest did not reach <url> in time; run bilbo pair again`, writes nothing, and exits 1

### Requirement: What the new device receives
On confirmation, A SHALL write, per scope paired, a version listing B with the epoch key sealed to B (pending, as the `scope-manifest` spec says), unless the newest lists B's id. It SHALL then send the owner signing seed, unless B is enrolled, and per scope its name, id, embedder setting, and newest version number and hash. B SHALL fetch versions 1 to n and check, before writing, the hash, each `prev`, each `owner` against the seed, every signature, B's listing and its sealed key.

#### Scenario: After pairing
- **WHEN** pairing succeeds for `personal`, whose newest manifest on A was version 3
- **THEN** B's store holds `personal` manifest versions 1 to 4, `bilbo device` on B prints the same owner fingerprint as on A and reports the scope valid, and version 4 on the transport lists B; B's store has no `manifest/4.pending`, and A's version 4 stays pending until A reads it back from the transport

#### Scenario: A manifest that does not verify
- **WHEN** the newest `personal` manifest B fetches does not match the hash A sent, an earlier version is missing or does not match its successor's `prev`, a version's `owner` is not the public key of the signing seed A sent, or a signature does not match it
- **THEN** B prints `the personal manifests on the transport do not match what the other device sent; nothing was written`, exits 1, and writes no key, manifest or config line

#### Scenario: Pairing again after an interrupted pairing
- **WHEN** B was listed in `personal` but stopped before writing its keys, and pairing runs again on B
- **THEN** B answers with the same device id, A writes no new `personal` manifest version, and B ends enrolled

### Requirement: Where the new device keeps it
A B without keys SHALL keep its pairing device key outside its keys folder until it succeeds, and reuse it next run. B SHALL write each manifest under `<root>/.bilbo/scopes/<scope id>/` as a confirmed copy, with no `.pending` marker, then its config, then, when it had none, its keys where `bilbo device` keeps them. In the config it SHALL set `scope.<name>.sync`, set `scope.<name>.embedder = local` when either device has it so, and keep every other line, with the old file kept as `<name>.bak`.

#### Scenario: The config after pairing
- **WHEN** B's config holds `embedder.url = http://127.0.0.1:8081`, A has `scope.personal.embedder = local`, and pairing succeeds for `personal`
- **THEN** B's config holds the embedder line unchanged, `scope.personal.sync = <the --via URL>` and `scope.personal.embedder = local`, and no `scope.personal.paths` line

#### Scenario: B's stricter embedder stays
- **WHEN** B's config already holds `scope.personal.embedder = local` and A has `any`
- **THEN** B's config still holds `scope.personal.embedder = local`

#### Scenario: Notes that already carry the scope
- **WHEN** B's store holds 50 notes with `scope: personal` under `scope.personal.sync = off`, and pairing succeeds for `personal`
- **THEN** B's stderr holds `50 notes already carry scope: personal and sync from now on` before its stdout lines

#### Scenario: The config cannot be written
- **WHEN** everything verifies but B cannot write its config file
- **THEN** B prints `cannot write <path>` and exits 1, its keys folder holds no key, and pairing again with a new code, once the file is writable, finishes with the same device id

### Requirement: The phrase stays hidden
Pairing SHALL NOT print, send or store the recovery phrase, on either device. Nothing pairing writes to the transport SHALL hold a key, a scope name or a URL in the clear.

#### Scenario: No phrase anywhere
- **WHEN** pairing succeeds
- **THEN** neither device's stdout or stderr holds any run of three consecutive words of the phrase, and no file pairing wrote holds the phrase

#### Scenario: The mailbox is opaque
- **WHEN** the mailbox files are read before B removes them
- **THEN** none of them holds the owner key, an epoch key, the string `personal` or the transport URL

### Requirement: Scopes to pair
A SHALL pair every scope whose `scope.<name>.sync` is a URL, or only those named by repeated `--scope <name>`, at most 12. A scope that does not sync, or whose `file://` URL is not literally the URL A pairs over, SHALL be a usage error, as SHALL a `--via` that no paired scope uses. Without `--via`, A SHALL pair over the one URL the scopes share.

#### Scenario: Narrowing
- **WHEN** `personal` and `shared` sync through `file:///srv/sync` and A runs `bilbo pair --scope shared`
- **THEN** A names only `shared` before the confirmation, and after pairing B's config sets only `scope.shared.sync`

#### Scenario: A scope that does not sync
- **WHEN** A runs `bilbo pair --scope client` and `scope.client.sync` is `off`
- **THEN** stderr says `scope client does not sync`, the exit code is 2, and no mailbox is created

#### Scenario: Scopes in two folders
- **WHEN** `personal` syncs through `file:///srv/a` and `shared` through `file:///srv/b`, and A runs `bilbo pair`
- **THEN** bilbo exits 2 with a message that names both URLs and `--via`, and creates no mailbox

#### Scenario: The same folder written two ways
- **WHEN** `personal` syncs through `file:///srv/sync/` and `shared` through `file:///srv/sync`, and A runs `bilbo pair`
- **THEN** bilbo exits 2 naming both URLs, because they differ as written

#### Scenario: A --via no scope uses
- **WHEN** `personal` syncs through `file:///srv/sync` and A runs `bilbo pair --via file:///srv/other`
- **THEN** stderr says `no scope paired syncs through file:///srv/other`, the exit code is 2, and no mailbox is created

#### Scenario: Too many scopes
- **WHEN** 13 scopes sync and A runs `bilbo pair` without `--scope`
- **THEN** stderr says that one pairing carries at most 12 scopes and names `--scope`, the exit code is 2, and no mailbox is created

### Requirement: Who can pair
A SHALL refuse, before creating a mailbox, when it holds no owner key, when no scope syncs, or unless stdin and stderr are terminals and neither `CLAUDECODE` nor `CODEX_THREAD_ID` is set and not empty. Before asking, A SHALL refuse a device of another owner, or whose name is A's own or another device's in any manifest of the owner. B SHALL refuse with no `<root>/notes/`, and before writing, a store of another owner. Each refusal SHALL exit 1.

#### Scenario: The phrase was never set
- **WHEN** a device with no owner key runs `bilbo pair`
- **THEN** stderr says `this device has no owner key; turn on sync for a scope first, which sets the recovery phrase`, the exit code is 1, and nothing is written

#### Scenario: An enrolled device joins another scope
- **WHEN** `bywater` is enrolled and listed in `personal`, A runs `bilbo pair --scope shared`, and `bywater` answers
- **THEN** A adds `bywater` to `shared` only, the reply carries no owner seed, `bywater`'s keys folder is unchanged, and its config gains `scope.shared.sync`

#### Scenario: Enrolled with another owner
- **WHEN** the answering device is enrolled with another owner
- **THEN** A says `<B's name> belongs to another owner (<its fingerprint>)`, B says `this device belongs to owner <its fingerprint>, the other device to <A's>`, nothing is sent, and both exit 1

#### Scenario: An agent shows a code
- **WHEN** an agent runs `bilbo pair` with stdin not a terminal, or with `CLAUDECODE=1` or `CODEX_THREAD_ID` set
- **THEN** stderr says `pairing is confirmed only in a terminal, by the user`, no mailbox is created, and the exit code is 1

#### Scenario: A taken name
- **WHEN** `personal` lists a device named `bywater` and a new device named `bywater` answers
- **THEN** A says `a device named bywater is already enrolled; pair again with --name on the new device`, B says `the name bywater is taken; run bilbo pair again with --name`, nothing is sent, and both exit 1

#### Scenario: A name taken in a scope not being paired
- **WHEN** `personal` lists a device named `bywater`, A runs `bilbo pair --scope shared`, and a new device named `bywater` answers
- **THEN** A refuses it as a taken name, and no manifest changes

#### Scenario: The showing device's own name
- **WHEN** A is named `rhosgobel` and the new device answers as `rhosgobel`
- **THEN** A refuses it as a taken name

#### Scenario: A store of another owner
- **WHEN** B holds no keys but its store holds a `personal` manifest signed by another owner key
- **THEN** B prints both owner fingerprints and `nothing was written`, writes no key, manifest or config line, and exits 1

#### Scenario: No store on the new device
- **WHEN** B's `BILBO_HOME` has no `notes/`
- **THEN** stderr says `no store at <root>`, the exit code is 1, and the code still works

### Requirement: Format versions
Every mailbox message SHALL carry a format number. A device that reads a format it does not know SHALL print `the other device runs a newer bilbo; update this one and pair again` and exit 1.

#### Scenario: A newer device shows the code
- **WHEN** `a.msg` carries format 2 and B knows only format 1
- **THEN** B prints that message, exits 1, and writes no answer

### Requirement: The mailbox
A mailbox SHALL be `pair/<nameplate>/` of the transport, laid out as the `sync-transport` spec says: `a.msg` and `c.msg` from A, `b.msg` from B, each created once and at most 4 KiB. `c.msg` SHALL carry one result: `enrolled` with the sealed payload, else `wrong-code`, `declined`, `expired`, `name-taken` or `other-owner` with no secret; `other-owner` SHALL carry A's owner signing public key sealed under the session, so the transport never sees it. On `file://`, B SHALL remove it after any result but `wrong-code`, A when its code expires unanswered, and any `bilbo pair` when `a.msg` is over 30 minutes old.

#### Scenario: Cleaned up after success
- **WHEN** pairing over `file:///srv/sync` succeeds
- **THEN** `/srv/sync/pair/<nameplate>/` no longer exists

#### Scenario: Kept after a wrong code
- **WHEN** B has read a `wrong-code` result
- **THEN** `/srv/sync/pair/<nameplate>/` still exists, until a `bilbo pair` runs more than 30 minutes after its `a.msg`

#### Scenario: A stale mailbox
- **WHEN** `/srv/sync/pair/7/a.msg` is 31 minutes old and a device runs `bilbo pair`
- **THEN** `/srv/sync/pair/7/` is gone

#### Scenario: A fresh mailbox stays
- **WHEN** another device's mailbox is 5 minutes old and a device runs `bilbo pair`
- **THEN** that mailbox is still there
