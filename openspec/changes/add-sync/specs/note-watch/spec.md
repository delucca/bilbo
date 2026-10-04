# Spec Delta

## MODIFIED Requirements

### Requirement: Watch leaves the notes alone
Watch SHALL NOT create, change, rename or delete anything in `<root>/notes/`, with two exceptions: a `notes/.bilbo-restore-<id>` file left by an interrupted `bilbo restore` or sync write, which watch SHALL sweep at each scan under the `note-restore` spec's rule for leftovers, completing an interrupted inbound write instead when the file holds a version waiting to be written from another device; and versions from other devices of a syncing scope, which it SHALL write as the `note-sync` spec says. Apart from that, it SHALL write only under `<root>/.bilbo/` and in the transports of the syncing scopes.

#### Scenario: Notes are untouched
- **WHEN** watch runs for a minute while an agent edits notes, and no other device pushes
- **THEN** every file in `notes/` has the bytes and modification time the agent left it with

#### Scenario: A restore leftover is swept
- **WHEN** watch is running and `notes/.bilbo-restore-<id of release>` holds bytes no version of `release` holds
- **THEN** within 10 seconds that file is gone, `bilbo history release` lists an `edited` version holding its bytes, and stderr names the file as recorded from an interrupted restore

#### Scenario: A version from another device
- **WHEN** `personal` syncs and another device pushes an edit of a note in `personal`
- **THEN** watch writes the edit into that note's file and changes no other file in `notes/`
