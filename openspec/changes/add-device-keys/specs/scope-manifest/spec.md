# Spec Delta

## Purpose
The signed record that says which devices may read a syncing scope and where it syncs: its id, its versions on disk, their fields, signature and validity, the epoch key sealed to each device and to the owner, and the new versions that creating, recovering, changing the URL and revoking write.

## ADDED Requirements

### Requirement: Scope id
A scope SHALL get an id when `bilbo device init` first finds its `scope.<name>.sync` set to a URL and no local manifest of that name: 128 random bits as 26 characters of lowercase RFC 4648 base32 without padding. No other form SHALL create one. The id SHALL stay the same when the URL changes or is set back to `off`. A scope renamed in the config SHALL get a new id.

#### Scenario: A first URL
- **WHEN** the config holds `scope.personal.sync = file:///Users/a/Sync/bilbo` and an enrolled device runs `bilbo device init`
- **THEN** `<root>/.bilbo/scopes/` holds one new folder whose name is 26 characters of `a` to `z` and `2` to `7`

#### Scenario: Off keeps the scope
- **WHEN** `personal` had a manifest and the config now holds `scope.personal.sync = off`
- **THEN** `bilbo device init` reports `scope personal kept`, writes no manifest, `bilbo device` prints no problem line for it, and the folder stays

#### Scenario: A renamed scope
- **WHEN** the config renames `scope.personal.*` to `scope.mine.*` with the same URL
- **THEN** `bilbo device init` creates a new id for `mine`, and `bilbo device` lists the old manifest under its id with the name `personal`

### Requirement: Manifest versions on disk
Version `n` of a scope's manifest SHALL be `<root>/.bilbo/scopes/<scope id>/manifest/<n>.json`, `n` counting from 1 with no gap. A version SHALL be written whole under a new name and SHALL never be rewritten, renamed or deleted, except a pending version that loses, as A pending version that loses says. Writers SHALL hold an exclusive lock on the file `<root>/.bilbo/scopes/lock` while they write, so two bilbo processes never write the same version.

#### Scenario: Versions accumulate
- **WHEN** `personal` was created, a device was recovered into it, and a device was revoked
- **THEN** its `manifest/` folder holds exactly `1.json`, `2.json` and `3.json`, and `1.json` is byte-identical to what init wrote

#### Scenario: A version that is already there
- **WHEN** a writer would write `manifest/3.json` and that file exists
- **THEN** it leaves the file untouched, writes nothing, and reports the scope as failed with the path

### Requirement: Pending versions
Every manifest version a device writes locally SHALL be pending, marked by an empty `manifest/<n>.pending` beside it, until a transport holds a version `n` with identical bytes. A version copied from a transport SHALL be confirmed on arrival and get no marker. bilbo puts a version into a store only in these two ways. With no transport, every version this device writes stays pending.

#### Scenario: No transport yet
- **WHEN** `bilbo device init` has created `personal`
- **THEN** `manifest/1.pending` exists, and `bilbo device` shows `manifest 1 pending` for it and exits 0

#### Scenario: A version this device did not write
- **WHEN** a version is copied in from a transport
- **THEN** it has no `.pending` marker

### Requirement: Pending epochs
A new version SHALL build on the latest local version, pending or not. No note data SHALL be encrypted under an epoch that only pending versions introduce; a manifest's own `sealed`, `chain` and `name` are not note data.

#### Scenario: A pending epoch is not used
- **WHEN** `revoke` wrote version 3 of `personal`, at epoch 2, and it is still pending
- **THEN** the epoch bilbo would encrypt note data under for `personal` is still 1, and a version 4 written now builds on version 3

### Requirement: A pending version that loses
When a transport holds a different version `n` than this device's pending one, the pending file SHALL move to `manifest/lost/<n>.json`, never to be deleted, and its change SHALL be applied again as a pending `n+1` on the winner. This move is the one exception to versions never being rewritten, renamed or deleted. Files in `lost/` SHALL be neither checked nor counted as versions.

#### Scenario: Another device's version wins
- **WHEN** version 3 is pending here and bilbo is given a different, valid version 3 as the transport's (in this change, by the library's caller in a test; `add-sync` brings the transport)
- **THEN** this device's version 3 moves to `manifest/lost/3.json`, the given version 3 becomes `manifest/3.json`, and the lost change is written again as a pending version 4

