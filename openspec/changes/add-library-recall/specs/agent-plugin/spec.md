# Spec Delta

## MODIFIED Requirements

### Requirement: The recall skill
The plugin SHALL hold a skill named `recall` in `skills/recall/SKILL.md`, whose frontmatter has only `name`, `description`, `license` and `allowed-tools`. The skill SHALL search only through `bilbo recall`, with the user's words after `--`, and SHALL act on the exit code: on 0 it shows the hits, says which query produced them, and passes on any `bilbo:` warning lines from stderr in one sentence; on 1 whose last stderr line is `bilbo: no notes match` or `bilbo: no sources match` it retries at most twice in the likely wording of the note or source, then says nothing matched; on any other 1, or on 2, it shows bilbo's first stderr line and stops. The skill SHALL search the notes unless the user asks what the library's sources say, asks to look a term up in the library, or names a corpus; then it SHALL run `bilbo recall --library`, adding `--corpus <corpus>` for each corpus the user named. It SHALL show library hits as it shows note hits, with each block's kind, reference and section lines, and SHALL NOT answer the user's question from their snippets: it SHALL hand the question to the `reference` skill with each hit as the pick `<corpus>/<name>#<heading path>`, a hit whose heading path is `-` as `<corpus>/<name>`, and a guide hit as the pick of the source its entry names. The description SHALL say that the skill also finds passages in the library, and that answering from sources belongs to the `reference` skill.

#### Scenario: Hits are shown
- **WHEN** a user asks to recall `flat layout` and `bilbo recall` exits 0
- **THEN** the agent says the hits came from `flat layout`, shows each hit's path, line, kind, created, heading path and snippet, and offers to open one

#### Scenario: Hits with a warning
- **WHEN** `bilbo recall` exits 0 and stderr is `bilbo: embedder unavailable (embedder http://bagend:8081 unreachable: Connection refused (os error 61)); keyword results only`
- **THEN** the agent shows the hits and says the results came from keywords alone because the embedder was unavailable

#### Scenario: Nothing matches
- **WHEN** `bilbo recall` exits 1 with `bilbo: no notes match` for the user's words and for two reworded queries
- **THEN** the agent says nothing matched and names the queries it tried

#### Scenario: Nothing matches after a warning
- **WHEN** `bilbo recall` exits 1 and stderr is `bilbo: 3 passages not indexed; run bilbo index` followed by `bilbo: no notes match`
- **THEN** the agent retries in the note's likely wording, as when nothing matches

#### Scenario: A usage error
- **WHEN** `bilbo recall` exits 2
- **THEN** the agent shows bilbo's first stderr line and runs no other query

#### Scenario: A library lookup
- **WHEN** a user asks "what does the Go library say about goroutine leaks"
- **THEN** the agent runs `bilbo recall --library --corpus go -- 'goroutine leaks'`

#### Scenario: Library hits go to the reference skill
- **WHEN** `bilbo recall --library` exits 0 with a block for `go/effective-go` whose heading path is `Concurrency > Goroutines` and whose section is `340-380`
- **THEN** the agent shows the block and runs the `reference` skill with the pick `go/effective-go#Concurrency > Goroutines`, and does not answer the question from the snippet

#### Scenario: Nothing in the library
- **WHEN** `bilbo recall --library` exits 1 with `bilbo: no sources match` for the user's words and for two reworded queries
- **THEN** the agent says nothing in the library matched and names the queries it tried

#### Scenario: No library
- **WHEN** `bilbo recall --library` exits 1 with `bilbo: no library at <root>`
- **THEN** the agent shows that line, runs no other query, and does not search the notes instead

#### Scenario: A notes question stays on the notes
- **WHEN** a user asks "did we decide on release tags"
- **THEN** the agent runs `bilbo recall` without `--library` or `--corpus`

#### Scenario: A guide hit hands on its source
- **WHEN** `bilbo recall --library` exits 0 with a `guide` block for `go` whose heading path is `errors`
- **THEN** the agent runs the `reference` skill with the pick `go/errors`

### Requirement: The reference skill
The plugin SHALL hold a skill named `reference` in `skills/reference/SKILL.md`, with the brief its readers get in `skills/reference/references/reader.md`. The frontmatter SHALL have only `name`, `description`, `license` and `allowed-tools`, and `allowed-tools` SHALL cover every command the skill runs, `Bash(bilbo recall *)` among them for its lookups. The description SHALL say to use it for a question answered from library sources, and not for notes or for adding a source.

#### Scenario: A question about a source
- **WHEN** a user asks "what does Effective Go say about goroutine leaks?"
- **THEN** the agent runs the `reference` skill

#### Scenario: A question about the notes
- **WHEN** a user asks "what did we decide about the release tags?"
- **THEN** the agent runs `recall`, not `reference`

#### Scenario: A lookup needs no approval
- **WHEN** the skill runs `bilbo recall --library --corpus rust -- 'needless_return'`
- **THEN** the command falls inside the skill's `allowed-tools`, and the tool asks for no approval

### Requirement: Reference picks
The reference skill SHALL find corpora only through `bilbo library` (or the corpus the user named), read each chosen guide through `bilbo library <corpus>`, and pick the sources whose entries answer the question, never a catalog whole. A catalog, a bare name such as a lint, a flag or an API, or a question that no guide entry covers SHALL go through `bilbo recall --library`, with `--corpus <corpus>` for each chosen corpus and the term after `--`. Each hit the skill keeps SHALL become the pick `<corpus>/<name>#<heading path>`, from the block's reference and second line; a hit whose heading path is `-` becomes `<corpus>/<name>`, and a guide hit the pick of the source its entry names. A pick the `recall` skill hands over SHALL be taken the same way, without searching again. A lookup that finds nothing SHALL be named in the answer with the queries it ran. Before it plans or reads, it SHALL post the picks as a message of its own: per corpus, `<k> of <n>` sources with `<n>` from `bilbo library`, and one clause per pick.

#### Scenario: Picks come first
- **WHEN** the skill picks `go/effective-go` and `go/errors` from a corpus of 14 sources
- **THEN** a message reading `picks from go (2 of 14 sources):` with one line per pick precedes any `bilbo library plan` call

#### Scenario: A catalog by section
- **WHEN** the question names the lint `needless_return`, `rust/clippy-lints` is a catalog, and `bilbo recall --library --corpus rust -- 'needless_return'` prints a block for `rust/clippy-lints` whose heading path is `needless_return`
- **THEN** the skill picks `rust/clippy-lints#needless_return`, and never the whole catalog

#### Scenario: A bare name across the library
- **WHEN** the question asks what `GOFLAGS` does and no guide entry mentions it
- **THEN** the skill runs `bilbo recall --library -- 'GOFLAGS'` and turns each hit it keeps into a `<corpus>/<name>#<heading path>` pick

#### Scenario: A pick handed over by the recall skill
- **WHEN** the `recall` skill hands over the pick `go/effective-go#Concurrency > Goroutines`
- **THEN** the picks message lists `go/effective-go#Concurrency > Goroutines`, and the skill runs no `bilbo recall` for it

#### Scenario: A lookup finds nothing
- **WHEN** `bilbo recall --library --corpus rust -- 'needless_retrun'` exits 1 with `bilbo: no sources match`
- **THEN** the skill picks nothing for that term, plans nothing for it, and its answer names the query `needless_retrun` as finding nothing

#### Scenario: No corpus fits
- **WHEN** no corpus listed by `bilbo library` covers the question
- **THEN** the skill says so, names the corpora, plans and reads nothing, and does not answer from memory
