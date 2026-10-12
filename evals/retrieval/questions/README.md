# The questions

Each question asks for something one project's sessions settled: what the developer decided, was told, tried or left
open. Its answer is a list of facts, each pointing at the session lines that state it, and holds at the end of the
project's sessions, since every store is built from all of them.

## Files

- `questions.jsonl`: the 115 questions, one per line. Committing it freezes it.
- `dropped.jsonl`: the 451 questions that failed a check or the audit, with the reason.
- `classes.json`: the classes, with the weight and the target count of each.

## The classes

| Class | Asks | Facts |
|---|---|---|
| `overview` | what is known about a topic | 3 to 6, from two sessions or more |
| `status` | where the work on something stopped, and what is left | the last state and the open items |
| `decision` | what was chosen, and why; what replaced what | the choice and its reason |
| `rules` | what to watch out for, how things are done in the project | the rule and when it applies |
| `procedure` | how to do something in the project | the steps, in order, with their commands |
| `fact` | a value: a port, a variable, a version, a setting | one or two, with the exact value |
| `refind` | when, or in which session, something happened | the session's date and one detail that identifies it |

A class's `weight` in `classes.json` is how many of 317 real memory queries asked for it. They are the queries that
coding agents and their user sent to their memory tools over a month, September to October 2026. A strategy's score
weights its class scores by them, so the classes count as much as real use asks for them. `target` is how many
questions a class should get: its weight's share of the 225 answerable questions, so the questions themselves follow
real use. The score keeps the weights, so a class that comes up short still counts at its share. A class with fewer
than 10 questions is reported on its own and left out of the headline, whose weights then rescale over the others.
`no_answer_target` is the number of questions with no answer.

Out of scope: questions about written sources (41% of all the real queries), and personal or home facts, which no
project's sessions hold.

## The counts

| Class | Target | Questions | iptvnator | kimi-cli | linear-cycle-drain | readaloud | port-cli |
|---|---:|---:|---:|---:|---:|---:|---:|
| `overview` | 61 | 21 | 3 | 3 | 6 | 5 | 4 |
| `status` | 29 | 18 | 4 | 0 | 4 | 4 | 6 |
| `decision` | 38 | 14 | 3 | 1 | 7 | 0 | 3 |
| `rules` | 38 | 9 | 3 | 0 | 0 | 5 | 1 |
| `procedure` | 24 | 4 | 0 | 0 | 1 | 2 | 1 |
| `fact` | 27 | 17 | 0 | 2 | 7 | 6 | 2 |
| `refind` | 8 | 8 | 1 | 1 | 2 | 2 | 2 |
| answerable | 225 | 91 | 14 | 7 | 27 | 24 | 19 |
| no answer | 25 | 24 | 5 | 5 | 5 | 4 | 5 |
| all | 250 | 115 | 19 | 12 | 32 | 28 | 24 |

- The headline covers `overview`, `status`, `decision` and `fact`: 70 questions, weighted 85, 41, 54 and 38.
- `rules` (9), `procedure` (4) and `refind` (8) have fewer than 10 questions each and are reported on their own.
- The no-answer questions look like `fact` (14), `decision` (8), `procedure` (1) and `refind` (1) questions.
- Every question is drafted; none is harvested (below).

The classes came up short because the audit (below) failed every question whose answer the project's files hold,
and much of what a session settles ends up in a file: a decision's reason in a code comment, a rule in `AGENTS.md`, a
procedure in the README. Besides `refind` (8 of 8), `status` and `fact` kept the most of their targets (62% and 63%),
`procedure` and `rules` the least (17% and 24%). readaloud ships no `decision` question, and kimi-cli no `status`, `rules` or `procedure` question.

## A question

```json
{
  "id": "port-cli-017",
  "repo": "port-experimental__port-cli",
  "class": "status",
  "abstain": false,
  "source": "drafted",
  "prompt": "can we pick up the users staged import where we left it?",
  "query": "users staged import progress pending",
  "facts": [
    {
      "id": "f1",
      "text": "...",
      "keys": ["..."],
      "evidence": [{ "session": "81d8640e-ca5d-4e57-b521-788834f2da1f", "lines": [120, 188], "quote": "..." }]
    }
  ],
  "stale": [
    {
      "id": "s1",
      "text": "...",
      "replaces": "f1",
      "evidence": [{ "session": "d0dbaa44-f91f-4495-b738-5e12e8baef63", "lines": [40, 52], "quote": "..." }]
    }
  ]
}
```

- `id`: the project's name and a number, never reused.
- `repo`: the project's folder in the corpus.
- `class`: one of `classes.json`. A question with no answer has the class it looks like.
- `abstain`: `true` when the sessions hold no answer.
- `source`: `drafted`, or `harvested` when it comes from a real moment where the developer reached back to an earlier
  session. A harvested question has `asked`, the evidence of that moment.
- `prompt`: what a developer would type at the start of a later session, without naming the project.
- `query`: the search an agent would run for that prompt.
- `facts`: what the answer must contain. Every fact is required. `keys` are exact strings a fact holds, when it has
  any. `procedure` lists its facts in step order.
- `stale`: older values a later session replaced, each naming the fact that `replaces` it. Status and decision
  questions carry them most.
- `near`: on a question with no answer only, what looks like its answer and is not, with its evidence.
- `evidence`: a session's id (the last part of its file name), the first and last line of its file, both counted from
  1, and a quote of the text there, decoded from JSON. The quote must be found in those lines with whitespace
  collapsed.

## What a question is not

A question is dropped, and written to `dropped.jsonl` with its `reason`, when:

- `in_repo`: the project's files, at the commit of its last session, state its facts, or all of them but what is
  generic, guessable or not asked. An agent would open the file. `detail` names the file and line. A file that holds
  only the names, and not the reason, the order or what was tried, does not count. Code comments, configs, docs and
  the agent instruction files count as files; the git log does not.
- `unsupported`: its evidence does not state a fact, or a question with no answer is answered.
- `not_current`: a later session, or a commit made between sessions, changed the answer.
- `not_unique`: more than one answer fits.
- `guessable`: a model with no memory of the sessions answers it.
- `duplicate`: another question asks the same.

A drop found by the audit has a `detail` that starts with `audit:`.

| Reason | At mining | At the audit | All |
|---|---:|---:|---:|
| `in_repo` | 45 | 116 | 161 |
| `unsupported` | 144 | 19 | 163 |
| `not_current` | 24 | 24 | 48 |
| `not_unique` | 23 | 18 | 41 |
| `guessable` | 6 | 28 | 34 |
| `duplicate` | 1 | 3 | 4 |
| all | 243 | 208 | 451 |

## How the questions were made

