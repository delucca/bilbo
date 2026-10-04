# Spec Delta

## MODIFIED Requirements

### Requirement: Recovery phrase
The recovery phrase SHALL be 12 words from the BIP39 English word list, encoding 128 random bits and their 4-bit BIP39 checksum. bilbo SHALL create a phrase only in `bilbo device init` or in `bilbo setup`'s sync step, on a device without an owner key, through the same ceremony, and show it with the owner fingerprint for the user to write down beside it. It SHALL NOT store the phrase, or the bits it encodes, in any file, and SHALL NOT print it to stdout.

#### Scenario: A new phrase
- **WHEN** a user runs `bilbo device init` in a terminal on a device with no keys
- **THEN** the terminal shows 12 numbered words, each in the BIP39 English list, whose checksum is valid, and the owner fingerprint they derive

#### Scenario: The phrase is kept nowhere
- **WHEN** `bilbo device init` has finished
- **THEN** no file under the store root, the state folder or the config folder holds the phrase or its 32 hexadecimal characters of entropy, and stdout does not hold any word sequence of the phrase

#### Scenario: A phrase from setup
- **WHEN** a user with no keys turns sync on in the `bilbo setup` wizard and confirms the new phrase
- **THEN** the phrase was shown and confirmed as `bilbo device init` shows it, and no file holds it

### Requirement: What recover writes
On a device without keys, `recover` SHALL write the owner and device keys. On an enrolled device it SHALL keep its device key. Either way, after fetching scopes from the transport as the Recover fetches scopes from the transport requirement says, it SHALL add this device to every local manifest of its owner whose latest version does not list it, as the `scope-manifest` spec says. It SHALL NOT create a scope id. A syncing scope that still has no local manifest SHALL be `unsealed`: when its transport was read and holds no scope of this owner by that name, naming `bilbo device init`; when this bilbo has no client for its URL's scheme, telling the user to bring in its manifest and run `recover` again, never `init`, which would fork the scope.

#### Scenario: A wiped laptop
- **WHEN** the store holds `personal`'s manifest listing `rivendell`, the keys were lost, and the user runs `bilbo device recover --name rivendell-2` with the right phrase
- **THEN** stdout has `owner recovered: <fingerprint>`, `device created: rivendell-2 <id>` and `scope personal updated: <scope id> manifest 2 epoch 1`, and `bilbo device list` shows both devices

#### Scenario: A fresh machine
- **WHEN** the store holds no manifest, the config gives `personal` a sync URL whose transport holds no scope of this owner named `personal`, and the user recovers and confirms the fingerprint
- **THEN** stdout has `owner recovered: <fingerprint>`, `device created: <name> <id>` and `scope personal unsealed: <url> holds no scope personal of this owner; run bilbo device init to create it`, and `<root>/.bilbo/scopes/` holds no new folder

#### Scenario: A relay URL
- **WHEN** the store holds no manifest, the config gives `personal` the URL `https://relay.example`, which this bilbo has no client for, and the user recovers and confirms the fingerprint
- **THEN** stdout has `scope personal unsealed: copy the store from an enrolled device, then run bilbo device recover again`, and `<root>/.bilbo/scopes/` holds no new folder

#### Scenario: Finishing an interrupted recover
- **WHEN** a recover wrote the keys but stopped before `shared`'s manifest, and the user runs `bilbo device recover` again with the same phrase
- **THEN** `device kept` and `scope shared updated` are printed, and `personal`, which already lists the device, is `kept`

## ADDED Requirements

### Requirement: Recover fetches scopes from the transport
Before adding the device, `recover` SHALL, for each scope with a `file://` sync URL and no local manifest, or whose every local version is pending and absent from the transport, list the owner's scopes under the folder's `scopes/`, verify each chain, open its sealed name with the phrase's box key, and copy the versions of the one named like the config's scope into the store as confirmed. When several match, it SHALL take the one listing more devices, on a tie the lower id, and say so on stderr. Pending local versions it replaces SHALL move to `manifest/lost/<n>.json`, never deleted. A folder it cannot read SHALL make that scope `failed`, naming the URL and the reason, and the exit code 1.

#### Scenario: Every device lost, folder
- **WHEN** every device is lost, `personal` lives in `file:///Users/a/Dropbox/bilbo`, and on a new machine with that folder synced and an empty store the user runs `bilbo device recover` with the phrase
- **THEN** the folder's chain of `personal` is copied in and extended with this device as manifest n+1, and no new scope id exists

#### Scenario: A dead-end fork heals
- **WHEN** an earlier `init` left a pending, never published version 1 of `personal` under its own scope id, and the folder holds this owner's published `personal`
- **THEN** recover moves the pending version to `manifest/lost/1.json`, copies the folder's chain in and adds this device to it

#### Scenario: Only the named scopes
- **WHEN** the folder holds `personal` and `shared` of this owner, and the config gives a sync URL only to `personal`
- **THEN** recover copies only `personal`'s versions and does not enroll the device into `shared`

#### Scenario: Two scopes with one name
- **WHEN** the folder holds two scopes of this owner whose names both open to `personal`, one listing 3 devices and the other 1
- **THEN** recover copies the one listing 3 devices, and stderr names both scope ids and the one it took

#### Scenario: A transport that does not answer
- **WHEN** `scope.personal.sync = file:///Volumes/usb/bilbo` and that volume is not mounted while the user recovers
- **THEN** stdout has `scope personal failed: file:///Volumes/usb/bilbo is not reachable: <reason>`, the keys are written, nothing of `personal` is copied, the exit code is 1, and running `bilbo device recover` again once the folder is back finishes the scope

#### Scenario: A chain that does not verify
- **WHEN** one scope in the folder has a manifest whose signature does not verify
- **THEN** recover copies nothing of that scope, names its id on stderr, and treats the other scopes as usual
