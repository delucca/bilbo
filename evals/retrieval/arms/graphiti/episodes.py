"""The Graphiti episodes of a corpus: every turn of every Claude Code session as one message episode, in one timeline
across all repositories. A turn is a prompt the user typed and everything Claude Code said in answer; tool calls and
results, sidechains, compaction summaries, notices and task notifications are not dialogue and never reach an episode.
Pure Python with no third-party imports: `python3 -I episodes.py <corpus> [N]` prints the plan, the build's no-LLM
self-test."""

import json
import re
import statistics
import sys
from collections import Counter
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

NOTICES = ("<local-command-stdout>", "<local-command-stderr>", "<local-command-caveat>", "<system-reminder>")
USER, ASSISTANT = "Developer (user)", "Claude Code (assistant)"


@dataclass
class Episode:
    name: str  # <session id>/<turn>, unique: a rerun finds the episodes already in the graph by it
    session: str
    body: str
    reference_time: datetime
    source_description: str


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def text_of(content):
    if isinstance(content, str):
        return content.strip()
    return "\n".join(b.get("text", "") for b in content or [] if isinstance(b, dict) and b.get("type") == "text").strip()


def slash_command(text):
    """A slash command as the user typed it, from Claude Code's <command-name> record."""
    name = re.search(r"<command-name>(.*?)</command-name>", text, re.DOTALL)
    args = re.search(r"<command-args>(.*?)</command-args>", text, re.DOTALL)
    return " ".join(x for x in (name.group(1).strip() if name else "", args.group(1).strip() if args else "") if x)


def sessions(corpus, limit=None):
    """Every Claude Code session as (start, repo, path), in the order they started across all repos. A transcript
    with no timestamp is not Claude Code's and is left out."""
    found, other = [], []
    for path in sorted(Path(corpus).glob("*/*.jsonl")):
        start = next((t for t in map(stamp, map(json.loads, path.open())) if t), None)
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    return found[:limit], [f"{repo}/{path.name}" for _, repo, path in other]


def turns(path):
    """The (time, user text, assistant texts) of each turn of a session, and the entries it dropped by reason. A turn
    starts at a prompt the user typed. Claude Code's synthetic assistant entries are dropped. An interrupted turn keeps the reply the user saw. A slash command stays only if
    the model answered it: a local command such as /clear has no reply."""
    entries = [json.loads(line) for line in path.open()]
    drops, found, current, when = Counter(), [], None, None
    for entry in entries:
        when = stamp(entry) or when
        content = (entry.get("message") or {}).get("content")
        blocks = content if isinstance(content, list) else []
        if entry.get("isSidechain"):
            drops["sidechain"] += 1
        elif entry.get("type") == "user" and any(isinstance(b, dict) and b.get("type") == "tool_result" for b in blocks):
            drops["tool result"] += 1
        elif entry.get("type") == "user":
            text = text_of(content)
            if entry.get("isMeta"):
                drops["meta"] += 1
            elif entry.get("isCompactSummary"):
                drops["compaction summary"] += 1
            elif not text:
                drops["empty"] += 1
            elif text.startswith("[Request interrupted"):
                drops["interrupt marker"] += 1
            elif text.startswith(NOTICES):
                drops["notice"] += 1
            elif text.startswith("<task-notification>"):
                drops["task notification"] += 1
            else:
                slash = "<command-name>" in text
                current = [when, slash_command(text) if slash else text, slash, []]
                found.append(current)
        elif entry.get("type") == "assistant" and (entry.get("message") or {}).get("model") == "<synthetic>":
            drops["synthetic"] += 1  # Claude Code's own notices ("No response requested.", API errors), not the model's words
        elif entry.get("type") == "assistant" and current and text_of(content):
            current[3].append(text_of(content))
    kept = []
    for at, user, slash, replies in found:
        if slash and not replies:
            drops["local command"] += 1
        else:
            kept.append((at, user, replies))
    return kept, drops


def plan(corpus, limit=None):
    """(episodes in time order across the corpus, skipped transcripts, dropped entries by reason). Ties on the time
    keep the order of session start, then of the turn."""
    chosen, skipped = sessions(corpus, limit)
    episodes, drops = [], Counter()
    for order, (_, repo, path) in enumerate(chosen):
        kept, dropped = turns(path)
        drops.update(dropped)
        sid = next(e["sessionId"] for e in map(json.loads, path.open()) if e.get("sessionId"))
        owner, name = repo.split("__", 1)
        for n, (at, user, replies) in enumerate(kept, 1):
            body = f"{USER}: {user}" + (f"\n\n{ASSISTANT}: " + "\n\n".join(replies) if replies else "")
            episodes.append((at, order, n, Episode(f"{sid}/{n:04d}", sid, body, at, f"Claude Code session in {owner}/{name}")))
    episodes.sort(key=lambda e: e[:3])
    return [e[3] for e in episodes], skipped, drops


def report(corpus, limit):
    episodes, skipped, drops = plan(corpus, limit)
    chosen, _ = sessions(corpus, limit)
    q = lambda xs, p: sorted(xs)[min(int(len(xs) * p), len(xs) - 1)]
    sizes = [len(e.body) for e in episodes]
    window = [sum(sizes[max(0, i - 10) : i]) for i in range(len(sizes))]
    repos = Counter(e.source_description.removeprefix("Claude Code session in ") for e in episodes)
    print(f"sessions {len(chosen)}, skipped {len(skipped)}: {skipped}")
    print(f"episodes {len(episodes)}; dropped entries {dict(sorted(drops.items()))}")
    print(f"by repo {dict(repos.most_common())}")
    print(f"body chars: sum {sum(sizes):,} median {statistics.median(sizes):.0f} p90 {q(sizes, .9):,} p99 {q(sizes, .99):,} max {max(sizes):,}")
    print(f"10-episode window chars: mean {statistics.mean(window):,.0f} p90 {q(window, .9):,} max {max(window):,}")
    names = [e.name for e in episodes]
    markers = ("<system-reminder>", "<task-notification>", "<local-command", "tool_use", "tool_result")
    print("checks:", {
        "empty bodies": sum(not e.body for e in episodes),
        "duplicate names": len(names) - len(set(names)),
        "naive times": sum(e.reference_time.tzinfo is None for e in episodes),
        "bodies not starting with the user line": sum(not e.body.startswith(f"{USER}: ") for e in episodes),
        "bodies with a tool or notice marker": sum(any(m in e.body for m in markers) for e in episodes),
        "out of order": sum(a.reference_time > b.reference_time for a, b in zip(episodes, episodes[1:])),
    })
    ids = list(dict.fromkeys(e.session for e in episodes))
    for session in (ids[0], ids[len(ids) // 2]):
        for e in [e for e in episodes if e.session == session][:3]:
            print(f"---- {e.name} {e.reference_time.isoformat()} ({e.source_description})\n{e.body[:600]}")


if __name__ == "__main__":
    report(sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else None)
