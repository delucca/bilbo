## MODIFIED Requirements

### Requirement: Human view of note hits
When stdout gets the `cli` spec's human view, `bilbo recall` SHALL print per hit, best first, a blank line between hits: a title line of the rank, right-aligned in at least two columns, then two spaces, the note's title in bold and each heading below it after `›`; up to three lines of the passage, two when the width is under 60, with Markdown markup removed, starting at the sentence of the first query word when the lines would not reach it, and the query words in bold; and a dim line of the kind, the scope when the config declares two or more scopes, the age, `by meaning` when no query word is in the passage's heading path or text, and the note's path from `~/` with its line. Each control character other than a tab in a title, a heading, a passage or a path SHALL be shown as an escape (`\u{1b}`), never sent to the terminal. The passage and dim lines SHALL be indented to the title. A blank line and a dim count line SHALL close the list: `<n> notes, best first` (`1 note` for one), or, when `--limit` left hits out, `<shown> of <total> notes, best first · --limit <total> shows all`. When nothing matches, stderr SHALL say `no notes match '<query>'` after `○`, and a hint line SHALL follow.

#### Scenario: Two hits on a terminal
- **WHEN** two notes hold `rollback`, created 5 days ago and 17 minutes ago, and a user runs `bilbo recall rollback` in a terminal
- **THEN** the first line starts with ` 1` and holds the best note's title, each hit's last line holds its kind, an age such as `5 days ago` or `17 min ago` and a path starting with `~/`, and the last stdout line is `2 notes, best first`

#### Scenario: A limit that cut the list
- **WHEN** five notes hold `rollback` and a user runs `bilbo recall --limit 2 rollback` in a terminal
- **THEN** the last stdout line is `2 of 5 notes, best first · --limit 5 shows all`

#### Scenario: A match in a heading only
- **WHEN** a note's heading `Rollback plan` sits over a passage without the word, an embedder is configured, and a user runs `bilbo recall rollback` in a terminal
- **THEN** that hit's dim line does not hold `by meaning`

#### Scenario: A long passage starts near the match
- **WHEN** a note's passage holds 900 characters before its first `rollback` and a user runs `bilbo recall rollback` in a terminal 100 columns wide
- **THEN** the hit's first passage line starts with `… ` and the passage lines hold `rollback`

#### Scenario: Nothing matches on a terminal
- **WHEN** no note holds `wumpus` and a user runs `bilbo recall wumpus` in a terminal
- **THEN** stdout is empty, stderr's first line is `○`, two spaces and `no notes match 'wumpus'`, a hint line follows indented three columns, and the exit code is 1

#### Scenario: A pipe keeps the blocks
- **WHEN** a user runs `bilbo recall rollback | cat` in a terminal
- **THEN** stdout is the three-line blocks of Recall prints the best passages, with no rank, count line or escape byte

#### Scenario: Control characters in a note
- **WHEN** a note's title holds the byte 0x07, a heading holds `ESC [ 2 J`, its passage holds `ESC [ 31 m` and a user runs `bilbo recall` for it in a terminal
- **THEN** each shows as `\u{7}` or `\u{1b}` followed by the rest of the text, and stdout holds no 0x07, no 0x1b byte the note wrote and no C1 control character

## ADDED Requirements

### Requirement: Waiting for the embedder
While `bilbo recall` waits for the embedder to embed the query, it SHALL show a spinner and `Waiting for the embedder` on stderr once the wait has lasted half a second, only when stderr gets the `cli` spec's human view and `TERM` is set and not `dumb`. When the wait ends, by an answer, an error or the timeout, the spinner SHALL be erased, leaving nothing of it on the screen, before any other line is printed.

#### Scenario: A slow embedder on a terminal
- **WHEN** the embedder takes 1.5 seconds to answer and a user runs `bilbo recall rollback` in a terminal
- **THEN** stderr showed `Waiting for the embedder` with a spinner, the spinner's line was erased before the hits were printed, and stdout holds the hits

#### Scenario: A quick embedder
- **WHEN** the embedder answers within 100 milliseconds and a user runs `bilbo recall rollback` in a terminal
- **THEN** stderr holds nothing

#### Scenario: An agent's terminal
- **WHEN** the embedder takes 1.5 seconds and `bilbo recall rollback` runs with stderr a terminal and `CLAUDE_CODE_CHILD_SESSION=1`
- **THEN** stderr holds nothing

