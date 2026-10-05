---
name: note
description: Keeps what a later session should know as a bilbo note, such as a decision, gotcha, plan or finding. Use for "note this", "save this as a decision/gotcha/plan", "keep this for later sessions", or when this session settled something durable. NOT for a passing "note that..." in conversation, or for searching notes (recall).
license: Apache-2.0
allowed-tools: Bash(command -v bilbo), Bash(bilbo recall *), Bash(bilbo new *), Bash(bilbo scope), Bash(bilbo scope set *), Bash(bilbo check), Bash(mv -n *), Read, Edit
---

# note

Writes what a later session should know into the bilbo store, through `bilbo new`, or updates the note that already holds the subject. It writes no file but the note's own, and sets a note's scope only through `bilbo scope set`.

## Steps

1. Check that bilbo is installed:

   ```bash
   command -v bilbo
   ```

   When it prints nothing, stop with exactly this line and do nothing else:

   ```
   note: bilbo is not on PATH; install the bilbo CLI first
   ```

2. Decide whether to write, and which kind. Write a note when the user asks to keep something for later sessions, or when the session settled something a later one would otherwise work out again. A passing "note that..." inside another request is not a request for a note. The kinds are `plan`, `spec`, `design`, `decision`, `gotcha`, `research`, `review`, `report` and `reference`; a note that fits none is a `report`. Use the kind the user named, or infer it: a decision taken is a `decision`, a trap found is a `gotcha`, steps for later work are a `plan`. When two kinds fit equally, ask the user once.

3. Find the note on the subject. The same subject is the same question, decision or component, under any kind or wording; a related but different subject gets its own note.

   ```bash
   bilbo recall --limit 5 -- '<words that name the subject>'
   ```

   - Put the words after `--` in single quotes, so a word starting with `-` stays a word. Write a `'` inside them as `'\''`.
   - Read each hit whose file name or heading path may be on the subject.
   - Run it as one command, with nothing chained.

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty or warning lines | read the plausible hits. Update the one on the subject (step 6), or create (step 4) |
   | 1 | last line `bilbo: no notes match` | run at most one more query in the words the note would likely use, then create (step 4) |
   | 1 | `bilbo: no store at <root>` | no notes yet: create (step 4); `bilbo new` makes the store |
   | 1 | anything else | print bilbo's first stderr line and stop |
   | 2 | anything | print bilbo's first stderr line and stop |

4. Find the scopes:

   ```bash
   bilbo scope
   ```

   It prints one line per declared scope, with the name first, then a last `(unassigned)` line, and exits 0. When stdout is only the `(unassigned)` line (stderr says no scopes are declared), skip every scope question and pass no `--scope`. Otherwise note the names. When the user named a scope for this note, pass it as `--scope <name>` in step 5; never pass one the user did not name, since `bilbo new` resolves one from the working directory. Run `bilbo scope` once per run, and ask the scope question (step 5) once per run. On exit 2, print bilbo's first stderr line and stop.

