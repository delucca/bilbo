# Spec Delta

## ADDED Requirements

### Requirement: Sync state in the digest
In a note's line, a note with an open conflict or undeclared dropped text SHALL show `(<kind>, <created>, conflict)`, and a note whose latest version is a `merged` version without conflict `(<kind>, <created>, auto-merged)`. A session's first digest SHALL end with `Sync conflicts wait in: <absolute path>, ... (run bilbo check)` when notes have an open conflict or undeclared dropped text, naming at most 3 and then `and <n> more`. When no note passes the gate, that line SHALL follow `<!-- bilbo digest: 0 of 0 notes -->` alone.

#### Scenario: A conflicted note is labelled
- **WHEN** `gotcha-nix.md` has an open conflict and passes the gate
- **THEN** its line holds `(gotcha, <created>, conflict)`

#### Scenario: An auto-merged note is labelled
- **WHEN** the latest version of `plan-release.md` is a `merged` version without conflict and the note passes the gate
- **THEN** its line holds `(plan, <created>, auto-merged)`

#### Scenario: Conflicts raised with nothing else to show
- **WHEN** a session's first prompt passes no note, and `gotcha-nix.md` has an open conflict
- **THEN** stdout is `<!-- bilbo digest: 0 of 0 notes -->` and `Sync conflicts wait in: <root>/notes/gotcha-nix.md (run bilbo check)`

#### Scenario: Raised once per session
- **WHEN** a session already had its first digest and `gotcha-nix.md` still has an open conflict
- **THEN** a later digest in that session has no `Sync conflicts` line

#### Scenario: Without sync
- **WHEN** no note has ever been merged
- **THEN** no line carries a `conflict` or `auto-merged` label and no block has a `Sync conflicts` line

## MODIFIED Requirements

### Requirement: How many notes
The notes that pass the gate SHALL be ordered as `recall` orders hits. Notes already shown earlier in the same session SHALL be left out. A session's first digest SHALL show at most 6 notes, a later one at most 3, and the whole block SHALL stay within 9,000 bytes, dropping notes from the end to fit. When no note is left to show, stdout SHALL be empty, unless the Sync state in the digest requirement raises open conflicts.

#### Scenario: A note is shown once per session
- **WHEN** `gotcha-slots.md` was shown in session `abc` and passes the gate again in session `abc`
- **THEN** the new digest does not list `gotcha-slots.md`

#### Scenario: A later prompt shows fewer notes
- **WHEN** session `abc` already had a digest and 5 notes pass the gate, none shown before
- **THEN** the digest lists 3 notes

#### Scenario: A new session starts fresh
- **WHEN** `gotcha-slots.md` was shown in session `abc` and passes the gate in session `def`
- **THEN** the digest for `def` lists `gotcha-slots.md`

#### Scenario: A block too large for 6 notes
- **WHEN** the first prompt of a session passes 6 notes whose lines take about 2,000 bytes each
- **THEN** the block lists 4 notes, starts with `<!-- bilbo digest: 4 of 6 notes -->` and is no larger than 9,000 bytes

#### Scenario: Nothing passes
- **WHEN** no note passes the gate and no note has an open conflict
- **THEN** stdout is empty and the exit code is 0

### Requirement: The digest block
The block SHALL be, in order: a line `<!-- bilbo digest: <shown> of <left> notes -->`, where `<left>` counts the notes that passed the gate and were not shown before in the session; a line `Notes that may bear on this prompt (open the file to read more):`; one line per note, `- <absolute path>:<line> (<kind>, <created>) <heading path>: <snippet>`, with `recall`'s line, created, heading path and snippet, and the parenthesis extended with a label as Sync state in the digest says; when `<left>` exceeds `<shown>`, a line `(<left minus shown> more passed; run bilbo recall for them)`; and the `Sync conflicts wait in` line when that requirement adds it.

#### Scenario: A first digest
- **WHEN** the first prompt of a session passes 8 notes through the gate
- **THEN** the block starts with `<!-- bilbo digest: 6 of 8 notes -->`, lists 6 notes and ends with `(2 more passed; run bilbo recall for them)`

#### Scenario: No overflow line when everything fits
- **WHEN** 2 notes pass the gate in a session's first prompt
- **THEN** the block lists both and has no `more passed` line

### Requirement: Session memory
After choosing a block, `bilbo digest` SHALL record the notes it lists, and whether it raised sync conflicts, in a file named after the session under `<cache folder>/bilbo/sessions/`, where the cache folder is the one `bilbo index` uses, before printing the block. A session's first digest is one with no such file. Each run with the digest on SHALL delete files in that folder not modified for 30 days.

#### Scenario: The memory file
- **WHEN** session `abc` gets a digest listing two notes
- **THEN** `<cache folder>/bilbo/sessions/abc` exists and names both notes

#### Scenario: Nothing shown, nothing remembered
- **WHEN** no note passes the gate in session `abc` and no note has an open conflict
- **THEN** `<cache folder>/bilbo/sessions/abc` does not exist, and the next digest in `abc` is still its first

#### Scenario: Raised conflicts are remembered
- **WHEN** no note passes the gate in session `abc`, and its first digest prints only the `Sync conflicts` line
- **THEN** `<cache folder>/bilbo/sessions/abc` exists, and the next digest in `abc` is not its first

#### Scenario: Old sessions are cleaned up
- **WHEN** a session file was last modified 31 days ago and any digest runs
- **THEN** that file no longer exists
