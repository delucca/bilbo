"""Reword the answerable questions for the wording test. Usage: reword.py <questions.jsonl> <questions-readme.md> <out.jsonl> <log.jsonl> [--jobs N].
gpt-6.1-sol, through `codex exec`, rewords each prompt seeing only the prompt; Claude Opus 5.5 checks that the
rewrite asks the same thing and repeats no topic word, and a failure goes back to sol once with the reason. The frozen
query_writer.md, read word for word from the questions README, then writes each query from the new prompt with Opus,
as the first round did. out.jsonl holds every answerable row with its prompt and query replaced, plus `reword` (the
originals, the kept terms and the check's last verdict); log.jsonl holds every call. A row already in out.jsonl is
not reworded again. BILBO_EVAL_JUDGE_CWD and BILBO_EVAL_CODEX_HOME are as for judge.py."""

import argparse
import json
import os
import re
import subprocess
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

WRITER = "gpt-6.1-sol"
CHECKER = "claude-opus-5-5"
QUERY_WRITER = "claude-opus-5-5"
BATCH = 12
TRIES = 3

REWORD = """You reword messages that a developer typed to their coding agent at the start of a session, asking about
earlier work on their project. For each message write `prompt`: a message that asks for exactly the same thing in
other words, as the same developer might have typed it on another day.

- The same request: the same thing asked about, the same kind of answer wanted (what was decided and why, where the
  work stopped, how to do something, a value, when something happened), the same scope. Someone who knows the project
  would give both messages the same answer.
- Other words: replace every word that names the topic with another word or phrasing of the same meaning, such as a
  synonym or a description. A change of form only (plural, tense, -ing) is not another word.
- Keep a term as it is only when it has no other name: code identifiers, file and path names, flags, commands, error
  text, numbers and versions, and the names of products, libraries, tools and services. List every term you kept in
  `kept`, empty when none.
- Add nothing the message does not hold: no value, file, name, number or detail. Leave out nothing it asks for.
- Keep its style: short and direct, often in lower case, a question or a request, in English. Do not name the project.
{repair}
## The messages

{messages}

## Output

One JSON object with `prompts`, one entry per message, by its `id`:
{{"prompts": [{{"id": "...", "prompt": "...", "kept": ["..."]}}]}}
"""

REPAIR = """
A check rejected an earlier rewording of each message below; its reason follows the message. Write a new one that
meets the rules and the reason.
"""

CHECK = """Each pair below is a developer's message to their coding agent about earlier work on their project, and a
rewording of it. For each pair decide:

- `same`: true when someone who knows the project would give both messages the same answer. False when the rewording
  asks for more, less or something else, widens or narrows the scope, or adds a detail.
- `reused`: the words of the original that name its topic and that the rewording repeats, as the same word or the
  same word in another form (plural, tense, -ing). Leave out function words, words that only say what kind of answer
  is wanted (decide, why, status, how, steps, when), and terms that have no other name: code identifiers, file and
  path names, flags, commands, error text, numbers and versions, and the names of products, libraries, tools and
  services. Empty when none.
- `reason`: one sentence when `same` is false or `reused` is not empty, else empty.

## The pairs

{pairs}

## Output

One JSON object with `checks`, one entry per pair, by its `id`:
{{"checks": [{{"id": "...", "same": true, "reused": [], "reason": ""}}]}}
"""


def frozen(readme, name):
    """The frozen prompt `name` from the questions README, word for word."""
    m = re.search(rf"<summary><code>{re.escape(name)}</code>.*?</summary>\s*````text\n(.*?)\n````", readme, re.S)
    if not m:
        raise SystemExit(f"reword: no {name} in the questions README")
    return m.group(1)


def ask(model, prompt):
    if model.startswith("claude-"):
        p = subprocess.run(
            ["claude", "-p", "--model", model, "--output-format", "json", "--tools", "", "--disable-slash-commands",
             "--strict-mcp-config", "--setting-sources", "project", "--settings", '{"disableAllHooks": true}',
             "--no-session-persistence", "--system-prompt", "You reply with JSON only."],
            input=prompt, capture_output=True, text=True, timeout=900, cwd=os.environ["BILBO_EVAL_JUDGE_CWD"])
        reply = json.loads(p.stdout)
        if reply.get("is_error"):
            raise ValueError(f"is_error: {str(reply.get('result'))[:200]}")
        return reply["result"]
    with tempfile.TemporaryDirectory() as tmp:
        last = Path(tmp) / "last.md"
        p = subprocess.run(
            ["codex", "exec", "-m", model, "-c", 'model_reasoning_effort="high"', "-c", 'web_search="disabled"',
             "--ignore-rules", "--ephemeral", "--skip-git-repo-check", "-s", "read-only", "-o", str(last),
             *(a for f in ("hooks", "plugins", "apps", "multi_agent") for a in ("--disable", f)),
             "-C", os.environ["BILBO_EVAL_JUDGE_CWD"], "-"],
            input=f"You reply with JSON only.\n\n{prompt}", capture_output=True, text=True, timeout=900,
            env={**os.environ, "CODEX_HOME": os.environ["BILBO_EVAL_CODEX_HOME"]})
        if p.returncode or not last.exists() or not last.read_text().strip():
            raise ValueError(f"codex: exit {p.returncode}: {p.stderr.strip()[-200:]}")
        return last.read_text()


