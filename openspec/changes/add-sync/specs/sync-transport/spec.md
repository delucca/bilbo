# Spec Delta

## Purpose
The transport is where devices leave each other their notes: a tree of encrypted, signed objects that each device only ever adds to. This spec fixes the tree, the write rule and the segment envelope, which the `file://` folder serves here and the relay serves later.

## ADDED Requirements

### Requirement: Transport URLs
A scope's transport SHALL be named by its `scope.<name>.sync` URL. A `file:///<absolute path>` URL SHALL name a folder that holds the tree. For a URL of any other scheme, watch SHALL print `bilbo: sync <name>: <scheme> transports are not supported yet; use a file:// folder` once and sync nothing for that scope.

#### Scenario: A folder transport
- **WHEN** the config holds `scope.personal.sync = file:///Users/a/Dropbox/bilbo`
- **THEN** watch reads and writes the tree under `/Users/a/Dropbox/bilbo`

#### Scenario: A relay URL
- **WHEN** the config holds `scope.personal.sync = https://relay.example`
- **THEN** watch prints the not-supported line naming `https` and writes nothing for `personal`

### Requirement: Transport layout
The transport SHALL hold only these objects: `scopes/<scope id>/manifest/<n>.json`, the manifest versions; `scopes/<scope id>/devices/<device id>/<seq>.seg`, each device's segments; and `pair/<nameplate>/<msg>.msg`, the short-lived pairing mailbox, `<nameplate>` being 1 to 64 and `<msg>` 1 to 16 of `[a-z0-9-]`. Scope and device ids are the 26-character base32 ids of the `scope-manifest` and `device-identity` specs. `<n>` is a decimal number from 1; `<seq>` one from 1, zero-padded to 20 digits. bilbo SHALL ignore any other name.

#### Scenario: The first segment
- **WHEN** device `abcdefghijklmnopqrstuvwxyz` pushes its first segment for scope `234567abcdefghijklmnopqrst`
- **THEN** the folder holds `scopes/234567abcdefghijklmnopqrst/devices/abcdefghijklmnopqrstuvwxyz/00000000000000000001.seg`

#### Scenario: Mailbox names
- **WHEN** the transport is asked to create `pair/7/a.msg`, and the folder also holds `pair/7/A_1.msg`
- **THEN** it creates `pair/7/a.msg`, and lists and reads nothing of `pair/7/A_1.msg`

#### Scenario: A cloud tool's conflict copy
- **WHEN** a sync tool leaves `00000000000000000003 (conflicted copy).seg` in a device folder
- **THEN** bilbo ignores that file

### Requirement: Create-only writes
Writing through a hidden temporary file named `.<device id>-<16 lowercase hexadecimal characters>.tmp` in the target folder, a device SHALL NOT overwrite, append to, rename or delete an object, with two exceptions on a `file://` transport: removing a pairing mailbox deletes `pair/<nameplate>/`, the one deletion allowed; and the Damaged own segments requirement replaces a damaged file in the device's own folder. It SHALL create a missing object again only with the same bytes. Creating an existing object SHALL fail.

#### Scenario: Only the owner writes its folder
- **WHEN** devices A and B sync one scope through a test run of edits, merges and deletions
- **THEN** every file under A's device folder was created by A, none was changed after it was created, and the same holds for B

#### Scenario: Removing a mailbox
- **WHEN** the `file://` transport is asked to remove nameplate `7`
- **THEN** `pair/7/` is gone, and nothing under `scopes/` changed

### Requirement: One writer per device folder
A device SHALL write segments only in its own device folder, so no segment has two writers. When a segment there verifies as signed by this device but this store neither wrote nor read it, watch SHALL print `bilbo: sync <name>: segment <seq> of this device holds other content; another store writes as this device` and stop pushing that scope. A store that lost its sync state SHALL first resume from the transport, as Resuming after lost state says.

