"""Render Claude Code transcripts into exchanges (PLAN D5). One renderer for every arm and for the user study.

An exchange is one real user prompt (a slash command counts; a tool result, a meta message, a compaction summary,
a task notification or an interrupt marker does not) plus everything up to the next one. Assistant text and tool calls
are inlined: a call as a ```tool NAME fence holding the input JSON, its result as a ```result fence. Every tool input
and result is capped at CAP characters (the first two thirds and the last third, with a marker between). Thinking
blocks, `<system-reminder>` text, hook output and sidechains are dropped. Redaction placeholders stay as they are.
An exchange with no assistant text and no tool call is dropped and counted. `render_sha` hashes this file and the
constants below.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
CAP = 800
ENCODING = "o200k_base"
REMINDER = re.compile(r"<system-reminder>.*?</system-reminder>", re.S)
NOT_PROMPT = ("<task-notification>", "<local-command-caveat>", "<local-command-stdout>", "<local-command-stderr>", "[Request interrupted by user")
COMMAND = re.compile(r"<command-name>\s*(.*?)\s*</command-name>", re.S)
COMMAND_ARGS = re.compile(r"<command-args>(.*?)</command-args>", re.S)


def render_sha(cap: int = CAP) -> str:
    h = hashlib.sha256()
    h.update(Path(__file__).read_bytes())
    h.update(json.dumps({"cap": cap, "encoding": ENCODING}, sort_keys=True).encode())
    return h.hexdigest()


def cap_text(text: str, cap: int) -> tuple[str, int]:
    """Cap to `cap` characters; returns the text and the number of characters cut."""
    if len(text) <= cap:
        return text, 0
    head = round(cap * 2 / 3)
    tail = cap - head
    cut = len(text) - cap
    return f"{text[:head]}[... {cut} characters cut ...]{text[len(text) - tail:]}", cut


def stamp(value) -> str | None:
    if not isinstance(value, str):
        return None
    try:
        t = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None
    if t.tzinfo is None:
        t = t.replace(tzinfo=timezone.utc)
    return t.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def block_text(content) -> str:
    """The visible text of a message content: a string, or text blocks with images shown as a marker."""
    if isinstance(content, str):
        return content
    out = []
    for b in content or []:
        if not isinstance(b, dict):
            continue
        if b.get("type") == "text":
            out.append(b.get("text", ""))
        elif b.get("type") == "image":
            out.append("[image]")
    return "\n".join(out)


def user_prompt(entry: dict) -> tuple[str, bool] | None:
    """The human's text of a user entry and whether it is a slash command, or None when it is not a real prompt."""
    if entry.get("isMeta") or entry.get("isCompactSummary"):
        return None
    content = entry.get("message", {}).get("content")
    if isinstance(content, list) and any(isinstance(b, dict) and b.get("type") == "tool_result" for b in content):
        return None
    text = REMINDER.sub("", block_text(content)).strip()
    if not text or text.startswith(NOT_PROMPT):
        return None
    m = COMMAND.search(text)
    if m:
        args = COMMAND_ARGS.search(text)
        return (f"{m.group(1)} {args.group(1).strip()}".strip() if args else m.group(1)), True
    return text, False


class Exchange:
    def __init__(self, ts: str, user: str, compact: bool):
        self.ts, self.ts_end, self.user, self.compact = ts, ts, user, compact
        self.command = False
        self.parts: list[dict] = []
        self.calls: dict[str, dict] = {}
        self.inlined: list[str] = []

    def inline(self, text: str) -> None:
        """A prompt the user typed while the agent worked, kept in the exchange it arrived in."""
        if text != self.user and text not in self.inlined:
            self.inlined.append(text)
            self.parts.append({"text": f"user (while the agent worked): {text}", "inlined": text})

    def text(self, text: str) -> None:
        if text.strip():
            self.parts.append({"text": text.strip()})

    def call(self, block: dict) -> None:
        part = {"name": block.get("name", "tool"), "input": block.get("input", {}), "result": None}
        self.parts.append(part)
        self.calls[block.get("id", "")] = part

    def result(self, block: dict) -> None:
        part = self.calls.get(block.get("tool_use_id", ""))
        if part is not None:
            part["result"] = REMINDER.sub("", block_text(block.get("content"))).strip()

    def render(self, cap: int) -> tuple[str, int]:
        cut_total, out = 0, []
        for p in self.parts:
            if "text" in p:
                out.append(p["text"])
                continue
            body, cut = cap_text(json.dumps(p["input"], ensure_ascii=False), cap)
            cut_total += cut
            block = f"```tool {p['name']}\n{body}\n```"
            if p["result"] is not None:
                body, cut = cap_text(p["result"], cap)
                cut_total += cut
                block += f"\n```result\n{body}\n```"
            out.append(block)
        return "\n\n".join(out), cut_total


