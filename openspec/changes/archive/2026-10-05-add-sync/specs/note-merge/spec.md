# Spec Delta

## Purpose
How bilbo combines two versions of a note that were edited apart, passage by passage, so that edits to different passages both survive and edits to the same passage are kept side by side for an agent to resolve.

## ADDED Requirements

### Requirement: Merge by passage
bilbo SHALL merge two versions of a note against their lowest common version, the base. The body SHALL be split into passages as the `note-recall` spec's Passages requirement defines them, before its 4,000-byte split, plus the text before the first heading. A passage changed on one side only SHALL take that side's text. A passage changed identically on both sides SHALL appear once. A passage changed differently on both sides SHALL be a conflict.

#### Scenario: Two passages, two devices
- **WHEN** the base has `## Setup` and `## Rollout`, device A edits `## Setup` and device B edits `## Rollout`
- **THEN** the merged note holds A's `## Setup` and B's `## Rollout`, and has no conflict

#### Scenario: A nested heading is its own passage
- **WHEN** A edits the text under `## Rollout` and B edits the text under `### Rollback`, nested in `## Rollout`
- **THEN** the merged note holds both edits and has no conflict

#### Scenario: One passage, two edits
- **WHEN** A and B both edit the text under `## Rollout`, differently
- **THEN** the merged note holds a conflict for `## Rollout` with both texts, and neither is dropped

#### Scenario: The same edit on both sides
- **WHEN** A and B make the same edit under `## Rollout`
- **THEN** the merged note holds that edit once and has no conflict

### Requirement: Added, deleted and renamed passages
A passage added on either side SHALL be kept, after the passage that precedes it on its side; at one place, the side whose version id sorts first comes first. A passage deleted on one side SHALL be deleted when the other left it unchanged, and kept with the edit, flagged `edit-beat-delete`, when the other edited it. A heading changed on one side, with or without a text edit, SHALL combine with the other side's edit of that passage.

#### Scenario: Both sides append
- **WHEN** A appends `## Notes A` and B appends `## Notes B` to the same note
- **THEN** the merged note ends with both passages, in the order of their versions' ids, and has no conflict

#### Scenario: Deleted against edited
- **WHEN** A deletes `## Rollout` and B edits it
- **THEN** the merged note holds B's `## Rollout` and the merge version is flagged `edit-beat-delete`

#### Scenario: Deleted against untouched
- **WHEN** A deletes `## Rollout` and B leaves it as in the base
- **THEN** the merged note has no `## Rollout`

#### Scenario: Renamed against edited
- **WHEN** A renames the heading `## Rollout` to `## Release` and B edits its text
- **THEN** the merged note holds `## Release` with B's text, once, and has no conflict

#### Scenario: Renamed and edited against edited
- **WHEN** A renames `## Rollout` to `## Release` and edits its text, and B edits the text of `## Rollout` differently
- **THEN** the merged note holds one conflict block for that passage, with A's `## Release` side and B's `## Rollout` side, and no second copy of either

#### Scenario: A parent heading renamed
- **WHEN** A renames `## Rollout` to `## Release`, and B edits `### Rollback` beneath it
- **THEN** the merged note holds `## Release` with `### Rollback` and B's edit beneath it, once

### Requirement: Frontmatter merge
Frontmatter SHALL merge key by key. `id` SHALL never change. `created` SHALL keep the base's value, and a side that changed it SHALL flag the merge `created-kept`. `sources` SHALL merge as a set: an item either side added is kept, an item one side removed and the other kept is removed, and the order is the base's order followed by added items in sorted order. `scope` SHALL follow the `note-sync` spec's Scope clash requirement. Any other key changed on both sides SHALL take the value of the side whose version id sorts first, flagged `key-kept`. No marker SHALL be written above the frontmatter's closing `---`.

#### Scenario: Sources from both sides
- **WHEN** A adds the source `url: https://a.example` and B adds `code: src/watch.rs:12`
- **THEN** the merged frontmatter lists both, after the base's sources

#### Scenario: A removed source
- **WHEN** A removes the source `url: https://old.example` and B leaves the sources alone
- **THEN** the merged frontmatter does not list it

#### Scenario: An unknown key changed on both sides
- **WHEN** both sides change a `project:` line the note should not hold, to different values
- **THEN** the merged frontmatter holds the value of the side whose id sorts first, no marker line, and the flag `key-kept`

#### Scenario: A changed created
- **WHEN** A changes `created` and B edits a passage
- **THEN** the merged note has the base's `created`, B's edit, and the flag `created-kept`

### Requirement: Conflict markers
Marker lines SHALL be read only outside fenced code blocks. A conflict SHALL be written in place of the passage as a block: a line `<<<<<<< bilbo <version> <time>` and that side's passage, then for each further side a line `======= bilbo <version> <time>` and its passage, then `>>>>>>> bilbo`. `<version>` is the side's version id as `bilbo history` lists it, and `<time>` the time it was recorded. Sides SHALL be ordered by version id. A side that deleted the passage SHALL be empty.

