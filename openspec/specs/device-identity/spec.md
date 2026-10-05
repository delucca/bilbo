# device-identity Specification

## Purpose
Who a device is and who owns it: the recovery phrase, the owner key derived from it, each device's own key, name and id, where these secrets live, and the `bilbo device` verb that creates, recovers, lists and revokes them.

## Requirements

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

### Requirement: Phrase confirmation
After showing the phrase, `bilbo device init` SHALL ask for the words at 3 distinct random positions, each matched as a word or its first 4 letters, without regard to case or surrounding spaces. A wrong word SHALL be asked again, and the user SHALL be able to see the phrase again. bilbo SHALL write no key and no manifest until all 3 words match. Cancelling SHALL write nothing and exit 1, and the next `init` SHALL create a new phrase.

#### Scenario: Confirmed
- **WHEN** the user types back the 3 asked words correctly
- **THEN** init writes the keys and reports them

#### Scenario: A prefix
- **WHEN** the asked word is `abandon` and the user types `ABAN`
- **THEN** the word counts as matched

#### Scenario: A wrong word
- **WHEN** the user types a word that is not the one at the asked position
- **THEN** init asks for that position again and has written no file

#### Scenario: Cancelled
- **WHEN** the user cancels during the confirmation
- **THEN** stderr says nothing was written, `<state>/bilbo/keys/` holds no key, and the exit code is 1

### Requirement: Terminal-only forms
`bilbo device recover`, `bilbo device revoke`, and `bilbo device init` when it would create a phrase or change a scope's transport, SHALL run that part only when stdin and stderr are both terminals and neither `CLAUDECODE` nor `CODEX_THREAD_ID` is set and not empty. Otherwise they SHALL show and read nothing, write nothing for that part, print a stderr line saying the user must run it in a terminal, and exit 1.

#### Scenario: An agent runs init
- **WHEN** an agent runs `bilbo device init` with stdin not a terminal, on a device with no keys
- **THEN** stdout is empty, stderr is one line naming `bilbo device init` and a terminal, no file is written, and the exit code is 1

#### Scenario: Under Claude Code with a terminal
- **WHEN** `CLAUDECODE=1` is set and `bilbo device recover` runs with terminals on stdin and stderr
- **THEN** it reads nothing, writes nothing and exits 1

#### Scenario: Under Codex
- **WHEN** `CODEX_THREAD_ID` is set and `bilbo device revoke bagend` runs
- **THEN** no manifest is written, stderr names a terminal, and the exit code is 1

#### Scenario: An enrolled device needs no terminal to seal a new scope
- **WHEN** an agent runs `bilbo device init` without a terminal on an enrolled device, and a scope has a sync URL and no manifest
- **THEN** init creates that scope's manifest and exits 0, without asking for or showing a phrase

### Requirement: Phrase on screen
The phrase and its entry SHALL be drawn on stderr only, and SHALL be cleared from the screen and the terminal's scrollback when the confirmation or the entry ends, whether it succeeded or was cancelled. Before drawing it, bilbo SHALL stop the process from writing a core file.

#### Scenario: Gone after the ceremony
- **WHEN** init's confirmation ends and the user scrolls back
- **THEN** no word of the phrase is on the screen or in the scrollback, and a line says the phrase was confirmed

#### Scenario: Gone after a cancel
- **WHEN** the user cancels while the phrase is shown
- **THEN** no word of the phrase is on the screen or in the scrollback

### Requirement: Owner key
The owner key SHALL be derived from the phrase's 128 bits with HKDF-SHA256, salt `bilbo-owner-1`: the 32 bytes expanded with info `sign` are the Ed25519 signing seed, and the 32 bytes expanded with info `box` are the X25519 private key, clamped, used as they are. The same phrase SHALL always give the same owner key, and two phrases SHALL give different ones.