def first_stamp(path: Path) -> str | None:
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            e = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(e, dict) and not e.get("isSidechain") and (t := stamp(e.get("timestamp"))):
            return t
    return None


def session_exchanges(path: Path, cap: int, fallback_ts: str) -> tuple[list[dict], dict]:
    """The exchanges of one transcript in order, and counts of what was dropped.

    Times come from the transcript's own entries; `fallback_ts` serves only a transcript that has none.
    """
    done: list[Exchange] = []
    cur: Exchange | None = None
    drops = {"no_assistant": 0, "bad_lines": 0, "orphan_prompts_kept": 0}
    compact, orphans, last_ts = False, [], first_stamp(path) or fallback_ts

    def close() -> None:
        nonlocal cur, compact
        if cur is None:
            return
        if cur.parts:
            done.append(cur)
        else:
            drops["no_assistant"] += 1
            compact = compact or cur.compact
            if not cur.command:
                orphans.append(cur.user)
                drops["orphan_prompts_kept"] += 1
        cur = None

    def open_exchange(ts: str, text: str, command: bool) -> None:
        nonlocal cur, compact
        close()
        if orphans:
            text = "\n\n".join([*orphans, text])
            orphans.clear()
        if done and text in done[-1].inlined:
            done[-1].parts = [p for p in done[-1].parts if p.get("inlined") != text]
        cur = Exchange(ts, text, compact)
        cur.command = command
        compact = False

    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            e = json.loads(line)
        except json.JSONDecodeError:
            drops["bad_lines"] += 1
            continue
        if not isinstance(e, dict) or e.get("isSidechain"):
            continue
        kind = e.get("type")
        ts = stamp(e.get("timestamp")) or last_ts
        last_ts = ts
        if kind == "system" and e.get("subtype") == "compact_boundary":
            compact = True
        elif kind == "user":
            msg = e.get("message") or {}
            if e.get("isCompactSummary"):
                compact = True
            prompt = user_prompt(e)
            if prompt is not None:
                open_exchange(ts, *prompt)
            elif cur is not None:
                for b in msg.get("content") if isinstance(msg.get("content"), list) else []:
                    if isinstance(b, dict) and b.get("type") == "tool_result":
                        cur.ts_end = max(cur.ts_end, ts)
                        cur.result(b)
        elif kind == "attachment":
            a = e.get("attachment") or {}
            if a.get("type") != "queued_command" or a.get("commandMode") != "prompt":
                continue
            origin = (a.get("origin") or {}).get("kind")
            text = REMINDER.sub("", block_text(a.get("prompt")) or "").strip()
            if not text or origin not in (None, "human"):
                continue
            if origin == "human":
                open_exchange(stamp(a.get("timestamp")) or ts, text, False)
            elif cur is not None:
                cur.inline(text)
        elif kind == "assistant" and cur is not None:
            msg = e.get("message") or {}
            if msg.get("model") == "<synthetic>":
                continue
            cur.ts_end = max(cur.ts_end, ts)
            content = msg.get("content")
            for b in content if isinstance(content, list) else [{"type": "text", "text": content or ""}]:
                if not isinstance(b, dict):
                    continue
                if b.get("type") == "text":
                    cur.text(REMINDER.sub("", b.get("text", "")))
                elif b.get("type") == "tool_use":
                    cur.call(b)
    close()
    rows = []
    for x in done:
        assistant, cut = x.render(cap)
        rows.append({"ts": x.ts, "ts_end": x.ts_end, "user": x.user, "assistant": assistant, "compact_before": x.compact, "cut_chars": cut})
    return rows, drops


