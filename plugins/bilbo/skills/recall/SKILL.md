---
name: recall
description: Searches the notes in the bilbo store with `bilbo recall` and shows the best passages. Use for "what do we know about X", "did we decide", "recall X". NOT for writing a note.
license: Apache-2.0
allowed-tools: Bash(command -v bilbo), Bash(bilbo recall *), Read
---

# recall

Finds the notes earlier sessions wrote, through `bilbo recall`, and shows the hits. It reads nothing else.

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

   - Put the words after `--` in single quotes, so a word starting with `-` stays a word. Write a `'` inside them as `'\''`.
   - `--kind K` passes through as given, repeated when the user named several kinds. The kinds are `plan`, `spec`, `design`, `decision`, `gotcha`, `research`, `review`, `report` and `reference`.
   - Add `--limit N` only when the user asked for a number of hits. The default is 10.
   - Run it as one command, with nothing chained: no `;`, `&&`, pipe or `echo $?`. The tool result already reports a non-zero exit code, and a chained command falls outside `allowed-tools` and asks for approval.

3. Act on the exit code and stderr:

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty | render the hits (step 5) |
   | 1 | `bilbo: no notes match` | retry (step 4) |
   | 1 | `bilbo: no store at <root>` | the store root comes from the user's environment: `BILBO_HOME`, else `$XDG_DATA_HOME/bilbo`, else `$HOME/.local/share/bilbo`. Never set `BILBO_HOME` yourself: a `VAR=x bilbo ...` prefix is outside `Bash(bilbo recall *)`. Report the path and stop |
   | 1 | anything else | print bilbo's first stderr line, and stop |
   | 2 | a usage error | print bilbo's first stderr line, and stop |

4. bilbo matches whole words, ignoring case and accents, with no stemming or synonyms: `notes` does not find `note`, and a Portuguese query does not find an English note. When nothing matched, run at most two more queries in the words the note itself would likely use: the singular or plural form, the other language (English or Portuguese), the note's likely title. Keep the user's options. When none matches, say that nothing matched, name the queries tried, and stop.

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

6. Offer to Read a hit. Read it only when the user picks one.

There is no other search path. When bilbo is missing or fails, say so and stop; never search the notes another way.
