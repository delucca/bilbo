---
name: note
description: Keeps what a later session should know as a bilbo note, which is a Markdown file in the notes folder bilbo follows, such as a decision, gotcha, plan or finding. Use for "note this", "save this as a decision/gotcha/plan", "keep this for later sessions", or when this session settled something durable. NOT for a passing "note that..." in conversation, or for searching notes (recall).
license: Apache-2.0
allowed-tools: Bash(bilbo config follow), Bash(bilbo remember *), Bash(bilbo recall *), Bash(bilbo read *), Bash(test -e *), Bash(test -f *), Bash(test -L *), Bash(realpath *), Bash(ls *), Bash(mv -n *), Read, Write, Edit
---

# note

A note is a Markdown file in the folder bilbo follows. This skill writes, updates, renames and deletes those files, and keeps each change in bilbo at once. It uses neither `bilbo remember -` nor `bilbo remember --edit`.

It reports what bilbo printed and reads no cause into it. It never writes over a file it saw, and it acts on a file only after the six checks of "Before it touches a file" hold, in that order, stopping at the first that fails and saying which one failed.

**Every path, every ref and the subject of a search is one argument.** A note's name may hold a space, a `*` or a `$`, and a subject may hold a `>`. In every command below a path, a ref or a subject stands in single quotes: write it so, the whole of it between the quotes, and write a single quote inside it as `'\''`. Never put one into a command without them.

The steps are one list, read from the top. Five sections after it hold what the steps send you to: "From a file to its entry", "Before it touches a file", "Before a write in place", "After every keep" and "Where the guards stop".

## Steps

1. **Find the notes folder.**

   ```bash
   bilbo config follow
   ```

   It prints the follows as one line of TOML, for example `[{ path = "/home/me/notes", kind = "markdown" }, { path = "/home/me/.claude/projects", kind = "session" }]`. The notes folder is the one `path` whose `kind` is `markdown` and which is a folder. With several, ask which. With none, the machine was not set up: `bilbo setup` makes a notes folder and follows it. Without it, set one up with the user's folder:

   ```bash
   bilbo remember '<their folder>' --kind markdown --follow
   ```

   On an empty folder this prints `nothing to keep: markdown claims no file under <folder>` and exits 1, and the follow is saved all the same: the next line on stderr says so (`saved the follow in <config>, …`). That is the setup working. The first follow a config gets is written after the two default follows, which the config then holds too.

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

   `--kind markdown` keeps the search to Markdown files, so a session on the same subject does not stand before a note. `--in='<folder>'` narrows it to entries whose label holds the folder's words. That is by words and not by folders: a file of `/home/me/notes-old` can answer for `/home/me/notes`. So a hit is a note of this folder only when its file stands in `<folder>`, which you see for each hit.

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

     The hit is a note of this folder when `<absolute path>` is `<folder>`, a slash and the rest: the same folders, name by name, and not only the same first characters. A hit whose file stands elsewhere is no note of this folder: pass it over and go on to the next hit. It is no reason to stop, and no note to update. When the `read` fails, or prints no `label: file:` line, you do not know where that hit's file stands: stop, and report what it printed. A hit on the subject that you cannot place is no reason to write a new note.
   - After the last hit's lines one more line can stand, which starts `more: `. It is no hit and no command to run: it says the list goes on. Take the next page only when this page shows a hit on the subject and none of its hits on the subject is a note of this folder. Then run your own command again with `--page 2` before the `--`, then `--page 3`, and so on:

     ```bash
     bilbo recall --page 2 --in='<folder>' --kind markdown -- '<words of the subject>'
     ```

     Stop at the first page that shows a note of this folder on the subject, that shows no hit on the subject, that ends without that line, or that says `nothing on page`. The list is ranked: behind a page that is off the subject no page is nearer to it.

   What the search said, over every page you ran, decides what comes next. Go by the first row that fits:

   | The search | What it means | What to do |
   |---|---|---|
   | A hit on the subject whose file stands in `<folder>` | A note is on the subject. | An update (step 5): `<id>` is that entry's id and `<path>` its file, once the six checks hold. |
   | A hit on the subject whose entry `bilbo read` did not show | Not known whether a note of this folder is on the subject. | Report the read's line and stop. Write no new note. |
   | Exit 1 with `bilbo: no store at <root>` | bilbo has kept nothing yet. That says nothing of the folder, whose files can be older than the store. | Look at the folder (below). |
   | A line `bilbo: --in '<folder>' names no entry` | bilbo has indexed no file of the folder. | Look at the folder (below). |
   | A line that starts `bilbo: the index is behind the store` or `bilbo: the index cannot be compared with the store's catalog` | What was kept lately is not searched yet: a note may be missing from the answer. | Look at the folder (below). |
   | Exit 1 with a last line that is neither `bilbo: nothing matched` nor `bilbo: nothing on page …` | No search ran. | Report the line, then look at the folder (below). |
   | Exit 2, or the shell's `command not found` | bilbo did not run. | Report the first line and stop. |
   | Anything else: pages with no note of this folder on the subject, the last of them ending as it may, or `bilbo: nothing matched` with none of the lines above | The search answered: no note of this folder is on the subject. A note that was deleted or forgotten is not given either: a deleted one that was never forgotten still holds its name (step 4), and a forgotten one's name is free. | A new note (step 4). Do not list the folder: it can hold hundreds of files, and the search answered for the ones bilbo keeps. |

   **Look at the folder only where a row above sends you:** the search could not answer for it, and no answer from the search is not no file.

   ```bash
   ls -R '<folder>'
   ```

   Take each file there whose name bears on the subject through "From a file to its entry", below the steps. Open none of them before: that section first sees that a file is a note's file of this folder, and only then reads it and hands it to bilbo. When it gives the entry of a file on the subject, that file is the note, and what you do is an update (step 5). When no file there is on the subject, it is a new note (step 4).

