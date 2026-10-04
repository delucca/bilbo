# Reader brief

Loaded by the reference skill on every run for Support judgment, and, when a plan has two to six partitions and the Agent tool is available, for the reader prompt. The block under "Reader prompt" is a complete prompt: fill the `<...>` placeholders, paste the common block where it is named, and send it as a `general-purpose` agent's prompt, named `reader-<k>` so a follow-up can continue it. Spawn every reader of one plan in one turn.

A reader shares nothing with the calling session. Give each reader exactly its own partition's slices: two readers on the same slices are one expensive reader. Other skills paste only the sections they need, such as Reading rules and Citation rules, or Support judgment for a verifier, and keep their own output contract.

## Common block

Paste verbatim into every reader prompt.

```
## Reading rules

Your slices are the whole of what you read and the whole of what you answer for. Read each
one through `bilbo library read <plan> <slice>`, one call per slice, in order. Check every result:
the header, line numbers with no gap, and the `-- end slice` line. A result that is cut is
read again with `bilbo library read <plan> <slice> --part 1/2` and `--part 2/2`, then in quarters.
Run no other command but `bilbo cite`: no file reads, no search, no directory listing, no
spawning. Other readers hold the other slices; do not guess what they contain. A slice bilbo
refuses goes in `not read:` with bilbo's message.

## Citation rules

Answer only from your slices. Make at most ten claims, the ones that answer the question best.
Each claim carries one citation:

    bilbo:<id>#<anchor> "<quote>"

- `<id>` is the one in the slice header.
- `<anchor>` is the heading path of the deepest section that holds the quote. The `-- in:`
  line gives it at the slice's first line, and each heading line in the slice opens a section
  below or beside it. A trailing part of the path is enough when it names one section. A
  source with no heading below its title is cited without an anchor.
- `<quote>` is copied from the slice without its line number and tab, never recalled or
  paraphrased: the clause that carries the claim, normally ten to twenty-five words, never
  fewer than six. `...` may join two fragments of one section, in order.

The citations are checked mechanically: the quote must sit under the anchor you name, and
you must have printed it. Silence is said, not filled: a claim the slices do not support is
not made, and the answer says the slices are silent on it. Where two passages disagree, give
both claims with their citations and say they disagree.

## Support judgment

For each claim, read its quote, and the lines around it when the quote is short or hedged.
Decide one of three:

- supports: the quote states the claim, or the claim is a faithful narrowing of it;
- supports in part: the quote states a weaker or narrower claim. Rewrite the claim to what
  the quote says;
- does not support: drop the claim.

A quote does not support a claim when it is on the topic but does not state it; when the
claim drops a condition, a version or a hedge the quote carries ("usually", "before Go 1.22",
"in this example"); when the claim turns an example into a rule; when the quote states the
opposite, or a position the source sets out in order to reject; or when the source marks it
as deprecated or superseded. Judge from the quote and its section, never from what you know
of the subject.

## Output contract

    1. <one-sentence claim>
       bilbo:<id>#<anchor> "<quote>"
    2. ...

Then one or two sentences on what the slices do not answer, or "the slices cover the
question". Before you report, check the claims, and fix or drop every one that fails:

    bilbo cite --plan <plan> <<'EOF'
    <your claims, as written above>
    EOF

Then close with exactly two lines:

    read: slices <the slices you read in full, comma-separated>
    not read: slices <the rest> (<why>), or "none"

## Boundaries

Read-only: no edits, no writes, and no command beyond `bilbo library read` and `bilbo cite`.
```

## Reader prompt

```
You are reading library sources to answer one question.

Question: <THE QUESTION, verbatim from the caller>

Plan: <the plan id>
Partition <k> of <P>. Your slices: slices <a>-<b>. Read them with
`bilbo library read <plan> <slice>`, one call each.

<One line per source: why it was picked, from the reference skill's picks.>

[COMMON BLOCK]
```

## Follow-ups

A follow-up question on the same slices goes to the same reader by name, with the question and the instruction to keep the same rules and closing lines. A reader whose report has no `read:` line, or names a slice of its partition in neither line, is asked once by name to finish. If it still does not, its slices are named in the answer as read by a reader that did not report.
