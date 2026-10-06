# Spec Delta

## MODIFIED Requirements

### Requirement: Sync step
For each scope whose `sync` is a URL, the `sync` step SHALL check that the device has a key, that the watcher is wanted and that the transport answers: for a `file://` URL, that the folder exists and is writable; for an `https://` or loopback `http://` URL, that an unsigned `GET <url>/v1/` answers 200 with the relay identification of the `relay-api` spec's The API root. It SHALL then report `ok: <name> through <url> (<n> notes)`, scopes joined by `, `. A failed check SHALL report `failed: sync needs the watcher; drop --no-watch`, `failed: <url> is not reachable: <reason>`, or, for a relay URL that answers without that identification, `failed: <url> is not a bilbo relay`. With no device key it SHALL report `skipped: no device key; run bilbo device init in a terminal`, and with no syncing scope `skipped: no scope syncs`. It SHALL write nothing.

#### Scenario: Nothing syncs
- **WHEN** a user runs `bilbo setup --yes` with a config that declares no syncing scope
- **THEN** the sync line says `skipped: no scope syncs`

#### Scenario: A scope syncs
- **WHEN** the config holds `scope.personal.sync = file:///srv/bilbo`, the device has a key, 12 notes have `scope: personal` and `/srv/bilbo` exists
- **THEN** the sync line says `ok: personal through file:///srv/bilbo (12 notes)` and the exit code is 0

#### Scenario: No device key
- **WHEN** the config syncs `personal` and the device has no key, as on a first home-manager activation, and a user runs `bilbo setup --yes`
- **THEN** the sync line says `skipped: no device key; run bilbo device init in a terminal`, and the step does not make the exit code 1

#### Scenario: Sync without the watcher
- **WHEN** the config syncs `personal` and a user runs `bilbo setup --yes --no-watch`
- **THEN** the sync line says `failed: sync needs the watcher; drop --no-watch` and the exit code is 1

#### Scenario: A scope on a relay
- **WHEN** the config holds `scope.personal.sync = https://relay.example`, the device has a key, 12 notes have `scope: personal`, and `GET https://relay.example/v1/` answers 200 with `{"relay":"bilbo","api":1}`
- **THEN** the sync line says `ok: personal through https://relay.example (12 notes)` and the exit code is 0

#### Scenario: A missing folder
- **WHEN** the config holds `scope.personal.sync = file:///Volumes/usb/bilbo` and that volume is not mounted
- **THEN** the sync line says `failed`, names the URL, no folder is created, and the exit code is 1

#### Scenario: A URL that is not a relay
- **WHEN** the config holds `scope.personal.sync = https://example.org`, whose `GET /v1/` answers 404
- **THEN** the sync line says `failed: https://example.org is not a bilbo relay` and the exit code is 1

#### Scenario: A relay that is down
- **WHEN** the config holds `scope.personal.sync = https://relay.example` and nothing answers there
- **THEN** the sync line says `failed: https://relay.example is not reachable: <reason>`, no request is signed, and the exit code is 1

### Requirement: Turning sync on in the wizard
After the watcher's question, the wizard SHALL ask whether to sync notes between devices, yes by default only when a scope already syncs. On yes, it SHALL ask for the scope (the declared ones and `personal`) and where to sync: a folder (absolute or starting with `~/`, its parent existing) or a relay URL (`https://`, or `http://` to a loopback host) whose unsigned `GET <url>/v1/` answers the relay identification. Then, when the device has no key, ask `Do you already have a recovery phrase from another device?` and run the ceremony `bilbo device recover` runs on yes, or `bilbo device init` on no, under the same terminal, `CLAUDECODE` and `CODEX_THREAD_ID` rules. Against a managed config it SHALL only show the syncing scopes.

