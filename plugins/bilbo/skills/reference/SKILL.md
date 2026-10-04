---
name: reference
description: Answers a question from the library's sources in the bilbo store, read in full through a plan and cited by id. Use for "what does the Go book say", "check the sources on X", "what do the docs say about Y". NOT for notes (recall) or adding a source.
license: Apache-2.0
allowed-tools: Bash(command -v bilbo), Bash(bilbo library *), Bash(bilbo cite *), Read, Agent(general-purpose), SendMessage
---

# reference

Answers a question from the library's sources: picks the sources that bear on it, reads them whole through `bilbo library plan` and `bilbo library read`, and checks every citation with `bilbo cite`. It writes no file and reads no source any other way.

## Steps

1. Check that bilbo is installed:

   ```bash
   command -v bilbo
   ```

   When it prints nothing, stop with exactly this line and do nothing else:

   ```
   reference: bilbo is not on PATH; install the bilbo CLI first
   ```

2. List the corpora:

   ```bash
   bilbo library
   ```

   - A corpus the user named is used as given. When `bilbo library` does not list it, show the list and stop.
   - Otherwise pick the corpora whose name and title fit the question, at most three.
   - An empty listing, or no corpus that fits: say so, name the corpora, and stop. Do not answer from memory.

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty | go on |
   | 1 | `bilbo: no store at <root>` | print it and stop. Never set `BILBO_HOME` yourself: a `VAR=x bilbo ...` prefix is outside `allowed-tools` |
   | 1 or 2 | anything else | print bilbo's first stderr line and stop |

3. Pick the sources. Read each chosen corpus's guide:

   ```bash
   bilbo library <corpus>
   ```

   A large corpus prints more than one tool result holds. When your tool saves the output to a file and shows only a preview, Read that saved file whole: it is the guide as bilbo printed it. When the output comes back cut instead (a truncation notice, or entries missing from the middle), pick from what arrived and name the corpus as cut in the answer.

   Pick by the entry prose, and only the sources that answer this question, even when the whole corpus would fit. A source whose facts line says `catalog` is picked by section only:

   ```bash
   bilbo library show '<corpus>/<name>#<anchor>'
   ```

   Use the name the question gives as the anchor. The section that resolves becomes the pick, as `<corpus>/<name>#<anchor>`. With no name to look up, the catalog is listed as `lookup only, not searched`. `show` exits 1 on a catalog when stderr lists several matching heading paths (ambiguous): take the full heading path that fits as the anchor. When no section matches, list the catalog as `lookup only, not searched`.

   Post the picks as a message of its own, before any plan, then go on:

   ```
   picks from <corpus> (<k> of <n> sources):
   - <name or name#anchor>: <one clause on why>
   ```

   `<n>` is the corpus's source count from `bilbo library`. Post one block per corpus.

4. Plan the reads, once, with every pick. First look at your own tool list. When it holds a tool named `Agent` (Claude Code), plan with the defaults:

   ```bash
   bilbo library plan '<ref>'...
   ```

   When it holds no tool named `Agent` (Codex, or any session without subagents), your shell may cut output near 10,000 bytes, so plan smaller slices:

   ```bash
   bilbo library plan --slice-bytes 8000 --slice-lines 250 '<ref>'...
   ```

   Those two numbers are the only slice limits this skill sets.

   The output names the plan, its `tokens:` and its `partitions:`, one `partition <k>: slices <a>-<b>` line each, then a row per slice. Decide by `partitions:`:

   | Partitions | Tool | What to do |
   |---|---|---|
   | 1 | any | read it yourself (step 5) |
   | 2 to 6 | Agent | one reader per partition (step 5) |
   | more than 6 | Agent | show the partition lines and ask the user to narrow the question or accept a sample, the first six partitions |
   | 2 or more | none | read the slices in order while the tokens read stay within 100,000; the rest is not read |

   Exit 1 with a catalog named: pick a section of it (step 3). Any other exit 1 or 2: print bilbo's first stderr line and stop.

5. Read every slice you read yourself, in order, one slice per call; a partition is read whole, and only the no-`Agent` budget of step 4 stops early:

   ```bash
   bilbo library read <plan> <slice>
   ```

   A result without its `-- end slice` line, with a gap in its line numbers, or with a truncation notice is read again in parts, then in quarters if a part is cut too. A tool can drop the middle and keep the end line, so check the numbers even when the end line is there, and never open a file a tool saved the output to:

   ```bash
   bilbo library read <plan> <slice> --part 1/2
   ```

   then `--part 2/2`. If a half is still cut, read the slice as quarters, `--part 1/4` to `--part 4/4`. Exit 1 (the source changed, the plan is gone): name the slice and bilbo's message in the answer, and make a new plan only when the user agrees.

   With readers: Read `references/reader.md`, fill its reader prompt once per partition (the question, the plan, `slices <a>-<b>`, one line on why each source was picked), and spawn every reader in one turn as a `general-purpose` agent named `reader-<k>`. A reader that returns without a `read:` line gets one follow-up, by name, to finish. If it still does not report, say so in the answer.

6. Cite. Draft from what was read, each claim with one citation, `bilbo:<id>#<anchor> "<quote>"`: the id from the slice header, the anchor is the heading path from the `-- in:` line and the headings in the slice, the quote copied from the slice without its line number. Then check the whole draft, with every plan of the run (`--plan <plan>` once per plan):

   ```bash
   bilbo cite --plan <plan> <<'EOF'
   <the draft>
   EOF
   ```

   | Verdict | What to do |
   |---|---|
   | `ok` | keep |
   | `quote_elsewhere`, `ambiguous` | take the heading path from the detail as the anchor |
   | `too_short` | lengthen the quote, from the same section, to six words or more |
   | `quote_missing`, `anchor_missing` | read the slice again and copy the quote again; if it still fails, drop the claim |
   | `unread` | read the slice the detail names, then check again; a source in no plan: drop the claim |
   | `id_missing` | drop the claim |

   Run it again after every change, until the exit code is 0 and every verdict is `ok`.

7. Judge support. Apply the "Support judgment" section of `references/reader.md` to every claim: keep it, rewrite it to what its quote says, or drop it. Run `bilbo cite` once more when anything changed.

8. Answer. In this order:
   - the answer, from what was read, each claim with its citation;
   - one line naming the catalogs left as `lookup only, not searched` and the readers that did not report, only when there are any;
   - the picks block from step 3;
   - the `citations:`, `coverage:` and `picked:` lines of the last `bilbo cite` run, word for word, last.

## Never

- Read or search any file in the store with Read, `cat`, `grep` or another tool: a source comes only through `bilbo library read`, a guide only through `bilbo library <corpus>`. Read is for `references/reader.md` and a saved `bilbo library <corpus>` output only.
- Run `bilbo library show` on a catalog without an anchor: its outline alone can run to hundreds of KB.
- Guess a source name: use the names `bilbo library <corpus>` prints.
- Answer from memory, or cite a passage nobody read in this run.
- Write any file.
- Set an environment variable in a command.
- Chain commands with `;`, `&&` or a pipe.
- Write the `coverage:` or `picked:` lines yourself.
