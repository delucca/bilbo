# Spec Delta

## Purpose
`bilbo watch` runs in the background and records a version of each note when it changes, so a note's past is never lost to an agent's rewrite and later changes have a history to merge against.

## ADDED Requirements

### Requirement: Watch runs until stopped
`bilbo watch` SHALL take no arguments, run until it is stopped by a signal, and watch `<root>/notes/`. When it starts, it SHALL print `bilbo: watching <root>/notes` to stderr. When `<root>` does not exist, it SHALL print `bilbo: no store at <root>` to stderr and exit 1. When `<root>` exists and `<root>/notes/` cannot be listed, it SHALL start and wait for the folder, as `A missing notes folder` says. An argument SHALL be a usage error.

#### Scenario: Watch starts
- **WHEN** an agent runs `bilbo watch` against a store
- **THEN** stderr holds `bilbo: watching <root>/notes` and the process keeps running

#### Scenario: No store
- **WHEN** `BILBO_HOME` names a folder that does not exist and a user runs `bilbo watch`
- **THEN** stderr is `bilbo: no store at <that folder>`, nothing is created, and the exit code is 1

#### Scenario: A store without notes
- **WHEN** `BILBO_HOME` names a folder that holds `library/` and no `notes/`, and a user runs `bilbo watch`
- **THEN** stderr holds `bilbo: cannot read <root>/notes: <reason>; waiting`, the process keeps running, and once `notes/` is created holding a note, stderr holds `bilbo: watching <root>/notes` and that note has an `added` version

#### Scenario: An argument
- **WHEN** a user runs `bilbo watch --now`
- **THEN** bilbo prints a message naming `--now` to stderr and exits 2

### Requirement: One watcher per store
At most one `bilbo watch` SHALL record for a store root at a time. A watcher that finds another one recording for its root SHALL print `bilbo: bilbo watch is already running for <root>; waiting` to stderr and wait, recording nothing, until the other one stops. It SHALL then print `bilbo: watching <root>/notes` and record as usual. A watcher SHALL NOT take a `bilbo history` run that is checking for a watcher as another watcher.

#### Scenario: A second watcher waits
- **WHEN** `bilbo watch` is running against a store and a user starts another against the same root
- **THEN** the second prints the already-running message and keeps running, the first keeps recording, and each change is recorded once

#### Scenario: The second takes over
- **WHEN** a second watcher is waiting and the first one is stopped
- **THEN** the second prints `bilbo: watching <root>/notes`, and an edit made after that is recorded

#### Scenario: History does not block a starting watcher
- **WHEN** an agent runs `bilbo history release` in a loop and `bilbo watch` starts
- **THEN** the watcher prints `bilbo: watching <root>/notes` and no already-running message

#### Scenario: Two stores, two watchers
- **WHEN** `bilbo watch` runs with `BILBO_HOME=/a` and a user starts one with `BILBO_HOME=/b`
- **THEN** both keep running

### Requirement: What watch records
Watch SHALL record a note when a regular, non-hidden file directly in `<root>/notes/`, named `<kind>-<topic>.md` as the `note-store` spec defines note names, opens with frontmatter whose `id` is a canonical ULID. The note is that id, whatever kind and topic the file's name holds. It SHALL record a new version of the note when the file's bytes or its name differ from the note's latest version:
- `added`: the id has no history yet;
- `edited`: the bytes changed and the name did not;
- `renamed`: the name changed, whatever happened to the bytes;
- `deleted`: no file holds the id any more;
- a file that holds an id whose latest version is `deleted` is recorded as `edited`.

Versions SHALL be recorded once `<root>/notes/` has had no change for 2 seconds, and at most 10 seconds after the first change while changes to any file in it keep coming. Changes to a note in that time collapse into one version.

#### Scenario: A new note
- **WHEN** watch is running and an agent runs `bilbo new decision release`
- **THEN** within 10 seconds `bilbo history release` lists one version, `added`

#### Scenario: An edit
- **WHEN** watch is running and an agent appends a paragraph to `decision-release.md`
- **THEN** within 10 seconds `bilbo history release` lists an `edited` version above the `added` one

#### Scenario: A rename keeps the note
- **WHEN** watch is running and an agent renames `decision-release.md` to `plan-release.md`, keeping its id
- **THEN** `bilbo history release` lists a `renamed` version whose file is `plan-release.md`, under the same note

#### Scenario: A deletion
- **WHEN** watch is running and an agent deletes `decision-release.md`
- **THEN** `bilbo history release` lists a `deleted` version first

#### Scenario: A burst of saves
- **WHEN** watch is running and an agent saves `decision-release.md` five times, one second apart
- **THEN** `bilbo history release` lists one new `edited` version for the burst, holding the last save

#### Scenario: Busy neighbors do not hold a note back forever
- **WHEN** watch is running, an agent saves `plan-other.md` every second, and another agent appends a paragraph to `decision-release.md`
- **THEN** within 10 seconds `bilbo history release` lists the `edited` version

#### Scenario: Touching a file records nothing
- **WHEN** watch is running and a user runs `touch notes/decision-release.md`
- **THEN** no version is recorded

