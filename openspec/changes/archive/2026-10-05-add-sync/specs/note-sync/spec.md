# Spec Delta

## Purpose
`bilbo watch` keeps the notes of each syncing scope the same on all of one owner's devices, through the scope's transport, without dropping either side of a concurrent edit and without letting a note reach a device or a scope it was not assigned to.

## ADDED Requirements

### Requirement: What syncs
A note SHALL sync in scope `<name>` when its file's frontmatter holds `scope: <name>`, the config declares `scope.<name>.sync` as a transport URL, and this device has a device key. A note with no `scope`, a scope the config does not declare, or a scope whose `sync` is `off` SHALL NOT leave the device, in any form.

#### Scenario: An assigned note syncs
- **WHEN** devices A and B both sync `personal` through one folder and an agent on A creates a note with `scope: personal`
- **THEN** the note appears in B's `notes/` with the same bytes

#### Scenario: The first sync uploads nothing
- **WHEN** a store holds 40 notes without a `scope` key and the user turns sync on for `personal`
- **THEN** no segment in the transport holds any of those notes, and `bilbo sync` counts 40 notes that sync nowhere

#### Scenario: A local scope stays local
- **WHEN** the config holds `scope.work.sync = off` and a note has `scope: work`
- **THEN** no segment in any transport holds that note

#### Scenario: No device key
- **WHEN** `scope.personal.sync` is a URL and the device has no key
- **THEN** watch prints `bilbo: sync personal: no device key; run bilbo device init or bilbo device recover` once, records history as before, and pushes nothing

### Requirement: Sync runs inside watch
`bilbo watch` SHALL sync every syncing scope while it runs, printing `bilbo: syncing <name> through <url>` per scope at start. It SHALL push a version as soon as it records it, and look for other devices' segments every `sync.poll_seconds`. It SHALL sync a scope only while the config URL matches the latest manifest's `transport` under the `scope-manifest` spec's Pinned transport requirement, and else print the pin line below and sync nothing for it. At the start of every sync cycle it SHALL read the config, the device keys and the local manifests again, so a config edit or a `bilbo device` command takes effect without a restart. Without a syncing scope, watch SHALL behave as the `note-watch` spec says and touch no transport.

#### Scenario: Sync starts with watch
- **WHEN** the config syncs `personal` through `file:///srv/bilbo` and watch starts
- **THEN** stderr holds `bilbo: watching <root>/notes` and `bilbo: syncing personal through file:///srv/bilbo`

#### Scenario: An edit reaches the other device
- **WHEN** `sync.poll_seconds` is 1 on both devices and an agent on A appends a paragraph to a synced note
- **THEN** within 15 seconds B's file holds the paragraph and B's history lists A's version, from A

#### Scenario: Turning a scope off
- **WHEN** watch runs and the user changes `scope.personal.sync` to `off`
- **THEN** from the next poll on, watch pushes and pulls nothing for `personal`, and every note stays in `notes/`

#### Scenario: The manifest pins another transport
- **WHEN** `personal`'s latest manifest pins `https://relay.example` and the config says `file:///srv/bilbo`
- **THEN** watch syncs nothing for `personal`, prints `bilbo: sync personal: the manifest pins https://relay.example, the config says file:///srv/bilbo; run bilbo device init to move the scope, or set the config back` once, and `bilbo sync` exits 1

#### Scenario: Another folder path for the same scheme
- **WHEN** `personal`'s manifest pins `file://`, the Mac's config says `file:///Users/a/Dropbox/bilbo`, and this Linux device's config says `file:///home/a/Dropbox/bilbo`
- **THEN** watch syncs `personal` through `/home/a/Dropbox/bilbo` and prints no pin line

#### Scenario: Keys created while watch runs
- **WHEN** watch runs with `personal` syncing and no device key, and the user then runs `bilbo device init`
- **THEN** at its next cycle watch starts syncing `personal`, without a restart