#### Scenario: Lost versions are kept
- **WHEN** `manifest/lost/` holds a file and any `bilbo device` form runs
- **THEN** the file is unchanged, and it is neither checked nor counted as a version

### Requirement: Manifest content
A manifest SHALL be one JSON object with exactly these members, in this order: `format` (1), `scope` (the id), `n`, `prev` (lowercase hex SHA-256 of version `n-1`'s file, or null for 1), `owner` and `owner_box` (the owner's Ed25519 and X25519 public keys), `devices` (objects `id`, `name`, `sign`, `box`), `transport`, `epoch`, `sealed`, `chain`, `name` and `sig`. Binary values SHALL be lowercase hex. bilbo SHALL write the scope's config name only encrypted.

#### Scenario: The plaintext fields
- **WHEN** a user opens `manifest/1.json` of `personal`, synced to `file:///Users/a/Sync/bilbo`
- **THEN** it holds the scope id, `transport` `file://` without the path, both owner public keys, and each device's id, name and public keys, and the word `personal` appears nowhere in it

#### Scenario: An unknown member
- **WHEN** a manifest file holds a member not in the list
- **THEN** `bilbo device` reports that file as invalid on stderr and exits 1, and `init`, `recover` and `revoke` refuse to write the scope's next version

### Requirement: Pinned transport
`transport` SHALL hold the scope's full URL for `https://` and `http://`, and only `file://` for a folder, whose path each device keeps in its own config. bilbo SHALL compare a config URL with `transport` whole for `https://` and `http://`, and by scheme only for `file://`.

#### Scenario: Two machines, one folder
- **WHEN** `personal` pins `file://`, the Mac's config says `file:///Users/a/Dropbox/bilbo` and the Linux laptop's says `file:///home/a/Dropbox/bilbo`
- **THEN** `bilbo device` prints no URL problem line on either machine

#### Scenario: Another relay
- **WHEN** `personal` pins `https://relay.example.net` and the config says `https://other.example.net`
- **THEN** `bilbo device` prints a problem line naming both URLs and exits 1

### Requirement: Manifest signature
`sig` SHALL be the owner's Ed25519 signature over `bilbo-manifest-1` and a newline followed by the manifest without `sig`, written as compact JSON in the member order above. The file SHALL be that JSON with `sig` appended, and a newline. Any other bytes, and a signature that does not verify with `owner`, SHALL make the version invalid.

#### Scenario: A tampered manifest
- **WHEN** a byte inside `devices` of `manifest/2.json` is changed by hand
- **THEN** `bilbo device` reports `manifest/2.json` as invalid and exits 1, and `bilbo device revoke` refuses to change that scope and exits 1

#### Scenario: Reformatted JSON
- **WHEN** `manifest/1.json` is rewritten with the same members, indented
- **THEN** `bilbo device` reports it as invalid and exits 1

### Requirement: Manifest validity
A version SHALL also be invalid unless: `scope` equals its folder's name; `n` equals its file name; `owner` equals version 1's; `prev` is the SHA-256 of version `n-1`'s file; `devices` is sorted by id with no id twice, each id derived from its `sign` key; `sealed`'s keys are exactly the listed ids and `owner`; and `chain` holds one entry for each epoch from 1 to `epoch`-1, in order, every entry of version `n-1` unchanged. These checks SHALL need no secret.

#### Scenario: A manifest in the wrong folder
- **WHEN** a valid `manifest/1.json` of one scope is copied into another scope's folder
- **THEN** `bilbo device` reports the copy as invalid and exits 1

#### Scenario: A broken chain of versions
- **WHEN** `manifest/2.json`'s `prev` is not the SHA-256 of `manifest/1.json`
- **THEN** `bilbo device` reports `manifest/2.json` as invalid and exits 1

#### Scenario: Unsorted devices
- **WHEN** a version lists its devices out of id order, correctly signed
- **THEN** it is invalid

#### Scenario: A chain with a gap
- **WHEN** a correctly signed version at epoch 3 holds a chain entry for epoch 1 only
- **THEN** it is invalid, whoever reads it

