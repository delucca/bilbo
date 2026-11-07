# Agent comparison

Check whether the eval’s fixed rules for opening search hits approximate what a real coding
agent reads. This runs both the rule and an agent on the same questions, judges their
retrieved context the same way, and compares the scores.

Supported arms are `bilbo-agent-v0_19`, `bilbo-assisted-v0_19`, `ai-memory`,
`claude-mem`, and `auto-memory`.
The default sample is the 20 IDs in [questions.txt](questions.txt).

## Run

Use the [eval prerequisites](../README.md#run-a-small-eval) and export
`CLAUDE_CODE_OAUTH_TOKEN` from a Claude subscription’s `claude setup-token` output.
From the repository root:

```sh
evals/retrieval/agent/run.sh bilbo-agent-v0_19
```

`--questions id,id,...` selects questions; `--jobs N` sets concurrency.
`BILBO_EVAL_AGENT_MODEL` overrides the default `claude-sonnet-5-5`.

The agent runs inside the arm’s container with its memory tools and no project code.
A hook limits reading to approximately 4,000 tokens; retrieved text is then packed and
judged under the main eval’s exact 4,000-token budget.

## Outputs

Runs resume from `~/.cache/bilbo-evals/retrieval/eval/agent-<arm>-<storekey>-<agentkey>/`
(or the base set by `BILBO_EVALS_CACHE`). They retain transcripts, retrieved context,
verdicts, and a comparison with the fixed rule.

[drive.py](drive.py) runs the agent, [budget.py](budget.py) enforces the reading limit,
and [compare.py](compare.py) compares scores and reading behavior.
See the [recorded experiment](../reference.md#running-the-eval) for findings and limits.