4. **A new note.** Only when step 3 ended at a new note: the search answered and gave none of this folder on the subject, or the folder was looked at and holds no file on it.

   1. Pick `<folder>/<kind>-<topic>.md`: `<kind>-<topic>.md` is one name, with no `/`, no backslash and no control character, that ends in `.md` in lower case, and the file stands directly in `<folder>`. This is `<path>`. See that nothing is at it: both of these must fail.

      ```bash
      test -e '<path>'
      test -L '<path>'
      ```

      The second catches a link that points nowhere, which the first takes for a free name. A name that is taken is another note's. Take the file at it through "From a file to its entry", with no look at the folder: when it is on the subject and its entry is found, it is the note, and what you do is an update (step 5). When it is not on the subject, or is no note's file of this folder, pick another name. A name where a look saw something is never written: nothing you saw is ever overwritten. A name that another writer takes between these looks and your write is the limit "Where the guards stop" names.
   2. Write the file with Write: plain Markdown that starts with its `# ` title, with no frontmatter.
   3. Keep it, then say the three things of "After every keep":

      ```bash
      bilbo remember '<path>'
      ```

   - `new 1`: a new note, and it is kept. `recall` finds it once the run has brought the search index up to date, which it does before it ends. When stderr says `the search index was not updated: …` the run exits 1 with the note kept and not yet found: report the line, and say that the note is kept and that `recall` will give it after the index is brought up to date.
   - Any other word (`appended 1`, `new generation 1`, `unchanged 1`): the name was already an entry's, and the file is now that entry's text. It is a result, not an error. One way there is a note that was deleted and withdrawn and has come back under its name, with its older text in the generation before. The word does not show which way it was: an entry no pass withdrew can give any of the three. Nothing reserves a name, and a free path does not show that a name was never a note's.
   - `bilbo: kept <path> again: its entry <id> was forgotten` on stderr: the file begins with what a forgotten note at this name held, so bilbo took it for that note and the entry is back. Report the line.
   - `refused 1` and exit 1: stderr says why, and nothing was kept. Report the line.

5. **An update.**

   1. You hold the note's `<id>` and `<path>`: from step 3's search, or from "From a file to its entry" when a file led you here. Pass the six checks. The fifth has kept the file as it is.
   2. See that the file has one name only ("Before a write in place"), then edit the file in place with Edit.
   3. ```bash
      bilbo remember '<path>'
      ```
   4. ```bash
      bilbo read '<id>'
      ```

   Standard output of 3 says one of `appended 1`, `new generation 1` and `unchanged 1`: the same entry and the same id. Report the word printed, the id and the generation step 4 prints ("After every keep"), and predict none of them: lines added at the end are `appended 1` most of the time and `new generation 1` when the file's last line had no line end before; the word is the store's to say. `recall` answers with the newest text only. `bilbo read '<id>#g<n>'` prints the outline of an earlier generation and `bilbo read '<id>#g<n>:r<a>-<b>'` its records.