One script made every model call, with the prompts below, frozen before mining began. Claude Opus 5.5 read, drafted
and wrote; gpt-6.1-sol, from another model family, checked; Claude Opus 5.5 reviewers with tools audited.

1. **Views.** Each session is shown with the line numbers of its raw file. A long payload shows its head and its
   tail, and an overflow call reads each cut middle whole, so a fact that lives only there is kept.
2. **The ledger.** Opus reads a project's sessions in time order, a chunk at a time, with a digest of the ledger so
   far. It lists what they settled (decisions, rules, procedures, values, the state of the work, events), grouped by
   piece of work, each with a quote and its lines. A script checks every quote at its lines, and a repair call
   re-finds the ones it misses. A last call merges duplicate pieces of work, fixes what replaced what, and says how
   each piece of work ended.
3. **Topics.** An overview needs a topic that several pieces of work touched: a component, a screen, a practice, with
   three items or more that still hold, from two sessions or more. One call finds the topics, and one per topic links
   what replaced or repeated what across its pieces of work.
4. **Drafting.** Opus drafts from packets of the ledger: one per piece of work, one per topic and one for the
   project. Each packet ends with the project's `CLAUDE.md`, `AGENTS.md` and README at the end of its sessions, and
   nothing they state is drafted. Topic packets give `overview` questions, the others any other class. Questions with
   no answer are drafted as close neighbours of what the project settled: the other flag, the second port, the option
   left open.
5. **Selection.** Each cell (a class in one project) takes candidates in a seeded order. A candidate is passed over
   when a question already in the set rests on the same ledger item, when its subject is unclear, or when its subject
   is in the agent's words and a fifth of the cell already is.
6. **Blind writers.** One call sees only the class and the subject, in the developer's words, and writes the
   `prompt`. Another sees only the prompt and writes the `query`. Neither sees the answer.
7. **Checks.** gpt-6.1-sol checks each question:
   - that each fact's own lines state it;
   - that no later line in any session changes it;
   - that only one answer fits;
   - that the prompt asks what the facts answer;
   - that the project's files do not state it, read at the commit of each of its last sessions from a search for its
     keys and names;
   - that an answer written with no access to the project does not hold two thirds of its facts.
   A question with no answer is checked against every line that names what it asks. A failure that points at other
   lines or a later change gets one repair call, then the checks again.
8. **Filling.** Selection, writers, checks and repair run batch after batch until each cell has one and a half times
   its count of passing questions or runs out of candidates. A class that runs out gets one more round of drafting.
   The first passing questions in seeded order fill the cell; the rest are spares.
9. **The audit.** Every question and every spare, 339 in all, went to an Opus reviewer that read its sessions and the
   project's files at the commits where its sessions ended, git history included. A failing question was replaced by
   the next passing spare in seeded order. Adjudicators re-ran the checks on the verdicts that decided what ships, and
   a last reviewer rebuilt each project's output with its own script. The audit failed 208: the files held the answer
   in code comments, configs, design docs, a vendored API spec or the last session's own plans (116), and commits
   made between sessions changed answers that no session shows. 16 audited spares passed and did not ship, since
   their cells were full.

No question is harvested. The ledger found moments where the developer reached back to an earlier session, but
none became a question: the subject of each sat in a later session than its answer, the prompt writer marked it
unclear, or a drafted question already rested on its answer.

The audit is the gate because the checks alone were not enough. In three pilot rounds on iptvnator, the reviewers
failed 50%, 29% and 40% of the questions that had passed every check: answers in the files, changes made in git,
facts their lines did not state.

## The prompts

The prompts, word for word. `{{name}}` marks what the script fills in: the ledger so far, a chunk of a session, a
packet, the lines a check reads. Opus answered every prompt except the five whose name starts with `val_`, which
gpt-6.1-sol answered. Every answer is a JSON object in the schema at the end of its prompt.

`{{classes}}` in `draft.md` is one sentence. For a topic packet: "This packet is a topic that several pieces of work
touched: draft only `overview` candidates from it, each telling what is known about the topic now, with facts from two
sessions or more." For any other packet: "Draft any class except `overview`: overview questions come only from the
topic packets." In the extra round for a class that ran out, it names the classes ("Draft only `fact` candidates from
this packet: the other classes are done. Do not repeat a question a candidate of another class would ask."), and adds
the subjects the packet already gave and the ledger items a question already rests on.

<details>
<summary><code>overflow.md</code>: reads the middle of each long payload that a view cut</summary>

````text
You read long payloads from the recorded coding sessions of one software project: tool results, long messages the
developer pasted, and long documents or plans the agent wrote. The pass that reads the sessions in order sees only the
head and the tail of each payload. Your job is the middle: list what the payload settles that a developer would later
want to recall, so that no fact that lives only there is lost.

Each payload is shown whole, under a header with its id, its session, its line and what produced it, with the tool
call that produced it and the agent's text just before and after it, when there are any. Those lines start with
`L<n>`, the line number of the raw session file.

## What to record

An item is one of:

- `decision`: a choice between options: what was chosen, why when it is stated, and what it replaced.
- `rule`: how things are done in this project, or what to watch out for, and when it applies.
- `procedure`: how to do something in this project: the steps in order, with their exact commands.
- `fact`: a value of this project: a port, a variable, a version, a path, a setting, a flag, a limit, an identifier.
- `state`: where a piece of work stands: what is done, what is left, what is blocked.
- `event`: something that happened that a developer may later need to place: a release, a merge, a migration, an
  incident, a bug found and its cause.

`standing` says how settled it is: `decided` (the developer chose or required it), `done` (the work did it and it held),
`proposed` (suggested, not accepted), `guessed` (not checked). A plan the agent wrote, and what a pull request under
review or a diff proposes, is `proposed` until it is accepted or merged; a test run's output is `done`.

Record only what lives in the cut middle or needs it to be understood, and only what matters later: a value, a cause, a
decision, a result. Skip general programming knowledge, file contents that are only code, log noise and anything a
reader would not ask about. Many payloads settle nothing: return no item for them.

Write `statement` as one sentence that stands alone: it names the project's thing and what was settled. Put the exact
value in `value` (copied as written), a stated reason in `reason`, a procedure's commands in `steps` in order, what is
left of open work in `status`, and a few words naming the topic in `topic`. Use empty strings and empty lists for the
fields that do not apply.

## Evidence