### Requirement: Inbound writes
Watch SHALL write versions from other devices into `<root>/notes/` only through an atomic exchange, or a rename that fails rather than replace a file, once per note per cycle, from the note's heads after the cycle's segments. Bytes it takes out that differ from the version the file held before the write SHALL be recorded as an `edited` version following that version, and handled as the Stale base requirement handles a save, with that version as the base. A local save SHALL follow the latest version the file held, never a version that waits to be written; a waiting version then merges with it as a concurrent head. Watch SHALL record an inbound version only after its write succeeded, SHALL NOT record its own write as a local change, SHALL NOT write a file it skips, and SHALL write nothing while `<root>/notes/` cannot be listed.

#### Scenario: An agent writes during the swap
- **WHEN** an agent's write lands on a note between watch reading it and swapping in a version from B
- **THEN** history holds the agent's bytes as an `edited` version following the version the agent read, the file holds the merge of the agent's bytes and B's version, and no text of either is lost

#### Scenario: A save not yet recorded at poll time
- **WHEN** an agent saved an edit of `## Rollout` one second before a poll applies B's edit of `## Setup` to the same note
- **THEN** the file holds both edits, and history lists the agent's save after the version it was made from, then a `merged` version

#### Scenario: Killed before the rename of a new note
- **WHEN** watch is killed after writing an incoming new note to `notes/.bilbo-restore-<id>` and before renaming it into place
- **THEN** the next watch completes the write, the note's file exists, history lists the incoming version once, and no `deleted` version is recorded or pushed

#### Scenario: Killed before the exchange of an edit
- **WHEN** watch is killed after preparing an incoming edit and before the exchange
- **THEN** the next watch writes the edit, and history lists the incoming version after the one the file held, with no extra `edited` or `merged` version

#### Scenario: A save while a version waits to be written
- **WHEN** B's edit of `## Setup` waits to be written because the write failed or the filesystem cannot swap, and an agent saves an edit of `## Rollout` made from the file
- **THEN** the agent's save follows the version the file held, B's edit survives as a concurrent head, and once a write succeeds the file holds both edits

#### Scenario: The notes folder is gone
- **WHEN** `<root>/notes/` cannot be listed and a version arrives for a note
- **THEN** watch writes nothing and creates no folder, and writes the note once `notes/` can be listed again

#### Scenario: A year of versions arrives
- **WHEN** a device joins and its first cycle applies 300 versions of one note
- **THEN** the note's file is written once in that cycle, with the newest result

#### Scenario: A filesystem that cannot swap
- **WHEN** `<root>/notes/` is on a filesystem that cannot exchange two files atomically
- **THEN** watch prints `bilbo: sync personal: cannot write notes on this filesystem: it cannot swap files atomically`, writes nothing into `notes/`, and keeps pushing local versions

### Requirement: Stale base
When a local save is the first change to a note since watch last wrote that note from another device, watch SHALL merge the save against the note's version from just before that first write, as the base, with the version watch wrote as the other side. When the merge equals the save, the save SHALL be recorded as following the written version. Otherwise both SHALL be kept, the merge written back and the merge version flagged `stale-base`. A write by `bilbo restore` or `bilbo scope set` SHALL NOT count as that save: the base stays, and the verb's version becomes the written one.

#### Scenario: An agent saves over text it never read
- **WHEN** an agent reads a note, B's edit of `## Setup` is written into it, and the agent then saves its own edit of `## Rollout` made from its old read
- **THEN** the file holds both B's `## Setup` and the agent's `## Rollout`, and history lists a `merged` version flagged `stale-base`

#### Scenario: An agent that read the new text
- **WHEN** B's edit is written into a note and an agent then reads it, edits another passage and saves
- **THEN** history records the save as an `edited` version that follows B's version, and no `merged` version

### Requirement: Deletes
A note deleted on one device SHALL be deleted on every device that syncs its scope, and the deletion recorded. When one device deletes a note while another edits it, the edit SHALL win on every device: the file comes back with the edit, and the merge version is flagged `edit-beat-delete`. This rule SHALL apply to `deleted` versions only, never to `left` ones.

#### Scenario: A deletion travels
- **WHEN** an agent on A deletes a synced note that B has not touched
- **THEN** within two polls the file is gone from B's `notes/`, and B's history lists a `deleted` version, from A

