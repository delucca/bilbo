# Spec Delta

## MODIFIED Requirements

### Requirement: Joining and leaving a scope
A device SHALL sync a scope only while the latest valid manifest version lists it and seals the epoch key to it; it gets there through `bilbo pair` with a member, through `bilbo device recover` with the phrase, or a version another member writes, never through watch alone. Watch SHALL publish no version 1 of a scope while this device is in no scope on that transport and the transport holds a scope of this owner that it cannot open, and print the not-in-the-scope line instead. Watch SHALL NOT seal an epoch key to any device, nor write a version that lists a device, except when re-applying a version this device wrote. A device that a newer version no longer lists SHALL stop syncing that scope, print `bilbo: sync <name>: this device was removed from the scope`, and keep its notes.

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
- **THEN** B's watch publishes no manifest, syncs nothing for `personal`, and prints `bilbo: sync personal: this device is not in the scope; run bilbo pair with a device that syncs <url>, or bilbo device recover on this device`

#### Scenario: Paired into the scope afterwards
- **WHEN** that device B then answers a code from A with `bilbo pair <code> --via <url>` and the user confirms on A
- **THEN** B's keys are unchanged, the latest `personal` version lists B, and B's watch syncs `personal` within one cycle
