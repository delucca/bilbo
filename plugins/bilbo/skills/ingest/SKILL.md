---
name: ingest
description: Adds a web page, text file or PDF to the bilbo library as a source that bilbo fetches and keeps verbatim. Use for "ingest this", "save this page as a source", "add this doc to the library". NOT for notes (note).
license: Apache-2.0
allowed-tools: Bash(command -v bilbo), Bash(command -v pdftotext), Bash(bilbo library), Bash(bilbo library *), Bash(bilbo check), Bash(mktemp -d), Bash(curl -fsSL -o *), Bash(pdftotext -layout *), Read, Edit
---

# ingest

Adds a source to the bilbo library through `bilbo library stage` and `bilbo library land`: bilbo fetches the text, you choose which lines to keep, and you write the source's guide entry. It never writes a source or a staged capture.

## Steps

1. Check that bilbo is installed:

   ```bash
   command -v bilbo
   ```

   When it prints nothing, stop with exactly this line and do nothing else:

   ```
   ingest: bilbo is not on PATH; install the bilbo CLI first
   ```

2. Decide what was given.

   - An `http(s)` URL: step 3.
   - A local text or Markdown file: stage it with `--origin`, `url: <where it came from>` when known, else `doc: <title or name of the document>`.
   - A saved HTML page: the same, plus `--html`.
   - A PDF: step 4.
   - Text pasted into the conversation is not a source: ask for its URL or a file.

3. Stage the URL:

   ```bash
   bilbo library stage '<url>'
   ```

   A file goes the same way, as `bilbo library stage '<file>' --origin '<origin>'`, with `--html` for a saved page. Stage prints `stage: <id>` and the path of its `capture.md`, the `title:` and `keep:` it suggests, any `existing: <corpus>/<name>` line, and warnings on stderr.

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty or warnings | choose the keep ranges (step 5) |
   | 1 | says it is a PDF | step 4 |
   | 1 | anything else (status, unreachable, not UTF-8, no text) | say bilbo could not fetch it, ask the user to save the page with a tool of their own and give its path, then stage that file with `--html` or as text. Never fetch it another way |
   | 2 | anything | fix the argument and run it once more. A second exit 2: print the first stderr line and stop |

   When many headings are lost, or the capture is mostly navigation, and the site offers a Markdown rendition (`<url>.md` on many docs sites), stage that URL once. Keep it only when `media type:` is `text/markdown` or `text/plain`: a 200 alone proves nothing. Otherwise go on with the first stage.

4. A PDF. Check for the tool:

   ```bash
   command -v pdftotext
   ```

   When it prints nothing, say that `pdftotext` (poppler) or a text file of the PDF is needed, and stop: stage nothing and write no file. Otherwise make a folder:

   ```bash
   mktemp -d
   ```

   Download the PDF into it, unless it is a local file:

   ```bash
   curl -fsSL -o '<dir>/source.pdf' '<url>'
   ```

   Write its text:

   ```bash
   pdftotext -layout '<pdf>' '<dir>/source.txt'
   ```

   `<pdf>` is the download, or the local file.

   Then stage the text as in step 3, with `--origin 'url: <url>'` (`doc: <name>` for a local PDF). A source made this way is `capture: external`: bilbo did not choose the tool.

5. Choose the keep ranges. Read the stage's `capture.md` with Read: from line 1 to 30 lines past the suggested `keep` start, the last 30 lines of the keep range and what follows, around each `navigation suspect`, and around each lost heading. Keep the document's own text, headings, code, tables and footnotes. Drop menus, breadcrumbs, "on this page" lists, edit and feedback links, previous and next links, footers and cookie notices. Cut only between blocks, never inside a code fence, and give several `--keep` ranges when chrome sits in the middle. The title is the document's, as `title:` prints it when that is right.

   A warning no range can fix, such as `unclosed fence`, stays as it is: leave `capture.md` alone and put the warning in the report.

6. Pick the corpus and the name:

   ```bash
   bilbo library
   ```

   Read the guide of a likely corpus with `bilbo library <corpus>`. Use the corpus whose guide fits, or a new name in the topic grammar when none does; ask the user once when two fit equally. The source name is two to six words of the title in lowercase kebab-case.

