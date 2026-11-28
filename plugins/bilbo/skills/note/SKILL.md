---
name: note
description: Keeps what a later session should know as a bilbo note, which is a Markdown file in the notes folder bilbo follows, such as a decision, gotcha, plan or finding. Use for "note this", "save this as a decision/gotcha/plan", "keep this for later sessions", or when this session settled something durable. NOT for a passing "note that..." in conversation, or for searching notes (recall).
license: Apache-2.0
allowed-tools: Bash(bilbo config follow), Bash(bilbo remember *), Bash(bilbo recall *), Bash(bilbo read *), Bash(test -e *), Bash(test -f *), Bash(test -L *), Bash(realpath *), Bash(ls *), Bash(mv -n *), Read, Write, Edit
---

# note

A note is a Markdown file in the folder bilbo follows. This skill writes, updates, renames and deletes those files, and keeps each change in bilbo at once. It uses neither `bilbo remember -` nor `bilbo remember --edit`.

It reports what bilbo printed and reads no cause into it. It never writes over a file it saw, and it acts on a file only after the six checks of "Before it touches a file" hold, in that order, stopping at the first that fails and saying which one failed.

**Every path, every ref and the subject of a search is one argument:** a name may hold a space, a `*` or a `$`, and a subject a `>`. Write each whole between single quotes, as the commands below show, a single quote inside it as `'\''`, and never without them.

## Steps

1. **Find the notes folder.**

   ```bash
   bilbo config follow
   ```

   It prints the follows as one line of TOML, for example `[{ path = "/home/me/notes", kind = "markdown" }, { path = "/home/me/.claude/projects", kind = "session" }]`. The notes folder is the one `path` whose `kind` is `markdown` and which is a folder. With several, ask which. With none, the machine was not set up: `bilbo setup` makes a notes folder and follows it. Without it, set one up with the user's folder:

   ```bash
   bilbo remember '<their folder>' --kind markdown --follow
   ```

   On an empty folder this exits 1 with `nothing to keep: markdown claims no file under <folder>` and still saves the follow. Whatever it printed, run `bilbo config follow` again and go on only when it lists that folder with `kind = "markdown"`. A folder followed as another kind, or a config bilbo could not write, fails here: report what both commands printed, and stop.

   Then resolve the notes folder's path once:

   ```bash
   realpath '<the notes folder>'
   ```

   `<folder>` below is the path that prints. bilbo's labels hold resolved paths, so build every path from this one.

   The timer `bilbo setup` installs runs `bilbo refresh`, which keeps what a person edits; this skill keeps what it writes itself, at once.

2. **Decide what to do.** Write a note when the user asks to keep something for later sessions, or when the session settled something a later one would otherwise work out again. A passing "note that..." inside another request is not a request for a note. Then find out whether a note on the subject exists (step 3). The kinds are `plan`, `spec`, `design`, `decision`, `gotcha`, `research`, `review`, `report` and `reference`; a note that fits none is a `report`.

