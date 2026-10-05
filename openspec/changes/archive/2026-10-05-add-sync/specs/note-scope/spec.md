# Spec Delta

## MODIFIED Requirements

### Requirement: Assigning never loses a write
`bilbo scope set` SHALL work under the `note-restore` spec's history lock and hidden file, `<root>/notes/.bilbo-restore-<id>`, sweeping leftovers first as `bilbo restore` does. It SHALL swap the new text in atomically and compare what came out with what it read. When they differ, it SHALL swap back, so the other writer's bytes are in place, and report the file. It SHALL delete only bytes it read or wrote itself; other bytes stay at the hidden name for the next sweep to record. Under the history lock it SHALL record the version it wrote, so the watcher does not take it for a local save; a pending stale-base entry SHALL keep its base, with that version as its written version.

#### Scenario: An agent writes during the swap
- **WHEN** an agent writes new text to `plan-a.md` after `bilbo scope set work` read it and before it swapped
- **THEN** `plan-a.md` holds the agent's text, stderr is `bilbo: notes/plan-a.md: changed while bilbo scope set ran; run it again`, and the exit code is 1

#### Scenario: A second write lands between the two swaps
- **WHEN** an agent writes `plan-a.md` before the first swap and again between the first swap and the swap back
- **THEN** `plan-a.md` holds the agent's first write, `notes/.bilbo-restore-<id of a>` holds the second, stderr names that hidden file, the exit code is 1, and the next `bilbo watch` scan records the second write as an `edited` version and removes the hidden file

#### Scenario: A filesystem that cannot swap
- **WHEN** the store sits on a filesystem that cannot exchange two files atomically and a user runs `bilbo scope set work <file>`
- **THEN** stderr is `bilbo: cannot set scopes on this filesystem: it cannot swap files atomically`, no file changes, and the exit code is 1

#### Scenario: A stale save after scope set
- **WHEN** an agent read `plan-a.md` at H, B's edit of `## Setup` was written into it by sync, the user runs `bilbo scope set personal notes/plan-a.md`, and the agent then saves its edit of `## Rollout` made from H
- **THEN** the file holds B's `## Setup`, the agent's `## Rollout` and `scope: personal`, and history lists the version `scope set` wrote and then a `merged` version flagged `stale-base`
