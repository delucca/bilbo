# Spec Delta

## MODIFIED Requirements

### Requirement: Transport URLs
A scope's transport SHALL be named by its `scope.<name>.sync` URL. A `file:///<absolute path>` URL SHALL name a folder that holds the tree. An `https://` URL, or an `http://` URL to a loopback host, SHALL name a relay that serves the tree, reached as the `relay-transport` spec says. For a URL of any other scheme, watch SHALL print `bilbo: sync <name>: <scheme> transports are not supported yet; use a file:// folder` once and sync nothing for that scope.

#### Scenario: A folder transport
- **WHEN** the config holds `scope.personal.sync = file:///Users/a/Dropbox/bilbo`
- **THEN** watch reads and writes the tree under `/Users/a/Dropbox/bilbo`

#### Scenario: A relay URL
- **WHEN** the config holds `scope.personal.sync = https://relay.example`
- **THEN** watch reads and writes the tree for `personal` through the relay at `https://relay.example/v1/`, and prints no not-supported line

### Requirement: Create-only writes
Writing through a hidden temporary file named `.<device id>-<16 lowercase hexadecimal characters>.tmp` in the target folder, a device SHALL NOT overwrite, append to, rename or delete an object, with two exceptions on a `file://` transport: removing a pairing mailbox deletes `pair/<nameplate>/`, the one deletion allowed; and the Damaged own segments requirement replaces a damaged file in the device's own folder. It SHALL create a missing object again only with the same bytes. Creating an existing object SHALL fail, except that a relay SHALL take a create of an object it holds with the same bytes as done and change nothing, as the `relay-api` spec says, so a create whose answer was lost can be sent again.

#### Scenario: Only the owner writes its folder
- **WHEN** devices A and B sync one scope through a test run of edits, merges and deletions
- **THEN** every file under A's device folder was created by A, none was changed after it was created, and the same holds for B

#### Scenario: Removing a mailbox
- **WHEN** the `file://` transport is asked to remove nameplate `7`
- **THEN** `pair/7/` is gone, and nothing under `scopes/` changed

#### Scenario: A lost answer from a relay
- **WHEN** a device's create of segment 7 reached the relay but the answer was lost, and the device sends the same bytes again
- **THEN** the create counts as done and the relay's copy is unchanged
