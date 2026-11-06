"""Write each answerable question's query variation into questions.jsonl. Usage:
vary.py <questions.jsonl> <questions-readme.md> <keep.jsonl> <log.jsonl> [--jobs N] [--redo id,id,...]
Only `wording.variation` is added, to the answerable rows without one and to the --redo rows, whose variation is dropped
first and made again with new calls. Every attempt goes to log.jsonl, and a call whose exact prompt the log answered is
not made again. The file is replaced at the end, and not at all if it changed since the run began.
BILBO_EVAL_JUDGE_CWD and BILBO_EVAL_CODEX_HOME are as for judge.py."""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import keep

WRITER = "gpt-6.1-sol"
CHECKER = "claude-opus-5-5"
QUERY_WRITER = "claude-opus-5-5"
BATCH = 12
TRIES = 3
CODEX_OFF = ("hooks", "plugins", "apps", "multi_agent", "code_mode_host", "shell_tool", "unified_exec", "view_image",
             "browser_use", "computer_use", "sleep_tool", "skill_search", "tool_suggest")
FIELDS = {"prompts": {"prompt": str}, "queries": {"query": str},
          "checks": {"same": bool, "prompt_ok": bool, "query_ok": bool, "renamed": list, "added": list, "reason": str}}

VARY = """You write variations of messages that a developer typed to their coding agent at the start of a session, asking
about earlier work on their project. For each message write `prompt`: the same request, as the same developer might
type it on another day.

- The same request: the same thing asked about, the same kind of answer wanted (what was decided and why, where the
  work stopped, how to do something, a value, when something happened), the same scope, quantities and time. Someone
  who knows the project would give both messages the same answer.
- Keep every term under `keep`. A term marked `exact` stays character for character, letter case and separators
  included. A term marked `capitals` stays in the same capitals; a plural is fine. Any other term keeps its words; a
  change of form (plural, tense, -ing) is fine. The one exception, for a term of any kind, an `exact` one included, is
  a listed `equivalent`, which the same developer typed in this project: it may stand in the term's place. Nothing
  else may.
- Keep the names the list may have missed as they are too: code identifiers, file and path names, flags, commands and
  subcommands, error text, numbers, versions, ticket and pull request ids, the names of products, libraries, tools and
  services, and the project's own names for its components, features, pages and units of work.
- Say the rest in your own words: the framing, the word order, and the ordinary words that describe the topic. Use
  words a developer would really type; a long description no one would type is not another way to say it. You need
  not replace every word.
- Add nothing the message does not hold: no value, file, name, number, mechanism or cause. Leave out nothing it asks
  for.
- Keep its style: short and direct, often in lower case, a question or a request, in English. Do not name the project.
{repair}
## The messages

{messages}

## Output

One JSON object with `prompts`, one entry per message, by its `id`:
{{"prompts": [{{"id": "...", "prompt": "..."}}]}}
"""

REPAIR = """
A check rejected an earlier variation of each message below; the rejected variation and the reasons follow the
message. Write a new one that meets the rules and the reasons.
"""