3. **Find a note on the subject.**

   ```bash
   bilbo recall --in='<folder>' --kind markdown -- '<words of the subject>'
   ```

   `--kind markdown` keeps the search to Markdown files. `--in='<folder>'` narrows it by the words of a label, not by folders: a file of `/home/me/notes-old` can answer for `/home/me/notes`. So a hit is a note of this folder only when its file stands in `<folder>`, which you see for each hit.

   - A hit is its header and the lines the header counts: `=== <ref>`, a tab, then `1 line` or `<n> lines`. Take exactly that many lines as the hit's text; a line among them that looks like a header is text. The entry is the ref's part before `#`, whatever its length (10 characters, or 26 when two entries end alike).
   - For each hit that is on the subject, find its file:

     ```bash
     bilbo read '<that part>'
     ```

     It prints first

     ```text
     bilbo:<id>
     kind: markdown
     label: file:<absolute path> (capture)
     ```

     The hit is a note of this folder when `<absolute path>` is `<folder>`, a slash and the rest: the same folders, name by name, and not only the same first characters. A hit whose file stands elsewhere is no note of this folder: pass it over and go on to the next hit. When the `read` fails, or prints no `label: file:` line, you do not know where that hit's file stands: stop, report what it printed, and write no new note.
   - After the last hit's lines one more line can stand, which starts `more: `. It is no hit and no command to run: it says the list goes on. Take the next page only when this page shows a hit on the subject and none of its hits on the subject is a note of this folder. Then run your own command again with `--page 2` before the `--`, then `--page 3`, and so on:

     ```bash
     bilbo recall --page 2 --in='<folder>' --kind markdown -- '<words of the subject>'
     ```

     Stop at the first page that shows a note of this folder on the subject, that shows no hit on the subject, that ends without that line, or that says `nothing on page`. The list is ranked: behind a page that is off the subject no page is nearer to it.

   What the search said, over every page you ran, decides what comes next. A line `bilbo: the query was not logged: …` is a warning of its own: report it, and read the rows as if it were not there. Go by the first row that fits:

   | The search | What it means | What to do |
   |---|---|---|
   | A hit on the subject whose file stands in `<folder>` | A note is on the subject. | That is the note: `<id>` is that entry's id and `<path>` its file. |
   | A hit on the subject whose entry `bilbo read` did not show | Not known whether a note of this folder is on the subject. | Report the read's line and stop. Write no new note. |
   | Exit 1 with `bilbo: no store at <root>` | bilbo has kept nothing yet. That says nothing of the folder, whose files can be older than the store. | Look at the folder (below). |
   | A line `bilbo: --in '<folder>' names no entry` | bilbo has indexed no file of the folder. | Look at the folder (below). |
   | A line that starts `bilbo: the index is behind the store` or `bilbo: the index cannot be compared with the store's catalog` | What was kept lately is not searched yet: a note may be missing from the answer. | Look at the folder (below). |
   | Exit 1 with a last line that is neither `bilbo: nothing matched` nor `bilbo: nothing on page …` | No search ran. | Report the line, then look at the folder (below). |
   | Exit 2, or the shell's `command not found` | bilbo did not run. | Report the first line and stop. |
   | Exit 0 on every page you ran, with no note of this folder on the subject; or exit 1 with the last line `bilbo: nothing matched` or `bilbo: nothing on page …` | The search answered: no note of this folder is on the subject. A note that was deleted or forgotten is not given either: a deleted one that was never forgotten still holds its name (step 4), and a forgotten one's name is free. | There is no note. Do not list the folder: it can hold hundreds of files, and the search answered for the ones bilbo keeps. |
   | Anything else: a run that was cut short, or an ending no row names | Not known. | Report what it printed and stop. Write no new note. |

   **Look at the folder only where a row above sends you:** the search could not answer for it, and no answer from the search is not no file.

   ```bash
   ls -R '<folder>'
   ```

   Take each file there whose name bears on the subject through "From a file to its entry", below the steps. Open none of them before: that section first sees that a file is a note's file of this folder, and only then reads it and hands it to bilbo. When it gives the entry of a file on the subject, that file is the note. When no file there is on the subject, there is no note.

   **With the note,** go on with what you came for: an update (step 5), a rename (step 6) or a delete (step 7). **With no note,** write a new one (step 4) when you came to write. When you came to rename or delete, say that no note on the subject was found, and stop: write none.

4. **A new note.** Only when step 3 found no note and you came to write one.

   1. Pick `<folder>/<kind>-<topic>.md`: `<kind>-<topic>.md` is one name, with no `/`, no backslash and no control character, that ends in `.md` in lower case, and the file stands directly in `<folder>`. This is `<path>`. See that nothing is at it: both of these must fail.

      ```bash
      test -e '<path>'
      test -L '<path>'
      ```

      The second catches a link that points nowhere, which the first takes for a free name. A name that is taken is another note's. Take the file at it through "From a file to its entry", with no look at the folder: when it is on the subject and its entry is found, it is the note, and what you do is an update (step 5). When it is not on the subject, or is no note's file of this folder, pick another name. A name where a look saw something is never written: nothing you saw is ever overwritten. A name that another writer takes between these looks and your write is the limit "Where the guards stop" names.
   2. Write the file with Write: plain Markdown that starts with its `# ` title, with no frontmatter.
   3. Keep it, then say what "After every keep" asks:

      ```bash
      bilbo remember '<path>'
      ```

   `new 1` is a new note, kept. Any other word (`appended 1`, `new generation 1`, `unchanged 1`) means the name was an entry's already, and the file is now that entry's text: a result, not an error. With `refused 1` nothing was kept. Report the word and each line of stderr as it is.