Every item names the payload it came from (`cut`, the payload's id) and carries evidence: `lines`, the first and last
`L` number (at most 40 apart; for a quote from the payload itself, the payload's own line twice), and `quote`, at least
six words or 30 characters copied character for character from one line's text as shown. The quote holds the words or
the value that state the item; when that text is short (a line of code, a setting), extend the quote with the text
right before or after it until it is long enough (line breaks count as spaces). Never quote across lines of different
`L` numbers, and never quote text that holds a redaction placeholder such as `[REDACTED` or `<PERSON>`.

## The payloads

{{payloads}}

## Output

One JSON object with `items`, the list of what the payloads settle, in payload order.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["items"],
  "properties": {
    "items": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["cut", "kind", "topic", "statement", "value", "reason", "steps", "status", "standing", "evidence"],
        "properties": {
          "cut": {"type": "string"},
          "kind": {"type": "string", "enum": ["decision", "rule", "procedure", "fact", "state", "event"]},
          "topic": {"type": "string"},
          "statement": {"type": "string"},
          "value": {"type": "string"},
          "reason": {"type": "string"},
          "steps": {"type": "array", "items": {"type": "string"}},
          "status": {"type": "string"},
          "standing": {"type": "string", "enum": ["decided", "done", "proposed", "guessed"]},
          "evidence": {
            "type": "array",
            "minItems": 1,
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["lines", "quote"],
              "properties": {
                "lines": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2},
                "quote": {"type": "string"}
              }
            }
          }
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>ledger.md</code>: builds the ledger, one chunk of a session at a time</summary>

````text
You are reading the recorded coding sessions of one software project, in time order, one chunk at a time. You build
a ledger: what the sessions settled that a developer would later want to recall without rereading them. You get the
ledger so far as a digest, then the next chunk.

## How to read the chunk

Every block of the chunk starts with `L<n>`, the line number of the raw session file it comes from. The first block of
a line also carries its time (UTC). Then comes who speaks:

- `USER`: the developer's typed turn, or text their tools put in the developer's turn.
- `ASSISTANT`: the coding agent's text.
- `TOOL CALL <name>`: what the agent ran. String inputs are shown as `key: value`, decoded.
- `TOOL RESULT <name>`: what came back.
- `COMPACT SUMMARY (not evidence)`: the agent's own summary of earlier context. Use it to follow the work, never as
  evidence.

A long payload shows its head and its tail with `[... N chars cut ...]` between them. When the cut text went to a
separate overflow pass, the marker reads `[... N chars cut; overflow <id> ...]` and is followed by the items that pass
found in the cut text, each with an id `o<number>`.

## What to record

An item is one of:

- `decision`: a choice between options: what was chosen, why when it is stated, and what it replaced.
- `rule`: how things are done in this project, or what to watch out for, and when it applies.
- `procedure`: how to do something in this project: the steps in order, with their exact commands.
- `fact`: a value of this project: a port, a variable, a version, a path, a setting, a flag, a limit, an identifier.
- `state`: where a piece of work stands: what is done, what is left, what is blocked.
- `event`: something that happened that a developer may later need to place: a release, a merge, a migration, an
  incident, a bug found and its cause.

Record what the sessions settled: what the developer stated or asked for, and what the work confirmed (a command that
ran, a fix that held, a test that passed, an answer the agent checked). `standing` says how settled an item is:

- `decided`: the developer chose it or required it.
- `done`: the work did it and it held.
- `proposed`: suggested and not accepted yet.
- `guessed`: the agent's guess, not checked.

What a pull request under review, a diff or a plan proposes is `proposed` until it is accepted or merged. Record a
proposal or a guess only when it matters later (an option the developer left open, a guess that work rests on), and
never word it as settled.

Skip general programming knowledge any developer has, the mechanics of one edit that nobody would ask about later, the
task text restating itself, and anything you can see only in a compaction summary. Be selective: a short chunk may
settle nothing; a long one rarely settles more than 25 things.

Write `statement` as one sentence that stands alone: it names the project's thing and what was settled, so that
someone who never saw the sessions understands it. Do not write "the user", "the agent", "this session" or "earlier".
Put the exact value in `value` when there is one (copied as written), the reason in `reason` when one is stated, the
commands of a procedure in `steps` in order, and what is left of open work in `status`. Use empty strings and empty
lists for the fields that do not apply.

## Workstreams

Group items by the piece of work they belong to: a feature, a bug, a migration, a release. Use a workstream id from the
digest (`w<number>`) when the item belongs to one. Otherwise give a new workstream a short name of a few words, the same
name for every item of it in this chunk. An item that belongs to no piece of work (a project-wide rule, a fact about
the setup) has the workstream `project`.

## Evidence

Every item, update and reach-back carries evidence: `lines`, the first and last `L` number of the lines that state it
(at most 40 apart, as tight as you can make them), and `quote`, at least six words or 30 characters (shorter only
when the whole block is shorter) copied character for character from one block of the chunk inside those lines.

- The quote holds the words or the value that state the item. When that text is short (a line of code, a setting),
  extend the quote with the text right before or after it in the same block until it is long enough; line breaks
  count as spaces.
- Copy the decoded text as shown, without the `L<n>`, the time, the role, `TOOL CALL <name>:` or a `key:` prefix, and
  never across a `[... cut ...]` marker.
- Never quote a compaction summary, and never quote text that holds a redaction placeholder such as `[REDACTED` or
  `<PERSON>`.
- Prefer the developer's words and the agent's prose; quote a tool call or a tool result when the value lives only
  there.

## Overflow items

Each overflow item shown at a cut already belongs to the ledger. In `overflow`, give each one shown in this chunk a
workstream (an id from the digest, a new name you also use for items here, or `project`), or set `drop` to true when it
repeats another item or does not matter. Do not repeat an overflow item as a new item.

## Updates to earlier items

When this chunk changes an item of the digest, or an overflow item, add an update with its id:

- `replaced`: a new value or choice took its place; `by` names the new item's `ref` from this answer.
- `confirmed`: it held again.
- `closed`: the open work it describes is done.
- `reopened`: work that was done or closed is open again.
- `abandoned`: the work was dropped.
- `reverted`: what it describes was undone.

`by` is an empty string except for `replaced`.

## Reach-backs

A reach-back is a developer's typed turn in this chunk that refers to work of an earlier session: "where did we leave
the importer", "like we did last time", "the crash from yesterday is back". Not a reference to earlier in the same
session, not the continuation of a compacted session, not a plan the agent wrote that the developer pasted back.
Record its evidence, what it asks in a few words, and the digest item ids it points at.

## Subjects

For each workstream this chunk works on, copy one to three spans of 2 to 15 words from a developer's typed turn that
name the topic of the work and state no value or answer, for example the words the developer used to ask for it. A
writer who never sees the sessions gets these words later. When only the agent ever names the topic (a pasted plan, a
run the developer did not steer), copy the span from `ASSISTANT` text and set `source` to `assistant`.

## The ledger so far

{{digest}}

## The chunk

{{chunk}}

## Output

One JSON object with:

- `items`: what this chunk settled. `ref` is `n1`, `n2`, ... in this answer.
- `overflow`: one entry per overflow item shown in this chunk.
- `updates`: changes to items of the digest or to overflow items.
- `reach_backs`: the reach-back turns in this chunk.
- `subjects`: spans that name the workstreams this chunk works on.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["items", "overflow", "updates", "reach_backs", "subjects"],
  "properties": {
    "items": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["ref", "kind", "workstream", "subject", "statement", "value", "reason", "steps", "status", "standing", "evidence"],
        "properties": {
          "ref": {"type": "string"},
          "kind": {"type": "string", "enum": ["decision", "rule", "procedure", "fact", "state", "event"]},
          "workstream": {"type": "string"},
          "subject": {"type": "string"},
          "statement": {"type": "string"},
          "value": {"type": "string"},
          "reason": {"type": "string"},
          "steps": {"type": "array", "items": {"type": "string"}},
          "status": {"type": "string"},
          "standing": {"type": "string", "enum": ["decided", "done", "proposed", "guessed"]},
          "evidence": {
            "type": "array",
            "minItems": 1,
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["lines", "quote"],
              "properties": {
                "lines": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2},
                "quote": {"type": "string"}
              }
            }
          }
        }
      }
    },
    "overflow": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["item", "workstream", "drop"],
        "properties": {
          "item": {"type": "string"},
          "workstream": {"type": "string"},
          "drop": {"type": "boolean"}
        }
      }
    },
    "updates": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["item", "action", "by", "note", "evidence"],
        "properties": {
          "item": {"type": "string"},
          "action": {"type": "string", "enum": ["replaced", "confirmed", "closed", "reopened", "abandoned", "reverted"]},
          "by": {"type": "string"},
          "note": {"type": "string"},
          "evidence": {
            "type": "array",
            "minItems": 1,
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["lines", "quote"],
              "properties": {
                "lines": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2},
                "quote": {"type": "string"}
              }
            }
          }
        }
      }
    },
    "reach_backs": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["asks", "items", "evidence"],
        "properties": {
          "asks": {"type": "string"},
          "items": {"type": "array", "items": {"type": "string"}},
          "evidence": {
            "type": "object",
            "additionalProperties": false,
            "required": ["lines", "quote"],
            "properties": {
              "lines": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2},
              "quote": {"type": "string"}
            }
          }
        }
      }
    },
    "subjects": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["workstream", "line", "quote", "source"],
        "properties": {
          "workstream": {"type": "string"},
          "line": {"type": "integer"},
          "quote": {"type": "string"},
          "source": {"type": "string", "enum": ["user", "assistant"]}
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>repair_quotes.md</code>: re-finds the evidence of ledger items whose quotes the script did not find</summary>

````text
A ledger of what a software project's recorded coding sessions settled cites each item with a line range and a quote.
A script could not find the quotes below at their lines: the quote is not a character-for-character copy, or it sits
on other lines. For each one, find where the session states the item and give lines and a quote that are exact.

Each case shows the item, the evidence that failed, and the session's lines around it. Every block of those lines
starts with `L<n>`, the line number of the raw session file.

- `lines`: the first and last `L` number that state the item, at most 40 apart.
- `quote`: at least six words or 30 characters (shorter only when the whole block is shorter), copied character for
  character from one block inside those lines. Copy the decoded text as shown, without the `L<n>`, the time, the role,
  `TOOL CALL <name>:` or a `key:` prefix, and never across a `[... cut ...]` marker. Never quote a compaction summary or
  text with a redaction placeholder such as `[REDACTED` or `<PERSON>`.
- `found`: false when the lines shown do not state the item; then `lines` is `[0, 0]` and `quote` is empty.

The quote must hold the words or the value that state the item. When the problem is the quote's length, keep the same
text and extend it with what comes right before or after it in the same block (line breaks count as spaces). Never
swap in other text that does not state the item: set `found` to false instead. Do not change what an item says; only
find its evidence.

## The cases

{{cases}}

## Output

One JSON object with `fixes`, one entry per case, by its `case` id.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["fixes"],
  "properties": {
    "fixes": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["case", "found", "lines", "quote"],
        "properties": {
          "case": {"type": "string"},
          "found": {"type": "boolean"},
          "lines": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2},
          "quote": {"type": "string"}
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>consolidate.md</code>: merges duplicate pieces of work, fixes what replaced what, and says how each ended</summary>

````text
Below is the ledger of one software project: what its recorded coding sessions settled, built one chunk at a time in
time order. Because each chunk was read with only a digest of the earlier ones, the same piece of work sometimes got
two workstreams, and some replacements were missed. Clean it up.

1. Merge workstreams that are the same piece of work under two names. Keep the oldest id and give the merged
   workstream a short name.
2. Fix replacement chains. An item is replaced only when a later item states a new value, choice or place for the
   same attribute of the same thing (an element moved out of a footer, the same setting changed again, a choice
   reversed). Look across workstreams too: a later task often changes a component an earlier task built. A change to
   another attribute of the thing (a hover value after a token fix) is not a replacement: the older item still holds.
   A later item that states the same thing again, with a scope or a detail added, is a restatement: list it in
   `repeats`, never as a replacement. Add a fix for every replacement the ledger misses (`superseded_by` the newer
   item), and for every one it states wrongly (`superseded_by` empty). Only an older item can be replaced.
3. For every workstream after the merges, say how it ended: `final_state`, one or two sentences from its latest items
   (done, left open, abandoned, blocked), and `open_items`, the items still open at the end (their ids).

Judge from the ledger only. Each workstream line gives its id, name, sessions, first and last time and its count of
items. Each item line gives its id, time, kind, workstream, standing, what replaces it or what it replaces, and its
statement.

## Workstreams

{{workstreams}}

## Items

{{items}}

## Output

One JSON object with `merges`, `chain_fixes`, `repeats` (a later item and the earlier one it restates) and
`workstreams` (one entry for every workstream left after the merges, `project` included).

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["merges", "chain_fixes", "repeats", "workstreams"],
  "properties": {
    "merges": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["into", "from", "name"],
        "properties": {
          "into": {"type": "string"},
          "from": {"type": "array", "items": {"type": "string"}},
          "name": {"type": "string"}
        }
      }
    },
    "chain_fixes": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["item", "superseded_by", "why"],
        "properties": {
          "item": {"type": "string"},
          "superseded_by": {"type": "string"},
          "why": {"type": "string"}
        }
      }
    },
    "repeats": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["item", "same_as", "why"],
        "properties": {
          "item": {"type": "string"},
          "same_as": {"type": "string"},
          "why": {"type": "string"}
        }
      }
    },
    "workstreams": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "name", "final_state", "open_items"],
        "properties": {
          "id": {"type": "string"},
          "name": {"type": "string"},
          "final_state": {"type": "string"},
          "open_items": {"type": "array", "items": {"type": "string"}}
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>topics.md</code>: groups the ledger into topics for overview questions</summary>

````text
You group the ledger of one software project into topics for overview questions. The ledger lists what the project's
recorded coding sessions settled, one item per line, grouped into workstreams. A workstream is one task; a topic sits
one level up: a component, a screen, an area of the code or a practice that several tasks touched over time (the
playlist switcher, the workspace header, the mock servers, how the running app is driven in tests).

Find every topic that:

- spans two sessions or more, through its items;
- has at least three items that still hold at the end (not replaced);
- is specific enough that one question could ask "what do we know about it" and get a bounded answer.

Rules:

- List the item ids that belong to the topic, replaced items included, so that its history can be told.
- An item can belong to more than one topic.
- Do not make a topic of a whole workstream that spans one session, of the whole project, or of a word that only
  happens to repeat (a test id, a common file name).
- Name the topic in a few plain words, as a developer would.

Each item line gives its id, its session (first 8 characters) and date, its kind, its workstream, and its statement;
replaced items are marked.

## Workstreams

{{workstreams}}

## Items

{{items}}

## Output

One JSON object with `topics`. `why` says in one sentence what holds the topic together.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["topics"],
  "properties": {
    "topics": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "items", "why"],
        "properties": {
          "name": {"type": "string"},
          "items": {"type": "array", "items": {"type": "string"}},
          "why": {"type": "string"}
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>topic_links.md</code>: links what replaced or repeated what within one topic</summary>

````text
Below are the items of one topic from the ledger of a software project: what its recorded coding sessions settled
about one component or area, in time order, across the tasks (workstreams) that touched it. Each was recorded while
reading one chunk of one session, so links between tasks are often missing. Find them.

1. **Replacements.** An item is replaced only when a later item states a new value, choice or place for the same
   attribute of the same thing (an element moved elsewhere, the same setting changed again, a choice reversed). A
   change to another attribute (a hover value after a token fix) is not a replacement: the older item still holds.
   For each replacement the ledger misses, give the older item and the newer one that replaces it. For each one it
   states wrongly, give the older item with `superseded_by` empty.
2. **Repeats.** An item that states again what an earlier item already stated (the same gotcha met again in a later
   session, a rule read again with a scope or a detail added) is a repeat, never a replacement. Give the later item and
   the earlier one it repeats.

Only an older item can be replaced or repeated. Judge from the items only. Each item gives its id, time, kind,
workstream, standing, the links it already has, and its statement.

## Topic: {{topic}}

{{items}}

## Output

One JSON object with `chain_fixes` and `repeats`, each possibly empty. `why` is one sentence.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["chain_fixes", "repeats"],
  "properties": {
    "chain_fixes": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["item", "superseded_by", "why"],
        "properties": {
          "item": {"type": "string"},
          "superseded_by": {"type": "string"},
          "why": {"type": "string"}
        }
      }
    },
    "repeats": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["item", "same_as", "why"],
        "properties": {
          "item": {"type": "string"},
          "same_as": {"type": "string"},
          "why": {"type": "string"}
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>draft.md</code>: drafts the questions of one packet</summary>

````text
You draft evaluation questions about one software project from a packet of its ledger: what the project's recorded
coding sessions settled about one piece of work, one topic that several pieces of work touched, or the project as a
whole, with the session lines that state it. Later, a developer starting a new session asks a memory system the question; the memory system holds only what it
stored from those sessions. A good question is one a developer would really need answered and could not answer by
opening the project's files or by general knowledge.

You draft the answers, not the wording a developer would type: another writer, who never sees the sessions, writes the
question's text later from its class and its subject.

## The classes

| class | asks | facts |
|---|---|---|
| `overview` | what is known about a topic | 3 to 6, from two sessions or more |
| `status` | where the work on something stopped, and what is left | the last state and the open items |
| `decision` | what was chosen, and why; what replaced what | the choice and its reason, at least two facts |
| `rules` | what to watch out for, how things are done in the project | the rule and when it applies |
| `procedure` | how to do something in the project | the steps, in order, with their commands, at least two |
| `fact` | a value: a port, a variable, a version, a setting | one or two, each with its exact value |
| `refind` | when, or in which session, something happened | exactly two: the UTC date and one detail that identifies the session |

{{classes}}

## Rules for a candidate

- **Current.** The answer must hold at the end of the sessions. Use an item's latest value; an item marked as replaced
  is never a fact. When a value changed, the old value goes in `stale`, naming the fact that `replaces` it, with the
  old value's own evidence, from an earlier line than the new value's.
- **Settled.** Only items standing `decided` or `done` become facts. A `proposed` or `guessed` item never does.
- **Facts say only what their quotes say.** A fact states what its evidence lines state, and nothing more: no file
  name, state, reason or detail that only the ledger's statement holds. When a fact needs several lines, cite each of
  them. If you cannot quote it, leave it out.
- **Atomic facts, all required.** Each fact states one thing a correct answer must contain. Do not pad a question with
  facts a correct answer could leave out.
- **Keys.** `keys` are exact strings copied from the fact's evidence that a correct answer must contain: a value, a
  flag, a command, a number with its label, a hash, a file name. Never the name of the topic itself, never a common
  word or a phrase. A key must appear in its fact's evidence lines. A fact with no such value has no keys. A `fact`
  question's facts each have a key.
- **Not general knowledge.** No fact that a programmer who never saw this project would know or guess, and no one-off
  workaround of the agent's that nobody settled as the way to do it.
- **Not in the files.** Never ask for a value the project's files store: a CSS value, a theme token, a size, a color, a
  constant, a port or setting written in the code or its config. A reader would open the file. Ask instead what the
  sessions hold and the code does not show: why a choice was made, what was tried and failed, what replaced what, what
  is left, the order of steps, a value set outside the repository. The packet ends with whichever of the project's
  `CLAUDE.md`, `AGENTS.md` and README exist at the end of the sessions: draft nothing they state (no rule, procedure,
  command, value or reason), and nothing a session wrote into one of them.
- **One answer.** A reader who found the right lines could not give a different answer.
- **Class shape.** Follow the facts column above. `procedure` facts go in step order, each step with its command when
  it has one. `overview` facts come from two sessions or more. A `refind` question has exactly two facts: the first is
  the UTC date of the line where the thing happened (`2026-03-22`, the date in that line's time; a session can run
  over several days, so never take the session's start date), with that date as its key and that line as its
  evidence; the second is one detail that identifies that session.
- **Evidence.** Copy evidence from the packet: the session id, `lines` (first and last `L` number, at most 40 apart)
  and a `quote` of at least six words or 30 characters copied character for character from one block of those lines.
  The quote holds the words or the value that state the fact; extend a short one with the text right around it.
  Never quote a compaction summary or text with a redaction placeholder such as `[REDACTED` or `<PERSON>`.
- **Subject.** A writer who never sees the sessions turns the class and the subject into the developer's question, so
  the subject decides what is asked. It names the specific thing the facts are about: the component, feature, file area
  or problem, with the aspect the question asks ("the copy button for the stream URL", "the mock server exiting under
  nx serve"). Generic words alone never do ("the style", "the design", "the hover effects", "the sidebar" when the
  project has two). No fact of the answer, no key, not the project's name. Choose it in this order:
  1. The `Subject:` of the candidate's main ledger item, as the packet shows it, when it names that specific thing:
     copy it exactly, with `source` set to `ledger`, and the session and first line of that item's evidence.
  2. Otherwise a developer's typed turn earlier in the same session that names the thing (anywhere before the
     evidence): copy the whole clause that names it, up to the next comma, full stop or the end of the turn, never a
     span cut before its point, with `source` set to `user`.
  3. Only when neither exists, the agent's prose (never a tool call's words), with `source` set to `assistant`.
  Never an anaphor ("that", "it", "the primary option"). Each candidate has its own subject; never reuse one. A
  candidate with no subject that names its specific thing is not drafted.
- **The question asks what the facts answer.** Before you keep a candidate, read its subject and class as the question
  a developer would type, and check that the facts answer exactly that question. Say in `note`, in one sentence, what
  the question asks and that the facts answer it.
- **Harvest moments.** When the packet shows a harvest moment (a developer reaching back to an earlier session), draft
  first the question that moment asked, if its answer still holds at the end: set `harvest` to the moment's id and
  use the moment's words as the subject. Otherwise `harvest` is empty.
- **Items.** List the ledger item ids the candidate rests on.

Propose every good candidate the packet supports, and no weak one: a packet may support none, and rarely more than
twelve. Two candidates of the same packet must not ask the same thing.

## The packet

{{packet}}

## Output

One JSON object with `candidates`. Each fact's id is its position (`f1`, `f2`, ...), so `replaces` names `f1`, `f2`, ...

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["candidates"],
  "properties": {
    "candidates": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["class", "harvest", "items", "subject", "facts", "stale", "note"],
        "properties": {
          "class": {"type": "string", "enum": ["overview", "status", "decision", "rules", "procedure", "fact", "refind"]},
          "harvest": {"type": "string"},
          "items": {"type": "array", "items": {"type": "string"}},
          "subject": {
            "type": "object",
            "additionalProperties": false,
            "required": ["session", "line", "quote", "source"],
            "properties": {
              "session": {"type": "string"},
              "line": {"type": "integer"},
              "quote": {"type": "string"},
              "source": {"type": "string", "enum": ["ledger", "user", "assistant"]}
            }
          },
          "facts": {
            "type": "array",
            "minItems": 1,
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["text", "keys", "evidence"],
              "properties": {
                "text": {"type": "string"},
                "keys": {"type": "array", "items": {"type": "string"}},
                "evidence": {
                  "type": "array",
                  "minItems": 1,
                  "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["session", "lines", "quote"],
                    "properties": {
                      "session": {"type": "string"},
                      "lines": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2},
                      "quote": {"type": "string"}
                    }
                  }
                }
              }
            }
          },
          "stale": {
            "type": "array",
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["text", "replaces", "evidence"],
              "properties": {
                "text": {"type": "string"},
                "replaces": {"type": "string"},
                "evidence": {
                  "type": "array",
                  "minItems": 1,
                  "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["session", "lines", "quote"],
                    "properties": {
                      "session": {"type": "string"},
                      "lines": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2},
                      "quote": {"type": "string"}
                    }
                  }
                }
              }
            }
          },
          "note": {"type": "string"}
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>noanswer.md</code>: drafts the questions with no answer</summary>

