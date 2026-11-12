# Retrieval results

Saved scores from the retrieval eval. Each arm’s JSON file is a copy of its run’s
`score.json`; the [results table](../reference.md#results) records the run dates, store keys,
class scores, and judge checks. The [variation analysis](../reference.md#query-variation)
reports sensitivity to question wording separately.

## Read the scores

- **Headline (`query`)**: the weighted mean of class scores. Each question scores the
  fraction of its reference facts found in retrieved context.
- **`prompt`**: retrieval using the developer’s message instead of the agent’s search query.
- **Misled**: an older value was retrieved without the current fact that replaces it.
- **Gold and random controls**: evidence lines and unrelated lines from the same project,
  packed to the same budget and judged the same way.

Runs use 143 answerable questions and a 4,000-token context budget. They measure retrieval,
not generated answers. No-answer questions are not scored yet.

## Comparison limits

The corpus is selected, with later projects screened for operational procedures. Tools use
different ingestion paths and models, and some return more results than their defaults.
Claude-mem’s `prompt` score measures its session-start injection rather than a prompt search.
bilbo-next searches one store of all 22 projects, and its contexts hold about 3,400 of the
4,000 tokens; every other arm searches the question’s project alone.
Query variations preserve project vocabulary, so they test framing rather than independence
from the wording of the source sessions.

Read the [arm-specific caveats](../reference.md#caveats) with the numbers.
To reproduce a run, follow the [eval instructions](../README.md#run-a-small-eval).