def call(log, step, model, prompt, key, ids):
    """One batch call whose reply lists exactly `ids` under `key`; logged whole."""
    last = "no try"
    for _ in range(TRIES):
        try:
            text = ask(model, prompt)
            body = re.sub(r"^```(?:json)?\s*|\s*```$", "", text.strip())
            rows = {r["id"]: r for r in json.loads(body)[key]}
            if set(rows) != set(ids):
                raise ValueError(f"ids {sorted(rows)} for {sorted(ids)}")
            log({"step": step, "model": model, "prompt": prompt, "reply": text})
            return rows
        except (ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as e:
            last = f"{type(e).__name__}: {e}"
    raise RuntimeError(f"{step} {ids[0]}..: {last}")


def batches(xs):
    return [xs[i:i + BATCH] for i in range(0, len(xs), BATCH)]


ap = argparse.ArgumentParser()
ap.add_argument("questions")
ap.add_argument("readme")
ap.add_argument("out")
ap.add_argument("log")
ap.add_argument("--jobs", type=int, default=4)
args = ap.parse_args()

query_writer = frozen(Path(args.readme).read_text(), "query_writer.md")
rows = [json.loads(line) for line in Path(args.questions).read_text().split("\n") if line.strip()]
out = Path(args.out)
done = {json.loads(line)["id"] for line in out.read_text().split("\n") if line.strip()} if out.exists() else set()
todo = [q for q in rows if not q["abstain"] and q["id"] not in done]
logf = Path(args.log).open("a")


def log(r):
    logf.write(json.dumps(r, ensure_ascii=False) + "\n")
    logf.flush()


def reword(qs, reasons):
    repair = REPAIR if reasons else ""
    msgs = "\n".join(f"- id: {q['id']}; message: {q['prompt']}" + (f"; reason: {reasons[q['id']]}" if reasons else "")
                     for q in qs)
    return call(log, "reword", WRITER, REWORD.format(repair=repair, messages=msgs), "prompts", [q["id"] for q in qs])


def check(qs, new):
    pairs = "\n".join(f"- id: {q['id']}; original: {q['prompt']}; rewording: {new[q['id']]['prompt']}" for q in qs)
    return call(log, "check", CHECKER, CHECK.format(pairs=pairs), "checks", [q["id"] for q in qs])


def queries(qs, new):
    msgs = "\n".join(f"- id: {q['id']}; message: {new[q['id']]['prompt']}" for q in qs)
    return call(log, "query", QUERY_WRITER, query_writer.replace("{{messages}}", msgs), "queries", [q["id"] for q in qs])


def merged(fn, groups, *extra):
    got = {}
    with ThreadPoolExecutor(args.jobs) as pool:
        for r in pool.map(lambda g: fn(g, *extra), groups):
            got.update(r)
    return got


new = merged(reword, batches(todo), {})
verdict = merged(check, batches(todo), new)
failed = [q for q in todo if not verdict[q["id"]]["same"] or verdict[q["id"]]["reused"]]
if failed:
    reasons = {q["id"]: verdict[q["id"]]["reason"] or "it repeats a topic word" for q in failed}
    for g in batches(failed):
        new.update(reword(g, {q["id"]: reasons[q["id"]] for q in g}))
    verdict.update(merged(check, batches(failed), new))
query = merged(queries, batches(todo), new)

with out.open("a") as f:
    for q in todo:
        v = verdict[q["id"]]
        f.write(json.dumps({**q, "prompt": new[q["id"]]["prompt"], "query": query[q["id"]]["query"],
                            "reword": {"prompt": q["prompt"], "query": q["query"], "kept": new[q["id"]]["kept"],
                                       "same": v["same"], "reused": v["reused"], "repaired": q in failed}},
                           ensure_ascii=False) + "\n")
still = [q["id"] for q in todo if not verdict[q["id"]]["same"] or verdict[q["id"]]["reused"]]
print(f"reword: {len(todo)} reworded, {len(failed)} repaired, {len(still)} still failing the check"
      + (f": {', '.join(still)}" if still else ""))