CHECK = """Each case below is a developer's message to their coding agent about earlier work on their project, the search
query written from it, and a variation of both: the message asked again in other words, and the query written from
that. `keep` lists the terms of the message that the variation had to keep, each with how (`exact`: character for
character; `capitals`: the same capitals) and the equivalents the same developer types. For each case decide:

- `same`: true when someone who knows the project would give the original and the variation the same answer: the same
  thing asked about, the same scope, the same quantities and time, the same kind of answer wanted. False when the
  variation asks for more, less or something else, widens or narrows it, or reads a term in another sense.
- `renamed`: the names that the variation's message replaces with another word or a description, or that the
  variation's query replaces or leaves out while the original query holds them. Names are the `keep` terms and any
  name the list missed: a code identifier, a file or path, a command or subcommand, a flag, a number, a version, a
  ticket or pull request id, the name of a product, library, tool or service, or the project's own name for a
  component, a feature, a page or a unit of work. A listed equivalent is no rename for a term of any kind, an `exact` one included; a change of form (plural, tense,
  -ing) or of letter case of an ordinary word is none either. Otherwise an `exact` term must keep its exact spelling.
  Empty when none.
- `added`: what the variation's message or query holds that the original message does not: a value, a name, a file, a
  mechanism, a cause, a detail of the answer. Words that only say what kind of answer is wanted (status, decision,
  why, steps, date) are not added. Empty when none.
- `prompt_ok`: true when the variation's message alone has none of these faults: another answer, a renamed name, an added
  detail.
- `query_ok`: true when the variation's query alone has none of them: it keeps the names the original query holds and
  adds nothing, whatever the message does.
- `reason`: one sentence when `same` is false, `prompt_ok` or `query_ok` is false, or `renamed` or `added` is not empty,
  else empty.

Reusing the original's words is fine: do not count it against the variation.

## The cases

{cases}

## Output

One JSON object with `checks`, one entry per case, by its `id`:
{{"checks": [{{"id": "...", "same": true, "prompt_ok": true, "query_ok": true, "renamed": [], "added": [], "reason": ""}}]}}
"""


def frozen(readme, name):
    """The frozen prompt `name` from the questions README, word for word."""
    m = re.search(rf"<summary><code>{re.escape(name)}</code>.*?</summary>\s*````text\n(.*?)\n````", readme, re.S)
    if not m:
        raise SystemExit(f"vary: no {name} in the questions README")
    return m.group(1)


class Failed(ValueError):
    """A failed call, with the usage and cost that were known."""

    def __init__(self, message, meta):
        super().__init__(message)
        self.meta = meta


def limited(text):
    return "429" in text or "rate limit" in text.lower()


def ask(model, prompt):
    """The reply's text and usage; a failure raises Failed."""
    if model.startswith("claude-"):
        p = subprocess.run(
            ["claude", "-p", "--model", model, "--output-format", "json", "--tools", "", "--disable-slash-commands",
             "--strict-mcp-config", "--setting-sources", "project", "--settings", '{"disableAllHooks": true}',
             "--no-session-persistence", "--system-prompt", "You reply with JSON only."],
            input=prompt, capture_output=True, text=True, timeout=900, cwd=os.environ["BILBO_EVAL_JUDGE_CWD"])
        try:
            reply = json.loads(p.stdout)
        except ValueError:
            if limited(p.stderr):
                time.sleep(60)
            raise Failed(f"claude: exit {p.returncode}: {p.stderr.strip()[-200:]}", {})
        meta = {"usage": reply.get("usage"), "cost_usd": reply.get("total_cost_usd")}
        if reply.get("is_error"):
            if limited(f"{reply.get('result')} {p.stderr}"):
                time.sleep(60)
            raise Failed(f"is_error: {str(reply.get('result'))[:200]}", meta)
        return reply["result"], meta
    with tempfile.TemporaryDirectory() as tmp:
        last = Path(tmp) / "last.md"
        p = subprocess.run(
            ["codex", "exec", "-m", model, "-c", 'model_reasoning_effort="high"', "-c", 'web_search="disabled"',
             "--ignore-rules", "--ephemeral", "--skip-git-repo-check", "-s", "read-only", "--json", "-o", str(last),
             *(a for f in CODEX_OFF for a in ("--disable", f)), "-C", os.environ["BILBO_EVAL_JUDGE_CWD"], "-"],
            input=f"You reply with JSON only.\n\n{prompt}", capture_output=True, text=True, timeout=900,
            env={**os.environ, "CODEX_HOME": os.environ["BILBO_EVAL_CODEX_HOME"]})
        usage = None
        for line in p.stdout.split("\n"):
            if '"turn.completed"' in line:
                usage = json.loads(line).get("usage")
        if p.returncode or not last.exists() or not last.read_text().strip():
            if limited(p.stderr):
                time.sleep(60)
            raise Failed(f"codex: exit {p.returncode}: {p.stderr.strip()[-200:]}", {"usage": usage})
        return last.read_text(), {"usage": usage}