#### Scenario: A sealed entry for an unlisted device
- **WHEN** `sealed` holds an entry for an id that `devices` does not list, or lacks `owner`
- **THEN** the version is invalid

### Requirement: Epoch key sealing
Each scope SHALL have a random 32-byte epoch key per epoch, epochs counting from 1. `sealed` SHALL map each listed device's id, and `owner`, to the epoch key sealed with HPKE base mode (DHKEM X25519 HKDF-SHA256, HKDF-SHA256, ChaCha20-Poly1305) to that device's `box` key or to `owner_box`, with info `bilbo-epoch-1` and aad `<scope id>`, newline, `<epoch>` in decimal, newline, and the device id or `owner`. The value SHALL be the 32-byte encapsulated key followed by the ciphertext.

#### Scenario: Every listed device can open it
- **WHEN** `personal`'s latest manifest lists `rivendell` and `bagend`
- **THEN** `sealed` has exactly three entries, `rivendell`'s id, `bagend`'s id and `owner`, and each opens to the same epoch key with its box key

#### Scenario: The phrase alone
- **WHEN** a user recovers from the phrase on a device with no other key
- **THEN** the `owner` entry of every local manifest of that owner opens with the box key the phrase derives

#### Scenario: An entry moved to another recipient
- **WHEN** a device's sealed value is copied under another device's id
- **THEN** it does not open with that other device's box key

### Requirement: Epoch chain
`chain` SHALL hold every earlier epoch key of the scope, oldest first, each as `{epoch, key}` where `key` is that epoch's key encrypted with XChaCha20-Poly1305 under the next epoch's key, with aad `bilbo-chain-1`, newline, `<scope id>`, newline, `<epoch>` in decimal, as the 24-byte nonce followed by the ciphertext.

#### Scenario: A device enrolled after a revocation
- **WHEN** `personal` is at epoch 3 and a device recovers into it
- **THEN** that device can open epoch 3 from `sealed`, then epochs 2 and 1 from `chain`

#### Scenario: The first epoch
- **WHEN** a manifest is at epoch 1
- **THEN** its `chain` is empty

#### Scenario: An entry moved to another scope
- **WHEN** a chain entry is copied from one scope's manifest into another's
- **THEN** it does not open there, because its aad names the first scope's id

### Requirement: Chain check by a member
A device that opens a version's epoch key SHALL treat the version as invalid when a `chain` entry does not open, or when the entry for an epoch whose key it opened from an earlier valid version decrypts to another key. An entry for an epoch the device never held SHALL be accepted unchecked.

#### Scenario: A rotation by a device that never held the current key
- **WHEN** `personal` is at epoch 2 after `bagend` was revoked, and a correctly signed version 4 at epoch 3, sealed to every device, holds a chain entry for epoch 2 that does not decrypt to the epoch 2 key `rivendell` holds
- **THEN** `rivendell` reports version 4 as invalid, never uses epoch 3, and `bilbo device` exits 1

#### Scenario: A chain entry that does not open
- **WHEN** a correctly signed version's chain entry for epoch 1 was encrypted under the wrong key
- **THEN** a device listed in it reports the version as invalid, and `bilbo device` exits 1

### Requirement: Sealed scope name
`name` SHALL be the scope's config name encrypted with XChaCha20-Poly1305 under the version's epoch key, with aad `bilbo-name-1`, newline, `<scope id>`, newline, `<epoch>` in decimal, as the 24-byte nonce followed by the ciphertext. bilbo SHALL find a scope's manifest by opening the names of the local manifests that list this device.

#### Scenario: A new epoch
- **WHEN** revocation moves `personal` to epoch 2
- **THEN** the new version's `name` opens with the epoch 2 key and not with the epoch 1 key

#### Scenario: A manifest that does not list this device
- **WHEN** a local manifest of this owner does not list this device
- **THEN** its name stays unread and its scope line shows `-`

### Requirement: Versions that init writes
`init` SHALL write version 1 for a scope with a sync URL and no manifest, listing this device and every device the owner's other latest manifests list. When the config's URL differs from `transport`, comparing `https://` and `http://` URLs whole and a `file://` URL by its scheme only, it SHALL write a new version with the same epoch, in a terminal only, as the `device-identity` spec's Terminal-only forms says. It SHALL NOT add this device to a manifest that does not list it.

