# Spec Delta

## MODIFIED Requirements

### Requirement: Replace a source
With `--replace`, `land` SHALL replace an existing source: it keeps the old `id` and writes every other key and the body anew from the stage. When the new digest differs from the old one, it SHALL first run the citation pre-check, then put `stale: re-ingested <YYYY-MM-DD>; re-read the source and revise this entry.`, with today's local date, directly under the entry's heading, replacing a stale line already there, or add the entry with its stub when there is none. When the digest is the same, the guide SHALL be left as it is. Without `--replace`, an existing source SHALL make `land` exit 1, naming the file and `--replace`; with `--replace`, a missing source SHALL make it exit 1.

#### Scenario: A re-ingest with new text
- **WHEN** `go/effective-go` exists with id `01M3EZ8NVEC2KJQNGK5DTK349R`, no note cites it, and an agent lands a new stage with `--replace` whose body differs
- **THEN** the source keeps that id, has the new `fetched`, `digest` and body, and the line under `## effective-go` is the stale line with today's date, followed by the old prose

#### Scenario: A re-ingest with the same text
- **WHEN** the new body is byte for byte the old one
- **THEN** the source has the new `fetched`, and `guide.md` has the same bytes as before

#### Scenario: A taken name
- **WHEN** `go/effective-go` exists and an agent runs `land` without `--replace`
- **THEN** stderr names `library/go/effective-go.md` and `--replace`, the exit code is 1, and the source and the stage folder are unchanged

#### Scenario: Nothing to replace
- **WHEN** `go/effective-go` does not exist and an agent runs `land` with `--replace`
- **THEN** the exit code is 1 and no file is written

## ADDED Requirements

### Requirement: Citation pre-check
Before `land --replace` writes a source whose digest changes, it SHALL check every citation of that source's id in the bodies of `<root>/notes/` against the old body and the new one, as the `citations` spec checks a draft without a plan. A citation degrades when its new verdict differs from its old one and is not `ok`. Citations whose verdict does not change, already failing ones included, do not block.

#### Scenario: A citation that keeps resolving
- **WHEN** a note cites `go/effective-go` with a quote that the new body still holds under the same anchor
- **THEN** `land --replace` lands the source and exits 0

#### Scenario: A citation that already failed
- **WHEN** a note's citation of `go/effective-go` is `quote_missing` against the old body and the new one
- **THEN** it does not block `land --replace`

#### Scenario: Citations in guides are not checked
- **WHEN** only `library/go/guide.md` cites the source
- **THEN** the pre-check finds no citation

### Requirement: Degraded citations block a replace
When any citation degrades and `--force` is not given, `land` SHALL exit 1 and write nothing, keeping the stage. Its stderr SHALL say how many citations would degrade and name `--force`, then give one line per citation: `notes/<file>:<line>: <old verdict> -> <new verdict>`. With `--force`, `land` SHALL replace the source and print the same per-citation lines to stderr. `--force` without `--replace` SHALL be a usage error.

#### Scenario: A quote the new text dropped
- **WHEN** `notes/gotcha-skill-frontmatter.md` cites `go/effective-go` on line 12 with an `ok` quote that the new body no longer holds, and an agent runs `land --replace` without `--force`
- **THEN** stderr holds a line naming 1 citation and `--force`, and the line `notes/gotcha-skill-frontmatter.md:12: ok -> quote_missing`; the exit code is 1; the source, the guide and the stage folder are unchanged, and no capture folder is written

#### Scenario: A heading renamed upstream
- **WHEN** a cited quote is still in the new body, under a heading whose text changed
- **THEN** its line reads `ok -> anchor_missing`, and `land` exits 1

#### Scenario: Replacing anyway
- **WHEN** the same agent runs the same `land --replace` with `--force`
- **THEN** the source is replaced, stderr holds `notes/gotcha-skill-frontmatter.md:12: ok -> quote_missing`, stdout holds the four lines of a successful land, and the exit code is 0

#### Scenario: Force alone
- **WHEN** an agent runs `land <stage> go/effective-go --keep 1-40 --force`
- **THEN** bilbo prints a message naming `--force` and `--replace` to stderr, exits 2 and writes no file