6. **A rename.**

   1. Find the note (step 3), its `<id>` and its `<old path>`, and pass the six checks.
   2. Build the new path, and see each of these before you touch the frontmatter or a name:
      - **Where:** resolve the folder the new name is to stand in:

        ```bash
        realpath '<that folder>'
        ```

        `<new folder>` is the path that prints, and `<new path>` is `<new folder>`, a slash and the new name. Use this path, and no other spelling of it, for the move and for the last comparison. `<new folder>` is `<folder>` or lies inside it, and no folder name between `<folder>` and the file starts with a dot: a follow's walk skips a folder whose name starts with a dot, so bilbo would never keep a later edit of a note under one.
      - **Its extension:** the name ends in `.md`, in lower case, after a name of its own that does not start with a dot. `md` is the one extension the kind claims; a file named `.md` alone has no extension, and a follow's walk skips every name that starts with a dot. A note renamed to `n.MD` or to `.md` would be withdrawn and never kept again.
      - **Its characters:** the whole of `<new path>`, its folders' names too, holds no backslash and no control character, so its label can be read back.

      Then see which of two renames it is:
      - **A rename by case only:** the new name differs from the old one only in the case of its letters. Then all three of these must hold:
        - `realpath '<new path>'` prints `<old path>`: the volume folds case, and the new spelling answers as this very file;
        - `test -L '<new path>'` fails: the new spelling is no link;
        - `ls '<new folder>'` lists the old name and does not list the new one.

        When one does not hold, stop and say which look failed and what it printed; name no cause.
      - **Every other rename:** nothing is at the new path: `test -e '<new path>'` and `test -L '<new path>'` both fail. A name that is taken stops the rename.
   3. Read the file's own id, as the fourth check reads it.
      - **None:** see that the file has one name only ("Before a write in place"), then write the entry's id. A file with no frontmatter gets three lines at its very top, `---`, `id: <id>`, `---`, and an empty line before its title; a file with a frontmatter block gets the line `id: <id>` right after the opening `---`. The line is `id:`, one space, the 26 characters, and nothing after. Then read the file's own id again: it must now be the entry's. If it is not, stop and say so, and move nothing.
      - **The entry's id:** go on.
      - **Anything else** stopped you at the fourth check.
   4. Move the file, by the branch of 2:
      - **By case only:**

        ```bash
        mv '<old path>' '<new path>'
        ```

        Here `mv -n` would do nothing, since the new name already answers; there is no other file to protect. A plain `mv` can overwrite, so ask the person first, naming both spellings, and run it only at a yes. It is not in this skill's allowed tools; a harness may ask again, and may not.
      - **Every other rename:**

        ```bash
        mv -n '<old path>' '<new path>'
        ```
   5. See what the move did. By case only: `ls '<new folder>'` lists the new spelling and not the old. Every other rename: nothing answers at the old name any more (`test -e '<old path>'` and `test -L '<old path>'` both fail) and a regular file is at the new one (`test -f '<new path>'` holds). When the look does not show the move, stop and say so: the `mv` was not run, or it failed, or, with `mv -n`, another file took the name first.
   6. ```bash
      bilbo remember '<new path>'
      ```
   7. ```bash
      bilbo read '<id>'
      ```

   It is done when step 7 prints `label: file:<new path> (capture)`, the new path byte for byte. Step 6 says `bilbo: moved <new path>: its id <id> was held by file:<old path>` when its own run moved the entry and its store closed well. It says none when another pass moved the entry first, and none when its own run failed after the move was written: report the lines step 6 printed and its exit code, and name no cause. Report the word step 6 printed, the id and the generation step 7 printed ("After every keep"), and predict no word: `new generation 1` is the usual one after step 3 wrote the id and `unchanged 1` when the id was there and kept, but the word is the store's to say.

   A refusal says what bilbo saw, and you say that and no more:
   - `… its id <id> is held by file:<old path>` and nothing after: bilbo found the id still held at the old path. That can be a file there that still gives the id, one it could not read, or a path it could not look at. Report the line beside what your step 5 saw, and remove nothing.
   - `… but this path was kept as entry <other id>`: the new name was another note's before. Report the line and the path the file now has, and stop: move nothing more. The person picks another name, or frees this one with `bilbo forget '<other id>'`, which takes that other note out of `recall`.
   - Any other reason (`cannot read: …`, a path that is not UTF-8): report the line.

