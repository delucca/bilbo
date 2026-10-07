## MODIFIED Requirements

### Requirement: Stage a file
`bilbo library stage <file> --origin "<url|doc>: <value>" [--fetched <YYYY-MM-DD>] [--html]` SHALL read the file as UTF-8 text, drop a leading byte order mark, turn every CRLF and every lone CR into LF, add a final newline when it lacks one, and write the result as `capture.md` in a new folder `<state>/bilbo/staging/<stage>/`, where `<state>` is the state folder `bilbo --help` names and `<stage>` a fresh ULID. With `--html`, `capture.md` SHALL instead hold the file's text converted by the `library-fetch` HTML conversion rules, then normalized the same way, and the folder SHALL also hold the file's bytes as `raw`. `--fetched` SHALL default to today's local date. Staging SHALL change nothing under the store root.

#### Scenario: A file is staged
- **WHEN** an agent runs `bilbo library stage /tmp/spec.txt --origin "url: https://go.dev/ref/spec"` on a file with CRLF line endings
- **THEN** the exit code is 0, `<state>/bilbo/staging/<stage>/capture.md` holds the file's text with LF line endings, and no entry under the store root is created or changed

#### Scenario: The origin and date are kept for landing
- **WHEN** an agent stages a file with `--origin "doc: The Go Programming Language, chapter 8" --fetched 2026-08-23` and then lands it
- **THEN** the source has `origin: "doc: The Go Programming Language, chapter 8"` and `fetched: 2026-08-23`

#### Scenario: A saved page is converted
- **WHEN** an agent runs `bilbo library stage /tmp/page.html --html --origin "url: https://platform.example.com/docs/agents"` on a file holding `<h1>Agents</h1><p>Text.</p>`
- **THEN** `capture.md` is `# Agents`, a blank line and `Text.`, `raw` holds the file's bytes, and a source landed from it has `capture: external`

#### Scenario: HTML without the flag stays as written
- **WHEN** an agent stages the same file without `--html`
- **THEN** `capture.md` holds the HTML text as it was, and the folder holds no `raw`