#### Scenario: A known phrase
- **WHEN** the phrase is `abandon` eleven times followed by `about`
- **THEN** the owner's Ed25519 public key is `12a801fee3d44e9780f252b49c3727e88ccec5d8af61a84219a2e917d9217c79`, its X25519 public key is `11985d0260365b681492a704eb1fbba906c0a9729a731b3428ad884b6d118b60`, and its fingerprint is `yb4b-5aju-v6zb-x2nm-nc5x-ompf`

#### Scenario: The same phrase, the same owner
- **WHEN** a user runs `bilbo device init` on one device and `bilbo device recover` with the same phrase on another
- **THEN** `bilbo device` prints the same `owner` fingerprint on both

#### Scenario: Another phrase, another owner
- **WHEN** one word of a phrase is replaced and the result still passes the checksum
- **THEN** it derives a different fingerprint

### Requirement: Owner secrets on a device
An enrolled device SHALL keep the owner's Ed25519 signing seed and X25519 public key, and SHALL NOT keep the owner's X25519 private key in any file. bilbo SHALL hold that private key only in memory, only inside `init` (to compute the public key) and `recover` (to open manifests' `owner` entries), and discard it before the verb ends.

#### Scenario: The owner file
- **WHEN** `bilbo device init` has written the keys
- **THEN** `owner.key` holds the signing seed and the owner's X25519 public key and nothing else secret, and that public key equals every manifest's `owner_box`

#### Scenario: Not listed, not readable
- **WHEN** this device's owner signed a manifest whose devices do not include this device
- **THEN** `bilbo device` shows `-` as that scope's name, because the device cannot open it

### Requirement: Owner fingerprint
The owner fingerprint SHALL be the first 24 characters of the lowercase RFC 4648 base32 encoding, without padding, of the SHA-256 of the owner's Ed25519 public key, written as six groups of four characters joined by `-`.

#### Scenario: The printed form
- **WHEN** an enrolled device runs `bilbo device`
- **THEN** the `owner` line's second field matches `^[a-z2-7]{4}(-[a-z2-7]{4}){5}$`

#### Scenario: No owner
- **WHEN** the device holds no keys and no local manifest
- **THEN** the `owner` line's second field is `none`

### Requirement: Device key
Each device SHALL have its own Ed25519 and X25519 key pair from 64 random bytes, and a name. Its id SHALL be the first 26 characters of the lowercase RFC 4648 base32 encoding, without padding, of the SHA-256 of its Ed25519 public key. A device's name SHALL be unique among the devices its owner's manifests list.

#### Scenario: The id
- **WHEN** `bilbo device` prints this device's line
- **THEN** its id is 26 characters of `a` to `z` and `2` to `7`, and equals the same prefix computed from the `sign` key of the device's entry in a manifest

#### Scenario: A damaged key file
- **WHEN** `device.key` holds a key that is not 64 hexadecimal characters, or a name that breaks the name rule
- **THEN** every `bilbo device` form that reads the keys names the file on stderr, writes nothing and exits 1

### Requirement: Device name
A device name SHALL follow the topic grammar (`[a-z0-9]+` segments joined by single hyphens) and be at most 32 characters. Without `--name`, the name SHALL be the host name up to its first `.`, lowercased, with each run of other characters turned into one hyphen and the ends trimmed.

#### Scenario: The default name
- **WHEN** the host name is `Daniels-MacBook-Pro.local` and init runs without `--name`
- **THEN** the device is named `daniels-macbook-pro`

#### Scenario: A bad name
- **WHEN** a user runs `bilbo device init --name Bag_End`
- **THEN** bilbo prints a usage message naming `--name` to stderr, writes nothing and exits 2

#### Scenario: No usable host name
- **WHEN** the host name holds no letter or digit and init runs without `--name`
- **THEN** init writes nothing, stderr asks for `--name`, and the exit code is 1

### Requirement: Where keys live
The owner and device secrets SHALL live in `<state>/bilbo/keys/`, where `<state>` is `$XDG_STATE_HOME` when absolute, else `$HOME/.local/state`, and never under the store root. The folder SHALL have mode 0700 and each file 0600, and a key file SHALL never be overwritten. When the folder or a key file grants any access to group or others, every `bilbo device` form that reads the keys SHALL refuse, name the path, and exit 1.

#### Scenario: Modes
- **WHEN** `bilbo device init` has written the keys
- **THEN** `<state>/bilbo/keys/` has mode 0700, and `owner.key` and `device.key` in it have mode 0600

#### Scenario: Loose permissions
- **WHEN** `<state>/bilbo/keys/` has mode 0755 and an agent runs `bilbo device list`
- **THEN** stderr names that folder, stdout is empty, and the exit code is 1

#### Scenario: A copied store
- **WHEN** a user copies `<root>` to a machine with no keys and runs `bilbo device` there
- **THEN** the `device` line says `none`, and the copied manifests still list only the original devices

### Requirement: Writing keys
bilbo SHALL build a new identity in `<state>/bilbo/keys.new/` and move that folder to `keys/` whole, only when `keys/` does not exist. Every verb that writes an identity, holding `<state>/bilbo/keys.lock` (here `init` and `recover`), SHALL remove a leftover `keys.new/` before it writes. Every other verb SHALL leave it, and `bilbo device`'s other forms SHALL print one stderr line naming it.

#### Scenario: A crash before the move
- **WHEN** `keys.new/` is left from an interrupted init and an agent runs `bilbo device`
- **THEN** stderr names `keys.new`, the `device` line says `none`, and `keys.new/` is untouched

#### Scenario: The next init
- **WHEN** `keys.new/` is left over and the user runs `bilbo device init` in a terminal
- **THEN** init removes it before showing a new phrase, and the keys it writes are the new ones

### Requirement: Show this device
`bilbo device` with no other argument SHALL print, tab-separated: `device`, the name and the id, or `device` and `none`; then `owner` and the fingerprint, or `owner` and `none`; then one line per scope with a sync URL or a local manifest, as the `scope-manifest` spec's Scope line gives. It SHALL change no file. It SHALL exit 1 when it printed a problem line for a scope, and 0 otherwise. Any other argument SHALL be a usage error.

#### Scenario: An enrolled device
- **WHEN** a device named `rivendell` is enrolled and `personal` syncs to `file:///Users/a/Sync/bilbo`
- **THEN** stdout is the `device` line with `rivendell` and its id, the `owner` line with its fingerprint, and the `scope` line for `personal`, and the exit code is 0

#### Scenario: A device with no keys
- **WHEN** no keys exist and the config declares no sync URL
- **THEN** stdout is `device`, tab, `none`, then `owner`, tab, `none`, and the exit code is 0

#### Scenario: A sync URL waiting for the phrase
- **WHEN** no keys exist and the config holds `scope.personal.sync = file:///Users/a/Sync/bilbo`
- **THEN** stderr has one line naming `personal` and telling the user to run, in a terminal, `bilbo device recover` with the phrase of an existing identity, or `bilbo device init` only to create a new one, and the exit code is 0

#### Scenario: An extra argument
- **WHEN** an agent runs `bilbo device now`
- **THEN** bilbo prints a usage message naming `now` to stderr and exits 2

### Requirement: Init
`bilbo device init [--name <name>]` SHALL, on a device without keys, run the phrase ceremony and write the owner and device keys, then create or update the manifests the config asks for, as the `scope-manifest` spec says. Without keys, it SHALL refuse when local manifests exist, naming their owner's fingerprint and `bilbo device recover`. Every refusal SHALL come before the phrase is shown. On an enrolled device, `--name` SHALL be a usage error.

#### Scenario: A first init
- **WHEN** a user runs `bilbo device init --name rivendell` in a terminal with `personal` syncing to a URL, and confirms the phrase
- **THEN** stdout is `owner created: <fingerprint>`, `device created: rivendell <id>` and `scope personal created: <scope id> manifest 1 epoch 1`, and the exit code is 0

#### Scenario: A rerun
- **WHEN** init succeeded and runs again with the same config
- **THEN** every line says `kept`, no file changes, and the exit code is 0

#### Scenario: A name on an enrolled device
- **WHEN** the device holds its keys and a user runs `bilbo device init --name bagend`
- **THEN** bilbo prints a usage message naming `--name` to stderr, changes no file and exits 2

#### Scenario: A store owned by someone else
- **WHEN** a device with no keys holds a store whose manifests were signed by another owner, and the user runs `bilbo device init`
- **THEN** no phrase is shown, stderr names that owner's fingerprint and `bilbo device recover`, nothing is written, and the exit code is 1

#### Scenario: No store for a syncing scope
- **WHEN** a scope has a sync URL, `<root>` does not exist, and the user runs `bilbo device init`
- **THEN** no phrase is shown, stderr names `<root>` and `bilbo setup`, and the exit code is 1

### Requirement: Recover
`bilbo device recover [--name <name>]` SHALL read the 12 words one by one on the phrase's screen, taking a word or its first 4 letters, and ask again for a word not in the list, naming only its position, or for all 12 when the checksum fails. It SHALL refuse a phrase whose owner differs from a local manifest's owner or from this device's stored owner, and a name a listed device already holds. Every refusal about the name or the keys SHALL come before any word is asked.

#### Scenario: The wrong phrase
- **WHEN** the typed phrase is valid but derives another owner than the local manifests'
- **THEN** stderr names both fingerprints, no key or manifest is written, and the exit code is 1

#### Scenario: A taken name
- **WHEN** a local manifest lists a device named `rivendell` and the user runs `bilbo device recover` on a host named `rivendell`
- **THEN** before any word is asked, stderr names `rivendell` and `--name`, and the exit code is 1

#### Scenario: A name on an enrolled device
- **WHEN** the device holds its keys and the user runs `bilbo device recover --name bagend`
- **THEN** bilbo prints a usage message naming `--name` to stderr and exits 2

### Requirement: Fingerprint check on recover
After reading the phrase, `recover` SHALL show the owner fingerprint it derives. When no local manifest is signed by that owner and this device holds no owner key, it SHALL ask the user to confirm that the fingerprint matches the one written down with the phrase or shown by `bilbo device` on another device, and SHALL write nothing and exit 1 when the user does not confirm.

#### Scenario: Nothing to check against
- **WHEN** the store holds no manifest and the user types a phrase
- **THEN** recover shows the derived fingerprint and asks whether it matches, before writing anything

#### Scenario: A mismatch
- **WHEN** the user answers that the fingerprint does not match
- **THEN** stderr says nothing was written, no key exists, and the exit code is 1

#### Scenario: A manifest vouches for the phrase
- **WHEN** a local manifest is signed by the derived owner
- **THEN** recover shows the fingerprint and asks no question about it

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

### Requirement: Step report
`init` and `recover` SHALL print one line per step: `owner`, `device`, then `scope <name>` for each scope with a sync URL or a manifest, sorted by name, with `-` for a scope this device cannot open. Each line SHALL be `<step> <status>: <detail>`, the status one of `created`, `recovered`, `kept`, `updated`, `unsealed` or `failed`. The exit code SHALL be 1 when a step failed, and a failed scope SHALL NOT stop the others.

#### Scenario: A scope that fails
- **WHEN** `personal` and `shared` sync, and `personal`'s next manifest version already exists on disk when init writes it
- **THEN** stdout has `scope personal failed: <path>` and `scope shared created: <scope id> manifest 1 epoch 1`, and the exit code is 1

#### Scenario: A scope this device cannot open
- **WHEN** the store holds a manifest of another owner and an enrolled device runs `bilbo device init`
- **THEN** stdout has `scope - kept: <scope id>`, and no version of that scope is written

### Requirement: List devices
`bilbo device list` SHALL print one tab-separated line per device listed in the latest manifest of any scope this owner signed, plus this device, sorted by name: the name, the id, and `this` for this device. It SHALL change no file and exit 0. On a device without keys it SHALL refuse with a line naming `bilbo device init` and exit 1.

#### Scenario: Two devices
- **WHEN** `personal`'s manifest lists `rivendell`, this device, and `bagend`
- **THEN** stdout is `bagend`, tab, its id, then `rivendell`, tab, its id, tab, `this`

#### Scenario: Not enrolled
- **WHEN** no keys exist and an agent runs `bilbo device list`
- **THEN** stdout is empty, stderr names `bilbo device init`, and the exit code is 1

### Requirement: Revoke a device
`bilbo device revoke <device>` SHALL take a device id or name, and write a new version of every manifest that lists both that device and this one, as the `scope-manifest` spec says, printing `scope <name> updated: <scope id> manifest <n> epoch <e>` for each. It SHALL refuse, writing nothing and exiting 1, on a device without keys, for this device, and for a device no manifest lists. A name two listed devices share SHALL be a usage error listing their ids.

#### Scenario: Revoking a lost laptop
- **WHEN** `personal` at manifest 2, epoch 1, lists `rivendell` and `bagend`, and the user runs `bilbo device revoke bagend` in a terminal on `rivendell`
- **THEN** stdout is `scope personal updated: <scope id> manifest 3 epoch 2`, and `bilbo device list` no longer shows `bagend`

#### Scenario: This device
- **WHEN** the user runs `bilbo device revoke rivendell` on `rivendell`
- **THEN** stderr says a device cannot revoke itself, no file changes, and the exit code is 1

#### Scenario: An unknown device
- **WHEN** no manifest lists `mordor`
- **THEN** stderr is `bilbo: no device named mordor`, no file changes, and the exit code is 1

#### Scenario: A missing argument
- **WHEN** a user runs `bilbo device revoke`
- **THEN** bilbo prints a usage message to stderr and exits 2

### Requirement: Revoking from a folder scope
For each scope whose `transport` is `file://`, `revoke` SHALL also print a stderr line telling the user to remove the revoked device from the account that syncs the folder, because revocation does not take away its write access there.

#### Scenario: A folder scope
- **WHEN** `personal` pins `file://` and the user revokes `bagend`
- **THEN** stderr has a line naming `personal` and telling the user to remove `bagend` from the account that syncs the folder, because revocation does not take away its write access there

#### Scenario: A relay scope
- **WHEN** `personal` pins `https://relay.example.net` and the user revokes `bagend`
- **THEN** stderr has no cloud-account line for `personal`

### Requirement: Recover fetches scopes from the transport
Before adding the device, `recover` SHALL, for each scope with a `file://` sync URL and no local manifest, or whose every local version is pending and absent from the transport, list the owner's scopes under the folder's `scopes/`, verify each chain, open its sealed name with the phrase's box key, and copy the versions of the one named like the config's scope into the store as confirmed. When several match, it SHALL take the one listing more devices, on a tie the lower id, and say so on stderr. Pending local versions it replaces SHALL move to `manifest/lost/<n>.json`, never deleted. A folder it cannot read SHALL make that scope `failed`, naming the URL and the reason, and the exit code 1. When no scope opens to the config's name and a scope in the folder does not verify, it cannot tell that the name is free: that scope SHALL be `failed` with `<url> holds scope <scope id> that does not verify: <reason>`, never naming `init`.

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

#### Scenario: A chain that does not verify, and no other by that name
- **WHEN** the only scope in the folder that could be `personal` has a manifest that does not verify, and the store holds no manifest of `personal`
- **THEN** stdout has `scope personal failed: <url> holds scope <id> that does not verify: <reason>` and does not name `bilbo device init`, and the exit code is 1