7. **A delete.**

   1. Find the note (step 3) and pass the six checks. The fifth is the capture before the delete: when it does not end clean (an exit code other than 0, or a file refused), stop and remove nothing. Stop too, and say so, when the note has a line bilbo keeps only as a placeholder (the fifth check says which): the file is then the only whole copy.
   2. **Ask the person,** in so many words, before anything is removed: name the note's file and say that the file will be deleted and its entry taken out of `recall`. Go on only at a yes. A request to delete this very note, made in this conversation, is that yes; a harness's own prompt is not, since it may not come.
   3. ```bash
      rm '<path>'
      ```
      When `rm` was not run, or fails, or something is still at the path after it (`test -e '<path>'` or `test -L '<path>'` holds), stop here: say that the file was not removed, and run neither step 4 nor step 5.
   4. ```bash
      bilbo forget '<id>'
      ```
      It takes the entry out of `recall` at once and frees its name; what the note held stays readable by its id. When it is not run (the person, or the harness, stopped it), say that the file is removed and that `bilbo forget` was not run, and say of the entry only what step 5's read shows, since a pass can have run in between. With its records under the label `file:<path>`, the entry is kept as it was, and `recall` can still give it. With `records: 0` under that label, a pass has withdrawn it: `recall` gives it no more, and its name is still its entry's until `bilbo forget '<id>'` is run. A `bilbo refresh` withdraws it only while its folder still shows a file to that pass: if it was the folder's last note, the entry stays as it was.
   5. ```bash
      bilbo read '<id>'
      ```

   Tell what happened from standard output of step 4 and from step 5, never from the exit code alone: a forget can be written and the run still exit 1.

   A forget is whole when the entry and everything that belongs to it are forgotten: the summary bilbo wrote of the note is such a thing, and it is forgotten after the entry is. Two things say what happened, and they say different things. The counts of step 4 say whether the run got through all it had to write. The read of step 5 says what the entry itself holds, and nothing of what belongs to it.

   | Standard output of step 4 | What it means | What to say and do |
   |---|---|---|
   | `forgotten 1, belonging <n>, unchanged 0` | This run forgot the entry and the `<n>` things that belong to it. The forget is whole, whatever the exit code. | With exit 0, stderr holds `forgot <id>: its label was file:<path>; bilbo read <id>#g<n> prints the outline of what it held`, and one `forgot <other id> with <id>: …` line for each thing that went with it: the notes and the summary bilbo wrote about it. With exit 1, something after the forget failed: report each line of stderr. The note is forgotten all the same, and `recall` does not give it: with `the search index was not updated: …` the index still lists it, and `recall`, which checks each hit against the store, leaves it out and says so (`left out <n> hits the store no longer gives as indexed`) until the index is brought up to date. |
   | `forgotten 0, belonging <n>, unchanged 1` | The entry was forgotten before this run. With `<n>` at 0 nothing was left to do. With `<n>` over 0 this run forgot `<n>` things that still belonged to it: it finished a forget that had stopped part way. Either way the forget is whole. | Say which of the two it was, with the line. |
   | nothing, and stderr holds `stopped part way: bilbo forget <id> finishes it` | The store failed while it wrote; some of it may be written. | Report both lines. That line names the command that finishes the forget once the cause is gone: run `bilbo forget '<id>'` once more then, and stop if it fails again. The forget is whole only once a run prints its counts. |
   | nothing else | Not known from this output. | Report the first line of stderr, then go by step 5, which can speak for the entry alone. |

   Then say what step 5 shows of the entry. Counts that step 4 printed stand whatever step 5 does:

   | Step 5 prints | What it means | What to do |
   |---|---|---|
   | `records: 0` and a label that starts `forgotten:` | The entry itself is forgotten. | With counts from step 4: done. With none: say that the entry is forgotten and that you do not know whether what belongs to it is, and run `bilbo forget '<id>'` once more: its counts say when the forget is whole. |
   | a label that starts `forgotten:` and a `records:` that is not 0 | The forget stopped between its two writes: the entry has its forgotten label and still holds its records. | Run `bilbo forget '<id>'` once more and read again. If it is still so, report both lines and stop. |
   | the label `file:<path>` | The entry is not forgotten. | Say so, with what step 4 printed. |
   | nothing: the read itself fails | The read says nothing. | Report the read's line. When step 4 printed its counts, say what they said: the forget is whole, and only the read back failed. When it printed none, say that you do not know whether the note was forgotten; run `bilbo read '<id>'` once more, and if it fails again, stop, and leave `bilbo forget '<id>'` to be run again once the read answers. |

   "Gone from `recall`" means the entry's id is not among the hits; another note with the same words still answers.