````text
You draft questions with no answer for an evaluation of memory systems. A memory system holds what it stored from one
software project's recorded coding sessions. A good no-answer question sounds like one a developer on this project
would ask, comes close to something the sessions did settle, and yet the sessions never answer it. A memory system
should say it does not know instead of offering the look-alike.

Below is the ledger of this project (its workstreams and what they settled), and the workstreams of other projects
with a few of their items. Draft {{n}} candidates, about half of each kind:

- `cross_repo`: a question natural for this project whose look-alike answer lives only in another project's sessions
  (for example this project also has a release process, but only the other project settled its release command).
  `near` names the other project and the item id of the look-alike from its list.
- `close_neighbour`: a sibling of something this project settled that was never settled itself: the other flag, the
  second port, the option discussed and left open, the environment that was never configured. `near` names this
  project and the item id of the settled sibling.

Rules:

- Each asks for one specific value: a name, a command, a path, a setting outside the code, a choice and its reason,
  a date. Never whether something exists or is supported: a definite "no" or "none" in the sessions would answer that.
- Never ask what is known about something or where its work stopped: the sessions always answer those ("what we
  have", "it stopped here"). Never ask about work the sessions flagged, skipped, deferred or left undone: a definite
  "not done" answers it.
- Never ask for a value the project's files store (a CSS value, a theme token, a constant, a config value, a command
  in `CLAUDE.md`, `AGENTS.md` or the README, a step of a workflow): the files answer it. The project's instruction
  files are shown below; draft nothing they answer.