5. **An update.**

   1. You hold the note's `<id>` and `<path>`: from step 3's search, or from "From a file to its entry" when a file led you here. Pass the six checks. The fifth has kept the file as it is.
   2. See that the file has one name only ("Before a write in place"), then edit the file in place with Edit.
   3. ```bash
      bilbo remember '<path>'
      ```
   4. ```bash
      bilbo read '<id>'
      ```

   When 3 prints one of `appended 1`, `new generation 1` and `unchanged 1`, the edit is kept, in the same entry under the same id: report the word, the id and the generation step 4 prints ("After every keep"), and predict none. When it prints none of them, the file is edited and not kept: say that, with the lines it printed, and name no generation. `recall` answers with the newest text only; `bilbo read '<id>#g<n>'` prints the outline of an earlier generation and `bilbo read '<id>#g<n>:r<a>-<b>'` its records.

6. **A rename.**

   1. Find the note (step 3), its `<id>` and its `<old path>`, and pass the six checks.
   2. Build the new path, and see each of these before you touch the frontmatter or a name:
      - **Where:** resolve the folder the new name is to stand in:

        ```bash
        realpath '<that folder>'
        ```

        `<new folder>` is the path that prints, and `<new path>` is `<new folder>`, a slash and the new name. Use this path, and no other spelling of it, for the move and for the last comparison. `<new folder>` is `<folder>` or lies inside it, and no folder name between them starts with a dot: a follow's walk skips such a folder, so bilbo would never keep a later edit of a note under one.
      - **Its extension:** the name ends in `.md`, in lower case, after a name of its own that does not start with a dot: a note renamed to `n.MD` or to `.md` would be withdrawn and never kept again.
      - **Its characters:** the whole of `<new path>`, its folders' names too, holds no backslash and no control character, so its label can be read back.

      Then see that nothing is at the new path: `test -e '<new path>'` and `test -L '<new path>'` both fail. A name that is taken stops the rename. A new name that differs from the old one only in case is taken where the volume folds case: this skill does no such rename, so stop there too and say so.
   3. Read the file's own id, as the fourth check reads it.
      - **None:** see that the file has one name only ("Before a write in place"), then write the entry's id. A file with no frontmatter gets three lines at its very top, `---`, `id: <id>`, `---`, and an empty line before its title; a file with a frontmatter block gets the line `id: <id>` right after the opening `---`. The line is `id:`, one space, the 26 characters, and nothing after. Then read the file's own id again: it must now be the entry's. If it is not, stop and say so, and move nothing.
      - **The entry's id:** go on.
      - **Anything else** stopped you at the fourth check.
   4. ```bash
      mv -n '<old path>' '<new path>'
      ```
   5. See what the move did: nothing answers at the old name any more (`test -e '<old path>'` and `test -L '<old path>'` both fail) and a regular file is at the new one (`test -f '<new path>'` holds). When the look does not show the move, stop and say so: the `mv` was not run, or it failed, or another file took the name first.
   6. ```bash
      bilbo remember '<new path>'
      ```
   7. ```bash
      bilbo read '<id>'
      ```

   It is done when step 7 prints `label: file:<new path> (capture)`, the new path byte for byte. Step 6 can say `bilbo: moved <new path>: its id <id> was held by file:<old path>`, and can say none, as when another pass moved the entry first. Its exit code is a thing apart: exit 1 can stand beside that line, when the search index was not updated after the move. Report the lines step 6 printed, its exit code and what step 7 printed, each as it is, and name no cause. Report the word, the id and the generation as "After every keep" says, and predict no word.

   A refusal says what bilbo saw: report its line and the path the file now has, and stop. Move and remove nothing more.