#### Scenario: Two sides
- **WHEN** A's version `3f9a2c1b0d4e…` and B's version `9c8d7e6f5a4b…` both edit `## Rollout`
- **THEN** the file holds `<<<<<<< bilbo 3f9a2c1b0d4e <A's time>`, A's `## Rollout` passage, `======= bilbo 9c8d7e6f5a4b <B's time>`, B's passage and `>>>>>>> bilbo`, where the passage was

#### Scenario: A third side joins the block
- **WHEN** a note already holds a conflict block for `## Rollout` and a third device's edit of `## Rollout` merges in
- **THEN** the block gains a third side and the file holds one block for `## Rollout`, not two nested ones

### Requirement: Conflicts are recorded
A merge with at least one conflict SHALL be recorded as a `merged` version that names, for each conflicting passage, its heading path and the versions of its sides. bilbo SHALL tell an open conflict from this record, never from the markers alone. A merge without conflict SHALL be recorded as a `merged` version too, so an automatic merge is always labelled.

#### Scenario: The history shows the conflict
- **WHEN** a merge left one passage in conflict
- **THEN** `bilbo history <note>` lists a `merged` version flagged `conflict`

#### Scenario: Pasted markers are not a conflict
- **WHEN** an agent pastes a full `<<<<<<< bilbo <version> <time>` block, outside a fence, into a note that has no recorded conflict
- **THEN** `bilbo sync` lists no conflict for it, and `bilbo check` reports the markers as stray

#### Scenario: Markers inside a fence
- **WHEN** a note shows bilbo's markers as an example inside a fenced code block
- **THEN** they are neither a conflict nor stray markers

### Requirement: Resolving a conflict
An agent SHALL resolve a conflict by editing the file so that it holds none of that conflict's blocks. When the first version recorded without the blocks lacks a non-blank line that one of the conflict's sides held, that line is dropped text. The version SHALL record the dropped lines per passage. Dropped text SHALL stay reported, as the `store-check` and `sync-status` specs say, until it is declared with `bilbo sync declare`.

#### Scenario: Keeping both sides
- **WHEN** an agent rewrites a conflict block into one passage that holds every line of both sides
- **THEN** the conflict is resolved and no dropped text is reported

#### Scenario: Dropping a side
- **WHEN** an agent replaces a conflict block with A's passage only, and B's side held two lines A's did not
- **THEN** the conflict is resolved, and those two lines are reported as dropped until declared

#### Scenario: Markers left in place
- **WHEN** an agent edits another passage and leaves the block
- **THEN** the conflict stays open

### Requirement: The file name
The merged file name SHALL be the base's when neither side renamed the note, the renaming side's when one did, and the name of the side whose version id sorts first when both did. When that name is held by another note on this device, the `note-sync` spec's Topic collision rule SHALL apply.

#### Scenario: Renamed against edited text
- **WHEN** A renames `plan-release.md` to `decision-release.md` and B edits its text
- **THEN** on both devices the merged note is `decision-release.md` with B's edit, with the same version id

#### Scenario: Neither renamed
- **WHEN** A and B both edit `plan-release.md`
- **THEN** the merged note is `plan-release.md`

### Requirement: Merges are deterministic
A merge SHALL depend only on its base and sides, never on which device runs it or when, so two devices that merge the same versions write the same bytes and record the same version id. Two heads holding the same file name and bytes SHALL NOT be merged: the next version follows both. Two merge versions with the same parents SHALL NOT be merged with each other: the one whose id sorts first is taken.

#### Scenario: Two devices merge alike
- **WHEN** devices A and B each receive the other's concurrent edit of a note and merge them
- **THEN** both files hold the same bytes and both histories list one `merged` version with the same id

#### Scenario: Same content, no merge
- **WHEN** A and B make the same edit to a note while apart
- **THEN** after both sync, neither history holds a `merged` version for it

### Requirement: Merging without a base
When no common version of the two sides is held, because it was pruned or never synced, the merge SHALL use an empty base: a passage the two sides hold differently SHALL be a conflict and a passage only one side holds SHALL be kept. When several lowest common versions exist, the base SHALL hold only the passages and keys all of them hold identically.

#### Scenario: A base pruned away
- **WHEN** a device that was stale returns with an edit whose base this device pruned
- **THEN** the merged note keeps every passage of both sides, and the passages that differ are conflicts

#### Scenario: Identical passages without a base
- **WHEN** the two sides share no common version and hold `## Setup` with the same text
- **THEN** the merged note holds `## Setup` once, not as a conflict

#### Scenario: Two bases that disagree
- **WHEN** the two lowest common versions differ in whether they hold `## Notes`, one side holds it and the other does not
- **THEN** the merged note keeps `## Notes`