#### Scenario: A rewrite that keeps the modification time
- **WHEN** watch is running and a user runs `cp -p` to put a file of the same length with different bytes over `decision-release.md`
- **THEN** within 10 seconds `bilbo history release` lists an `edited` version holding the new bytes

### Requirement: A missing notes folder
When watch cannot list `<root>/notes/`, at start or while it runs, it SHALL record nothing, print `bilbo: cannot read <root>/notes: <reason>; waiting` to stderr once, and keep running. Once the folder can be listed again, it SHALL print `bilbo: watching <root>/notes` and record what changed, under the rules above. When the folder lists no file that holds a note id while history holds a note whose latest version is not `deleted`, watch SHALL record nothing and print `bilbo: <root>/notes holds no notes; not recording deletions` to stderr once.

#### Scenario: The folder is moved away and back
- **WHEN** watch is running against a store of 3 notes, and a user moves `notes/` to `notes.bak`, waits 15 seconds, and moves it back
- **THEN** stderr holds the cannot-read line and then the watching line, and no note has a `deleted` version

#### Scenario: The folder is emptied
- **WHEN** watch is running against a store of 3 notes and every file in `notes/` is removed
- **THEN** stderr holds the holds-no-notes line and no note has a `deleted` version

#### Scenario: One note among others is deleted
- **WHEN** watch is running against a store of 3 notes and an agent deletes one of them
- **THEN** that note has a `deleted` version and the other two do not

### Requirement: Changes while not running
On start, before waiting for changes, watch SHALL compare every file in `<root>/notes/` with history and record what changed while it was not running, under the rules above: one version per note, holding the file as it is now.

#### Scenario: An edit while stopped
- **WHEN** watch is stopped, an agent edits `decision-release.md` twice and deletes `plan-old.md`, and watch starts again
- **THEN** history holds one new `edited` version of `release` and one `deleted` version of `old`

#### Scenario: A first start on an existing store
- **WHEN** a store holds 40 notes, no history exists, and watch starts
- **THEN** each of the 40 notes has one `added` version

### Requirement: What watch skips
Watch SHALL NOT record hidden entries, subfolders, entries that are not regular files, files whose name is not `<kind>-<topic>.md`, files larger than 1 MiB, files whose frontmatter has no canonical ULID `id`, or two or more files that hold the same id. For each file skipped for its name, its id or its size, or for a shared id, it SHALL print one line to stderr, `bilbo: notes/<name>: not recorded: <reason>`, and SHALL print it again only after that file changes. A note whose only file becomes skipped SHALL NOT be recorded as deleted.

#### Scenario: A file without an id
- **WHEN** watch is running and an agent writes `notes/plan-x.md` with no frontmatter
- **THEN** stderr holds one line naming `notes/plan-x.md` and the missing id, and no version is recorded

#### Scenario: A shared id
- **WHEN** an agent copies `decision-release.md` to `plan-release-copy.md`
- **THEN** stderr names both files, neither is recorded until one of them changes its id or is removed, and `release` has no `deleted` version

#### Scenario: A file that is not named as a note
- **WHEN** watch is running and an agent writes `notes/Release.md` with a valid id
- **THEN** stderr holds one line naming `notes/Release.md` and saying its name is not `<kind>-<topic>.md`, and no version is recorded

#### Scenario: A hidden file
- **WHEN** an editor writes `notes/.decision-release.md.swp`
- **THEN** nothing is recorded and nothing is printed

#### Scenario: A huge file
- **WHEN** an agent writes a 5 MiB file with a valid id into `notes/`
- **THEN** stderr names it as larger than 1 MiB and no version is recorded

### Requirement: Watch leaves the notes alone
Watch SHALL NOT create, change, rename or delete anything in `<root>/notes/`, with one exception: a `notes/.bilbo-restore-<id>` file left by an interrupted `bilbo restore`. Watch SHALL sweep such a file at each scan, under the `note-restore` spec's rule for leftovers. Apart from that, it SHALL write only under `<root>/.bilbo/`.

#### Scenario: Notes are untouched
- **WHEN** watch runs for a minute while an agent edits notes
- **THEN** every file in `notes/` has the bytes and modification time the agent left it with

#### Scenario: A restore leftover is swept
- **WHEN** watch is running and `notes/.bilbo-restore-<id of release>` holds bytes no version of `release` holds
- **THEN** within 10 seconds that file is gone, `bilbo history release` lists an `edited` version holding its bytes, and stderr names the file as recorded from an interrupted restore

### Requirement: Pruning schedule
Watch SHALL prune history, under the `note-history` spec's retention rule, once when it starts and then once every 24 hours while it runs. When a prune drops versions, it SHALL print `bilbo: pruned <n> versions older than <date>` to stderr.

#### Scenario: Pruning at start
- **WHEN** history holds versions older than `history.keep_days` and watch starts
- **THEN** those versions are dropped as the retention rule says, and stderr holds the pruned line

#### Scenario: Nothing to prune
- **WHEN** every version is newer than `history.keep_days` and watch starts
- **THEN** no pruned line is printed
