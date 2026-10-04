# Spec Delta

## Purpose

`bilbo library plan` and `bilbo library read` let an agent read sources whole without one tool's read cap deciding where a source ends: bilbo cuts the picks into slices that fit one shell call, groups the slices into partitions that fit one reader, prints each slice with its line numbers, and logs every read so `bilbo cite --plan` can count coverage in code.

## ADDED Requirements

### Requirement: Plan picks
`bilbo library plan <ref>... [--budget-tokens <n>] [--slice-bytes <n>] [--slice-lines <n>]` SHALL take one or more source references, as the `library-browse` spec's Source references defines them. A reference without an anchor picks the source from its title line to its last line. A reference with an anchor picks the section it resolves to, as the `library-store` spec's Outline and Anchors define it. Picks keep the order given.

#### Scenario: A whole source and a section
- **WHEN** an agent runs `bilbo library plan go/effective-go 'go/errors#Wrapping'`
- **THEN** the first pick runs from the title line of `effective-go.md` to its last line, and the second from the `Wrapping` heading to the end of that section

#### Scenario: A pick by id
- **WHEN** `go/effective-go` has id `01M3EZ8NVEC2KJQNGK5DTK349R` and an agent runs `bilbo library plan 01M3EZ8NVEC2KJQNGK5DTK349R`
- **THEN** the plan picks the whole of `effective-go.md`

#### Scenario: A note's id is not a pick
- **WHEN** an agent runs `bilbo library plan` with the id of a note
- **THEN** the exit code is 1, stderr names the note's absolute path, and no plan is written

### Requirement: Plan refusals
`library plan` SHALL refuse, with exit 1 and no plan written, a catalog picked without an anchor, naming it and `bilbo library show`. Any reference error of `library show` SHALL have the same exit code here. Two picks of one source whose line ranges overlap, a missing reference, a `--budget-tokens` below 1,000, a `--slice-bytes` below 1,000 or above 30,000, and a `--slice-lines` below 10 SHALL be usage errors. When neither `XDG_STATE_HOME` nor `HOME` is absolute, `plan` SHALL exit 2 naming them.

#### Scenario: A catalog picked whole
- **WHEN** `rust/clippy-lints` is a catalog and an agent runs `bilbo library plan rust/clippy-lints`
- **THEN** stderr names `rust/clippy-lints` as a catalog and `bilbo library show`, the exit code is 1, and no plan is written

#### Scenario: A catalog section is accepted
- **WHEN** an agent runs `bilbo library plan 'rust/clippy-lints#needless_return'`
- **THEN** the exit code is 0 and the plan picks that section

#### Scenario: Overlapping picks
- **WHEN** an agent runs `bilbo library plan go/effective-go 'go/effective-go#Concurrency'`
- **THEN** bilbo prints a message naming both references to stderr, exits 2 and writes no plan

#### Scenario: An ambiguous anchor
- **WHEN** an agent runs `bilbo library plan 'rust/clippy-lints#What it does'`
- **THEN** stderr lists each matching heading path with its start line, the exit code is 1, and no plan is written

#### Scenario: A tiny slice size
- **WHEN** an agent runs `bilbo library plan go/errors --slice-bytes 500`
- **THEN** bilbo prints a message naming `--slice-bytes` to stderr and exits 2

#### Scenario: A slice size over one tool result
- **WHEN** an agent runs `bilbo library plan go/effective-go --slice-bytes 5000000`
- **THEN** bilbo prints a message naming `--slice-bytes` and the limit of 30,000 to stderr, exits 2 and writes no plan

### Requirement: Slices
`plan` SHALL cut each pick into slices, in order, never one slice across two picks. A slice's size is the bytes `library read` prints for it, its two header lines and end marker included. The cut points are the pick's section starts; a slice takes the runs between cut points while its size stays within `--slice-bytes` (default 24,000) and its lines within `--slice-lines` (no default).

#### Scenario: Sections packed into slices
- **WHEN** a pick holds three sections whose printed sizes are 10,000, 9,000 and 8,000 bytes, with the default slice size
- **THEN** the plan has two slices: the first two sections, then the third

#### Scenario: A line cap
- **WHEN** a pick of 600 short lines in one section prints in 9,000 bytes and the agent passes `--slice-lines 250`
- **THEN** no slice holds more than 250 lines

#### Scenario: A headingless source
- **WHEN** a pick has no section and prints in 20,000 bytes
- **THEN** the plan has one slice holding the whole pick

#### Scenario: Picks never share a slice
- **WHEN** an agent plans two sources of 2,000 bytes each
- **THEN** the plan has two slices, one per source

### Requirement: Oversized runs
A run between cut points that alone exceeds the slice limits SHALL close the slice before it and be cut on its own: into pieces that end at a blank line and stay within the limits, then, for a piece that still exceeds them, at line boundaries. A single line that exceeds `--slice-bytes` SHALL be a slice by itself.