def parse(text, key, ids):
    """The reply's entries by id, each with every field of FIELDS[key] in its type; anything else raises."""
    body = re.sub(r"^```(?:json)?\s*|\s*```$", "", text.strip())
    entries = json.loads(body)[key]
    if not isinstance(entries, list):
        raise ValueError(f"{key} is not a list")
    rows = {}
    for r in entries:
        if not isinstance(r, dict) or not isinstance(r.get("id"), str):
            raise ValueError(f"an entry of {key} has no id")
        if r["id"] in rows:
            raise ValueError(f"{r['id']} twice")
        for field, kind in FIELDS[key].items():
            v = r.get(field)
            if not isinstance(v, kind) or (kind is list and not all(isinstance(x, str) for x in v)) \
                    or (kind is str and field != "reason" and not v.strip()):
                raise ValueError(f"{r['id']}: {field} is {v!r}")
        if key == "checks" and not (r["same"] and r["prompt_ok"] and r["query_ok"] and not r["renamed"] and not r["added"]) \
                and not r["reason"].strip():
            raise ValueError(f"{r['id']}: a failing check has no reason")
        rows[r["id"]] = r
    if set(rows) != set(ids):
        raise ValueError(f"ids {sorted(rows)} for {sorted(ids)}")
    return rows


def batches(xs):
    """Batches of rows, the --redo rows apart from the others so that each batch has one call round."""
    parts = [[q for q in xs if q["id"] not in redo], [q for q in xs if q["id"] in redo]]
    return [part[i:i + BATCH] for part in parts for i in range(0, len(part), BATCH)]


def listed(terms):
    def one(t):
        how = {"identifier": " (exact)", "acronym": " (capitals)"}.get(t["kind"], "")
        return t["term"] + how + (f" (equivalents: {', '.join(t['equivalents'])})" if t["equivalents"] else "")
    return "; ".join(one(t) for t in terms) or "(none)"


def lost(terms, original, variation):
    """Kept terms the original text holds and the variation does not, in the form each term's kind allows."""
    return [t["term"] for t in terms if keep.found(t, original) and not keep.found(t, variation)]


def under(original, variation):
    """True when the variation's words are the original's but for function words and contractions."""
    def content(text):
        return Counter(keep.stem(p) for _, _, b in keep.words(text) if b not in keep.STOP for p in b.split("-"))
    return content(original) == content(variation)


ap = argparse.ArgumentParser()
ap.add_argument("questions")
ap.add_argument("readme")
ap.add_argument("keep")
ap.add_argument("log")
ap.add_argument("--jobs", type=int, default=4)
ap.add_argument("--redo", default="")
args = ap.parse_args()

query_writer = frozen(Path(args.readme).read_text(), "query_writer.md")
qpath = Path(args.questions)
start = qpath.read_bytes()
lines = [line for line in start.decode("utf-8").split("\n") if line.strip()]
rows = [json.loads(line) for line in lines]
for line, q in zip(lines, rows):
    if json.dumps(q, ensure_ascii=False) != line:
        raise SystemExit(f"vary: {q['id']} does not read back as written; nothing to do until it does")
redo = {i for i in args.redo.split(",") if i}
unknown = sorted(redo - {q["id"] for q in rows if not q["abstain"]})
if unknown:
    raise SystemExit(f"vary: --redo names {', '.join(unknown)}, which is not an answerable question")
for q in rows:
    if q["id"] in redo:
        q["wording"].pop("variation", None)
keeps = {r["id"]: r for r in (json.loads(line) for line in Path(args.keep).read_text().split("\n") if line.strip())}
todo = [q for q in rows if not q["abstain"] and "variation" not in q["wording"]]
missing = [q["id"] for q in todo if q["id"] not in keeps]
if missing:
    raise SystemExit(f"vary: no keep-list for {', '.join(missing[:5])}; build it again")


