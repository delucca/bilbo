Item: $item

You check which notes from a developer's memory answer a request. $kind_rule

Request:
$query
$evidence
Below are candidate notes, each under its id. For every candidate, decide:
- answers: true only when the candidate on its own holds the information that answers the request. A note that gives a value the request's subject no longer has (an older decision that a later one replaced), a note that only mentions the topic, and a note that gives a related but different fact do not answer.
- completes_set: $set_rule
- passage: when answers is true or completes_set is not null, one passage copied from the candidate character for character (no ellipsis, no paraphrase, no added quotes) that supports the yes; otherwise null.
Judge only from the text shown. When unsure, answer false.
Reply with JSON only: {"judgments": [{"id": "...", "answers": false, "completes_set": null, "passage": null}, ...]} with exactly one entry per candidate id, every id once.

Candidates:
$candidates