#### Scenario: First device
- **WHEN** a user with no device key answers yes, picks `personal`, enters `~/Dropbox/bilbo`, writes down the new phrase, types its 3 words back and confirms
- **THEN** the config sets `scope.personal.sync = file:///Users/a/Dropbox/bilbo`, the folder exists, a device key exists, and the sync line says `ok: personal through file:///Users/a/Dropbox/bilbo (0 notes)`

#### Scenario: Second device
- **WHEN** a user on a second machine answers yes, picks `personal`, enters the same synced folder and types the first device's phrase
- **THEN** the config and the key are written, and once the watcher polls, the scope's manifest lists both devices

#### Scenario: First device on a relay
- **WHEN** a user with no device key answers yes, picks `personal`, enters `https://relay.example`, whose relay was started with the fingerprint the new phrase derives, writes down the phrase, types its 3 words back and confirms
- **THEN** the config sets `scope.personal.sync = https://relay.example`, a device key exists, the sync line says `ok: personal through https://relay.example (0 notes)`, and once the watcher runs the relay holds `personal`'s manifest 1

#### Scenario: Enrolled, and the folder holds a scope it cannot open
- **WHEN** a device that holds keys, is in no scope on the folder and has no manifest of `personal` turns sync on for `personal`, and the folder holds a scope of the same owner that this device cannot open
- **THEN** the wizard mints no scope id, says `the folder holds scopes of this owner that this device cannot open; run bilbo device recover on this device`, and the sync line says `skipped`

#### Scenario: A folder whose parent is missing
- **WHEN** the user enters `/nope/bilbo` and `/nope` does not exist
- **THEN** the wizard says the parent folder does not exist and asks again

#### Scenario: Under Claude Code
- **WHEN** `CLAUDECODE=1` or `CODEX_THREAD_ID` is set and the device has no key, and the user answers yes
- **THEN** the wizard says the phrase is shown only in a terminal outside an agent, turns sync off for this run, and the sync line says `skipped: no device key; run bilbo device init in a terminal`

#### Scenario: Declining sync
- **WHEN** the user answers no
- **THEN** the config gains no sync setting and the sync line says `skipped: no scope syncs`

#### Scenario: A URL that is not a relay
- **WHEN** the user enters `https://example.org`, whose `GET /v1/` answers 404
- **THEN** the wizard says `https://example.org is not a bilbo relay` and asks again

#### Scenario: Plain HTTP to another host
- **WHEN** the user enters `http://bree:8738`
- **THEN** the wizard says plain `http://` reaches only a loopback host, sends no request, and asks again

### Requirement: Applying sync from the wizard
After the summary is confirmed, and only then, the wizard SHALL write `scope.<name>.sync` with the folder's `file://` URL or the relay URL to the config, create a folder's last component when missing, copy into the store only the manifests of the phrase's owner whose sealed name is the picked scope, fetched from the folder or the relay as `bilbo device recover` fetches them, and write the keys and manifests as `bilbo device init` or `recover` does, so a second device joins that scope and no other. It SHALL mint a scope id only when the folder or relay holds no scope with the picked name and, unless this device is in a scope on that transport, no scope of this owner that the keys in hand cannot open.

#### Scenario: The key waits for the confirmation
- **WHEN** a user picks the folder `~/Sync/bilbo`, confirms a new phrase and then presses Ctrl-C at the summary
- **THEN** no device key, no `~/Sync/bilbo` and no config change exist

#### Scenario: Only the picked scope
- **WHEN** the folder holds this owner's `personal` and `shared`, and the user picks `personal` and types the phrase
- **THEN** only `personal`'s manifest is in the store, `bilbo device` lists this device in `personal` only, and no version of `shared` lists it

#### Scenario: Every device lost, through the wizard
- **WHEN** every device is lost, and on a new machine the user runs `bilbo setup`, turns sync on for `personal` with `https://relay.example`, types the phrase and confirms
- **THEN** `personal`'s manifests are fetched from the relay with the owner key, the device is added as the next version, the sync line says `ok` for `personal`, and no new scope id exists