#### Scenario: A changed URL
- **WHEN** `personal` is at manifest 1 pinned to `file://`, the config now says `https://relay.example.net`, and the user runs `bilbo device init` in a terminal
- **THEN** stdout has `scope personal updated: <scope id> manifest 2 epoch 1`, and `2.json`'s `transport` is the new URL

#### Scenario: Another folder path
- **WHEN** `personal` pins `file://` and the config's folder moves from `file:///Users/a/Sync/bilbo` to `file:///home/a/Sync/bilbo`
- **THEN** `bilbo device init` reports `scope personal kept` and writes no version, and `bilbo device` prints no problem line for it

#### Scenario: A changed URL without a terminal
- **WHEN** the same config change is made and an agent runs `bilbo device init`
- **THEN** stdout has `scope personal failed: changing the URL needs a terminal`, no version is written for it, other scopes are still created, and the exit code is 1

#### Scenario: A second syncing scope
- **WHEN** `personal`'s manifest lists `rivendell` and `bagend`, and the user adds `scope.shared.sync` on `rivendell` and runs `bilbo device init`
- **THEN** `shared`'s manifest 1 lists both devices and seals its key to both and to the owner

#### Scenario: A device a manifest dropped
- **WHEN** an earlier version of `personal` listed this device and the latest does not, and the user runs `bilbo device init`
- **THEN** init reports `scope - kept: <scope id>` and writes no version

### Requirement: Versions that recover writes
`recover` SHALL write, for each local manifest of its owner whose latest version does not list this device, a new version with the same epoch that adds this device and seals the epoch key to it, opening the key through the `owner` entry. It SHALL write nothing for a manifest that already lists this device.

#### Scenario: Recovered into a scope
- **WHEN** `personal` is at manifest 1, epoch 1, listing `rivendell`, and `rivendell-2` recovers
- **THEN** manifest 2 lists both devices, its `epoch` is 1, and its `sealed` has entries for both and for `owner`

#### Scenario: Already listed
- **WHEN** the latest version of `personal` lists this device and the user runs `bilbo device recover` on it
- **THEN** recover reports `scope personal kept` and writes no version

### Requirement: Versions that revoke writes
`revoke` SHALL write a new version without the revoked device, under a new random epoch key sealed to the remaining devices and to the owner, with the previous epoch's key appended to `chain`. No key the revoked device holds SHALL open the new epoch key from that version.

#### Scenario: Revocation seals nothing to the revoked device
- **WHEN** `bagend` is revoked from `personal` at epoch 1
- **THEN** the new version's `sealed` has no `bagend` entry, its `epoch` is 2, its `chain` holds epoch 1's key, and neither `bagend`'s `device.key` nor its `owner.key` opens any entry of it

### Requirement: Scope line
`bilbo device` SHALL print each scope as tab-separated fields: `scope`, the name (`-` if unreadable), the id, `manifest <n>` (plus ` pending` while pending), `epoch <e>`, `<k> devices` and the latest valid version's `transport`. A syncing scope with no manifest SHALL print `scope`, the name, `unsealed` and the URL. An invalid latest version, another owner's manifest, or a `transport` that differs from the config's URL as `init` compares them SHALL each add a stderr problem line; `off` adds none.

#### Scenario: A scope with a manifest
- **WHEN** `personal` is at manifest 3, epoch 2, listing one device, synced to `file:///Users/a/Sync/bilbo`
- **THEN** its line is `scope`, `personal`, the id, `manifest 3 pending`, `epoch 2`, `1 devices` and `file://`, tab-separated

#### Scenario: Another owner's manifest
- **WHEN** the store holds a manifest signed by an owner other than this device's
- **THEN** its line shows `-` as the name, stderr names its id and that owner's fingerprint, no command changes it, and `bilbo device` exits 1

#### Scenario: A URL changed in the config
- **WHEN** `personal`'s latest version pins `file://` and the config now says `https://relay.example.net`
- **THEN** stderr names `personal`, both URLs and `bilbo device init`, and `bilbo device` exits 1