def count_tokens(enc, text: str) -> int:
    return len(enc.encode(text, disallowed_special=()))


def render_repo(repo: str, info: dict, raw: Path, cap: int, enc) -> tuple[list[dict], dict]:
    """Exchanges of every session of a repo in real-time order; `s` numbers the sessions by their first exchange."""
    key = info["key"]
    drops = {"no_assistant": 0, "bad_lines": 0, "orphan_prompts_kept": 0, "empty_sessions": 0}
    per = []
    for sess in info["sessions"]:
        xs, d = session_exchanges(raw / "transcripts" / f"{sess['session_id']}.jsonl", cap, stamp(sess["created_at"]))
        for k in d:
            drops[k] += d[k]
        if not xs:
            drops["empty_sessions"] += 1
            continue
        floor = xs[0]["ts"]
        for x in xs:  # a session's time never goes backwards
            x["ts"] = max(x["ts"], floor)
            x["ts_end"] = max(x["ts_end"], x["ts"])
            floor = x["ts"]
        per.append((xs[0]["ts"], sess["created_at"], sess["session_id"], xs))
    rows = []
    for s, (_, _, sid, xs) in enumerate(sorted(per), 1):
        for n, x in enumerate(xs, 1):
            x.update(repo=key, session_id=sid, s=s, n=n, xid=f"{sid}#{n}")
            x["tokens"] = count_tokens(enc, x["user"] + x["assistant"])
            rows.append(x)
    rows.sort(key=lambda r: (r["ts"], r["s"], r["n"]))
    return rows, drops


def percentile(values: list[int], q: float) -> int:
    if not values:
        return 0
    v = sorted(values)
    return v[min(len(v) - 1, int(q * len(v)))]


def repo_stats(rows: list[dict], drops: dict) -> dict:
    toks = [r["tokens"] for r in rows]
    return {
        "exchanges": len(rows),
        "sessions": len({r["session_id"] for r in rows}),
        "tokens": sum(toks),
        "tokens_p50": percentile(toks, 0.5),
        "tokens_p95": percentile(toks, 0.95),
        "tokens_max": max(toks, default=0),
        "compactions": sum(r["compact_before"] for r in rows),
        "cut_chars": sum(r["cut_chars"] for r in rows),
        "cut_exchanges": sum(1 for r in rows if r["cut_chars"]),
        "dropped": drops,
    }


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(prog="eval.py render")
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--selection", type=Path, default=HERE / "data" / "selection.json")
    ap.add_argument("--repo", action="append", help="repo_id; default every chosen repo")
    ap.add_argument("--cap", type=int, default=CAP)
    ap.add_argument("--out", type=Path, help="exchanges folder; default <data>/exchanges")
    a = ap.parse_args(argv)
    import tiktoken

    enc = tiktoken.get_encoding(ENCODING)
    sel = json.loads(a.selection.read_text())
    repos = a.repo or [sel["dev"], *sel["test"]]
    out = a.out or a.data / "exchanges"
    out.mkdir(parents=True, exist_ok=True)
    stats_path = a.data / "rendered_stats.json"
    stats = json.loads(stats_path.read_text()) if stats_path.exists() else {}
    sha = render_sha(a.cap)
    for repo in repos:
        info = sel["repos"][repo]
        rows, drops = render_repo(repo, info, a.data / "raw", a.cap, enc)
        text = "".join(json.dumps(r, sort_keys=True, ensure_ascii=False) + "\n" for r in rows)
        (out / f"{info['key']}.jsonl").write_text(text, encoding="utf-8")
        stats[repo] = {**repo_stats(rows, drops), "render_sha": sha, "cap": a.cap, "exchanges_sha256": hashlib.sha256(text.encode()).hexdigest()}
        s = stats[repo]
        print(f"render: {repo}: {s['exchanges']} exchanges, {s['tokens']} tokens, p95 {s['tokens_p95']}, max {s['tokens_max']}")
    stats_path.write_text(json.dumps(dict(sorted(stats.items())), indent=1, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
