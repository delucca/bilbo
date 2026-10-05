# Spec Delta

## MODIFIED Requirements

### Requirement: What recover writes
On a device without keys, `recover` SHALL write the owner and device keys. On an enrolled device it SHALL keep its device key. Either way, after fetching scopes from the transport as the Recover fetches scopes from the transport requirement says, it SHALL add this device to every local manifest of its owner whose latest version does not list it, as the `scope-manifest` spec says. It SHALL NOT create a scope id. A syncing scope that still has no local manifest SHALL be `unsealed`: when its transport was read and holds no scope of this owner by that name, naming `bilbo device init`; when this bilbo has no client for its URL's scheme, telling the user to bring in its manifest and run `recover` again, or to pair this device with one that syncs the scope, never `init`, which would fork the scope.

#### Scenario: A wiped laptop
- **WHEN** the store holds `personal`'s manifest listing `rivendell`, the keys were lost, and the user runs `bilbo device recover --name rivendell-2` with the right phrase
- **THEN** stdout has `owner recovered: <fingerprint>`, `device created: rivendell-2 <id>` and `scope personal updated: <scope id> manifest 2 epoch 1`, and `bilbo device list` shows both devices

#### Scenario: A fresh machine
- **WHEN** the store holds no manifest, the config gives `personal` a sync URL whose transport holds no scope of this owner named `personal`, and the user recovers and confirms the fingerprint
- **THEN** stdout has `owner recovered: <fingerprint>`, `device created: <name> <id>` and `scope personal unsealed: <url> holds no scope personal of this owner; run bilbo device init to create it`, and `<root>/.bilbo/scopes/` holds no new folder

#### Scenario: A relay URL
- **WHEN** the store holds no manifest, the config gives `personal` the URL `https://relay.example`, which this bilbo has no client for, and the user recovers and confirms the fingerprint
- **THEN** stdout has `scope personal unsealed: copy the store from an enrolled device, then run bilbo device recover again, or run bilbo pair with a device that syncs it`, and `<root>/.bilbo/scopes/` holds no new folder

#### Scenario: Finishing an interrupted recover
- **WHEN** a recover wrote the keys but stopped before `shared`'s manifest, and the user runs `bilbo device recover` again with the same phrase
- **THEN** `device kept` and `scope shared updated` are printed, and `personal`, which already lists the device, is `kept`