- Never ask for something git or the hosting service records (when a pull request was merged, who committed what).
- From what the ledger shows, the sessions must not settle it, not even in passing. When you are not sure, choose
  another.
- It must not be answerable from general programming knowledge.
- Use only the classes `fact`, `decision`, `procedure` and `refind`: give each the class of the question it looks like.
- `subject`: 2 to 15 words naming what is asked, as a developer would say it, without the answer and without the
  project's name. A writer who never sees the sessions turns it into the question's text.
- `asks`: the question in one plain sentence, for the checker.
- `why_open`: one sentence on why the sessions leave it open.

## This project's ledger

{{digest}}

## Other projects

{{others}}

## This project's instruction files at the end of the sessions

{{files}}

## Output

One JSON object with `candidates`.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["candidates"],
  "properties": {
    "candidates": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "class", "subject", "asks", "near", "why_open"],
        "properties": {
          "kind": {"type": "string", "enum": ["cross_repo", "close_neighbour"]},
          "class": {"type": "string", "enum": ["decision", "procedure", "fact", "refind"]},
          "subject": {"type": "string"},
          "asks": {"type": "string"},
          "near": {
            "type": "object",
            "additionalProperties": false,
            "required": ["repo", "item", "why"],
            "properties": {
              "repo": {"type": "string"},
              "item": {"type": "string"},
              "why": {"type": "string"}
            }
          },
          "why_open": {"type": "string"}
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>prompt_writer.md</code>: writes the `prompt` from the class and the subject</summary>

````text
You write what a developer types at the start of a new coding session when they need something from earlier work on
their project. Each case below gives the kind of thing the developer needs (its class) and a few words that name the
topic, taken from what the developer once said, or from the coding agent's words when the case says so.

For each case write `prompt`: the one message the developer would type to their coding agent, in English, as people
really write them: short and direct, often in lower case, sometimes a request ("pick up the X where we left it")
rather than a question.

- Ask for what the class asks, about the topic the subject names.
- Use the subject's words for the topic where they read naturally; translate them to English when they are in another
  language.
- Do not name the project or the product.
- Add no detail the subject does not hold: no value, file name, number, flag or date. You do not know the answer, and
  a guess would leak into the question.
- Do not mention sessions, transcripts or memory, and no dates unless the class asks when something happened.
- Each prompt stands alone; do not refer to another case.
- Set `unclear` to true when the subject does not tell you which specific thing to ask about (it names no component,
  feature or problem, or it could mean several things in a project); still write your best prompt. Otherwise false.

## The classes

| class | the developer needs |
|---|---|
| `overview` | what is known about a topic |
| `status` | where the work on something stopped, and what is left |
| `decision` | what was chosen, and why; what replaced what |
| `rules` | what to watch out for, how things are done in the project |
| `procedure` | how to do something in the project |
| `fact` | a value: a port, a variable, a version, a setting |
| `refind` | when, or in which session, something happened |

## The cases

{{cases}}

## Output

One JSON object with `prompts`, one entry per case, by its `id`, each with `unclear`.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["prompts"],
  "properties": {
    "prompts": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "prompt", "unclear"],
        "properties": {
          "id": {"type": "string"},
          "prompt": {"type": "string"},
          "unclear": {"type": "boolean"}
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>query_writer.md</code>: writes the `query` from the prompt</summary>

````text
A coding agent received each developer message below. Before it answers, it searches a memory of earlier sessions on
the same project with one query. Write that query for each message.

Agents' memory searches, in general:

- They are keyword strings, not sentences: no question mark, no filler words, no greeting.
- Most are five to ten words long; few are over fifteen.
- The topic's distinctive words come first. Identifiers, file names, flags, error text and issue or pull request
  numbers are packed in when the message holds them.
- Words such as status, decision, why, steps, how, date or when are added when the message asks for them.

Rules:

- Use only what the message says or plainly implies. Never add a value, a name, a file or a detail the message does
  not hold.
- Do not add the project's name unless the message holds it.
- One query per message, in English.

## The messages

{{messages}}

## Output

One JSON object with `queries`, one entry per message, by its `id`.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["queries"],
  "properties": {
    "queries": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "query"],
        "properties": {
          "id": {"type": "string"},
          "query": {"type": "string"}
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>val_support.md</code>: checks support, currency, uniqueness and what the prompt asks</summary>

````text
You check one evaluation question about a software project against the project's recorded coding sessions. The
question comes with a reference answer: a list of facts, each with the session lines meant to state it, and sometimes
stale values, older values that a later fact replaced. Judge from the lines below only, not from what you know.

Lines are shown as `<session> L<n> <time> <who>: <text>`; `<session>` is the first 8 characters of a session id and
`L<n>` a line of that session's file. Long texts are shortened around the part that matters.

Answer four things:

1. **Asks another thing.** Read the developer's message with its class. If it asks for something other than what the
   facts answer (another aspect of the topic, another feature, a question the facts do not address), set
   `asks_other` to true and say what it asks in `reason`. Otherwise false.
2. **Supported.** For each fact, say whether its own evidence lines state it (`stated`): the lines must say what the
   fact says, its keys included, not merely make it plausible. When they do not, but another line shown here states
   it (the context around the evidence, or the other lines of the same session), give that line in `found_at`
   (session, line, and a short exact quote of the words that state it); otherwise leave `found_at` empty. Do the same
   for each stale value and its own evidence. The question is supported when every fact and every stale value is
   stated by its own evidence.
3. **Current.** The later lines are lines after the evidence, in any session, that name the question's keys, stale
   values, identifiers or topic, oldest first. The answer is current unless a later line settles a different value,
   choice or place for one of the facts, or undoes it. A later line that only repeats, quotes, mentions or discusses
   an old value does not change anything. List every change in `later_changes` (the fact's id, the session, the line,
   a short exact quote and the new value), the latest one for each fact; leave the list empty when the answer is
   current.
4. **Unique.** A reader who found the right lines could give only this answer. It is not unique only when the lines
   support a second, different answer to the same question. Give that answer in `other_answer`; otherwise leave it
   empty. A message that asks another thing is item 1, not this one.

`reason`: one or two sentences.

## The question

{{question}}

## The reference answer

{{answer}}

## Evidence lines, with the lines around them

Lines marked `>>` are the cited evidence; the others are the context 30 lines either side, shortened.

{{evidence}}

## Other lines of the same sessions that hold the facts' distinctive words

{{nearby}}

## Later lines

{{later}}

## Output

One JSON object. `found_at` uses empty strings and 0 when there is nothing to give.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["asks_other", "facts", "stale", "supported", "current", "later_changes", "unique", "other_answer", "reason"],
  "properties": {
    "asks_other": {"type": "boolean"},
    "facts": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "stated", "found_at"],
        "properties": {
          "id": {"type": "string"},
          "stated": {"type": "boolean"},
          "found_at": {
            "type": "object",
            "additionalProperties": false,
            "required": ["session", "line", "quote"],
            "properties": {
              "session": {"type": "string"},
              "line": {"type": "integer"},
              "quote": {"type": "string"}
            }
          }
        }
      }
    },
    "stale": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "stated"],
        "properties": {
          "id": {"type": "string"},
          "stated": {"type": "boolean"}
        }
      }
    },
    "supported": {"type": "boolean"},
    "current": {"type": "boolean"},
    "later_changes": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["fact", "session", "line", "quote", "new_value"],
        "properties": {
          "fact": {"type": "string"},
          "session": {"type": "string"},
          "line": {"type": "integer"},
          "quote": {"type": "string"},
          "new_value": {"type": "string"}
        }
      }
    },
    "unique": {"type": "boolean"},
    "other_answer": {"type": "string"},
    "reason": {"type": "string"}
  }
}
```
````

</details>

<details>
<summary><code>val_in_repo.md</code>: checks whether the project's files state the facts</summary>

````text
You check whether a software project's own files already answer an evaluation question. If they do, an agent would
open the file instead of asking its memory, and the question is dropped.

Below are the question, the facts its answer must contain, and excerpts of the project's files at the commits where
its sessions ended: the places where a search for the question's keys and identifiers found a match, and, for
questions about rules and procedures, the project's instruction files and README. Each excerpt is headed by the
commit, the file and the line range; its lines carry their numbers.

For each fact, say whether a file states it (`stated`), with the file, the line and a short exact quote of that line.
A file states a fact only when it says what the fact says, its value included. Configuration files (`project.json`,
`package.json`, `*.config.*`) and comments in the code count like any other text. A fact that only negates another
fact a file states (the project is called X, so it is not called Y) counts as stated too. A file that holds only the
names (a flag's name, a function, a setting's key) and not the value, the reason, the order of the steps or what was
tried does not state it. Judge from the excerpts only.

## The question

{{question}}

## The facts

{{facts}}

## The files

{{files}}

## Output

One JSON object with `facts`, one entry per fact by its id, and `reason`, one sentence. `file` and `quote` are empty
and `line` is 0 when a fact is not stated.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["facts", "reason"],
  "properties": {
    "facts": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "stated", "commit", "file", "line", "quote"],
        "properties": {
          "id": {"type": "string"},
          "stated": {"type": "boolean"},
          "commit": {"type": "string"},
          "file": {"type": "string"},
          "line": {"type": "integer"},
          "quote": {"type": "string"}
        }
      }
    },
    "reason": {"type": "string"}
  }
}
```
````

