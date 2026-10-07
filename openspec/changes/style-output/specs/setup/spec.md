## MODIFIED Requirements

### Requirement: Sync step
For each scope whose `sync` is a URL, the `sync` step SHALL check that the device has a key, that the watcher is wanted and that the transport answers: for a `file://` URL, that the folder exists and is writable; for an `https://` or loopback `http://` URL, that an unsigned `GET <url>/v1/` answers 200 with the relay identification of the `relay-api` spec's The API root. It SHALL then report `ok: <name> through <url> (<n> notes)`, with `note` when `<n>` is 1, scopes joined by `, `. A failed check SHALL report `failed: sync needs the watcher; drop --no-watch`, `failed: <url> is not reachable: <reason>`, or, for a relay URL that answers without that identification, `failed: <url> is not a bilbo relay`. With no device key it SHALL report `skipped: no device key; run bilbo device init in a terminal`, and with no syncing scope `skipped: no scope syncs`. It SHALL write nothing.

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

#### Scenario: One note
- **WHEN** the config holds `scope.work.sync = file:///srv/work`, the device has a key, 1 note has `scope: work` and `/srv/work` exists
- **THEN** the sync line says `ok: work through file:///srv/work (1 note)`

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
After the watcher's question, the wizard SHALL ask whether to sync notes between devices, yes by default only when a scope already syncs. On yes, it SHALL ask for the scope (the declared ones and `personal`) and where to sync: a folder (absolute or starting with `~/`, its parent existing) or a relay URL (`https://`, or `http://` to a loopback host) whose unsigned `GET <url>/v1/` answers the relay identification. Then, when the device has no key, ask `Do you already have a recovery phrase from another device?` and run the ceremony `bilbo device recover` runs on yes, or `bilbo device init` on no, under the same terminal and agent rules, those of the `device-identity` spec's Terminal-only forms. Against a managed config it SHALL only show the syncing scopes.

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
- **WHEN** `CLAUDE_CODE_CHILD_SESSION=1` or `CODEX_THREAD_ID` is set and the device has no key, and the user answers yes
- **THEN** the wizard says the phrase is shown only in a terminal outside an agent, turns sync off for this run, and the sync line says `skipped: no device key; run bilbo device init in a terminal`

#### Scenario: In an IDE terminal
- **WHEN** only `CLAUDECODE=1` is set, the device has no key, and the user answers yes in a terminal
- **THEN** the wizard runs the phrase ceremony as it does with no marker

#### Scenario: Declining sync
- **WHEN** the user answers no
- **THEN** the config gains no sync setting and the sync line says `skipped: no scope syncs`

#### Scenario: A URL that is not a relay
- **WHEN** the user enters `https://example.org`, whose `GET /v1/` answers 404
- **THEN** the wizard says `https://example.org is not a bilbo relay` and asks again

#### Scenario: Plain HTTP to another host
- **WHEN** the user enters `http://relay.example:8738`
- **THEN** the wizard says plain `http://` reaches only a loopback host, sends no request, and asks again

## ADDED Requirements

### Requirement: Human view of the step report
When stdout gets the `cli` spec's human view, the step report SHALL print one line per step in the same order: a mark for its status, `■` for `failed`, `○` for `skipped`, `◇` for `kept` and `◆` for any other status, then the step, the status and the detail in aligned columns, the detail's paths under the home folder starting with `~/`. A blank line and a summary SHALL follow: `■  <n> of <m> steps failed` when a step failed, else `◆  Setup done: ` and the number of steps of each status, in the order the statuses first appear, joined by `, `. This holds after the wizard too. The exit code SHALL be as in Step report.

#### Scenario: A rerun on a terminal
- **WHEN** a user runs `bilbo setup --yes` in a terminal on an installed machine with no embedder and no syncing scope
- **THEN** each step line starts with `◇` or `○`, the store line shows its path from `~/`, and the last line starts with `◆  Setup done: ` and holds `kept` and `skipped`

#### Scenario: A failed step on a terminal
- **WHEN** a user runs `bilbo setup --yes` in a terminal with an embedder configured and `launchctl` is not on `PATH`
- **THEN** the timer and watch lines start with `■`, the last line is `■  2 of 12 steps failed`, and the exit code is 1

#### Scenario: The home-manager module reads the plain report
- **WHEN** the home-manager module runs `bilbo setup --yes` with stdout not a terminal
- **THEN** stdout is the `<step> <status>: <detail>` lines of Step report
