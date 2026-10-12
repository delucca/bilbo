# The questions

Each question asks for something one project's sessions settled: what the developer decided, was told, tried or left
open. Its answer is a list of facts, each pointing at the session lines that state it, and holds at the end of the
project's sessions, since every store is built from all of them.

## Files

- `questions.jsonl`: the questions, one per line. Committing it freezes it.
- `dropped.jsonl`: every candidate question that did not make it, with the reason.
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

A candidate is dropped, and written to `dropped.jsonl` with its `reason`, when:

- `in_repo`: the project's files, at the commit of its last session, state every fact. An agent would open the file.
  `detail` names the file and line. A file that holds only the names, and not the reason, the order or what was
  tried, does not count.
- `unsupported`: its evidence does not state a fact.
- `not_current`: a later session changed the answer.
- `not_unique`: more than one answer fits.
- `guessable`: a model with no memory of the sessions answers it.
- `duplicate`: another question asks the same.

## How the questions are made

1. Claude Opus 5.5 reads each project's sessions in order and lists its work: what was decided, the rules, the steps,
   the state of each piece of work, each with a quote and its lines. A script checks every quote.
2. Moments where the developer reached back to an earlier session become questions first, when their answer still
   holds at the end.
3. Opus drafts the rest per class from the list. Another call, which sees only the class and the subject in the
   developer's own words, writes the `prompt`; a fixed model writes the `query` from the prompt and nothing else.
4. gpt-6.1-sol, from another model family, checks every question against the rules above.
5. Opus audits a sample, then the questions are committed.

The prompts are added here, word for word, with the questions.

## Corrections

Any change after the questions are committed is listed here, one line each.
