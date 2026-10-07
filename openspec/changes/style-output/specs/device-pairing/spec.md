## MODIFIED Requirements

### Requirement: Who can pair
A SHALL refuse, before creating a mailbox, when it holds no owner key, when no scope syncs, or unless stdin and stderr are terminals and none of the agent markers of the `device-identity` spec's Terminal-only forms is set and not empty. Before asking, A SHALL refuse a device of another owner, or whose name is A's own or another device's in any manifest of the owner. B SHALL refuse with no `<root>/notes/`, and before writing, a store of another owner. Each refusal SHALL exit 1.

#### Scenario: The phrase was never set
- **WHEN** a device with no owner key runs `bilbo pair`
- **THEN** stderr says `this device has no owner key; turn on sync for a scope first, which sets the recovery phrase`, the exit code is 1, and nothing is written

#### Scenario: An enrolled device joins another scope
- **WHEN** `bywater` is enrolled and listed in `personal`, A runs `bilbo pair --scope shared`, and `bywater` answers
- **THEN** A adds `bywater` to `shared` only, the reply carries no owner seed, `bywater`'s keys folder is unchanged, and its config gains `scope.shared.sync`

#### Scenario: Enrolled with another owner
- **WHEN** the answering device is enrolled with another owner
- **THEN** A says `<B's name> belongs to another owner (<its fingerprint>)`, B says `this device belongs to owner <its fingerprint>, the other device to <A's>`, nothing is sent, and both exit 1

#### Scenario: An agent shows a code
- **WHEN** an agent runs `bilbo pair` with stdin not a terminal, or with `CLAUDE_CODE_CHILD_SESSION=1` or `CODEX_THREAD_ID` set
- **THEN** stderr says `pairing is confirmed only in a terminal, by the user`, no mailbox is created, and the exit code is 1

#### Scenario: A person in an IDE terminal
- **WHEN** only `CLAUDECODE=1` is set and a user runs `bilbo pair` in a terminal on an enrolled device whose scope syncs
- **THEN** A creates the mailbox and shows the code

#### Scenario: A taken name
- **WHEN** `personal` lists a device named `bywater` and a new device named `bywater` answers
- **THEN** A says `a device named bywater is already enrolled; pair again with --name on the new device`, B says `the name bywater is taken; run bilbo pair again with --name`, nothing is sent, and both exit 1

#### Scenario: A name taken in a scope not being paired
- **WHEN** `personal` lists a device named `bywater`, A runs `bilbo pair --scope shared`, and a new device named `bywater` answers
- **THEN** A refuses it as a taken name, and no manifest changes

#### Scenario: The showing device's own name
- **WHEN** A is named `rhosgobel` and the new device answers as `rhosgobel`
- **THEN** A refuses it as a taken name

#### Scenario: A store of another owner
- **WHEN** B holds no keys but its store holds a `personal` manifest signed by another owner key
- **THEN** B prints both owner fingerprints and `nothing was written`, writes no key, manifest or config line, and exits 1

#### Scenario: No store on the new device
- **WHEN** B's `BILBO_HOME` has no `notes/`
- **THEN** stderr says `no store at <root>`, the exit code is 1, and the code still works