</details>

<details>
<summary><code>val_closed.md</code>: answers the prompts with no access to the project</summary>

````text
Below are messages a developer typed to a coding agent at the start of a session on the open-source project
{{repo}}. You have no access to the project's code, documentation or history, and nobody will help you.

For each message, answer it as well as you can from what you know about the project and about how such projects
usually work. Give your best concrete answer, with specific values when you can; a reasonable guess is fine. Set
`unknown` to true, with an empty answer, only when you have no reasonable guess at all.

## The messages

{{messages}}

## Output

One JSON object with `answers`, one entry per message, by its `id`.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["answers"],
  "properties": {
    "answers": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "unknown", "answer"],
        "properties": {
          "id": {"type": "string"},
          "unknown": {"type": "boolean"},
          "answer": {"type": "string"}
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>val_closed_mark.md</code>: marks which facts those answers hold</summary>

````text
Each case below has an answer that someone wrote without any access to a project's history, and the reference facts
a correct answer must contain. For each fact, say whether the answer holds it (`held`): it states the same thing, with
the same value where the fact has one. A paraphrase counts; a vaguer statement, a different value or a list of options
that merely includes the fact does not. An answer marked unknown holds no fact.

## The cases

{{cases}}

## Output

One JSON object with `marks`, one entry per case, by its `id`, with one entry per fact.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["marks"],
  "properties": {
    "marks": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "facts"],
        "properties": {
          "id": {"type": "string"},
          "facts": {
            "type": "array",
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["fact", "held"],
              "properties": {
                "fact": {"type": "string"},
                "held": {"type": "boolean"}
              }
            }
          }
        }
      }
    }
  }
}
```
````

</details>

<details>
<summary><code>val_noanswer.md</code>: checks that the sessions leave a question with no answer open</summary>

````text
You check whether a software project's recorded coding sessions answer a question that is meant to have no answer
there. Below are the question; its look-alike (what the sessions did settle close to it) with the look-alike's own
lines and the lines around them; other lines of the look-alike's session that name the question's or the
look-alike's identifiers; and every line of the project's sessions that names one of the question's identifiers, with
the lines around each, oldest first. Lines are shown as `<session> L<n> <time> <who>: <text>`.

`answered` is true when the lines state an answer to the question, even a partial one, or show for certain that the
thing asked about does not exist, is not configured, or was flagged, skipped or left undone: a definite "no", "none"
or "not done" is an answer. A value set in code that the lines show also answers it. It is false only when the lines
leave the question open. When it is true, give the session, the line and a short exact quote of the best
answering text; otherwise leave them empty and 0. `reason`: one sentence.

## The question

{{question}}

## The lines

{{lines}}

## Output

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["answered", "session", "line", "quote", "reason"],
  "properties": {
    "answered": {"type": "boolean"},
    "session": {"type": "string"},
    "line": {"type": "integer"},
    "quote": {"type": "string"},
    "reason": {"type": "string"}
  }
}
```
````