8. **A note with no file.** A note typed or piped into bilbo has no file, and step 3 never gives it: its search is of Markdown files. It cannot be changed, and it fails the first check below; a wrong one can only be taken out of `recall`. Only when the user asks for that:

   1. Take the note's ref from the user, or find it:

      ```bash
      bilbo recall --kind note -- '<words of the note>'
      ```

      `--kind note` gives every note that is no file: the ones a person typed, and the paragraphs bilbo wrote about sessions. A hit's ref, and a ref a user hands you, ends in a record's place and does not say which of the two it is.
   2. Read the entry itself, by the part of the ref before `#`:

      ```bash
      bilbo read '<that part>'
      ```

      It prints first `bilbo:<id>`, then `kind:` and `label:`, and a line `records: <n>`. Go on only when the kind is `note` and the label is `note:` followed by exactly the id the first line prints. A label that starts `derive:` is the entry of every note bilbo wrote about one session: never forget it for the sake of one paragraph, and say so. A label that starts `forgotten:`, or any other: stop and say what it is.
   3. Read the note itself, by that whole id. What step 2 printed under the header is an outline, the first line of the note and no more, and two notes can begin with the same line:

      ```bash
      bilbo read '<id>#r1'
      ```

      With `records: <n>` over 1, read `'<id>#r1-<n>'`. The last line of what it prints is bilbo's own. When it starts `end:`, the page was the last. When it starts `more:`, run `bilbo read` with the ref that line gives, and so on to a line that starts `end:`. When it starts `cut:`, the page's last record came cut, and the line names what reads that record whole: a ref of the form `<id>#g<n>:r<k>`, and a number after `--budget`. Make your own command of those two, and of nothing else on the line:

      ```bash
      bilbo read '<id>#g<n>:r<k>' --budget <that number>
      ```

      Then go on as the rest of that line said, which is a `more:` or an `end:` after the `cut:` part. You have read the note whole when every record came and none came cut. When a read fails, or a record still comes cut at the budget its own line named, stop: say that you could not read the note to its end, and forget nothing. Two notes can share a page of beginning and part at the end.
   4. Show the user the note's text as step 3 printed it, and the id: the whole text, or, for a long one that you read whole, its beginning and its end with a word on what you left out. Ask as in step 7, naming the id: a yes is for this text under this id.
   5. Run `bilbo forget '<id>'` with the whole id of 2, then `bilbo read '<id>'`, and report by step 7's two tables. Where they say the label `file:<path>`, this entry's is `note:<id>`.

## From a file to its entry

A file you found on disk, in the folder's list (step 3) or at a name you wanted (step 4), is a note you may update only once bilbo holds it as an entry under its own path. `<its path>` is `<folder>`, a slash and the rest, as the list or your name gives it. Do these in order.

1. **See that it is a note's file of this folder, before you open it or hand it to bilbo:**
   - its name ends in `.md`, in lower case, and neither the name nor a folder between `<folder>` and the file starts with a dot;
   - it is a regular file and no link: `test -f '<its path>'` holds and `test -L '<its path>'` fails;
   - `realpath '<its path>'` prints `<its path>` again, byte for byte.

   A file that fails one of them is no note of this folder: do not read it and do not keep it. Pass it over, and say which look failed when it is the one file you came for. A link in the notes folder can name a file anywhere on the machine, and `bilbo remember` would keep that file under its real path.