#### Scenario: A long section with paragraphs
- **WHEN** one section of a pick prints in 60,000 bytes and holds blank lines every 2,000 bytes, with the default slice size
- **THEN** that section becomes three slices, each ending at a blank line or at the section's end, and each printing in at most 24,000 bytes

#### Scenario: A long section with no blank line
- **WHEN** one section prints in 50,000 bytes with no blank line in it
- **THEN** it is cut at line boundaries into slices of at most 24,000 printed bytes

#### Scenario: A section under the limit is not cut
- **WHEN** one section of a pick prints in 20,000 bytes and holds blank lines, with the default slice size
- **THEN** the section is not cut at its blank lines: it sits whole in one slice

#### Scenario: One huge line
- **WHEN** a pick holds one line of 30,000 bytes
- **THEN** that line is a slice by itself, and the plan still exits 0

### Requirement: Partitions
`plan` SHALL group the slices into partitions in order: a slice joins the current partition when the partition's tokens plus its own stay within `--budget-tokens` (default 60,000), and starts the next partition otherwise. A slice's tokens SHALL follow the `library-store` spec's Derived sizes over the bytes of its source lines. Every partition is a run of consecutive slices.

#### Scenario: Two partitions
- **WHEN** a plan has slices of 9,000 tokens each and the default budget
- **THEN** slices 1 to 6 form partition 1, and slice 7 starts partition 2

#### Scenario: A slice over the budget
- **WHEN** an agent passes `--budget-tokens 5000` and a slice holds 9,000 tokens
- **THEN** that slice forms a partition by itself and the plan exits 0

#### Scenario: A slice that fits opens no partition
- **WHEN** partition 1 holds 50,000 tokens and the next slice holds 9,000, with the default budget
- **THEN** that slice joins partition 1, and no new partition starts for it

### Requirement: Plan output
On success, `plan` SHALL print `plan: <plan>`, `picks: <n>`, `slices: <n>`, `tokens: <n>`, `partitions: <n>`, one `partition <k>: slices <a>-<b>, <tokens> tokens` line per partition, a blank line, then one row per slice: its number, its partition, `<corpus>/<name>`, `<start>-<end>` in the source's physical lines, `<tokens> tokens`, and the heading path of the deepest section holding its first line, or `-`, joined by tabs. It SHALL exit 0.

#### Scenario: A one-slice plan
- **WHEN** an agent plans `go/errors`, a 20-line source of 1,000 bytes titled on line 7
- **THEN** stdout holds `slices: 1`, `partition 1: slices 1-1, 400 tokens`, and the row `1`, `1`, `go/errors`, `7-20`, `400 tokens`, `-`, joined by tabs

#### Scenario: A slice that starts inside a section
- **WHEN** a slice starts at a blank-line cut inside `Concurrency > Goroutines`
- **THEN** its row ends with `Concurrency > Goroutines`

#### Scenario: A refused plan prints no plan
- **WHEN** `plan` exits 1 or 2
- **THEN** stdout is empty and no file is written under `<state>/bilbo/plans/`

### Requirement: The plan file
`plan` SHALL write the plan to `<state>/bilbo/plans/<plan>.json`, where `<plan>` is a fresh ULID, recording the store root, each pick with its source's id and `digest`, and each slice. It SHALL change nothing under the store root. It SHALL remove the plan files and read logs in that folder that are more than 30 days old.

#### Scenario: The store is untouched
- **WHEN** an agent runs `bilbo library plan go/effective-go`
- **THEN** `<state>/bilbo/plans/<plan>.json` exists, and every entry under the store root has the same bytes and modification time as before

#### Scenario: An old plan is removed
- **WHEN** `<state>/bilbo/plans/` holds a plan and its read log last changed 31 days ago, and an agent runs `bilbo library plan go/errors`
- **THEN** both files of the old plan are gone

#### Scenario: A recent plan is kept
- **WHEN** a plan was written yesterday
- **THEN** the next `library plan` leaves it in place

### Requirement: Read slices
`bilbo library read <plan> <slice>... [--part <k>/<n>]` SHALL print each named slice, in the order given: the line `-- slice <i>/<total>: <corpus>/<name> <id> lines <a>-<b> --`, the line `-- in: <heading path> --` (the deepest section holding line `<a>`, or `-`), then each line of the slice as `<line>\t<text>`, then `-- end slice <i>/<total> --`. Line numbers are the source's physical lines. It SHALL exit 0.

#### Scenario: One slice
- **WHEN** an agent runs `bilbo library read <plan> 3` and slice 3 of 9 holds lines 820 to 1104 of `go/effective-go`, which starts with `## Concurrency`
- **THEN** stdout starts with `-- slice 3/9: go/effective-go 01M3EZ8NVEC2KJQNGK5DTK349R lines 820-1104 --` and `-- in: Concurrency --`, then `820`, a tab and `## Concurrency`, and ends with `-- end slice 3/9 --`