def load_log(path):
    """The log's records. A torn last line, from an append cut short, is dropped; a bad line before it is an error."""
    if not path.exists():
        return []
    *complete, tail = path.read_bytes().split(b"\n")
    records, offset = [], 0
    for raw in complete:
        if raw.strip():
            try:
                records.append(json.loads(raw))
            except ValueError:
                raise SystemExit(f"vary: {path} is corrupt at byte {offset}; fix or remove that line")
        offset += len(raw) + 1
    if tail.strip():
        try:
            records.append(json.loads(tail))
            with path.open("ab") as f:
                f.write(b"\n")
        except ValueError:
            print(f"vary: dropped a torn last line of {path} ({len(tail)} bytes)", file=sys.stderr)
            os.truncate(path, offset)
    return records


logpath = Path(args.log)
records = load_log(logpath)
answered = {r["key"]: r["reply"] for r in records if r.get("ok")}
redo_round = 0
if redo:
    opened = {r["round"] for r in records if r.get("redo") == sorted(redo)} - {r["redo_done"] for r in records if "redo_done" in r}
    redo_round = max(opened, default=1 + max([r.get("round", 0) for r in records], default=0))
logf = logpath.open("a")
log_lock = threading.Lock()


def log(r):
    with log_lock:
        logf.write(json.dumps(r, ensure_ascii=False) + "\n")
        logf.flush()


if redo and redo_round not in opened:
    log({"redo": sorted(redo), "round": redo_round})