5. Then create the note:

   ```bash
   bilbo new <kind> <topic> [--title '<title>'] [--scope <name>]
   ```

   The topic is lowercase kebab-case ASCII, two to five words that name the subject, never a date or a task (`release-tags`, not `2026-10-03-task`). Add `--title` when the default (hyphens as spaces, first letter uppercase) reads badly, for example with names or acronyms.

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty | stdout is the new file's absolute path: Read it; when its `scope:` line names a scope that looks wrong for the subject, ask the scope question (below); then write the body (step 7) |
   | 0 | `bilbo: no scope for <path>; scopes: ...` | the note is unassigned: ask the scope question (below), then write the body (step 7) |
   | 1 | `bilbo: topic '<topic>' already has a note: <path>` | that note is the one on the subject: update it (step 6) |
   | 1 | anything else | print bilbo's first stderr line and stop |
   | 2 | names the kind, the topic or `--title` | fix that argument and run `bilbo new` once more. A second exit 2: print the first stderr line and stop |
   | 2 | names a scope the config does not declare | ask the scope question with the names `bilbo scope` listed, never picking one silently, then run `bilbo new` once more with the answer, or without `--scope` when there is none. A second exit 2: print the first stderr line and stop |
   | 2 | anything else, such as `BILBO_HOME` | print bilbo's first stderr line and stop |

   The scope question: ask the user once per run, naming the declared scopes, which one the note belongs to. Ask it when the note is unassigned, when the scope `bilbo new` resolved looks wrong for the subject (for example `personal` for a note about the user's employer), or when the user named an undeclared scope. When `bilbo check` printed `holds marks of <names>`, name those scopes as the likely answer. Apply an answer before writing the body:

   ```bash
   bilbo scope set <name> '<path>'
   ```

   Use `--force` (`bilbo scope set --force <name> '<path>'`) only to replace a scope the note already has, and only after the user's answer. Without an answer, or in a run that cannot ask the user (such as `codex exec`), set nothing, write the note, and report it `unassigned`.

   | Exit | output | What to do |
   |---|---|---|
   | 0 | stdout `notes/<file>: set <name>`, `replaced <old> with <name>` or `kept <name>` | done |
   | 0 | stdout `notes/<file>: kept <old>; --force replaces it` | the user already answered: run it once more with `--force` |
   | 1 | stderr `bilbo: notes/<file>: changed while bilbo scope set ran; run it again` | run it once more. A second exit 1: report the line and leave the scope |
   | 1 or 2 | anything else, such as `cannot set scopes on this filesystem` | report bilbo's first stderr line, leave the scope, report the note `unassigned` |

6. Update the note. Read it, then Edit its body in place: add what is new, correct what is wrong, and remove what no longer holds. Keep `id`, `created` and `scope` as found; change a scope only through the scope question in step 5. When the user named a scope for this note, that is the answer: run `bilbo scope set --force <name> '<path>'` (without `--force` when the note has no `scope`). When the note now records another kind, rename it first, with the new kind in the file name and nothing else changed:

   ```bash
   mv -n '<root>/notes/<old kind>-<topic>.md' '<root>/notes/<new kind>-<topic>.md'
   ```

   `mv -n` never overwrites. Do not trust its exit code: step 8 runs `bilbo check`, which names both files when the move did not happen. When the file of the new kind already exists beside the old one, there is nothing to rename: update the note of the new kind, leave the other file's bytes alone, never delete or rename either, and report the shared-topic lines from `bilbo check` so the user can decide.

7. Write the body. Read the file, then Edit below the `# ` title line. Keep the prose short: what was found or decided and why, then pointers (paths, commands, URLs) that show it. Use `##` sections and never a second `# ` line outside a code fence. The frontmatter keys are only `id`, `created`, `scope` and `sources`, so never add `kind`, `supersedes` or another key, and never edit `id`, `created` or `scope`.

   Sources go after the last key, before the closing `---`:

   ```
   sources:
     - "code: src/new.rs"
     - "url: https://example.org/page"
   ```

   - The types are `url`, `code`, `doc` and `search`. The value is the URL, the path (with `:line` when one line matters), the document's name, or the query.
   - Only what you read or ran in this session, and the note rests on, counts as a source. Never one recalled from training or from another note. With none, omit the key; never write `sources: []`.

8. Check the store:

   ```bash
   bilbo check
   ```

   | Exit | output | What to do |
   |---|---|---|
   | 0 | empty, or only `(warning)` lines | report (step 9); report the note's warning lines unfixed and count the others |
   | 1 | stdout lines | fix every line that starts with `notes/<the note's file name>: `, except a `topic:` or `id:` line naming another file, a `scope: '<name>' is not declared` line and a `(warning)` line, which go in the report unfixed. A `scope: missing` line is the scope question of step 5, asked once: apply the answer with `bilbo scope set`; with no answer it goes in the report unfixed and the note is `unassigned`. Run `bilbo check` again only after a fix, at most three runs in all. Leave lines for other notes alone and count them |
   | 1 | stderr `bilbo: no store at <root>` | print it and stop |
   | 2 | anything | print bilbo's first stderr line and stop |

   If lines still name the note after the third run, report them.

9. Report. Say `created`, `updated` or `renamed`, give the absolute path (both paths for a rename), name the note's scope or say `unassigned` when it has none or one this device does not declare, and say how many `bilbo check` lines were left for other notes. When you stopped before writing, say what stopped you and claim no note.

## Never

- Write any file but the note's own.
- Edit a `scope:` line by hand: `bilbo scope set` is the only way to change it.
- Create a note without `bilbo new`.
- Create a second note on a subject that has one.
- Set an environment variable in a command: a `BILBO_HOME=... bilbo ...` prefix falls outside `allowed-tools`.
- Chain commands with `;`, `&&` or a pipe.
- Invent a source.