#### Scenario: Two stores with one device key
- **WHEN** two stores on one machine share a device key and both sync `personal` through one folder
- **THEN** the second one to push prints the other-content line and stops pushing `personal`, and no segment is overwritten

### Requirement: Resuming after lost state
When a store has no sync state for a scope, as after `<root>/.bilbo/` was removed, watch SHALL read every segment of the scope from seq 1, its own device folder included, and push from the highest seq of its own folder plus one.

#### Scenario: The history folder was deleted
- **WHEN** A had pushed 40 segments to `personal`, the user deletes A's `<root>/.bilbo/` and A's watch starts
- **THEN** A reads its 40 segments, pushes its next versions as segment 41, and prints no other-content line

### Requirement: Damaged own segments
On a `file://` transport, when a file in its own device folder does not verify as a segment this device signed, the device SHALL replace it with the copy it kept, and when it kept none, print `bilbo: sync <name>: segment <seq> of this device is damaged and no copy is left` and keep pushing after it.

#### Scenario: A truncated segment
- **WHEN** a cloud tool truncates A's segment 7 before B applied it
- **THEN** at A's next poll the file holds its full bytes again, and B applies it

### Requirement: Segment envelope
A segment SHALL be one JSON object with exactly the keys `format` (1), `scope`, `device`, `seq`, `epoch` (its manifest epoch), `nonce`, `ciphertext` and `sig`, the last three in padded standard base64. `ciphertext` SHALL be XChaCha20-Poly1305 under the epoch's key and the 24-byte nonce, with associated data `bilbo-segment-1`, scope, device, seq and epoch, each ending in a newline. `sig` SHALL be the device's Ed25519 signature over that data, then `nonce` and `ciphertext`, each ending in a newline.

#### Scenario: Nothing readable in the clear
- **WHEN** a note with topic `release-plan` and text `ship on friday` syncs in scope `personal`
- **THEN** no file in the transport holds `release-plan`, `ship on friday` or `personal`

#### Scenario: A tampered segment
- **WHEN** one byte of a segment's `ciphertext` is changed
- **THEN** every other device refuses it, applies none of its versions, and prints a line naming the device and seq

### Requirement: Segment size
A segment file SHALL be at most 8 MiB. Versions that do not fit one segment SHALL be pushed in several, in consecutive seqs.

#### Scenario: A large first push
- **WHEN** a user assigns notes holding 20 MiB of text to a syncing scope
- **THEN** the device writes at least three segments, each at most 8 MiB

#### Scenario: A small push
- **WHEN** an agent saves one 4 KiB note
- **THEN** the device writes one segment for it

### Requirement: Reading segments in order
A device SHALL apply each other device's segments in seq order, and only when the signature verifies against a device that some confirmed version at the segment's epoch lists and that the latest confirmed version lists, and it decrypts. A device the latest manifest dropped SHALL be read no further than the seq applied when that version was adopted. On a missing seq, a failing segment or an unknown `format`, it SHALL stop reading that device, print a line naming the device and seq, and retry each poll.

#### Scenario: A gap
- **WHEN** B's folder holds segments 1, 2 and 4
- **THEN** A applies 1 and 2, prints a line naming B and segment 3, and applies 3 and 4 once 3 appears

#### Scenario: A revoked device keeps writing
- **WHEN** A adopted a version that drops C, having applied C's segments up to 12, and C then writes segment 13 under the old epoch
- **THEN** A never applies segment 13

#### Scenario: A device added without a rotation
- **WHEN** version 1 of `personal` lists A at epoch 1, version 2 adds B at epoch 1, and B pushes segments sealed under epoch 1
- **THEN** A applies them

#### Scenario: A newer format
- **WHEN** B writes a segment with `format` 2
- **THEN** A applies none of it, and prints a line naming B and saying to upgrade bilbo

### Requirement: Acknowledgements
Each segment SHALL carry, encrypted, the highest seq this device applied from each other device. A device SHALL write a segment that holds no version, only to acknowledge, at most once an hour and only when it applied a segment holding versions since its own last segment.