#### Scenario: Lines as they are
- **WHEN** a slice's lines hold tabs, trailing spaces and a fenced block
- **THEN** the text after each line's tab equals the source line byte for byte

#### Scenario: Two slices in one call
- **WHEN** an agent runs `bilbo library read <plan> 4 5`, and slices 4 and 5 together print within the plan's slice size
- **THEN** stdout holds slice 4 with its end marker, then slice 5 with its end marker

#### Scenario: The frontmatter is never printed
- **WHEN** an agent reads slice 1 of a whole-source pick
- **THEN** its first numbered line is the title line, and no frontmatter line is printed

### Requirement: One read fits one tool result
A `read` that names more than one slice, whose output together would exceed the plan's slice size (its `--slice-bytes`, or 24,000), SHALL be a usage error that prints nothing to stdout and logs nothing, naming the slices and the limit. With `--part`, the bytes counted are those of the parts it would print. A read of one slice SHALL always print it, a slice that is one oversized line included.

#### Scenario: Too much in one call
- **WHEN** slices 1 to 9 of a plan print 20,000 bytes each and an agent runs `bilbo library read <plan> 1 2 3 4 5 6 7 8 9`
- **THEN** stderr names the slices and the limit of 24,000 bytes, stdout is empty, the exit code is 2, and `bilbo cite --plan <plan>` counts none of them as read

#### Scenario: Small slices together
- **WHEN** slices 4 and 5 print 6,000 bytes each
- **THEN** `bilbo library read <plan> 4 5` prints both and logs both

#### Scenario: One oversized slice
- **WHEN** slice 2 is one line of 30,000 bytes
- **THEN** `bilbo library read <plan> 2` prints it and exits 0

### Requirement: Read a slice in parts
With `--part <k>/<n>`, `read` SHALL print only the k-th of n runs of each named slice, cut at line boundaries into runs of nearly equal bytes, with `part <k>/<n>` after `<i>/<total>` in the header and end marker, and the run's own line range and `in:` line. `n` SHALL be 2 to 8 and `k` 1 to `n`, or the run is a usage error. A slice of fewer than `n` lines SHALL be a usage error.

#### Scenario: A slice in halves
- **WHEN** an agent runs `bilbo library read <plan> 3 --part 1/2`, then `--part 2/2`
- **THEN** the first prints `-- slice 3/9 part 1/2: ...` and the lines of the first half, ending `-- end slice 3/9 part 1/2 --`; the second prints the remaining lines; together they print every line of slice 3 once

#### Scenario: A bad part
- **WHEN** an agent runs `bilbo library read <plan> 3 --part 3/2`
- **THEN** bilbo prints a message naming `--part` to stderr, exits 2 and logs nothing

### Requirement: The read log
Each `read` that prints SHALL append to the plan's read log, `<state>/bilbo/plans/<plan>.log`, the source lines it printed. A line of a plan is read when a logged read printed it, and a slice is read when all its lines are. Runs of `read` on one plan at the same time SHALL each log their lines. `read` SHALL change nothing under the store root.

#### Scenario: A read is logged
- **WHEN** an agent reads slice 3 of a plan
- **THEN** `bilbo cite --plan <plan>` counts slice 3 as read

#### Scenario: Parts add up
- **WHEN** an agent reads slice 3 with `--part 1/2` only
- **THEN** slice 3 is not read, and the lines of its first half are

#### Scenario: Six readers at once
- **WHEN** six processes each read a different slice of one plan at the same moment
- **THEN** all six slices are read

### Requirement: Read refusals
`read` SHALL exit 1, print nothing to stdout and log nothing when the plan does not exist, when the plan was made for another store root, or when a named slice's source no longer exists or its `digest` differs from the one the plan recorded, naming the source and saying to make a new plan. A plan argument that is not a ULID, no slice, or a slice that is not a number in the plan SHALL be a usage error.

#### Scenario: A re-ingested source
- **WHEN** `go/effective-go` was landed again with `--replace` after the plan was made, and an agent reads one of its slices
- **THEN** stderr names `go/effective-go` and says to make a new plan, stdout is empty, the exit code is 1, and nothing is logged

#### Scenario: A moved source
- **WHEN** `go/effective-go.md` was moved to `go/effective-go-2009.md` with its bytes unchanged
- **THEN** reading its slices still exits 0, and the header names `go/effective-go-2009`

#### Scenario: A slice past the end
- **WHEN** a plan has 9 slices and an agent runs `bilbo library read <plan> 10`
- **THEN** bilbo exits 2 and stdout is empty

#### Scenario: Another store
- **WHEN** a plan was made with `BILBO_HOME=/a` and an agent reads it with `BILBO_HOME=/b`
- **THEN** stderr names `/a`, and the exit code is 1
