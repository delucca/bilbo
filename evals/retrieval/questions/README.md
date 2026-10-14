# The questions

Each question asks for something one project's sessions settled: what the developer decided, was told, tried or left
open. Its answer is a list of facts, each pointing at the session lines that state it, and holds at the end of the
project's sessions, since every store is built from all of them.

## Files

- `questions.jsonl`: the 139 questions, one per line. Committing it freezes it.
- `dropped.jsonl`: the 561 questions that failed a check or the audit, with the reason.
- `classes.json`: the classes, with the weight of each.

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
weights its class scores by them, so the classes count as much as real use asks for them, however many questions each
has: every class is in the headline, and the class tables give each class's count beside its score.

Out of scope: questions about written sources (41% of all the real queries), and personal or home facts, which no
project's sessions hold.

## The counts

The first five projects were chosen by metadata, the five added for `procedure` by a screen (A third round, below).

| Project | `overview` | `status` | `decision` | `rules` | `procedure` | `fact` | `refind` | answerable | no answer | all |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| iptvnator | 3 | 4 | 3 | 3 | 0 | 0 | 7 | 20 | 5 | 25 |
| kimi-cli | 3 | 0 | 1 | 0 | 0 | 2 | 0 | 6 | 5 | 11 |
| linear-cycle-drain | 6 | 4 | 7 | 1 | 1 | 7 | 4 | 30 | 5 | 35 |
| readaloud | 5 | 4 | 0 | 7 | 2 | 6 | 2 | 26 | 4 | 30 |
| port-cli | 4 | 6 | 3 | 1 | 1 | 2 | 7 | 24 | 5 | 29 |
| **first five** | 21 | 18 | 14 | 12 | 4 | 17 | 20 | 106 | 24 | 130 |
| safecast-map-MCP | 0 | 0 | 0 | 0 | 1 | 0 | 0 | 1 | 0 | 1 |
| bosarsalog | 0 | 0 | 0 | 0 | 1 | 0 | 0 | 1 | 0 | 1 |
| go-nuts | 0 | 0 | 0 | 0 | 2 | 0 | 0 | 2 | 0 | 2 |
| neuralips26 | 0 | 0 | 0 | 0 | 1 | 0 | 0 | 1 | 0 | 1 |
| contoso | 0 | 0 | 0 | 0 | 4 | 0 | 0 | 4 | 0 | 4 |
| **added five** | 0 | 0 | 0 | 0 | 9 | 0 | 0 | 9 | 0 | 9 |
| **all ten** | 21 | 18 | 14 | 12 | 13 | 17 | 20 | 115 | 24 | 139 |

- The headline covers all seven classes, 115 answerable questions, weighted 85, 41, 54, 54, 34, 38 and 11.
- A question's share of the headline is its class's weight over its count: about 1.3% for an `overview` question, 0.8%
  for a `procedure` question (34 of 317, over 13 questions) and 0.2% for a `refind` question (11 of 317, over 20
  questions).
- The no-answer questions look like `fact` (14), `decision` (8), `procedure` (1) and `refind` (1) questions.
- Every question is drafted; none is harvested (below).
- iptvnator ships no `procedure` or `fact` question, readaloud no `decision` question, and kimi-cli no `status`,
  `rules`, `procedure` or `refind` question. The five added projects ship `procedure` questions only (9 of the 13), and
  no question with no answer.
- Every class but `refind` has fewer questions than the plan gave it, since the audit failed every question whose
  answer the project's files hold (Filling and the audit, below).
  `refind` has 20, against the plan's 8, after a later fill (The refind fill, below). `procedure` has 13, against 24,
  after the third round (below).

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
- `commit_event`: a `refind` question whose event is a commit, push, merge, tag or release, or a pull request opened
  or merged, or whose identifying detail is a commit hash, a commit message or one of those events. Git or the
  hosting service answers it, not memory.

A drop found by the audit has a `detail` that starts with `audit:`. A drop by the commit rule has one that starts
with `rule:`. The audit also settled these cases:

- The commit of the last session is an end tree, even on a branch that was never merged (kimi-cli-001).
- Git history is not a file for `in_repo`, but a commit counts for `not_current` and `not_unique`.
- A `refind` date is the UTC day of its line. When the developer's local day differs, two days fit and the question
  fails `not_unique` (iptvnator-230, readaloud-098).
- A question with no answer fails when the sessions show its premise false at the time (readaloud-112). It passes
  when the premise held when the value was set (iptvnator-237), or when nothing contradicts it (kimi-cli-029).

| Reason | At mining | At the audit | By rule | All |
|---|---:|---:|---:|---:|
| `in_repo` | 53 | 126 | 0 | 179 |
| `unsupported` | 176 | 21 | 0 | 197 |
| `not_current` | 26 | 29 | 0 | 55 |
| `not_unique` | 38 | 31 | 0 | 69 |
| `guessable` | 7 | 38 | 0 | 45 |
| `duplicate` | 1 | 4 | 0 | 5 |
| `commit_event` | 0 | 1 | 10 | 11 |
| all | 301 | 250 | 10 | 561 |

## How the questions were made

In the first round, one script made every model call, with the prompts below, frozen before mining began. Claude
Opus 5.5 read, drafted and wrote; gpt-6.1-sol, from another model family, checked; Claude Opus 5.5 reviewers with
tools audited. A second round for rules and procedure (below) added agents with tools around the same
frozen writers and checks. A third round added `procedure` questions from five more projects, and a later fill for
`refind` (the last subsection) ran the same frozen writers and checks on drafts the first round left unused.

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
8. **Filling.** The plan was 225 answerable questions, split over the classes by weight (`overview` 61, `status` 29,
   `decision` 38, `rules` 38, `procedure` 24, `fact` 27, `refind` 8) and over the projects by the mean of their shares
   of the sessions and of the text, plus 5 with no answer per project. Selection, writers, checks and repair run batch
   after batch until each cell has one and a half times its count of passing questions or runs out of candidates. A
   class that runs out gets one more round of drafting. The first passing questions in seeded order fill the cell; the
   rest are spares.
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