#### Scenario: Edit beats delete
- **WHEN** A deletes a synced note while B, offline, edits it, and both then sync
- **THEN** both devices hold the note with B's edit, and both histories list a `merged` version flagged `edit-beat-delete`

### Requirement: Topic collision
When two notes with different ids hold one topic after a sync, the note whose id sorts later SHALL be renamed on every device to `<kind>-<topic>-<the last 4 characters of its id, lowercased>.md`, adding characters while that name is taken. The rename SHALL be recorded as a `renamed` version flagged `topic-taken`. Devices where the suffixed name is free record the same version; a device where it is taken converges with them after one merge.

#### Scenario: One topic created on two devices
- **WHEN** A and B, offline, each run `bilbo new decision release`, and B's id sorts later and ends in `Q2X7`
- **THEN** after both sync, both devices hold `decision-release.md` with A's note and `decision-release-q2x7.md` with B's, and `bilbo check` reports no shared topic

#### Scenario: A rename onto a taken topic
- **WHEN** A renames `plan-x.md` to `plan-release.md` while B creates `decision-release.md` with an older id
- **THEN** after both sync, A's note is renamed with its suffix and B's keeps `decision-release.md`

### Requirement: Scope moves
When a note's `scope` changes from a syncing scope to another value, the old scope's transport SHALL receive a `left` marker holding none of its text, file name or new scope. A device SHALL apply a `left` only when it follows the device's head for the note: it then removes the file, records the `left`, keeps the history, and prints `bilbo: sync <name>: notes/<file> left the scope; its history stays`. A `left` concurrent with a local head SHALL be recorded and never merged. A note moving into a syncing scope SHALL be pushed there with its current text only.

#### Scenario: A note moves to a local scope
- **WHEN** a note in `personal` is set to `scope: work`, where `work` does not sync
- **THEN** no segment pushed after the move holds the note's text, every other device of `personal` removes the file, and `bilbo history <id>` there lists `left` first

#### Scenario: A note leaves by losing its key
- **WHEN** an agent drops the `scope` key from a synced note on A
- **THEN** other devices of `personal` remove the file and list `left`, and A keeps the note

#### Scenario: A left marker against a local edit
- **WHEN** C syncs only `personal`, edits a note offline while A moves it to `work`, and C then pulls A's marker
- **THEN** C keeps the file with its edit and prints no left line, its edit reaches a device holding both scopes through `personal`, and once that device's merge comes back as a `left` that follows C's edit, C removes the file

#### Scenario: A note that comes back
- **WHEN** C holds only a `left` for a note and a later version moves it back into `personal`
- **THEN** C writes the file with that version, and no version is flagged `edit-beat-delete`

#### Scenario: Assigning an old note
- **WHEN** a note with 12 local versions gets `scope: personal`
- **THEN** the transport receives one version of it, holding its current text

### Requirement: Marks on a pushed note
The first time watch pushes a version of a note that holds a mark of another declared scope, as the `store-check` spec's Scope marks requirement finds it, it SHALL print `bilbo: notes/<file>: pushed to '<scope>' while holding a mark of '<other>'` once.

#### Scenario: A work word in a personal note
- **WHEN** `scope.work.marks` holds `acme` and an agent saves a note with `scope: personal` whose text holds `acme`
- **THEN** watch pushes it and prints the mark line once, naming `personal` and `work`

#### Scenario: No mark
- **WHEN** a pushed note holds no mark of another scope
- **THEN** watch prints no mark line

### Requirement: Scope clash
When two concurrent versions changed `scope` to different values, the merged note SHALL take the value that shares less: no `scope` key when either side has none; else, when exactly one side names a scope this device syncs, the other side's value; else no `scope` key. The merge version SHALL be flagged `scope-clash`.

#### Scenario: Synced against local
- **WHEN** a note is in `personal`, A moves it to `shared`, B (which syncs `personal` and `shared` but not `work`) moves it to `work` while offline, and B then receives A's version
- **THEN** B's merged note has `scope: work`, is not pushed anywhere, and is flagged `scope-clash`