</details>

<details>
<summary><code>repair_question.md</code>: repairs a question that pointed at other lines or missed a later change</summary>

````text
An evaluation question about a software project failed one check, and the check named something that can be fixed.
Fix the question's reference answer, or say it cannot be fixed. There is one fix per question.

Two problems can be fixed:

- **Wrong lines.** A fact is true, but its evidence lines do not state it; the check found other lines that do. Point
  the fact's evidence at lines that state it.
- **Later changes.** Later lines settle a new value, choice or place for one fact or more. For each, the new value
  becomes the fact, with the later line as its evidence, and the old value becomes a stale value that the fact
  `replaces`, keeping its old evidence. Check that nothing after the new value changes it again. Add a stale value
  only when the later line changes the same attribute of the same thing (a different error, a later count of
  something else or a related setting is not a replacement); a stale value the ledger does not link to its fact is
  removed afterwards.

Keep the question's class and the meaning of every other fact. Each fact keeps its keys or gets the new value's keys,
copied from its new evidence. Evidence is a session id, the first and last `L` number (at most 40 apart) and a quote of
at least six words or 30 characters copied character for character from one block of those lines; never from a
compaction summary or text with a redaction placeholder. Copy session ids and lines only from the lines shown here.
Set `fixed` to false, with the facts and stale values unchanged, when the lines shown here do not allow a clean fix.

