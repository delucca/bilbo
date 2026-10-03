---
name: note
description: Keeps what a later session should know as a bilbo note, such as a decision, gotcha, plan or finding. Use for "note this", "save this as a decision/gotcha/plan", "keep this for later sessions", or when this session settled something durable. NOT for a passing "note that..." in conversation, or for searching notes (recall).
license: Apache-2.0
allowed-tools: Bash(command -v bilbo), Bash(bilbo recall *), Bash(bilbo new *), Bash(bilbo check), Bash(mv -n *), Read, Edit
---

# note

Writes what a later session should know into the bilbo store, through `bilbo new`, or updates the note that already holds the subject. It writes no file but the note's own.

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
   | 0 | empty or warning lines | read the plausible hits. Update the one on the subject (step 5), or create (step 4) |
   | 1 | last line `bilbo: no notes match` | run at most one more query in the words the note would likely use, then create (step 4) |
   | 1 | `bilbo: no store at <root>` | no notes yet: create (step 4); `bilbo new` makes the store |
   | 1 | anything else | print bilbo's first stderr line and stop |
   | 2 | anything | print bilbo's first stderr line and stop |

4. Create the note:

   ```bash
   bilbo new <kind> <topic> [--title '<title>']
   ```

   The topic is lowercase kebab-case ASCII, two to five words that name the subject, never a date or a task (`release-tags`, not `2026-10-03-task`). Add `--title` when the default (hyphens as spaces, first letter uppercase) reads badly, for example with names or acronyms.

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty | stdout is the new file's absolute path: write the body (step 6) |
   | 1 | `bilbo: topic '<topic>' already has a note: <path>` | that note is the one on the subject: update it (step 5) |
   | 1 | anything else | print bilbo's first stderr line and stop |
   | 2 | names the kind, the topic or `--title` | fix that argument and run `bilbo new` once more. A second exit 2: print the first stderr line and stop |
   | 2 | anything else, such as `BILBO_HOME` | print bilbo's first stderr line and stop |

5. Update the note. Read it, then Edit its body in place: add what is new, correct what is wrong, and remove what no longer holds. Keep `id` and `created`. When the note now records another kind, rename it first, with the new kind in the file name and nothing else changed:

   ```bash
   mv -n '<root>/notes/<old kind>-<topic>.md' '<root>/notes/<new kind>-<topic>.md'
   ```

   `mv -n` never overwrites. Do not trust its exit code: step 7 runs `bilbo check`, which names both files when the move did not happen. When the file of the new kind already exists beside the old one, there is nothing to rename: update the note of the new kind, leave the other file's bytes alone, never delete or rename either, and report the shared-topic lines from `bilbo check` so the user can decide.

6. Write the body. Read the file, then Edit below the `# ` title line. Keep the prose short: what was found or decided and why, then pointers (paths, commands, URLs) that show it. Use `##` sections and never a second `# ` line outside a code fence. The frontmatter keys are only `id`, `created` and `sources`, so never add `kind`, `supersedes` or another key.

   Sources go between `created` and the closing `---`:

   ```
   sources:
     - "code: src/new.rs"
     - "url: https://example.org/page"
   ```

   - The types are `url`, `code`, `doc` and `search`. The value is the URL, the path (with `:line` when one line matters), the document's name, or the query.
   - Only what you read or ran in this session, and the note rests on, counts as a source. Never one recalled from training or from another note. With none, omit the key; never write `sources: []`.

7. Check the store:

   ```bash
   bilbo check
   ```

   | Exit | output | What to do |
   |---|---|---|
   | 0 | empty | report (step 8) |
   | 1 | stdout lines | fix every line that starts with `notes/<the note's file name>: `, except a `topic:` or `id:` line naming another file, which goes in the report unfixed. Then run `bilbo check` again, at most three runs in all. Leave lines for other notes alone and count them |
   | 1 | stderr `bilbo: no store at <root>` | print it and stop |
   | 2 | anything | print bilbo's first stderr line and stop |

   If lines still name the note after the third run, report them.

8. Report. Say `created`, `updated` or `renamed` and give the absolute path (both paths for a rename), and how many `bilbo check` lines were left for other notes. When you stopped before writing, say what stopped you and claim no note.

## Never

- Write any file but the note's own.
- Create a note without `bilbo new`.
- Create a second note on a subject that has one.
- Set an environment variable in a command: a `BILBO_HOME=... bilbo ...` prefix falls outside `allowed-tools`.
- Chain commands with `;`, `&&` or a pipe.
- Invent a source.