7. **A delete.**

   1. Find the note (step 3) and pass the six checks. The fifth is the capture before the delete: when it does not end clean (an exit code other than 0, or a file refused), stop and remove nothing. Read the file to its end with Read. Stop, and say so, when the read was cut, or when the file has a line that is JSON on its own and holds any string of 256 or more characters, whatever they are: bilbo may hold a placeholder for it, and the file is then the only whole copy.
   2. **Ask the person,** in so many words, before anything is removed: name the note's file and say that the file will be deleted and its entry taken out of `recall`. Go on only at a yes. A request to delete this very note, made in this conversation, is that yes; a harness's own prompt is not, since it may not come.
   3. ```bash
      rm '<path>'
      ```
      When `rm` was not run, or fails, or something is still at the path after it (`test -e '<path>'` or `test -L '<path>'` holds), stop here: say that the file was not removed, and run neither step 4 nor step 5.
   4. ```bash
      bilbo forget '<id>'
      ```
      It takes the entry out of `recall` at once and frees its name; what the note held stays readable by its id. When it is not run (the person, or the harness, stopped it), never try it again. Say that the file is removed and that `bilbo forget` was not run, run step 5, and report the label and the `records:` count it prints, with no word on how they came to be. The two tables below are for a `forget` that ran.
   5. ```bash
      bilbo read '<id>'
      ```

   Tell what happened from standard output of step 4 and from step 5, never from the exit code alone: a forget can be written and the run still exit 1. Report each line of stderr as it is. A forget is whole when the entry and what belongs to it, such as the summary bilbo wrote of it, are forgotten: step 4's counts say that, and step 5 speaks for the entry alone.

   | Standard output of step 4 | What it means |
   |---|---|
   | `forgotten 1, belonging <n>, unchanged 0` | This run forgot the entry and the `<n>` things that belong to it. The forget is whole. |
   | `forgotten 0, belonging <n>, unchanged 1` | The entry was forgotten before this run, which forgot the `<n>` things still left of it. The forget is whole. |
   | nothing | Not known. Go by step 5. |

   Then say the label and the `records:` count step 5 prints. Counts that step 4 printed stand whatever step 5 does:

   | Step 5 prints | What to do |
   |---|---|
   | `records: 0` and a label that starts `forgotten:` | The entry is forgotten. With counts from step 4: done. With none: run `bilbo forget '<id>'` once more; its counts say when the forget is whole. |
   | a label that starts `forgotten:` and a `records:` that is not 0 | The forget stopped part way. Run `bilbo forget '<id>'` once more and read again. If it is still so, report both lines and stop. |
   | the label `file:<path>` | The entry is not forgotten: say so, with what step 4 printed. |
   | nothing: the read itself fails | Report the read's line. With counts from step 4 the forget is whole; with none, say that you do not know whether the note was forgotten, and stop. |

8. **A note with no file.** A note typed or piped into bilbo has no file, and step 3 never gives it: its search is of Markdown files. This skill neither changes nor forgets one. Taking a wrong one out of `recall` is the person's to do, with `bilbo forget '<its id>'`: say so, and run nothing.

## From a file to its entry

A file you found on disk, in the folder's list (step 3) or at a name you wanted (step 4), is a note you may update only once bilbo holds it as an entry under its own path. `<its path>` is `<folder>`, a slash and the rest, as the list or your name gives it. Do these in order.

1. **See that it is a note's file of this folder, before you open it or hand it to bilbo:**
   - its name ends in `.md`, in lower case, and neither the name nor a folder between `<folder>` and the file starts with a dot;
   - it is a regular file and no link: `test -f '<its path>'` holds and `test -L '<its path>'` fails;
   - `realpath '<its path>'` prints `<its path>` again, byte for byte.

   A file that fails one of them is no note of this folder: neither read nor keep it. Pass it over, and say which look failed when it is the one file you came for. A link in the notes folder can name a file anywhere on the machine, and `bilbo remember` would keep that file under its real path.
