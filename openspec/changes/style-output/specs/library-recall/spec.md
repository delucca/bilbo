## ADDED Requirements

### Requirement: Human view of library hits
When stdout gets the `cli` spec's human view, library recall SHALL print each hit as note-recall's Human view of note hits does, with these differences: the bold title is the source's title, or `<corpus> guide` for a guide, followed by the headings below it; the dim line holds `source` or `guide`, the reference and `lines <start>-<end>`; and the count line counts files, as `<n> files, best first` or `<shown> of <total> files, best first · --limit <total> shows all`.

#### Scenario: A source hit on a terminal
- **WHEN** the best passage for `bilbo recall --library goroutine` sits under `Concurrency > Goroutines` of `go/effective-go`, titled `Effective Go`, on lines 340-380, and a user runs it in a terminal
- **THEN** its first line holds `Effective Go`, `›`, `Concurrency`, `›` and `Goroutines`, and its last line is `source · go/effective-go · lines 340-380`

#### Scenario: Escaped underscores read as written
- **WHEN** a source's heading is `unnecessary\_clippy\_cfg` and it is a hit in a terminal
- **THEN** the title line shows `unnecessary_clippy_cfg`

#### Scenario: Nothing matches on a terminal
- **WHEN** no source holds `wumpus` and a user runs `bilbo recall --library wumpus` in a terminal
- **THEN** stderr's first line is `○`, two spaces and `no sources match 'wumpus'`, a hint line follows, and the exit code is 1

#### Scenario: A pipe keeps the blocks
- **WHEN** a user runs `bilbo recall --library goroutine | cat`
- **THEN** stdout is the blocks of Library hit blocks, byte for byte
