## MODIFIED Requirements

### Requirement: Terminal-only forms
`bilbo device recover`, `bilbo device revoke`, and `bilbo device init` when it would create a phrase or change a scope's transport, SHALL run that part only when stdin and stderr are both terminals and none of `AI_AGENT`, `CLAUDE_CODE_CHILD_SESSION`, `CODEX_THREAD_ID` and `CODEX_CI` is set and not empty. Otherwise they SHALL show and read nothing, write nothing for that part, print a stderr line saying the user must run it in a terminal, and exit 1.

#### Scenario: An agent runs init
- **WHEN** an agent runs `bilbo device init` with stdin not a terminal, on a device with no keys
- **THEN** stdout is empty, stderr is one line naming `bilbo device init` and a terminal, no file is written, and the exit code is 1

#### Scenario: Under Claude Code with a terminal
- **WHEN** `CLAUDE_CODE_CHILD_SESSION=1` is set and `bilbo device recover` runs with terminals on stdin and stderr
- **THEN** it reads nothing, writes nothing and exits 1

#### Scenario: Under Codex
- **WHEN** `CODEX_THREAD_ID` is set and `bilbo device revoke bywater` runs
- **THEN** no manifest is written, stderr names a terminal, and the exit code is 1

#### Scenario: Under an agent that names itself
- **WHEN** `AI_AGENT=x` or `CODEX_CI=1` is set and `bilbo device init` runs with terminals on stdin and stderr on a device with no keys
- **THEN** it shows no phrase, writes nothing and exits 1

#### Scenario: A person in an IDE terminal
- **WHEN** only `CLAUDECODE=1` is set and a user runs `bilbo device init` in a terminal on a device with no keys
- **THEN** the phrase ceremony starts

#### Scenario: An enrolled device needs no terminal to seal a new scope
- **WHEN** an agent runs `bilbo device init` without a terminal on an enrolled device, and a scope has a sync URL and no manifest
- **THEN** init creates that scope's manifest and exits 0, without asking for or showing a phrase

## ADDED Requirements

### Requirement: Human view of devices
When stdout gets the `cli` spec's human view, `bilbo device` SHALL print a dim `This device` label, the device's name in bold and its id dim, a dim `Owner` label and the owner fingerprint dim, then, after a blank line, a table of scopes under the dim header `SCOPE`, `SYNC`, `DEVICES`, `MANIFEST`, `EPOCH` and `ID`, with a `▲` row for a scope that is unsealed or invalid. A device with no keys SHALL print one `○` line naming `bilbo setup` and `bilbo device recover`. `bilbo device list` SHALL align names in bold and ids dim, with `this device` in green on this device's row. The `init` and `recover` step report SHALL take the marks and columns of the `setup` spec's Human view of the step report, with `▲` for `unsealed`, and no summary line.

#### Scenario: An enrolled device on a terminal
- **WHEN** `rhosgobel` is enrolled, `personal` syncs to `file:///srv/bilbo` with 2 devices, and a user runs `bilbo device` in a terminal
- **THEN** a line holds `rhosgobel` and its id, a line holds the owner fingerprint, and a table row starts with `personal`

#### Scenario: No keys on a terminal
- **WHEN** the device has no keys and a user runs `bilbo device` in a terminal
- **THEN** stdout is one line starting with `○` that names `bilbo setup` and `bilbo device recover`, and the exit code is 0

#### Scenario: A listed device on a terminal
- **WHEN** a user runs `bilbo device list` in a terminal on `rhosgobel`
- **THEN** the `rhosgobel` row ends with `this device`

#### Scenario: Init with keys on a terminal
- **WHEN** an enrolled device runs `bilbo device init` in a terminal and every step is kept
- **THEN** every stdout line starts with `◇`

#### Scenario: A pipe keeps the tabs
- **WHEN** an agent runs `bilbo device` with stdout piped
- **THEN** stdout is the tab-separated lines of Show this device