2. **Read it.** A byte-order mark at its start stops you here, as at the fourth check. Otherwise see whether it is on the subject. When it is not, it is another note: go back to the step that sent you here.
3. **Keep it.** It answers for a file bilbo held already too:

   ```bash
   bilbo remember '<its path>'
   ```

   With exit 0 and any count's word but `refused 1`, the file is an entry ("After every keep" reads the word). With `refused 1`, or another exit code, report the lines and stop.
4. **Ask for its entry** by the file's own path and words, not the subject's: a note on drainage answers a question about waterlogging and may hold none of your words.

   ```bash
   bilbo recall --in='<its path>' --kind markdown -- '<words of the file's title>'
   ```

   For each hit, run `bilbo read '<the ref's part before #>'`. The entry is the file's when that prints `label: file:<its path> (capture)`, the path byte for byte: its first line gives `<id>`, and `<path>` is `<its path>`.
5. **When no hit has that label,** ask once more, with the words of another line of the file. When that gives none either, stop. Say that the file is on the subject, what `bilbo remember` printed for it, and that `recall` gave no entry for it. Write no new note on the subject, which would be a second one, and change no file whose entry you did not read.

## Before it touches a file

Before it edits, renames or removes a note's file it shows all of these, in this order, and stops at the first that fails:

1. **The entry.** `bilbo read` gave an entry of the kind `markdown` whose label is `file:<path>`. The printed path holds no backslash: `bilbo read` prints a control character inside a name (a line end, a tab) as an escape that starts with a backslash, and a real backslash as itself, so a path with a backslash in it can be the name of two different files. Stop there, before any change, and say so.
2. **The path is a file.** `<path>` itself is a regular file and no link (`test -f '<path>'`, and `test -L '<path>'` fails), and lies inside the notes folder.
3. **The path is the label's.** `realpath '<path>'` prints `<path>` again, byte for byte. A folder above the file that became a link, or another spelling that answers for the name, fails here.
4. **The file's own id.** A byte-order mark at the file's start stops you here: bilbo reads an id behind it, and this check does not. A file gives an id only in a block at its top: its very first line is `---`, and the first later line that is `---` closes it. Look at the first line between the two that starts with `id:`:
   - there is none, or the file opens with no such block: the file gives no id. Go on.
   - it is, byte for byte, `id: ` and the entry's id: go on.
   - it is anything else: stop and say so. bilbo reads another id or none from such a file, and the file at that path is not the note you found.
5. **The store agrees.**

   ```bash
   bilbo remember '<path>'
   ```

   exits 0 and its first line ends in `refused 0`. The store then holds that file as the entry you found, an edit nobody kept yet included, with one exception: in a line that is JSON on its own, bilbo may keep a placeholder for a long string, not its text. Any other result stops you and you report it.
6. **The entry is still the path's.** After that run, `bilbo read '<id>'` prints `label: file:<path> (capture)` again, the same path byte for byte. That run may have served another bilbo's request for a pass over the follows at its close, and such a pass may have moved the entry to another file that carries its id. The label is then another path: stop and change nothing. A `read` that fails stops you too.

A hit that fails one of them is no note you may touch; say which check failed.

## Before a write in place

Before you write into a file in place (the edit of step 5, or the `id` line of step 6), see that it has one name only: `ls -ld '<path>'` shows a link count of 1. A file with a second name is another path's file too, and a write in place would change both: stop and say so. A delete and a move need no such look.

## After every keep

After every `bilbo remember` that exits 0 (steps 4, 5 and 6) say the count's word: of the five counts on the first line of standard output, the one whose number is 1, said whole (`new 1`, `appended 1`, `new generation 1` or `unchanged 1`). Of a note whose id you hold, in an update or a rename, say the id too, and the generation `bilbo read '<id>'` prints after the keep. Of a new note say the word and its path, and name no id: you read none. Predict nothing: the store says it.

## Where the guards stop

You act on what each look showed. Nothing here guards against another writer that changes a name between a look and the act that follows it: no instruction can. The checks do not compare the file's text with what the store holds. A file replaced by another of the same size and the same modification time is not read by bilbo either; that limit stands here too. A new note's name is one such look and act: two sessions that pick one name in the same seconds can both see it free, and the second write then stands where the first was.