The audit is also why the classes came up short of the plan. It failed every question whose answer the project's
files hold, and much of what a session settles ends up in a file: a decision's reason in a code comment, a rule in
`AGENTS.md`, a procedure in the README. Besides `refind` (8 of 8), `status` and `fact` kept the most of their counts
(62% and 63%), `procedure` and `rules` the least (17%, and 32% with the second round's three). The third round
brought `procedure` to 54%. 24 of the 25 questions with no answer shipped.

### A second round for rules and procedure

After the first round, rules had 9 questions and procedure 4, both under the floor of 10 questions that then kept a
class out of the headline (since dropped, see Corrections). A second round tried to fill them.

- **The probe.** Before any drafting, Claude Sonnet 5.5 workers judged whether each candidate item of the ledgers
  could carry a `rules` or `procedure` question that passes the audit. A calibration failed first: on 33 groups the
  first audit had ruled on, two workers agreed with it 55% of the time, against a gate of 85%, and their judgment
  calls killed 12 of the 13 groups that shipped. So the probe kills only on evidence (such as a file that states it,
  or a later line or commit that changed it) and forwards every judgment call. On the calibration groups, that rule
  kills none of the 13.
  Then 12 workers judged 426 items of the five projects and forwarded 95.
- **The check of the forwards.** Three Claude Opus 5.5 reviewers ruled 174 rows: the 95 forwards and 79 kills (51
  borderline, 24 random, 4 picked by hand). They upheld 17 forwards and 76 kills. The 3 kills they overturned stayed
  kills, for another reason. 14 rules groups survived, and no procedure group.
- **Drafting and checks.** Claude Sonnet 5.5 drafters wrote each group's facts and subject. The frozen writer prompts
  and gpt-6.1-sol then ran as in the first round: two questions had their evidence repointed to the lines sol found,
  and four got their one repair call.
- **The audit.** Two Claude Opus 5.5 auditors judged each question on their own. An adjudicator ruled where they
  differed: the two questions they split on, and the fixes of a third.
- **The funnel.** 14 drafted, 4 passed the checks, 3 passed the audit: linear-cycle-drain-172, readaloud-156 and
  readaloud-158. Rules goes from 9 to 12. Of the 11 drops, 10 came at mining and 1 at the audit.
- **Procedure.** No procedure question came out of the round, and procedure stays at 4. gpt-6-astra, from another
  model family, also read the sessions for flows, without the ledgers. A first pass checked 30 and failed them all.
  An exhaustive second pass grouped 747 command shapes and 557 developer asks and checked 60. It left one candidate,
  a one-time setup with a weak subject, which was not drafted or audited.

### A third round: procedure from screened projects

After the second round `procedure` had 4 questions. A third round looked for more in projects outside the corpus,
screened for work the project's files do not hold, which is what survives the audit.

- **The screen.** A no-LLM screen read SWE-chat's tables, not the transcripts. Of 596 Claude Code repositories it kept
  185 new ones (3 sessions or more, an open licence, no fork, 80% of the sessions or more from one developer) and scored
  them with 44 labelled before, the five corpus projects and 39 an earlier scan had read, for recurring commands that
  work outside the repository (ssh, rsync, deploy and cloud CLIs, other machines' paths) and that its files do not name.
  It ranked the 5 corpus projects as their yields did (rank correlation 0.95, on five), but it is a ranking: it missed
  runbooks written in stock commands.
- **The gate.** For 24 projects (23 candidates, and readaloud read blind as a calibration) a Claude Opus 5.5 judge read
  the sessions in the window, the command clusters and every instruction file, and listed the procedure groups that
  could pass the audit. It killed a group only on evidence, such as a file that states it or a later change, and rated
  the rest `high`, `medium` or `low`. A forecast of 1.0 or more (0.6 per `high`, 0.3 per `medium`, 0.1 per `low`)
  qualified a project: 13 did. 11 were mined; the other two held dev-workflow groups, the kind the first audits failed
  as guessable.
- **The windows.** No added project exceeds the first five's largest on any measure (39 sessions, 688 prompts, 5,432
  tool calls). A larger one went in as the contiguous window with the highest screen score, chosen before any model
  read it: neuralips26 is 22 of its 105 sessions, contoso 9 of its 10. go-nuts is 12 of 14 (two began outside its
  repository).
- **Two drafting routes.** The first round's frozen tools ran with the new projects added as data, and a second route
  drafted rows from the gate's groups that the first left uncovered, by the second round's method. The frozen prompts,
  sol's checks and the fill loop were the same.
- **The audit.** Every row that passed sol (76) went to a Claude Opus 5.5 reviewer that read the sessions and the files
  at the end tree, and an adjudicator ruled on each verdict. 67 failed and 9 shipped.
- **The funnel.** 229 repositories scored, 24 gated, 117 groups forwarded by the 23 candidates (67 in the 11 mined),
  151 rows sent to sol, 76 passed sol, 9 passed the audit, from 5 projects. The other 6 mined projects ship nothing and
  stay out of the corpus.
- **The result.** `procedure` goes from 4 questions to 13, short of the 24 the plan gave it.
- **The bias.** Operations-heavy, thinly documented projects pass this screen and the audit more often, so the added
  projects are not a sample, and 5 of the 9 new questions ask about services outside the repository.

### The refind fill

- **Why.** `refind` had the plan's 8 questions. The target became 20.
- **The commit rule.** A `refind` question may not ask about a commit, push, merge, tag, release, or a pull request
  opened or merged, nor hold a commit hash or message as its detail (What a question is not, above). It dropped 5 of
  the 8 shipped questions and 5 of the 6 audited spares. readaloud-100's event was an incident but its detail was a
  hotfix commit, so Claude Opus 5.5 re-drafted the detail from another line of its session; it was checked and
  audited again and passed. port-cli-083, an audited spare, was promoted.
- **The day check.** At selection, the local day of a candidate is read from the developer's own commits. A candidate
  whose line falls on another local day than its UTC day is not sent.
- **The pool.** 130 unused drafts, taken in seeded order; 45 were eligible after the rule and the day check.
- **The cells.** The first round's shares gave 3, 2, 5, 5 and 5. A repo with no candidates left handed its shortfall
  to the repos that had some, in a fixed order. The final split is iptvnator 7, kimi-cli 0, linear-cycle-drain 4,
  readaloud 2 and port-cli 7, counting the 3 questions that stood.
- **The checks.** The first round's fill loop: the writer check before ids, the query writer, gpt-6.1-sol's checks
  with repoint and one repair, and 1.5 times over-production. Two Claude Opus 5.5 auditors judged each question on
  their own, and an adjudicator ruled on splits and fixes. A last screen of the shipped questions found that
  kimi-cli-051's evidence quoted a commit message, and it was dropped at the audit as `commit_event`.
- **The funnel.** 45 eligible, 40 sent, 2 refused as unclear before ids, 38 ids (39 with readaloud-100), 28 sol
  passes, 19 audit passes, 18 after the last screen, and 20 questions shipped with the 3 that stood (port-cli-083
  among them). One audit pass is a spare. A Claude Sonnet 5.5 worker ran the tools.

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

### The second round's briefs

The instructions the round's agents followed, word for word. Their paths are relative to the round's work folder,
not to this repository.

<details>
<summary><code>BRIEF-probe-v2.md</code>: tells a probe worker to kill an item only on evidence and forward every judgment call</summary>

````text
# Brief: probe the supply of rules and procedure questions (v2)

You are a probe worker (Claude Sonnet 5.5). The probe runs before any new question is drafted. It sorts each candidate
item of a project's ledger into two piles:
- **Killed:** items that hard evidence shows could not carry a `rules` or `procedure` question that passes the audit.
- **Forwarded:** everything else, which goes to an Opus reviewer with your notes on what to look at.

**You kill only on evidence: a file line, a later line or commit, a committed question, a chain head.** Every
judgment call (is it guessable, is it settled, does it fit the class) is the reviewer's: you record it as a flag and
forward the item. You draft nothing and change nothing; you write one row per item.

The folder that holds this brief is the work folder (below, `./`); every path here is relative to it. The message
that started you gives your letter. Your row in `plan-rp/planner-assign.md` names your input, your project's folder
and clone, the committed questions file, and the folders you must never read.

## What a question in these classes needs

- `rules`: what to watch out for, how things are done in the project. Its facts are the rule and when it applies.
  - Rules include a project convention, a gotcha, and a quirk of a tool as this project's setup meets it.
  - They include a lesson from a bug or an incident (what broke, why, what to check next time), and a known failure
    to ignore (errors that were already there).
  - They include a recipe for an agent's tools that the sessions worked out.
- `procedure`: how to do something in the project. Its facts are the steps, in order, with their commands: two or
  more. A flow the sessions ran more than once is a procedure, as is a one-time setup whose own values (a remote, a
  host, a path) a later question can name.

The audit passes a question only when all of these hold:

1. **Supported:** session lines state each fact.
2. **Not in the files:** the project's files at its end trees do not state its facts, or all of them but what is
   generic, guessable or not asked.
   - Files are the code and its comments, configs, tests, and docs: ADRs, plans, architecture notes, and the last
     session's own docs even on an unmerged branch.
   - The agent instruction files count too (`CLAUDE.md`, `AGENTS.md`, `CONTRIBUTING*`, `.claude/`, `.cursor/rules/`,
     `.agents/skills/*/SKILL.md`), and so do READMEs, changelogs and vendored specs.
   - A file that holds only the names, and not the rule, the reason, the order of the steps or what was tried, does
     not count. Another repository's files are not this project's files. The git log is not a file.
3. **Not guessable, fact by fact:** a developer who knows the stack, with no memory of these sessions, would not give
   most of its facts in substance. A question keeps its value when one of its facts is session-only:
   - a name, a value, a path or a host the project chose;
   - the cause of a failure, or an incident;
   - how a tool behaved in this setup.

   It fails when every substantive part is stock knowledge: a tool's documented behaviour or flags, a fix the tool's
   own error or help text prescribes, or the usual practice for the situation.
4. **Current:** no later session, and no commit made between sessions, changed it before the end of the sessions.
5. **Unique:** the sessions hold no second fair answer, such as another way they did the same thing.
6. **Worth asking:** a later developer would want to know it.
   - Neither the developer's own words nor repetition is required: a quirk observed once and recorded counts.
   - What does not count:
     - a move incidental to one task that nobody would ask about again;
     - a claim nothing acted on (a passing reviewer's note);
     - a tool's, plugin's or skill's own printed instructions that the sessions only followed, or did otherwise.
   - Running another repository's tools or gates as part of this project's own workflow is this project's procedure.
7. **Askable:** a developer can ask it without stating the answer, in words a later session would use for the topic.

## Inputs

- **The ledger:** `<folder>/ledger.jsonl`, one item per line. Its fields: `id`, `kind`, `statement`, `reason`,
  `steps`, `value`, `standing`, `evidence` (`session`, `lines`, `quote`), `first_seen`, `last_seen`, `supersedes`,
  `superseded_by`, `workstream`. From the project's folder read only `ledger.jsonl`, `workstreams.jsonl`,
  `trees.json` and `view/`.
- **The views:**
  - Print a session's lines with `python3 -I tools/render.py show <repo> <session> <first> <last>`. Every line
    starts with `L<raw line> <UTC time> <ROLE>:`.
  - The same views are in `<folder>/view/<first 8 characters of the session>.txt`. Read them only by line range,
    never a whole session.
  - Search them only with `grep -a`: the shell's `grep` skips a file it takes for binary, and one view holds a NUL
    byte.
- **The clone:** `repos/<owner>__<repo>.git`, a bare clone. Use read-only git only: `grep`, `show`, `log`,
  `ls-tree`. It runs past the end of the sessions, so name trees by sha, never `HEAD`, `main`, `master` or a branch.
- **The end trees** of an item:
  `python3 -I -c "import sys; sys.path.insert(0, 'tools'); import render; print(render.question_trees('<owner>__<repo>', ['<session id>']))"`,
  with the full session ids of the item's evidence.
- **Bundles:**
  - When `probe-rp/bundles/<repo>/<item>.md` exists, read it first. In calibration, read
    `probe-rp/bundles/cal/<repo>/<item>.md` instead.
  - A bundle holds:
    1. the item and its chain;
    2. the evidence with 8 lines either side;
    3. the end trees;
    4. the `git grep` hits at every end tree;
    5. the later view lines that name the item's terms;
    6. the git log after the item;
    7. the committed questions and drops that cite the same lines (absent in calibration).
  - A bundle saves commands, not reading. Its grep hits are on the item's own names only, so read the component's
    code and the instruction files yourself.
  - Without a bundle, run the commands in the steps below.

## Per item

1. **The item.** Read it in the ledger. When `superseded_by` is not empty, follow the chain to its head, the item
   that nothing replaces. If the head is in your input, write `superseded` with `head`: a kill. Otherwise judge the
   head in this row and set `head`.
2. **The best question.**
   - Read the evidence lines with some lines around them, and look at the item's neighbours: the other items of its
     workstream, its chain, and later lines that repeat it.
   - Ask what the best `rules` or `procedure` question is that the item could carry, alone or with those neighbours,
     and set `class_fit`.
   - An item whose `source` is a regex pick (`event_cause`, `decision_way`, `manual_steps`, `fact_cue`, `event_cue`,
     `state_cue`) is a pure record when it holds nothing to watch out for and no way of doing something: a change
     made, a value, a state of the work. Write `not_class` for it: a kill.
   - Any other item, and any regex pick you are unsure of, goes on with a `class_fit` flag when the fit is doubtful.
3. **Overlap** (probe only, not calibration). Search the committed questions file for the item's session and lines
   and for its distinctive terms, and read section 7 of its bundle. When a committed question already holds the
   fact, write `overlap` with its id: a kill.
4. **Files.**
   - At every end tree, grep the item's names, values and distinctive phrases:
     `git -C repos/<owner>__<repo>.git grep -n -I -i -F -e '<term>' <sha> --`.
   - Read the hits in context: `git -C ... show <sha>:<path> | sed -n '<a>,<b>p'`.
   - Read the component's code and comments, and every instruction file of criterion 2 that the tree has, whatever
     their hits. List them with `git -C ... ls-tree -r --name-only <sha>`.
   - Read a one-line JSON spec with `python3 -I -c 'import json, sys; ...'`, never by eye.
   - When a file line states the item's core fact (the rule, the reason, the steps), write `in_files` with that line
     as `answer`: a kill.
   - When the files state part of it and you judge the rest generic, still write `in_files`, but with `borderline`
     `true`, and name the leftover in `note`.
5. **Current.**
   - Grep the later view lines (`grep -a -n -i -F '<term>' <folder>/view/*.txt`, keeping hits whose time is after
     the item's `last_seen`) and read them.
   - At each end tree, run `git -C ... log --format='%h %ad %s' --date=iso <sha> --since='<last_seen>' -- <paths>`
     and read the diffs that touch the rule or the steps (`git -C ... show <commit> -- <path>`).
   - When a later line or a commit changes the item's answer, write `changed` with `changed_by` set to
     `<session 8>:L<line>` or `commit <sha 8>`: a kill.
   - A wrong later claim that nothing corrects is not a change; flag it as `second_answer`.
6. **Judgment flags.** For each of these that you doubt, add a flag with one line of why, then forward the item:
   - `generic` (criterion 3): name the facts you think are stock and the session-only fact if there is one;
   - `second_answer` (criterion 5): give the other answer's session and line;
   - `one_off` (criterion 6): an incidental move or a claim nothing acted on;
   - `third_party` (criterion 6): a tool's own printed steps;
   - `class_fit` (step 2);
   - `askable` (criterion 7).
7. **Forward.** Write `survive`, with:
   - `flags`;
   - `residue`: one sentence on what only the sessions hold;
   - `group`: a key shared by the items one question would rest on, `<repo>/<a few words in kebab case>`;
   - `subject`: a span of 2 to 15 words from one line of the developer's turns (`USER` lines in the view), copied
     exactly, that names the topic. It must not hold an exact value of the answer (a flag, a command, a path, a
     number) or the project's name, and must not state the rule or the steps. It may be in the developer's language.
     When only the agent's words name the topic, take them and set `"source": "assistant"`. When nothing does, write
     `null` and add an `askable` flag.

- **Order:** the first kill that applies, in step order, is the row's verdict; note any other in `note`.
- **When unsure whether evidence kills, forward:** a wrong forward costs one review, a wrong kill loses the item.
- **Time:** about 5 minutes an item. Past 8 minutes, forward it with the flags you have.

## The row

One JSON object per line in `probe-rp/verdicts-<letter>.jsonl`, appended as soon as the item is judged. On a restart,
skip the items already there.

| field | value |
|---|---|
| `letter` | your letter |
| `repo` | the project's short name (`readaloud`, `port-cli`, ...) |
| `item` | the item's id; in calibration the group's id (`cal07`) |
| `items` | the ledger ids judged: `[item]`, or the group's items |
| `kind`, `source` | from your input; in calibration the main item's kind and `"calibration"` |
| `class_fit` | `rules`, `procedure` or `none` |
| `verdict` | `survive` (forwarded), or a kill: `in_files`, `changed`, `overlap`, `superseded`, `not_class` |
| `borderline` | `true` for a kill you are unsure of, else `false` |
| `flags` | for `survive`: `[{"criterion": "generic", "why": "..."}]`, empty when none; else `[]` |
| `answer` | `{"file": "<path>", "line": <n>, "tree": "<sha 8>"}` for `in_files`, else `null` |
| `changed_by` | `"<session 8>:L<n>"` or `"commit <sha 8>"` for `changed`, else `null` |
| `other_answer` | `"<session 8>:L<n>"` when a `second_answer` flag is set, else `null` |
| `overlaps` | the committed question's id for `overlap`, else `null` |
| `head` | the chain head's id when the item is superseded, else `null` |
| `group` | for `survive`, else `null` |
| `residue` | for `survive`, else `null` |
| `subject` | `{"session": "<full id>", "line": <n>, "quote": "...", "source": "user"}` or `null` |
| `note` | at most 300 characters: why, and any other kill that also applies |

## Calibration (workers A and B)

The calibration checks the kills before the probe runs on new items: every kill must be right, and no question the
audit passed may be killed.

- **Input:** A takes groups `cal01` to `cal17` of `plan-rp/planner-calibration.jsonl`, B takes `cal18` to `cal33`.
- **The unit:** a group is the set of ledger items that one question rested on. Judge it as one `rules` or
  `procedure` question built on all its items, and write one row per group.
- **Steps:** 1, 2 and 4 to 7 apply to the whole group; skip step 3. A group is never a regex pick.
- **Bundles:** read only `probe-rp/bundles/cal/<repo>/<item>.md`, never `probe-rp/bundles/<repo>/`.
- **Never read** anything that carries an earlier verdict, besides the list under Rules:
  - `review/`, any `AUDIT.md` and any `final/`;
  - any `dropped.jsonl` or `questions.jsonl`, the committed ones included;
  - `spares.jsonl`, `selected.jsonl`, `drafts.jsonl`, `verdicts/` and any `rulings*` file;
  - `LEAD-LOG*`, `PROGRESS*`, every `DONE-*` and every `PLAN*.md`;
  - `plan-rp/planner-calibration-key.jsonl`, `plan-rp/planner-funnel.jsonl` and `plan-rp/P1-diagnosis.md`;
  - the other calibration worker's files.

## Rules

- Do not spawn agents, forks or teammates. Do not use SendMessage or ListAgents.
- No LLM calls of any kind: no `tools/mine.py` stage, no proxy, no `codex`, no `claude`. `tools/render.py show` and
  importing `render` for `question_trees` are fine.
- Write only `probe-rp/verdicts-<letter>.jsonl`, `probe-rp/progress-<letter>.txt` and your DONE file. Name any helper
  `probe-<letter>-*`, under `probe-rp/` or the scratchpad: other agents share the scratchpad.
- Never edit `tools/`, a project folder, `review/`, `plan-rp/`, the bundles or another worker's files. Make no git
  writes. Make no commits.
- Never read the outputs of the earlier mining and audit: every file in the calibration list above. Two exceptions,
  outside calibration only: the committed questions file in step 3, and section 7 of a bundle. The same holds for
  the `BRIEF-*` files other than this one, the `FIXES-*` files, `llm-calls.jsonl`, other workers' files in `probe-rp/`,
  and `plan-rp/` except `planner-assign.md` and your input. Never read the folders `planner-assign.md` lists under
  "never read". Use no notes or memory tool.
- Read sessions only by line range.

## Progress and finish

- **Progress:** at least every 30 minutes, append one line to `probe-rp/progress-<letter>.txt`:
  `<UTC time> <done>/<total> last=<item>`.
- **When every item has its row,** write `DONE-probe-<letter>.md` in `./` (at most 12 lines): rows per `verdict` and
  per `class_fit`, the forwarded count with the flags most used, the borderline kills, any item not judged and why,
  the minutes taken, and anything odd.
- Then stop with a final message of at most 5 lines.
````

</details>

<details>
<summary><code>BRIEF-probe-review.md</code>: tells an Opus reviewer to rule on the probe's forwards and kills</summary>

````text
# Brief: P3, review the probe's verdicts

You are a probe reviewer (Claude Opus 5.5), X or Y; the message that started you gives your letter. The probe workers
(Sonnet 5.5) judged every candidate item of the rules and procedure supply against the audit's standard. You rule on
the verdicts that decide the yield ceiling, so the lead can tell the user how many questions a new round could give.
You draft nothing.

The folder that holds this brief is the work folder (below, `./`); paths are relative to it.

## Read first

- `./BRIEF-probe-v2.md`: the standard (criteria 1 to 7), the inputs, the steps and the row format. The probe kills
  only on evidence and forwards every judgment call to you as a `survive` row with `flags`, so most of your rows are
  forwards: rule on each flag, then on everything else. Apply the standard as the full audit applied it, with these
  settled points:
  - **Files:** code comments, configs, tests, docs, the agent instruction files, vendored specs, and the last
    session's own docs at its commit (even on an unmerged branch) are files. The git log is not.
  - **Current and unique:** commits made between sessions count for both.
  - **Guessable:** judge substance, not wording, and fact by fact. One session-only fact the question asks for (a
    value, a cause, an incident, a tool's behaviour in this setup) keeps it.
  - **Worth asking:** lessons from bugs and incidents, known pre-existing failures, observed tool quirks, agent-tool
    recipes, one-time setups whose values a prompt can pin, and the project's use of another repo's tools all passed
    the audit.
  - **A wrong later claim** that nothing corrects is a second answer, not a change.
- `./plan-rp/planner-assign.md`: the folders, the clones, the committed questions file and the folders you must never
  read.

## Your rows

`plan-rp/planner-p3-split.py` drew them into `probe-rp/p3-<letter>.jsonl`. Each line is a probe row plus `p3_reason`.

- **Every `survive`.**
- **Every borderline kill**, of any verdict but `not_class` and `superseded`. That is wider than borderline
  `in_files` alone, since a borderline `generic` or `changed` decides the ceiling as much.
- **Seeded random kills** (seed 20261013): with `--kills 24`, 16 from rule and procedure items and 8 from the regex
  picks, as far as they exist.
- **The split:** whole repos go to the reviewer with fewer rows, the largest repo first. When that leaves one reviewer
  more than 30% above the other, the repos are split by `group` instead, so a group's items stay together. A third
  reviewer (Z) is added with `--reviewers X,Y,Z` when the rows exceed about 35 a reviewer.

## Per row

1. **Read** the probe row, its bundle (`probe-rp/bundles/<repo>/<item>.md`) and the ledger item, and its chain head
   when it has one.
2. **A `survive` (a forward):** rule on each of its `flags` first, then re-run every other criterion yourself, with
   the commands of `BRIEF-probe-v2.md`.
   - The files at every end tree, by sha, read beyond the bundle's grep hits: the component's code and comments, and
     every instruction file and doc the tree holds.
   - Guessable in substance; current over later sessions and the git log; unique; settled; askable.
   - **Class fit:** a `procedure` group needs two distinct steps or more; with fewer, it is a `rules` candidate or
     nothing.
   - **Overlap:** search the committed questions file. A committed question that holds the fact makes it `overlap`.
   - **Subject:** the quote is found on that line, names the topic, holds no exact value of the answer and no project
     name, and does not state the rule or the steps. Replace a bad subject with a better span from the sessions if one
     exists, or write `null`.
   - **Group:** give the final `group` key. Items that one question would rest on share it, even when different
     workers named them. If `p3-<other letter>.jsonl` holds rows of your repo, read its survivors and reuse their key
     for the same question.
3. **A kill:** re-run the check that killed it, and judge whether the probe's evidence holds (the file and line
   state the fact; the later line or commit changes it; the other answer is fair). When you overturn it, run the other
   criteria too, as for a `survive`.
4. **Rule:** `uphold` keeps the probe's verdict; `overturn` replaces it with yours. A fix that changes only the group,
   the subject or the class fit is an `uphold`, with the new values.

## The row

One JSON object per line in `probe-rp/review-<letter>.jsonl`, appended as soon as the row is ruled. On a restart, skip
the rows already there.

| field | value |
|---|---|
| `reviewer` | your letter |
| `repo`, `item`, `items` | from the probe row |
| `probe_letter`, `probe_verdict`, `p3_reason` | the probe row's `letter`, `verdict` and `p3_reason` |
| `ruling` | `uphold` or `overturn` |
| `verdict` | the final verdict: `survive`, or a kill: `in_files`, `changed`, `overlap`, `superseded`, `not_class`, `generic`, `second_answer`, `one_off`, `third_party`, `askable` (the judgment kills are yours alone) |
| `class_fit` | `rules`, `procedure` or `none` |
| `answer`, `changed_by`, `other_answer`, `overlaps` | as in the probe row, for the final verdict |
| `group` | the final key for a final `survive`, else `null` |
| `steps` | for a `procedure` survivor, the number of distinct steps its group holds, else `null` |
| `subject` | the final subject (`{"session", "line", "quote", "source"}`) or `null` |
| `subject_ok` | `true` when a usable subject exists for a survivor, else `false` |
| `residue` | for a final `survive`: what only the sessions hold, one sentence |
| `note` | at most 300 characters: why, above all for an overturn |

The ceiling table per class and repo is the lead's to build from these rows.

## Rules

- Do not spawn agents, forks or teammates. Do not use SendMessage or ListAgents.
- No LLM calls of any kind: no `tools/mine.py` stage, no proxy, no `codex`, no `claude`.
- Write only `probe-rp/review-<letter>.jsonl`, `probe-rp/progress-review-<letter>.txt` and your DONE file. Name any
  helper `probe-review-<letter>-*`: other agents share the scratchpad.
- Never edit `tools/`, a project folder, `review/`, `plan-rp/`, the bundles or the probe's files. Make no git writes.
  Make no commits.
- You may read the earlier audit's files (`review/`, the `AUDIT.md` files, the committed `dropped.jsonl`) to keep your
  rulings consistent with it. Never read `../l3-conversations/l3v2/` or `../l3-conversations/recall-query-study/`.
  Use no notes or memory tool.
- Read sessions only by line range, search views only with `grep -a`, and name trees only by sha.

## Progress and finish

- **Progress:** at least every 30 minutes, append one line to `probe-rp/progress-review-<letter>.txt`:
  `<UTC time> <done>/<total> last=<item>`.
- **When every row has its ruling,** write `DONE-probe-review-<letter>.md` in `./` (at most 12 lines): rows ruled,
  upheld and overturned by probe verdict, and the final survivors as groups per class and repo. Add the patterns
  behind the overturns (for example, the probe too lenient on guessable) and anything odd.
- Then stop with a final message of at most 5 lines.
````

</details>

<details>
<summary><code>BRIEF-draft-rp.md</code>: tells a drafter to write the facts and subject of each surviving rules group</summary>

````text
# Brief: draft the rows of the rules round

You are a drafter (Claude Sonnet 5.5). The probe and its reviewers left a set of groups: ledger items on which one
`rules` question could pass the audit. You turn each group into one candidate row. Afterwards a script issues the ids,
the frozen prompt and query writers write the row's `prompt` and `query` without seeing your facts, and sol and an
Opus audit check it. You write no prompt and no query.

The folder that holds this brief is the work folder (below, `./`); every path here is relative to it. The message
that started you gives your letter.

| letter | repos | ledger folder | your output |
|---|---|---|---|
| U | readaloud | `ngngardner__readaloud/` | `ngngardner__readaloud-rp/drafts-rp.jsonl` |
| U | linear-cycle-drain | `ababushkin__linear-cycle-drain/` | `ababushkin__linear-cycle-drain-rp/drafts-rp.jsonl` |
| V | iptvnator | `4gray__iptvnator-final/` | `4gray__iptvnator-rp/drafts-rp.jsonl` |
| V | port-cli | `port-experimental__port-cli/` | `port-experimental__port-cli-rp/drafts-rp.jsonl` |

## Your groups

- The groups are the rows with `"verdict": "survive"` in `probe-rp/review-X.jsonl`, `-Y.jsonl` and `-Z.jsonl` whose
  `repo` is yours. Rows with the same `group` make one question: draft one row per group.
- Each review row gives:
  - `items`: the ledger ids;
  - `residue`: what only the sessions hold, which is the answer your facts must carry;
  - `subject`;
  - `note`: what the files already hold, what a committed question already asks, and what the question must
    therefore avoid.

## What you read, and nothing else

- The review rows above, and the probe rows of the same items in `probe-rp/verdicts-*.jsonl`.
- The bundles `probe-rp/bundles/<repo>/<item>.md`.
- The ledger, `<ledger folder>/ledger.jsonl`: each item's statement, its evidence (session, lines, quote), and its
  chain (`supersedes`, `superseded_by`).
- The sessions:
  - Print lines with `python3 -I tools/render.py show <repo> <session> <first> <last>`. Every line starts with
    `L<raw line> <UTC time> <ROLE>:`, and that `L<n>` is the line number evidence uses.
  - Or read `<ledger folder>/view/<first 8 characters of the session>.txt` by line range, searching it only with
    `grep -a`.
  - Never read a whole session.

## A rules question

`rules` asks what to watch out for, or how things are done in the project. Its facts are the rule and when it
applies: what to do or avoid, and the session-only reason or circumstance behind it. One to three facts. The question
must stand on the review row's residue.

## The draft row

One JSON object per line in your output file, one per group:

```json
{
  "cand": "rp-<the part of the group key after the slash>",
  "group": "<repo>/<...>",
  "class": "rules",
  "items": ["<ledger id>", "..."],
  "facts": [
    {"text": "...", "keys": ["..."],
     "evidence": [{"session": "<full session id>", "lines": [<first>, <last>], "quote": "..."}]}
  ],
  "stale": [
    {"text": "...", "replaces": "f1",
     "evidence": [{"session": "<full session id>", "lines": [<first>, <last>], "quote": "..."}]}
  ],
  "subject": {"session": "<full session id>", "line": <n>, "quote": "...", "source": "user"},
  "note": "..."
}
```

- **Facts:** listed in order, f1 first. `stale` may be empty.
- **`note`:** one or two sentences: what the question asks, and which fact answers what. When you replace the
  subject or leave a part of the residue out, add why.

## Facts

- **One claim each.** Every fact is required of an answer, so no fact may be optional or decorative.
- **Only what its own evidence states.** These rows failed before because a fact said more than its quote did. If the
  line says "in some attempts", the fact says "in some attempts", not "always". If a claim needs a second line, add a
  second evidence entry. If no line states a claim, leave the claim out.
- **The rule first,** then when it applies or why: the cause, the circumstance, the incident.
- **Leave out what the files hold** (the review `note` names them) and what any developer would say anyway. The
  question rests on the session-only part.
- **True at the end of the sessions.** When the chain or a later line changed a value, the fact takes the latest one,
  and the older one becomes a `stale` item. Do not pin a number that kept changing, such as a size or a count. State
  the fact without it, or with the latest value and its line.
- **No fact a committed question already holds:** the review `note` names any such question.

## Keys

- A key is an exact string from the fact's evidence lines that a right answer must contain: an error message, a path,
  a flag, a setting, a value.
- Never the topic's name. Zero to two per fact.
- A key may not appear in the subject.

## Evidence

- `session`: the full session id, as the ledger gives it.
- `lines`: `[first, last]`, raw line numbers (the view's `L<n>`), at most 40 apart.
- `quote`: copied from the text of one line in that range (whitespace may differ):
  - at least six words or 30 characters;
  - no redaction placeholder;
  - not from a compaction summary (the view marks those as not evidence).
- Start from the ledger's evidence of the group's items. Add lines only after reading them in the view.

## Stale items

Only when a later line or a newer chain item replaced a value:
- `text`: the old value;
- `replaces`: the fact that holds the new one;
- evidence older than that fact's newest evidence.

## The subject decides what is asked

The prompt writer sees only the class and the subject, and writes what a developer would type. So the subject must
name exactly the topic your facts answer.

- **Not broader than the facts.** A subject that names a whole task ("look into recent failures") makes the writer ask
  about everything in it, and the facts then answer less than the prompt asks. These rows failed for that before.
- **Not the rule itself.** A subject that states the rule makes the prompt hand over the answer, and the question
  becomes guessable.
- **Default: the review row's subject.** Replace it when it fails the checks below or names a broader or narrower
  topic than the facts, and say why in `note`.
- **Sources,** in order of preference:
  - `user`: a whole clause of a developer's typed turn (`USER` line), copied exactly, 2 to 15 words. It must be in a
    session the row's evidence or its items' evidence comes from, at or before the last evidence line there.
  - `ledger`: the `subject` field of one of the row's items, copied exactly: `{"quote": "...", "source": "ledger"}`,
    with no session or line.
  - `assistant`: the agent's prose (`ASSISTANT` line), under the same rules as `user`. Use it only when no
    developer's words and no ledger subject name the topic.
- **In every case:**
  - no project name and no key;
  - it names something specific;
  - it is not cut before its topic;
  - it does not start with "that", "this" or "it".
  - It may be in the developer's language.

## Check before you finish

Run `python3 -I plan-rp/planner-issue-ids.py --repo <repo>` for each of your repos. It is a dry run and writes
nothing. It applies the checker's own rules to your rows and prints:
- `PROBLEM` lines: fix every one.
- `WARNING` lines: read every one. A fact that quotes the same line text as a committed question is a duplicate:
  rewrite the fact, or drop the group and say so.

Run it until it prints no `PROBLEM`. Never pass `--write`: the lead issues the ids.

A group you cannot draft gets no row: report it in your DONE file with why. Examples are a residue that no line
states, or a subject that cannot pass.

## Rules

- Do not spawn agents, forks or teammates. Do not use SendMessage or ListAgents.
- No LLM calls of any kind: no `tools/mine.py`, no proxy, no `codex`, no `claude`.
- Write only your output files and your DONE file. Name any helper `draft-<letter>-*`, in the scratchpad: other agents
  share it.
- Never edit `tools/`, `plan-rp/`, `probe-rp/`, a ledger folder, `review/`, or any file outside your outputs. Make no
  git writes. Make no commits.
- Read only what "What you read" lists. In particular, do not read:
  - the earlier mining's or audit's outputs (`selected.jsonl`, `drafts.jsonl`, `verdicts/`, `final/`, `review/`, any
    `questions.jsonl` or `dropped.jsonl`);
  - the other briefs, plans, logs or DONE files;
  - `../l3-conversations/l3v2/` or `../l3-conversations/recall-query-study/`.
- Use no notes or memory tool.

## Finish

Write `DONE-draft-rp-<letter>.md` in `./` (at most 10 lines):
- the groups drafted, with facts per row;
- the subjects you replaced, and why;
- the groups you dropped, and why;
- the last dry run's result;
- anything odd.

Then stop with a final message of at most 5 lines.
````

</details>

### The third round's briefs

The instructions the round's gate judges and gate-route drafters followed, word for word, except that `<work>` stands
for the absolute path of the round's work folder; the sha256 is the original file's. Their paths are not this
repository's.

<details>
<summary><code>BRIEF-gate.md</code>: tells a gate judge to list the procedure groups of one project that could pass the audit</summary>

sha256 `48978d09c38aa83bee58aefa3af15e87b3ed6d97aece0b89e82b8528a322fa4b`

````text
# Brief `gate`: does this repo hold procedure questions that can pass the audit?

You are a gate judge (Claude Opus 5.5). Your repo (or repos) and your stream name, `gate-<short>`, are in the message
that started you. Your folder: `PS/gate/<owner>__<repo>/`. No model call. You read one repo's sessions in its window,
its command clusters and its files, and you list the procedure groups that could carry a question that passes the
eval's audit. Mining a repo costs a corpus change and hours of model calls, so your list decides where that is paid.

Paths: `W` = `<work>`, `PS` =
`W/procedure-screen`, `RQ` = `W/retrieval-questions`, `C` = `PS/corpus/corpus`.

## What a procedure question needs

`procedure` asks how to do something in the project. Its facts are the steps, in order, with their commands: two or
more. A flow the sessions ran more than once is a procedure, and so is a one-time setup whose own values (a remote, a
host, a path) a later question can name. Running another repository's tools or gates as part of this project's own
workflow is this project's procedure. The audit passes a question only when all of these hold:

1. **Supported:** session lines state each step.
2. **Not in the files:** the repo's files at its end trees do not state the steps, or all of them but what is generic,
   guessable or not asked.
   - Files are the code and its comments, configs, tests, scripts, CI workflows, docs (ADRs, plans, architecture
     notes, runbooks) and the last session's own docs even on an unmerged branch.
   - The agent instruction files count too: `CLAUDE.md`, `AGENTS.md`, `CONTRIBUTING*`, `.claude/` (commands, skills,
     settings), `.cursor/rules/`, `.agents/skills/*/SKILL.md`, and READMEs, changelogs and vendored specs.
   - **A runbook written with stock commands counts** (`wrangler deploy --env production` in a README; a deploy skill
     in `.claude/skills/`). The funnel's token grep missed exactly these (hevy-planner, SprintSpark), so read every
     instruction file and doc at the end trees in full, not only grep hits.
   - A file that holds only the names, and not the order of the steps, the host, the path or what was tried, does not
     count. Another repository's files, a file on a server, and the git log are not this project's files.
3. **Not guessable, step by step:** a developer who knows the stack, with no memory of these sessions, would not give
   most of the steps in substance. A question keeps its value when one of its facts is session-only: a name, value,
   path or host the project chose; the cause of a failure; how a tool behaved in this setup; an order that matters
   and is written nowhere. It fails when every substantive step is stock: a tool's documented flags, a fix its error
   text prescribes, the usual practice.
4. **Current:** no later session in the window, and no commit before the window's end, changed it.
5. **Unique:** the window holds no second fair answer, such as another way the sessions did the same thing, or two
   people doing it differently.
6. **Worth asking:** a later developer would want to know it. Not a move incidental to one task, not a claim nothing
   acted on, not a tool's own printed steps that the sessions only followed.
7. **Askable:** a developer can ask it without stating the answer, in words a later session would use for the topic.

## Inputs, and nothing else

- `PS/gate/<owner>__<repo>/clusters.md`: the funnel's recurring command clusters that two or more window sessions
  hold, outside-the-repo and project-specific ones first, with example commands and sessions. A ranking aid: the
  clusters find command-shaped flows only; a flow typed as prose or done through an MCP tool is yours to find in the
  views.
- `PS/gate/<owner>__<repo>/window.json`: the window's sessions (id, time, size, cwd, view file). Only these sessions
  exist for the eval: a group whose evidence lies outside them does not count. When the clusters show a group the
  window cut, note it (`cut_by_window`).
- The views: `RQ/<owner>__<repo>/view/<first 8 of the session>.txt`. Every line starts `L<raw line> <UTC time>
  <ROLE>:`; `L<n>` is the number evidence uses. Read them only by line range (`sed -n`), never whole; search them
  only with `grep -a -n` (the shell's `grep` skips a file it takes for binary). Or print a range with
  `RQ_CORPUS=$C python3 -I RQ/tools-v2/render.py show <owner>__<repo> <session> <first> <last>` (from `RQ/`).
- The clone: `RQ/repos/<owner>__<repo>.git`, read-only git (`grep`, `show`, `log`, `ls-tree`). It runs past the
  window, so name trees by sha, never `HEAD` or a branch.
- The end trees: `RQ/<owner>__<repo>/trees.json` (`commits`, `last_session`, `default_head`, `branch_last`,
  `session_branches`). List a tree's files with `git -C <clone> ls-tree -r --name-only <sha>`.
- For the calibration repo (`ngngardner__readaloud`) only: its views are in `RQ/ngngardner__readaloud/view/`, its
  clone in `RQ/repos/`, its trees in `RQ/ngngardner__readaloud/trees.json`, and the tools are `RQ/tools/` with no
  `RQ_CORPUS`. **Never read** anything that carries an earlier verdict on it: `questions.jsonl`, `dropped.jsonl`,
  `spares.jsonl`, `selected.jsonl`, `drafts.jsonl`, `verdicts/`, `final/`, `review/`, any `AUDIT.md`, `LEAD-LOG*`,
  `DONE-*`, `PLAN*`, `BRIEF-*` other than this one, the eval's README and question files, and the bilbo notes.
- For every repo: do not read `W/procedure-repos/SHORTLIST.md` or other judges' folders. Use no notes or memory tool.

## Per repo

1. **Orient (10 minutes).** Read `clusters.md` and `window.json`. List the end tree's instruction files and docs and
   read them all (they are what kills most groups). Note the window's languages.
2. **Find the groups.** Start from the outside and specific clusters, then the developer's own asks in the views
   (`grep -a -n -i` on `USER` lines for deploy, release, backup, restore, migrate, ssh, server, prod, run, rerun,
   setup, again, how do, remember, always, and the clusters' tokens). A group is one flow: the steps one question
   would ask for. Examine at most 12 groups, the most promising first.
3. **Per group, kill only on evidence:**
   - `in_files`: a file line at an end tree states the steps (or all but what is generic): give `file`, `line`,
     `tree` (sha 8). When the files state part and you judge the rest generic, still `in_files`, with
     `borderline: true` and the leftover in `note`.
   - `changed`: a later window line or a commit before the window's end changed the steps: give `<session 8>:L<n>`
     or `commit <sha 8>`.
   - `outside_window`: the evidence that makes it a procedure lies outside the window.
   - `not_class`: no steps (a state, a value, a single command with no order), or a pure record of a change made.
   When unsure whether evidence kills, forward: a wrong forward costs one review, a wrong kill loses the group.
4. **Forward the rest** with flags, each with one line of why: `generic` (name the stock steps and the session-only
   one), `second_answer` (the other answer's session and line), `one_off`, `third_party`, `askable`, `two_people`.
   Then rate the group's odds of passing the audit, as a ranking aid only, never a kill:
   - `high`: no flag, or one you think the audit will clear; a session-only fact sits in the steps themselves.
   - `medium`: one flag you think is a real risk, or the session-only part is a single value.
   - `low`: likely fails (mostly stock, one-off, or a second answer in sight).
5. **Rules and refind, in passing.** Count the strong `rules` items and refind events (incidents, findings, deploys,
   not commits) you met while reading. A count, no list, no extra reading.

Time: about 45 minutes a repo; past 75, finish with what you have.

## Output

`PS/gate/<owner>__<repo>/groups.jsonl`, one object per examined group, appended as soon as it is judged:

| field | value |
|---|---|
| `repo` | `owner__repo` |
| `group` | `<short>/<a few words in kebab case>` |
| `verdict` | `forward`, or a kill: `in_files`, `changed`, `outside_window`, `not_class` |
| `odds` | for `forward`: `high`, `medium` or `low`; else `null` |
| `flags` | `[{"criterion": "generic", "why": "..."}]`, empty when none |
| `steps` | the steps in order, each one line with its command or value |
| `evidence` | `[{"session": "<full id>", "lines": [a, b]}]`, one per step or per session that runs the flow |
| `runs` | the sessions (8 characters) that ran the flow, and their UTC days |
| `residue` | one sentence: what only the sessions hold |
| `subject` | `{"session": "<full id>", "line": n, "quote": "...", "source": "user"}`: 2 to 15 words copied exactly from one developer turn that names the topic, holding no step, command, value or the project's name; `"source": "assistant"` when only the agent's words name it; `null` with an `askable` flag when nothing does |
| `answer` | for `in_files`: `{"file": "...", "line": n, "tree": "<sha 8>"}`; else `null` |
| `changed_by` | for `changed`; else `null` |
| `cut_by_window` | `true` when the clusters show runs outside the window that would have strengthened it |
| `note` | at most 300 characters |

Then `PS/gate/<owner>__<repo>/VERDICT.md` (at most 15 lines): forwarded groups by odds; the forecast `0.6 x high +
0.3 x medium + 0.1 x low` and whether it reaches 1.0; kills by kind; groups the window cut; the rules and refind counts
of step 5; languages; anything the lead must know (two people, another project's sessions, a secret you saw: count
only, never the value).

## Rules

- Do not spawn agents, forks or teammates. Do not use SendMessage or ListAgents.
- No model call of any kind: no `mine.py` stage, no proxy, no `claude`, no `codex`. `render.py show` is fine.
- Write only `PS/gate/<owner>__<repo>/groups.jsonl`, `VERDICT.md`, `PROGRESS.md` and `PS/DONE-gate-<short>.md`. Name
  scratch helpers `gate-<short>-*` in the scratchpad. No git writes. Never edit a view, a clone, `RQ/` or `PS/` files
  of anyone else.
- Transcripts hold real secrets: never quote one into your output or your messages.

## Progress and finish

- Append `<UTC time> <groups done>/<examined> last=<group>` to `PS/gate/<owner>__<repo>/PROGRESS.md` at least every
  30 minutes.
- `PS/DONE-gate-<short>.md`: one line with the counts and the forecast. Final message: three lines.
````

</details>

<details>
<summary><code>BRIEF-draft-gp.md</code>: tells a drafter to write a row for each procedure group the first drafting left uncovered</summary>

sha256 `5ca4cd1a9be6a681321b9574ff8c9dda079db8327d93f64ff988e05e43e1bfad`

````text
# Brief `draft-gp`: draft rows for the gate's uncovered procedure groups

You are a drafter (Claude Sonnet 5.5): the lane worker of `BRIEF-mine.md` at its step 7, or a drafter the lead
started with this brief. The gate judge forwarded procedure groups for your repo `R`, and the frozen drafting left some
of them without a live row (`PS/mine/<short>/coverage.json`, `uncovered`). You turn each uncovered group with odds
`high` or `medium` into one candidate row. A script issues the ids; the frozen prompt and query writers then write the
row's `prompt` and `query` without seeing your facts; sol and an Opus audit check it. You write no prompt and no
query. This is the second round's method (`RQ/BRIEF-draft-rp.md`, which drafted rules rows from probe groups), used
for procedure.

Paths: `W` = `<work>`, `PS` =
`W/procedure-screen`, `RQ` = `W/retrieval-questions`, `C` = `PS/corpus/corpus`. Run commands from `RQ/` with
`RQ_CORPUS=$C` on the line.

## What you read, and nothing else

- Your groups: `PS/gate/<R>/groups.jsonl` rows named in `coverage.json`'s `uncovered`, with odds `high` or `medium`:
  their `steps`, `evidence`, `runs`, `residue`, `subject`, `flags` and `note`.
- The ledger, `RQ/<R>/ledger.jsonl`: find the items whose evidence the group's lines fall in (same session, lines
  within 20); they become the row's `items`. A group with no ledger item cannot be drafted: report it.
- The sessions: `RQ_CORPUS=$C python3 -I tools-v2/render.py show <R> <session> <first> <last>`, or
  `RQ/<R>/view/<first 8>.txt` by line range, searched only with `grep -a`. Never a whole session.
- The live procedure rows, to avoid them: `RQ/<R>/selected.jsonl` (`row.facts` evidence and `meta.items`).
- Do not read `verdicts/`, `review/`, any `final/`, the other repos' folders, the plan's other briefs or logs.

## A procedure row

`procedure` asks how to do something in the project. Its facts are the steps, in order, with their commands, two or
more, listed in step order. The question must stand on the group's residue: the steps or values only the sessions
hold.

## The draft row

One JSON object per line in `RQ/<R>-gp/drafts-gp.jsonl`, one per group:

```json
{
  "cand": "gp-<the part of the group key after the slash>",
  "group": "<short>/<...>",
  "class": "procedure",
  "items": ["<ledger id>", "..."],
  "facts": [
    {"text": "...", "keys": ["..."],
     "evidence": [{"session": "<full session id>", "lines": [<first>, <last>], "quote": "..."}]}
  ],
  "stale": [],
  "subject": {"session": "<full session id>", "line": <n>, "quote": "...", "source": "user"},
  "note": "..."
}
```

## Facts

- **One step each,** in the order the sessions run them. Every fact is required of an answer: no optional or
  decorative step.
- **Only what its own evidence states.** If a step needs a second line, add a second evidence entry. If no line
  states a step, leave it out. The earlier rounds lost rows to facts that said more than their quotes.
- **Leave out what the files hold** (the gate's `answer` or `note` names it) and what any developer would do anyway,
  unless the order or a value makes it this project's; the question rests on the session-only part.
- **True at the end of the window.** When a later line changed a step (a host, a path, a flag), the fact takes the
  latest one and the older one becomes a `stale` item with `replaces`.
- **No fact a live row already holds** (`selected.jsonl`).

## Keys

An exact string from the fact's evidence lines that a right answer must contain: a command, a host, a path, a flag,
a script name. Never the topic's name. Zero to two per fact. Never in the subject.

## Evidence

`session`: the full id. `lines`: `[first, last]`, raw line numbers (the view's `L<n>`), at most 40 apart. `quote`:
copied from the text of one line in that range (whitespace may differ), at least six words or 30 characters, no
redaction placeholder, not from a compaction summary, **and never a secret** (a token, password, key or connection
string with credentials): pick another line or another quote.

## The subject decides what is asked

The prompt writer sees only the class and the subject. The subject must name exactly the flow the facts answer: not
broader (a whole task makes the prompt ask more than the facts give), not the steps themselves (the prompt would hand
over the answer). Default: the gate's `subject`. Sources, in order: `user` (a whole clause of a developer's typed
turn, 2 to 15 words, copied exactly, from a session of the row's evidence, at or before its last evidence line),
`ledger` (an item's `subject`, copied exactly, `{"quote": "...", "source": "ledger"}`), `assistant` (the agent's
prose, only when nothing else names it). No project name, no key, nothing that starts with "that", "this" or "it". It
may be in the developer's language.

## Check before you finish

`python3 -I ../procedure-screen/scripts/issue_ids_gp.py --repo <short>` (a dry run that writes nothing) applies the
checker's own rules and prints `PROBLEM` and `WARNING` lines. Fix every `PROBLEM`; read every `WARNING` (a fact quoting
a committed question's line is a duplicate: rewrite or drop the group). The lane worker runs `--write` once it is
clean; a drafter the lead started does not.

## Rules

- Do not spawn agents, forks or teammates. Do not use SendMessage or ListAgents.
- No model call of any kind while drafting.
- Write only `RQ/<R>-gp/drafts-gp.jsonl` and your notes in `PS/mine/<short>/`. No git writes. Never print a secret.

## Finish

In `PS/mine/<short>/gp-notes.md` (at most 10 lines): the groups drafted with facts per row, subjects you replaced and
why, groups you could not draft and why, the last dry run's result.
````

</details>

## Corrections

Any change after the questions are committed is listed here, one line each.

- 2026-10-13: added 3 rules questions (linear-cycle-drain-172, readaloud-156, readaloud-158) from a second round,
  and 11 rows to `dropped.jsonl`; counts, headline classes, method and briefs updated.
- 2026-10-13: dropped the floor of 10 questions per class: every class now counts in the headline at its weight,
  decided before any strategy was scored.
- 2026-10-13: dropped `target` and `no_answer_target` from `classes.json` and the target column from the counts:
  nothing scores by them, and the plan they set is under Filling.
- 2026-10-14: dropped 5 `refind` questions (kimi-cli-024, linear-cycle-drain-105, linear-cycle-drain-106,
  readaloud-101, port-cli-081) whose event or detail is a commit, push or pull request, as `commit_event`, and 5
  audited refind spares for the same reason; re-drafted readaloud-100's detail, a commit hash, and checked and
  audited it again.
- 2026-10-14: filled `refind` to 20 with port-cli-083, an audited spare, and 16 questions from the first round's
  unused drafts; 21 rows to `dropped.jsonl`; counts, drops, method and the `commit_event` reason updated.
- 2026-10-14: added 9 `procedure` questions from five projects chosen by a screen (safecast-map-MCP, bosarsalog,
  go-nuts, neuralips26, contoso; 89 sessions) and 68 rows to `dropped.jsonl`; the corpus, counts, drops, method and
  briefs updated.