#### Scenario: Idle devices go quiet
- **WHEN** A pushes one edit, and both devices then run idle for 30 seconds with `sync.poll_seconds` at 1
- **THEN** A's folder gained one segment and B's at most one

### Requirement: Own segments survive
A device SHALL keep a copy of each segment it wrote until every device of the scope that is not stale acknowledged it. On a `file://` transport, when one of those segments is missing, the device SHALL create it again with the same bytes. On any other transport, which keeps what it stored, a device SHALL NOT create a stored segment again; this rule and Damaged own segments apply to `file://` only.

#### Scenario: A deleted segment comes back
- **WHEN** a user deletes A's segment 5 from the folder before B applied it
- **THEN** at A's next poll the file exists again with its old bytes, and B applies it

#### Scenario: An acknowledged segment
- **WHEN** every device of the scope that is not stale acknowledged A's segment 5, and a user then deletes it from the folder
- **THEN** A does not create it again

### Requirement: Manifests on the transport
A device SHALL publish each pending manifest version of its store, as the `scope-manifest` spec's Pending versions requirement defines it, and copy in each version of its scopes that is valid there, chain included. Watch SHALL confirm a pending version, clearing `manifest/<n>.pending`, once it reads identical bytes back; a `file://` version after version 1 that changes the epoch only after 10 minutes. It SHALL seal only under an epoch the `scope-manifest` spec's Pending epochs allows, push nothing while a pending version introduces a newer one, and on a different version `n` apply A pending version that loses.

#### Scenario: A forged manifest
- **WHEN** a file `manifest/3.json` appears that the owner key did not sign
- **THEN** every device ignores it, prints a line naming it, and keeps version 2

#### Scenario: Two writers of one version
- **WHEN** A revokes C while B, apart, writes a version adding D, and both write `manifest/4.json`, and the folder settles on A's
- **THEN** B's version 4 moves to `manifest/lost/4.json`, B writes a pending version 5 adding D on A's version 4, and once it is confirmed the latest version lists D and not C

#### Scenario: A new scope syncs at once
- **WHEN** `bilbo device init` wrote version 1 of a new scope `personal` on a `file://` folder and watch publishes it
- **THEN** watch reads it back, clears `manifest/1.pending` in the same cycle, and pushes the scope's notes without waiting

#### Scenario: A revocation waits for confirmation
- **WHEN** A revokes C on a `file://` scope, writing a pending version 3 at epoch 2, and an agent then edits a synced note
- **THEN** A records the edit but pushes no segment to that scope until it reads version 3 back unchanged and clears `manifest/3.pending`, and then seals it under epoch 2

### Requirement: A confirmed version replaced
When a `file://` transport later holds a different version `n` than one this device confirmed, watch SHALL stop syncing that scope and print `bilbo: sync <name>: manifest <n> on the transport differs from the confirmed one; run bilbo device to compare` once.

#### Scenario: A late cloud conflict
- **WHEN** a cloud tool replaces A's confirmed `manifest/3.json` with another valid version 3 an hour later
- **THEN** A stops syncing that scope, prints the line once, and `bilbo sync` exits 1

#### Scenario: Unchanged
- **WHEN** the transport still holds the bytes A confirmed
- **THEN** A prints no such line

### Requirement: Nothing is removed from the transport
No bilbo command SHALL delete or shorten anything under `scopes/`, apart from Damaged own segments. A device that starts syncing a scope SHALL read every segment of every device from seq 1.

#### Scenario: A device joins late
- **WHEN** a scope's folder holds 500 segments from two devices and a third device joins
- **THEN** it reads every segment from seq 1 and ends with every note of the scope

#### Scenario: Old segments stay
- **WHEN** every device has acknowledged every segment of a scope for 200 days
- **THEN** every segment file is still in the folder