#### Scenario: Two synced scopes
- **WHEN** a device syncing both `shared` and `team` receives one side moving a note to `shared` and the other moving it to `team`
- **THEN** the merged note has no `scope` key and syncs nowhere, a `left` goes to both scopes, devices syncing only one of them remove the file, and `bilbo check` on this device reports its missing scope

### Requirement: Devices converge
Once every device of a scope has pulled every segment, all of them SHALL hold the same bytes for each note of the scope, and their histories the same version ids. They SHALL then write no further version until a note changes.

#### Scenario: No merge storm
- **WHEN** A and B each edit a different passage of one note while offline, then both come online with `sync.poll_seconds` at 1
- **THEN** within 15 seconds both files hold the same merged bytes, and in the 30 seconds after that neither history gains a version

#### Scenario: One side moved the note to a local scope
- **WHEN** a note is in `personal`, A moves it to `shared`, B (syncing `personal` and `shared`, not `work`) moves it to `work` while offline, and both then sync with `sync.poll_seconds` at 1
- **THEN** within 15 seconds B holds the note with `scope: work` and A no longer holds the file, and in the 30 seconds after that neither history gains a version and no segment holding a version is written

### Requirement: Joining and leaving a scope
A device SHALL sync a scope only while the latest valid manifest version lists it and seals the epoch key to it; it gets there through `bilbo device recover` with the phrase, or a version another member writes, never through watch alone. Watch SHALL publish no version 1 of a scope while this device is in no scope on that transport and the transport holds a scope of this owner that it cannot open, or while the transport holds a scope of this owner by that name, and print the not-in-the-scope line instead; nor while it holds a scope whose version 1 is missing or does not verify, printing `bilbo: sync <name>: <folder> holds scope <scope id> that does not verify: <reason>`. Watch SHALL NOT seal an epoch key to any device, nor write a version that lists a device, except when re-applying a version this device wrote. A device that a newer version no longer lists SHALL stop syncing that scope, print `bilbo: sync <name>: this device was removed from the scope`, and keep its notes.

#### Scenario: A second device set up through the wizard
- **WHEN** device B turns sync on for `personal` in the wizard with A's folder and A's recovery phrase
- **THEN** `personal`'s manifest is copied into B's store before the keys are written, `personal`'s next version lists both devices, and B receives every note of `personal`

#### Scenario: A scope minted beside one this device cannot open
- **WHEN** an enrolled device B is in no scope on the folder, holds no manifest for `personal`, its config syncs `personal`, it ran `bilbo device init`, and the folder holds a scope of the same owner that B cannot open
- **THEN** B's watch publishes no version 1, and prints the not-in-the-scope line

#### Scenario: A member creates a new scope beside one it cannot open
- **WHEN** device C is in `personal` on the folder but was kept out of `shared`, and runs `bilbo device init` for a new scope `notes2`
- **THEN** C's watch publishes `notes2`'s version 1

#### Scenario: A listing written by someone else
- **WHEN** a manifest version that A did not write adds device D to `personal`, and A's watch reads it
- **THEN** A writes no manifest version and seals nothing to D; D reads `personal` only if that version sealed the key to it

#### Scenario: A removed device
- **WHEN** the owner removes device C from `personal` and C's watch pulls
- **THEN** C prints the removed line, pushes nothing more to `personal`, keeps every note in `notes/`, and writes no manifest version

#### Scenario: Recovered before the folder was read
- **WHEN** device B ran `bilbo device recover` by hand on a store with no manifests while A's folder had not reached B yet, so it enrolled into no scope, and B's config syncs `personal` through A's folder
- **THEN** B's watch publishes no manifest, syncs nothing for `personal`, and prints `bilbo: sync personal: this device is not in the scope; run bilbo device recover on this device`

### Requirement: Manifest changes are shown
When watch adopts a manifest version this device did not write and that is valid under the `scope-manifest` spec, including its Chain check by a member, it SHALL do so, and for each device the version adds print `bilbo: sync <name>: device <device name> added by <signer> (manifest <n>)` once, and for a new epoch `bilbo: sync <name>: epoch changed (manifest <n>)` once. `<signer>` is the writing device's name when the version names it, else `owner key`. It SHALL keep these changes for `bilbo sync` to list for 30 days.