7. Land. A re-ingest lands onto the `<corpus>/<name>` that `existing:` printed, and step 6 is skipped for it:

   ```bash
   bilbo library land <stage> <corpus>/<name> --keep <ranges> --title '<title>'
   ```

   Add `--replace` only when the user asked to re-ingest that source, or agreed when you asked. When stage printed `existing: <corpus>/<name>` and the user only asked to ingest the URL, ask whether to replace it before running `land`.

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty or warnings | `land` prints the source's path, its id and the `guide.md`: write the entry (step 8) |
   | 1 | says it already exists and names `--replace` | the name is taken: ask the user whether to replace it (`--replace` onto that name) or use another name |
   | 1 | says there is nothing to replace | the target is wrong: land onto the name `existing:` printed, or without `--replace` when none was printed |
   | 1 | lists citations that would degrade | show them and ask before adding `--force` |
   | 1 | anything else | print the first stderr line and stop |
   | 2 | names `--keep`, `--title` or the target | fix that argument and run `land` once more. A second exit 2: print the first stderr line and stop |

8. Write the guide entry. When `land --replace` left the guide as it was (same digest, so no stale line), the entry stands: skip this step and say so in the report. Otherwise read the outline:

   ```bash
   bilbo library show <corpus>/<name>
   ```

   Read the body only through a plan, never with Read. First look at your own tool list. When it holds a tool named `Agent` (Claude Code), plan with the defaults:

   ```bash
   bilbo library plan <corpus>/<name>
   ```

   When it holds none (Codex, or any session without subagents), your shell may cut output near 10,000 bytes, so plan smaller slices. Those two numbers are the only slice limits this skill sets:

   ```bash
   bilbo library plan --slice-bytes 8000 --slice-lines 250 <corpus>/<name>
   ```

   Then every slice, one per call, in order:

   ```bash
   bilbo library read <plan> <slice>
   ```

   - Read every slice when `tokens:` is at most 60,000.
   - Above that, or for a catalog (`catalog: yes`, which `plan` refuses whole), plan the opening and the main sections by anchor, `bilbo library plan '<corpus>/<name>#<anchor>'`, up to 60,000 tokens in all, and say in the entry that the source is a reference to look things up in. When the outline comes back cut, run `bilbo library show <corpus>/<name> --depth 2`.
   - A result without its `-- end slice` line, with a gap in its line numbers, or with a truncation notice is read again in parts, then in quarters if a part is cut too. A tool can drop the middle and keep the end line, so check the numbers even when the end line is there, and never open a file a tool saved the output to:

     ```bash
     bilbo library read <plan> <slice> --part 1/2
     ```

     then `--part 2/2`. If a half is still cut, read `--part 1/4` to `--part 4/4`.

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty | go on |
   | 1 | the source changed, the plan is gone | plan again, once |
   | 1 or 2 | anything else | print bilbo's first stderr line and stop |

   Base the entry on what you read. Edit the `guide.md` that `land` printed: replace `TODO: describe this source.`, or the stale line after a `--replace`, with two or three sentences of your own words on what the source covers, when to consult it, and what it gets wrong or leaves out. When the corpus is new, replace its `TODO: describe this corpus.` line with a lead on what the corpus grounds.

9. Check the library:

   ```bash
   bilbo check
   ```

   | Exit | output | What to do |
   |---|---|---|
   | 0 | empty | report (step 10) |
   | 1 | stdout lines | fix every line that names `library/<corpus>/guide.md`; a line naming a source file goes in the report unfixed. Then run `bilbo check` again, at most three runs in all. Leave lines for notes and other corpora alone and count them |
   | 1 | stderr `bilbo: no store at <root>` | print it and stop |
   | 2 | anything | print bilbo's first stderr line and stop |

   If lines still name the corpus after the third run, report them.

10. Report. Give the source's absolute path, its id, its `<corpus>/<name>`, whether bilbo fetched it or it is `capture: external`, the `--keep` ranges, the stage warnings you left unresolved, and how many `bilbo check` lines you left for other files. When you stopped before `land` succeeded, say what stopped you and claim no source.

## Never

- Use WebFetch or a browser tool for the text.
- Write or Edit a source or a `capture.md`, or type source text into any file.
- Use `curl` for anything but a PDF.
- Read a landed source with Read: its body comes only through `bilbo library read`.
- Run `land --replace` or `--force` without the user's yes.
- Set an environment variable in a command: a `VAR=x bilbo ...` prefix falls outside `allowed-tools`.
- Chain commands with `;`, `&&` or a pipe.