## The question

{{question}}

## What the check found

{{verdict}}

## Lines

{{lines}}

## Output

One JSON object: `fixed`, the full revised `facts` and `stale` lists (ids `f1`, `f2`, ... and `s1`, `s2`, ...), and
`note`, one sentence on what changed.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["fixed", "facts", "stale", "note"],
  "properties": {
    "fixed": {"type": "boolean"},
    "facts": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "text", "keys", "evidence"],
        "properties": {
          "id": {"type": "string"},
          "text": {"type": "string"},
          "keys": {"type": "array", "items": {"type": "string"}},
          "evidence": {
            "type": "array",
            "minItems": 1,
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["session", "lines", "quote"],
              "properties": {
                "session": {"type": "string"},
                "lines": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2},
                "quote": {"type": "string"}
              }
            }
          }
        }
      }
    },
    "stale": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "text", "replaces", "evidence"],
        "properties": {
          "id": {"type": "string"},
          "text": {"type": "string"},
          "replaces": {"type": "string"},
          "evidence": {
            "type": "array",
            "minItems": 1,
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["session", "lines", "quote"],
              "properties": {
                "session": {"type": "string"},
                "lines": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2},
                "quote": {"type": "string"}
              }
            }
          }
        }
      }
    },
    "note": {"type": "string"}
  }
}
```
````

</details>

## Corrections

Any change after the questions are committed is listed here, one line each.
