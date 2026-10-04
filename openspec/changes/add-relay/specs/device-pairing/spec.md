# Spec Delta

## MODIFIED Requirements

### Requirement: The transport URL on the new device
The new device SHALL take its transport only from `--via`: a `file:///<absolute path>` URL, or `https://` or loopback `http://` as the config spec's sync URLs allow, the last two reaching a relay as the `relay-transport` spec says. For each scope paired, it SHALL use the `--via` URL when the scope's URL on A is literally the URL A pairs over, and the scope's own URL when that is an `https://` URL. A `--via` folder that does not exist SHALL be refused.

#### Scenario: The folder has another path
- **WHEN** A syncs `personal` through `file:///Users/a/Dropbox/bilbo` and B runs `bilbo pair <code> --via file:///home/a/Dropbox/bilbo`, the same synced folder
- **THEN** B's config holds `scope.personal.sync = file:///home/a/Dropbox/bilbo`, and no manifest version is written for the path, since a manifest pins only `file://` for a folder

#### Scenario: A missing folder
- **WHEN** B runs `bilbo pair <code> --via file:///nope`
- **THEN** stderr says `no folder at /nope`, the exit code is 1, and the code still works

#### Scenario: A remote plain-HTTP URL
- **WHEN** B runs `bilbo pair <code> --via http://bagend:8090`
- **THEN** bilbo prints a message naming the URL to stderr, exits 2, and touches no mailbox

#### Scenario: A relay URL
- **WHEN** A syncs `personal` through `https://relay.example`, shows a code over it, and B runs `bilbo pair <code> --via https://relay.example`
- **THEN** the two devices pair through the relay's mailbox, and B's config holds `scope.personal.sync = https://relay.example`
