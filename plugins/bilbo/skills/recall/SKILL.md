---
name: recall
description: Searches the notes in the bilbo store with `bilbo recall` and shows the best passages; also finds passages in the library. Use for "what do we know about X", "did we decide", "recall X". NOT for writing a note (note) or answering from sources (reference).
license: Apache-2.0
allowed-tools: Bash(command -v bilbo), Bash(bilbo recall *), Read
---

# recall

Finds the notes earlier sessions wrote, or on request the passages of the library's sources, through `bilbo recall`, and shows the hits. It reads nothing else.

## Steps

1. Check that bilbo is installed:

   ```bash
   command -v bilbo
   ```

   When it prints nothing, stop with exactly this line and do nothing else:

   ```
   recall: bilbo is not on PATH; install the bilbo CLI first
   ```

2. Run the query with the user's words verbatim, plus the options the user asked for:

   ```bash
   bilbo recall [--kind K]... [--limit N] -- '<the user's words>'
   ```

   - Search the notes. Search the library instead, with `--library`, only when the user asks what the library's sources say, asks to look a term up in the library, or names a corpus:

     ```bash
     bilbo recall --library [--corpus C]... [--limit N] -- '<the user's words>'
     ```

     Add `--corpus C` for each corpus the user named. `--kind` never goes with `--library`. For a library query, put after `--` the term the user wants looked up, without the words that only name the library or the corpus.
   - Put the words after `--` in single quotes, so a word starting with `-` stays a word. Write a `'` inside them as `'\''`.
   - `--kind K` passes through as given, repeated when the user named several kinds. The kinds are `plan`, `spec`, `design`, `decision`, `gotcha`, `research`, `review`, `report` and `reference`.
   - Add `--limit N` only when the user asked for a number of hits. The default is 10.
   - Run it as one command, with nothing chained: no `;`, `&&`, pipe or `echo $?`. The tool result already reports a non-zero exit code, and a chained command falls outside `allowed-tools` and asks for approval.

3. Act on the exit code and stderr. Judge exit 1 by the last stderr line: warning lines can come before it.

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty | render the hits (step 5) |
   | 0 | warning lines | render the hits (step 5), then pass the warnings on in one sentence: `embedder unavailable (...); keyword results only` means the hits come from keywords alone, `<n> passages not indexed; run bilbo index` means the newest notes were matched by keywords only |
   | 1 | last line `bilbo: no notes match` or `bilbo: no sources match` | retry (step 4) |
   | 1 | `bilbo: no library at <root>` | print it and stop. Do not search the notes instead |
   | 1 | `bilbo: no store at <root>` | the store root comes from the user's environment: `BILBO_HOME`, else `$XDG_DATA_HOME/bilbo`, else `$HOME/.local/share/bilbo`. Never set `BILBO_HOME` yourself: a `VAR=x bilbo ...` prefix is outside `Bash(bilbo recall *)`. Report the path and stop |
   | 1 | anything else | print bilbo's first stderr line, and stop |
   | 2 | a usage error | print bilbo's first stderr line, and stop |

4. bilbo matches whole words, ignoring case and accents, with no stemming: `notes` does not find `note`. When an embedder is configured it also matches by meaning, across wording and languages; without one, or when it is unavailable, a Portuguese query does not find an English note. When nothing matched, run at most two more queries in the words the note or source itself would likely use: the singular or plural form, the other language (English or Portuguese), the note's likely title or the source's likely heading. `--library` matches by keyword only, whatever the embedder: reword a library miss in the words and the language the source uses. Keep the user's options. When none matches, say that nothing matched in the notes or in the library, whichever was searched, name the queries tried, and stop.

5. bilbo prints one block per note, best first, blocks separated by a blank line:

   ```
   <absolute path>:<line>	<kind>	<created, or - when invalid>
   <heading path>
   <snippet, or - when the passage has no text>
   ```

   First say which query produced the hits: the user's words, or the reworded query from step 4. Then render each block as one line, then its snippet indented under it:

   ```
   <path>:<line>  <kind>  <created>  <heading path>
       <snippet>
   ```

   When the snippet is `-`, show nothing under the line.

   With `--library`, a block has a different first line, with the section's line range as its fourth column:

   ```
   <absolute path>:<line>	<source or guide>	<reference>	<start>-<end>
   <heading path below the title, or - for the title passage>
   <snippet, or - when the passage has no text>
   ```

   The reference is `<corpus>/<name>` for a source and `<corpus>` for a guide. Render it the same way:

   ```
   <path>:<line>  <kind>  <reference>  <start>-<end>  <heading path>
       <snippet>
   ```

6. Library hits only locate sources, so never answer from the snippets. When the user only asked to find passages, show the blocks (step 5) and offer to run the `reference` skill on them, then stop. Run the `reference` skill otherwise, with the user's question and each hit as a pick:
   - a source hit: `<corpus>/<name>#<heading path>`, from the reference and the heading path as printed;
   - a hit whose heading path is `-`: `<corpus>/<name>`;
   - a `guide` hit: its reference is the corpus and its heading path starts with the entry's name, which is the source's name, so the pick is `<reference>/<first part of the heading path>`: a `guide` block for `go` with the heading path `errors` is the pick `go/errors`, and `modules-reference > When to read` is `go/modules-reference`;
   - a `guide` hit whose heading path is `-` matched the guide's lead and names no source: leave it out.

7. For note hits, offer to Read a hit. Read it only when the user picks one.

There is no other search path. When bilbo is missing or fails, say so and stop; never search the notes another way.
