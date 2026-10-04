# Spec Delta

## MODIFIED Requirements

### Requirement: Frontmatter shape
A note SHALL open with a line holding exactly `---`, then its keys, then a second line holding exactly `---`. The keys SHALL be `id` and `created`, both required, and optionally `sources` and `scope`, in any order, each at most once. No other key is allowed.

#### Scenario: Minimal valid frontmatter
- **WHEN** a note's file starts with `---`, `id: 01M3YJ7R6HK6NQ30DCDB1P4DYB`, `created: 2026-10-02T14:23-03:00`, `---`
- **THEN** the frontmatter is valid

#### Scenario: A scope key is allowed
- **WHEN** a note's frontmatter holds `id`, `created` and `scope: work`
- **THEN** the frontmatter is valid

#### Scenario: A removed key is invalid
- **WHEN** a note's frontmatter also holds `kind: decision`, `supersedes: ...` or `project: bilbo`
- **THEN** the note is invalid, and the problem names that key

#### Scenario: Two scope lines are invalid
- **WHEN** a note's frontmatter holds `scope: work` and `scope: personal`
- **THEN** the note is invalid, and the problem names `scope`

#### Scenario: Missing frontmatter is invalid
- **WHEN** a note's first line is `# Title`
- **THEN** the note is invalid

## ADDED Requirements

### Requirement: Scope key
When `scope` is present, it SHALL be one line, `scope: <name>`, where the name has the topic's grammar: segments of `[a-z0-9]+` joined by single hyphens. The key never changes the note's path. Whether the name is declared is not a store rule: `bilbo check` reports that from the config, under the `store-check` spec's Scope problems.

#### Scenario: A valid scope
- **WHEN** a note has `scope: client-x`
- **THEN** `scope` is valid

#### Scenario: Other forms are invalid
- **WHEN** a note has `scope: Work`, `scope: a b`, `scope: [work, personal]`, `scope:` or `scope: work-`
- **THEN** the note is invalid, and the problem names `scope`

#### Scenario: The scope is not in the path
- **WHEN** a note has `scope: work`
- **THEN** its file is still `<root>/notes/<kind>-<topic>.md`