#### Scenario: A pipe
- **WHEN** the embedder takes 1.5 seconds and an agent runs `bilbo recall rollback` with stderr piped
- **THEN** stderr holds nothing

#### Scenario: A dumb terminal
- **WHEN** the embedder takes 1.5 seconds and a user runs `bilbo recall rollback` in a terminal with `TERM=dumb`
- **THEN** stderr holds nothing

#### Scenario: The embedder times out
- **WHEN** the embedder never answers and a user runs `bilbo recall rollback` in a terminal
- **THEN** the spinner is erased after 5 seconds, the keyword hits print, and the last stderr line is `▲`, two spaces and the `embedder unavailable` warning

### Requirement: Code passages in the human view
In recall's human view, a passage whose first line is a code fence SHALL show, in place of its flattened text, the lines inside that fenced block as written, its blank lines left out: at most three, two when the width is under 60, each dim, its tabs as four spaces, its control characters escaped, and cut with `…` when wider than the line. When the block holds more lines, or text follows it in the passage, the last line shown SHALL end with ` …`. A block with no line inside SHALL show no passage lines. Any other passage SHALL show as the Human view of note hits says.

#### Scenario: A code block
- **WHEN** a hit's passage is a fenced block of `PRAGMA busy_timeout = 5000;`, `PRAGMA journal_mode = WAL;`, `PRAGMA synchronous = NORMAL;` and `PRAGMA foreign_keys = ON;`, and a user runs `bilbo recall busy_timeout` in a terminal 100 columns wide
- **THEN** the hit's passage lines are `PRAGMA busy_timeout = 5000;`, `PRAGMA journal_mode = WAL;` and `PRAGMA synchronous = NORMAL; …`, indented to the title, with no word in bold

#### Scenario: Indentation is kept
- **WHEN** a hit's passage is a fenced block whose second line starts with four spaces
- **THEN** the second passage line keeps the four spaces after the indent

#### Scenario: Prose that holds a block
- **WHEN** a hit's passage is `Run this first:` followed by a fenced block
- **THEN** its passage lines are the flattened text, as the Human view of note hits says

#### Scenario: An escape in a block
- **WHEN** a hit's passage is a fenced block holding the byte 0x1b
- **THEN** the passage line shows `\u{1b}` and stdout holds no escape that the note wrote

#### Scenario: A pipe keeps the snippet
- **WHEN** a user runs `bilbo recall busy_timeout | cat` for the block above
- **THEN** the hit's third line is the passage's text with its whitespace collapsed, as Recall prints the best passages says

### Requirement: Links in the human view
When `BILBO_HYPERLINKS` is `1` and recall's human view may use escapes, each hit's path on its dim line SHALL be an OSC 8 link to `file://<host><absolute path>`, the host name of the machine and each byte of the path outside letters, digits, `-`, `.`, `_`, `~` and `/` percent-encoded, opened with `ESC ] 8 ; ; <uri> ESC \` and closed with `ESC ] 8 ; ; ESC \`. A path broken over lines SHALL carry the link on each piece. Without it, or with any other value, no link SHALL be printed; the plain view SHALL never hold one.

#### Scenario: A linked path
- **WHEN** a user runs `bilbo recall rollback` in a terminal on the host `shire-box` with `BILBO_HYPERLINKS=1`, and the hit is `/home/a/.local/share/bilbo/notes/plan-roll back.md` at line 12
- **THEN** the dim line holds `ESC ] 8 ; ; file://shire-box/home/a/.local/share/bilbo/notes/plan-roll%20back.md ESC \`, then `~/.local/share/bilbo/notes/plan-roll back.md:12`, then `ESC ] 8 ; ; ESC \`

#### Scenario: Off by default
- **WHEN** a user runs `bilbo recall rollback` in a terminal without `BILBO_HYPERLINKS`
- **THEN** stdout holds no OSC 8 sequence

#### Scenario: Another value
- **WHEN** a user runs `bilbo recall rollback` in a terminal with `BILBO_HYPERLINKS=yes`
- **THEN** stdout holds no OSC 8 sequence

#### Scenario: No escapes, no links
- **WHEN** a user runs `bilbo recall rollback` in a terminal with `BILBO_HYPERLINKS=1` and `NO_COLOR=1`
- **THEN** stdout holds no escape byte

#### Scenario: A pipe never links
- **WHEN** an agent runs `bilbo recall rollback | cat` with `BILBO_HYPERLINKS=1`
- **THEN** stdout is the plain hit blocks, with no escape byte