#### Scenario: A device added elsewhere
- **WHEN** a new device `moria` runs `bilbo device recover` with the phrase, writing manifest 4 of `personal` that adds it, and A's watch adopts it
- **THEN** A's stderr holds `bilbo: sync personal: device moria added by owner key (manifest 4)` once, A syncs with `moria`, and `bilbo sync` on A lists the change for 30 days

#### Scenario: An epoch nobody here revoked for
- **WHEN** a version this device did not write moves `personal` to a new epoch
- **THEN** watch adopts it, prints `bilbo: sync personal: epoch changed (manifest <n>)` once, and `bilbo sync` lists it

#### Scenario: Own versions are not reported
- **WHEN** this device runs `bilbo device revoke bagend`, writing manifest 5 with a new epoch
- **THEN** watch publishes it and prints no added or epoch line for it

### Requirement: One scope per name
When a device can open two published scopes with one name, it SHALL keep the one whose latest version lists more devices, or on a tie the one whose id sorts first, push its notes of the other into it, stop syncing the other, and print `bilbo: sync <name>: scope <id> replaces <id>`.

#### Scenario: Two first devices
- **WHEN** A and B each created and published `personal` before knowing of each other, and B later holds A's scope too, which then lists A and B
- **THEN** B keeps A's scope, pushes its notes of `personal` there, stops syncing its own, and A receives them

#### Scenario: Only one of two can be opened
- **WHEN** this device syncs its published `personal`, and the folder also holds a scope of the same owner that it cannot open
- **THEN** it keeps syncing `personal` and prints no replaces line

### Requirement: A folder without this device's scopes
When a `file://` scope's folder holds none of the confirmed manifest versions this device holds for its scopes, watch SHALL print `bilbo: sync <name>: <folder> holds none of this device's scopes; if the folder moved, change scope.<name>.sync on every device` once, and sync nothing for that scope until it does.

#### Scenario: A folder moved on one device only
- **WHEN** the user points this device's `scope.personal.sync` at a new, empty folder while `personal` is confirmed in the old one
- **THEN** watch prints the line once, naming the new folder, and pushes nothing there

#### Scenario: A folder in step
- **WHEN** the folder holds this device's confirmed version of `personal`
- **THEN** watch prints no such line

### Requirement: An unreachable transport
When a scope's transport cannot be read or written, watch SHALL keep recording history, retry with growing waits of at most 10 minutes, and print `bilbo: sync <name>: <url> is not reachable: <reason>` once until it is reachable again. Watch SHALL NOT create a `file://` transport's folder.

#### Scenario: An unmounted folder
- **WHEN** the scope's folder is on a volume that is not mounted
- **THEN** watch prints the not-reachable line once, creates no folder at that path, and pushes the waiting versions once the volume is back

### Requirement: A full transport
When the transport refuses a write because it is full, such as a `file://` folder on a full disk, watch SHALL print `bilbo: sync <name>: <the transport's message>` once, apart from the not-reachable line, keep recording history, keep pulling, and retry the push at each poll.

#### Scenario: A full disk
- **WHEN** the folder of `personal` sits on a disk with no space left and an agent edits a synced note
- **THEN** watch prints one line naming `personal` and the full disk, records the edit, still applies other devices' segments, and pushes the edit once space is free

### Requirement: Conflicts are raised
When a merge leaves a conflict, watch SHALL print `bilbo: notes/<file>: conflict in <n> passages; run bilbo check`, with `passage` for one, and write the file with its conflict blocks, as the `note-merge` spec says.

#### Scenario: A clean merge
- **WHEN** B's edit of `## Setup` merges with A's own edit of `## Rollout`
- **THEN** A's stderr holds no conflict line

#### Scenario: A conflict on pull
- **WHEN** B's edit of `## Rollout` merges with A's own edit of it
- **THEN** A's stderr holds `bilbo: notes/<file>: conflict in 1 passage; run bilbo check`, and the file holds the block