2. **Read it,** and see whether it is on the subject. When it is not, it is another note: go back to the step that sent you here.
3. **Keep it.** The same command answers for a file bilbo held already:

   ```bash
   bilbo remember '<its path>'
   ```

   With exit 0 and any count's word but `refused 1`, the file is an entry ("After every keep" reads the word). With `refused 1`, or another exit code, report the lines and stop.
4. **Ask for its entry** by the file's own path and the file's own words, not the subject's: a note on drainage answers a question about waterlogging and may hold none of your words.

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
4. **The file's own id.** Read it as bilbo does: a byte-order mark at the very start of the file is not part of its first line, and a carriage return at the end of a line is not part of that line. A file gives an id only when its very first line is `---`, the first later line that is `---` closes the block, and between the two the first line that starts with `id:` is exactly `id:`, one space, the 26 characters and nothing after. Look at that one line, the first between the two that starts with `id:` at the start of the line:
   - there is none, or the file opens with no such block: the file gives no id. Go on.
   - it is, byte for byte, `id: ` and the entry's id: go on.
   - it is anything else, another id or an `id:` line bilbo does not read as an id (a second space, a space at its end, quotes, lower-case letters): stop and say so. bilbo reads another id or none from such a file, a later `id:` line does not count, and the file at that path is not the note you found.
5. **The store agrees.**

   ```bash
   bilbo remember '<path>'
   ```

   exits 0 and its first line ends in `refused 0`. The store then holds that file as the entry you found, an edit nobody kept yet included, with one exception: a line that is JSON on its own and holds a string of 256 or more base64 characters (letters, digits, `+`, `/`, `-`, `_`) is kept as a placeholder, not as its text. Any other result stops you and you report it.
6. **The entry is still the path's.** After that run, `bilbo read '<id>'` prints `label: file:<path> (capture)` again, the same path byte for byte. That run may have served another bilbo's request for a pass over the follows at its close, and such a pass may have moved the entry to another file that carries its id. The label is then another path: stop and change nothing. A `read` that fails stops you too.

A hit that fails one of them is no note you may touch; say which check failed.

## Before a write in place

Before you write into a file in place (the edit of step 5, or the `id` line of step 6), see that it has one name only: `ls -ld '<path>'` shows a link count of 1. A file with a second name is another path's file too, and a write in place would change both: stop and say so. A delete and a move need no such look.

## After every keep

After every `bilbo remember` that exits 0 (steps 4, 5 and 6) say three things: the count's word, the entry's id, and the generation `bilbo read '<id>'` prints after the keep.

- **The count's word** is, of the five counts on the first line of standard output, the one whose number is 1, said whole: `new 1`, `appended 1`, `new generation 1` or `unchanged 1`. On `new 0, appended 0, new generation 1, unchanged 0, refused 0` it is `new generation 1`, never the line's first word.
- **The id of a new note** is found from its file, as part 4 of "From a file to its entry" asks for one: `bilbo recall --in='<path>' --kind markdown -- '<words of its title>'`, and the hit whose `bilbo read` prints `label: file:<path> (capture)`. When no hit has that label, say the word and that `recall` gave no hit for the file; name no id you did not read.

Predict none of the three: the store says them.

## Where the guards stop

You act on what each look showed. Nothing here guards against another writer that changes a name between a look and the act that follows it: no instruction can. The checks do not compare the file's text with what the store holds. A file replaced by another of the same size and the same modification time is not read by bilbo either; that limit stands here too. A new note's name is one such look and act: two sessions that pick one name in the same seconds can both see it free, and the second write then stands where the first was.

## What a note with no id costs

A new note carries no `id`, and the rename above writes the entry's id into the file. A rename by this skill then costs, as a rule, one generation, which holds the frontmatter. What costs more is a rename behind your back, a person's `mv` or a moved folder, of a note that never got an `id`: the file is a new entry under a new id, and a ref an agent holds reads only the old one; the old entry is withdrawn at the next pass when its folder is still followed and still shows a file. When the whole folder moved it never is, and `recall` then shows one of the two and says `passed over: 1 copy`; unless a follow above the old folder still shows a file and so covers the old path: that follow withdraws it. `bilbo forget '<the old id>'` takes the older of the two out of `recall`.
