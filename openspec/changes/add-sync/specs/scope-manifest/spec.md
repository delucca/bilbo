# Spec Delta

## MODIFIED Requirements

### Requirement: Versions that init writes
`init` SHALL write version 1 for a scope with a sync URL and no manifest, listing this device and every device that the latest version of every other manifest of its owner that this device opens lists, except a device that any of those manifests listed once and no longer lists. When the config's URL differs from `transport`, comparing `https://` and `http://` URLs whole and a `file://` URL by its scheme only, it SHALL write a new version with the same epoch, in a terminal only, as the `device-identity` spec's Terminal-only forms says. It SHALL NOT add this device to a manifest that does not list it. It SHALL match a config name through the newest version that lists this device, so a scope whose latest version dropped this device is never created again, and while a local manifest of its owner has never listed this device, or is invalid for this device before any version it can read, it SHALL create no scope id for a syncing scope it matched to no manifest, reporting that scope `unsealed`, as recover does. Before writing version 1 for a scope whose transport it can reach, it SHALL read the transport, and SHALL NOT write it while the transport holds a scope of this owner and this device is in no scope there; that scope SHALL then fail with `run bilbo device recover on this device`.

#### Scenario: A changed URL
- **WHEN** `personal` is at manifest 1 pinned to `file://`, the config now says `https://relay.example.net`, and the user runs `bilbo device init` in a terminal
- **THEN** stdout has `scope personal updated: <scope id> manifest 2 epoch 1`, and `2.json`'s `transport` is the new URL

#### Scenario: Another folder path
- **WHEN** `personal` pins `file://` and the config's folder moves from `file:///Users/a/Sync/bilbo` to `file:///home/a/Sync/bilbo`
- **THEN** `bilbo device init` reports `scope personal kept` and writes no version, and `bilbo device` prints no problem line for it

#### Scenario: A changed URL without a terminal
- **WHEN** the same config change is made and an agent runs `bilbo device init`
- **THEN** stdout has `scope personal failed: changing the URL needs a terminal`, no version is written for it, other scopes are still created, and the exit code is 1

#### Scenario: A second syncing scope
- **WHEN** `personal`'s manifest lists `rivendell` and `bagend`, and the user adds `scope.shared.sync` on `rivendell` and runs `bilbo device init`
- **THEN** `shared`'s manifest 1 lists both devices and seals its key to both and to the owner

#### Scenario: A device a manifest dropped
- **WHEN** an earlier version of `personal` listed this device and the latest does not, and the user runs `bilbo device init`
- **THEN** init reports `scope - kept: <scope id>` and writes no version

#### Scenario: A manifest this device never read
- **WHEN** the store holds a manifest of this owner that has never listed this device, the config sets `scope.personal.sync` to a URL, no manifest this device can open is named `personal`, and the user runs `bilbo device init`
- **THEN** init reports `scope - kept: <scope id>` for that manifest and `scope personal unsealed: copy the store from an enrolled device, then run bilbo device recover again`, and `<root>/.bilbo/scopes/` holds no new folder

#### Scenario: The transport already holds this owner's scope
- **WHEN** an enrolled device that is in no scope on `file:///Users/a/Dropbox/bilbo` runs `bilbo device init` for `personal`, and that folder holds a scope of the same owner
- **THEN** stdout has `scope personal failed: run bilbo device recover on this device`, no version 1 is written for it, and the exit code is 1

#### Scenario: A member creates a scope
- **WHEN** a device listed in `personal` on that folder runs `bilbo device init` for a new scope `shared`
- **THEN** `shared`'s version 1 is written as before