def call(step, model, prompt, key, ids, retry=0):
    """One batch call whose reply holds exactly `ids` under `key`, each entry whole; every attempt logged. `retry`
    makes the same prompt a new call."""
    rnd = redo_round if redo & set(ids) else 0
    salt = (f"\nredo {rnd}" if rnd else "") + (f"\nretry {retry}" if retry else "")
    call_key = hashlib.sha256(f"{model}\n{prompt}{salt}".encode()).hexdigest()
    if call_key in answered:
        return parse(answered[call_key], key, ids)
    last = "no try"
    for attempt in range(1, TRIES + 1):
        meta = {}
        try:
            text, meta = ask(model, prompt)
            rows = parse(text, key, ids)
        except (ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as e:
            last = f"{type(e).__name__}: {e}"
            meta = getattr(e, "meta", meta)
            log({"step": step, "model": model, "key": call_key, "round": rnd, "retry": retry, "ids": ids,
                 "attempt": attempt, "ok": False, "error": last, **meta})
            continue
        log({"step": step, "model": model, "key": call_key, "round": rnd, "retry": retry, "ids": ids, "attempt": attempt, "ok": True,
             "prompt": prompt, "reply": text, **meta})
        return rows
    raise RuntimeError(f"{step} {ids[0]}..: {last}")


def original(q):
    return q["wording"]["original"]


def vary(qs, repair):
    msgs = "\n".join(f"- id: {q['id']}; message: {original(q)['prompt']}; keep: {listed(keeps[q['id']]['keep'])}"
                     + (f"; rejected variation: {repair[q['id']][0]}; reasons: {repair[q['id']][1]}" if repair else "")
                     for q in qs)
    return call("vary", WRITER, VARY.format(repair=REPAIR if repair else "", messages=msgs), "prompts",
                [q["id"] for q in qs])


def queries(qs, new, retry=0):
    msgs = "\n".join(f"- id: {q['id']}; message: {new[q['id']]['prompt']}" for q in qs)
    return call("query", QUERY_WRITER, query_writer.replace("{{messages}}", msgs), "queries", [q["id"] for q in qs],
                retry)


def check(qs, new, query):
    cases = "\n".join(f"- id: {q['id']}; keep: {listed(keeps[q['id']]['keep'])}; original message: "
                      f"{original(q)['prompt']}; original query: {original(q)['query']}; variation's message: "
                      f"{new[q['id']]['prompt']}; variation's query: {query[q['id']]['query']}" for q in qs)
    return call("check", CHECKER, CHECK.format(cases=cases), "checks", [q["id"] for q in qs])


def merged(fn, groups, *extra):
    got = {}
    with ThreadPoolExecutor(args.jobs) as pool:
        for r in pool.map(lambda g: fn(g, *extra), groups):
            got.update(r)
    return got


def verdict(q, new, query, checked):
    """The check as stored, and the terms the code found lost in the prompt and in the query."""
    terms, o = keeps[q["id"]]["keep"], original(q)
    in_prompt = lost(terms, o["prompt"], new[q["id"]]["prompt"])
    in_query = lost(terms, o["query"], query[q["id"]]["query"])
    gone = sorted(set(in_prompt + in_query))
    c = checked[q["id"]]
    reason = "; ".join(r for r in (c["reason"], f"it loses {', '.join(gone)}" if gone else "") if r)
    return {"lost": sorted(set(gone) | set(c["renamed"])), "added": c["added"], "same": c["same"],
            "prompt_ok": c["prompt_ok"], "query_ok": c["query_ok"], "reason": reason}, in_prompt, in_query


def fails(v):
    return not (v["same"] and v["prompt_ok"] and v["query_ok"]) or bool(v["lost"]) or bool(v["added"])


new = merged(vary, batches(todo), {})
query = merged(queries, batches(todo), new)
checked = merged(check, batches(todo), new, query)
detail = {q["id"]: verdict(q, new, query, checked) for q in todo}
verdicts = {i: d[0] for i, d in detail.items()}
failed = [q for q in todo if fails(verdicts[q["id"]])]
if failed:
    # A failure only in the query, by the checker's prompt_ok and query_ok and by the code, runs the frozen query
    # writer again on the same input as a new logged call; anything else goes back to the prompt writer.
    query_only = [q for q in failed if checked[q["id"]]["prompt_ok"] and not detail[q["id"]][1]
                  and (not checked[q["id"]]["query_ok"] or detail[q["id"]][2])]
    prompt_fix = [q for q in failed if q not in query_only]
    for g in batches(prompt_fix):
        new.update(vary(g, {q["id"]: (new[q["id"]]["prompt"], verdicts[q["id"]]["reason"]) for q in g}))
    query.update(merged(queries, batches(prompt_fix), new))
    query.update(merged(queries, batches(query_only), new, 1))
    checked.update(merged(check, batches(failed), new, query))
    for q in failed:
        detail[q["id"]] = verdict(q, new, query, checked)
        verdicts[q["id"]] = detail[q["id"]][0]

if qpath.read_bytes() != start:
    raise SystemExit("vary: questions.jsonl changed while this run made its calls; nothing written. The calls are in "
                     "the log, so running again repeats none of them.")
repaired = {q["id"] for q in failed}
out = []
for line, q in zip(lines, rows):
    if q["id"] in verdicts:
        o, v = original(q), verdicts[q["id"]]
        q["wording"]["variation"] = {
            "prompt": new[q["id"]]["prompt"], "query": query[q["id"]]["query"],
            "check": {**v, "under": under(o["prompt"], new[q["id"]]["prompt"]),
                      "framing_only": keeps[q["id"]]["framing_only"],
                      "keep": [t["term"] for t in keeps[q["id"]]["keep"]], "repaired": q["id"] in repaired}}
        line = json.dumps(q, ensure_ascii=False)
    out.append(line + "\n")
tmp = qpath.with_name(qpath.name + ".part")
tmp.write_text("".join(out), encoding="utf-8")
os.replace(tmp, qpath)
if redo:
    log({"redo_done": redo_round})

checks = [q["wording"]["variation"]["check"] for q in rows if not q["abstain"] and "variation" in q["wording"]]
still = [q["id"] for q in rows if not q["abstain"] and "variation" in q["wording"]
         and fails(q["wording"]["variation"]["check"])]
print(f"vary: this run varied {len(todo)} and repaired {len(repaired)}; the file holds {len(checks)} variations of "
      f"{sum(not q['abstain'] for q in rows)} answerable questions, {sum(c['repaired'] for c in checks)} repaired, "
      f"{sum(c['under'] for c in checks)} under-reworded, {sum(c['framing_only'] for c in checks)} framing-only, "
      f"{len(still)} failing the check" + (f": {', '.join(still)}" if still else ""))
