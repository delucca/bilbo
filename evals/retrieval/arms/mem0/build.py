"""Build the mem0 store: replay every session through mem0's own Claude Code plugin, hook by hook and in one timeline
across all sessions, and send what the plugin would send to mem0's hosted service to open-source mem0 instead. Then save
every Qdrant collection as a snapshot with mem0's SQLite file and a manifest in /work/store. Whenever no session is
open, the same files go to /work/checkpoint, and a run that finds them resumes from there."""

import heapq
import json
import os
import re
import shutil
import sqlite3
import sys
import time
from collections import Counter
from datetime import datetime, timedelta, timezone
from importlib.metadata import version
from pathlib import Path

sys.path.insert(0, "/app/plugin")

import httpx
import mem0.memory.main as mem0_main
import memory_core as plugin
import openai
import transcript as plugin_transcript
from mem0 import Memory
from mem0.exceptions import LLMError
from qdrant_client import QdrantClient

USER = "dev"  # one developer across every repo
COLLECTION = "mem0"
IDLE = timedelta(seconds=300)  # the plugin's hook_runner flushes 5 minutes after a Stop that no other Stop follows
FLUSHED = ("semantic-succeeded", "nothing-to-flush")
NOTICES = ("<local-command-stdout>", "<local-command-stderr>", "<local-command-caveat>", "<system-reminder>")

CORPUS = Path("/corpus")
WORK = Path("/work")
CHECKPOINT = WORK / "checkpoint"
LIVE = Path("/tmp/live")
QDRANT = os.environ["QDRANT_URL"]
LLM_URL = os.environ["BILBO_EVAL_LLM_URL"].rstrip("/")
LLM_KEY = os.environ["BILBO_EVAL_LLM_KEY"]
LLM_MODEL = os.environ["BILBO_EVAL_LLM_MODEL"]
LLM_HEADERS = json.loads(os.environ.get("BILBO_EVAL_LLM_HEADERS") or "{}")
TRANSIENT = (openai.RateLimitError, openai.InternalServerError, openai.APIConnectionError)

# The plugin only flushes with an API key; its data dir holds the evidence it records from the hooks.
os.environ.update({"MEM0_API_KEY": "local", "MEM0_CODE_USER_ID": USER, "MEM0_CODE_DATA_DIR": str(LIVE / "plugin")})
plugin.configure_harness("claude-code", data_dir_name="claude-code-plugin", source_tag="claude_code_plugin")


def log(msg):
    print(f"mem0: {msg}", file=sys.stderr, flush=True)


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def sessions():
    """Every Claude Code session as (start, repo, path), in the order they started across all repos. A transcript
    with no timestamp is not Claude Code's and is left out; the plugin never sees it."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        start = next((t for t in map(stamp, map(json.loads, path.open())) if t), None)
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    limit = os.environ.get("BILBO_EVAL_SESSIONS")
    return found[: int(limit) if limit else None], [f"{repo}/{path.name}" for _, repo, path in other]


# What the session being replayed sees: the time of its current event, and what git answers in its checkout.
current = {"now": None, "remote": "", "branch": ""}


def fake_git(cwd, *args):
    return {
        ("rev-parse", "--show-toplevel"): cwd,
        ("config", "--get", "remote.origin.url"): current["remote"],
        ("branch", "--show-current"): current["branch"],
    }.get(args, "")


plugin._git = fake_git

# OSS mem0 dates the extraction prompt with today's date; a replay passes the date of the session instead.
_prompt = mem0_main.generate_additive_extraction_prompt


def dated_prompt(*args, **kwargs):
    kwargs["current_date"] = kwargs["timestamp"] = current["now"].date().isoformat()
    return _prompt(*args, **kwargs)


mem0_main.generate_additive_extraction_prompt = dated_prompt


def memory():
    return Memory.from_config(
        {
            "vector_store": {"provider": "qdrant", "config": {"collection_name": COLLECTION, "url": QDRANT, "embedding_model_dims": 1024}},
            "embedder": {"provider": "openai", "config": {"model": "embedder", "openai_base_url": os.environ["EMBEDDER_URL"], "api_key": "none"}},
            "llm": {"provider": "openai", "config": {"model": LLM_MODEL, "openai_base_url": LLM_URL, "api_key": LLM_KEY}},
            "history_db_path": str(LIVE / "history.db"),
        }
    )


def hybrid_search_on(client):
    """mem0 turns BM25 or entity matching off with only a warning when fastembed or spaCy fails to load: refuse to
    build a store without them."""
    from fastembed import SparseTextEmbedding
    from mem0.utils.spacy_models import get_nlp_full

    try:
        SparseTextEmbedding("Qdrant/bm25")
    except ValueError as e:
        sys.exit(f"mem0: the BM25 encoder does not load, so keyword search would be off: {e}")
    if get_nlp_full() is None:
        sys.exit("mem0: spaCy's en_core_web_sm does not load, so entity matching would be off")
    if "bm25" not in (client.get_collection(COLLECTION).config.params.sparse_vectors or {}):
        sys.exit(f"mem0: the {COLLECTION} collection has no bm25 slot, so keyword search would be off")


def count_usage(mem, counts):
    mem.llm.client = mem.llm.client.with_options(default_headers=LLM_HEADERS)
    create = mem.llm.client.chat.completions.create

    def counted(*args, **kwargs):
        response = create(*args, **kwargs)
        counts["llm_calls"] += 1
        if response.usage:
            counts["prompt_tokens"] += response.usage.prompt_tokens or 0
            counts["completion_tokens"] += response.usage.completion_tokens or 0
        return response

    mem.llm.client.chat.completions.create = counted


def fit_embedder(mem, counts):
    """mem0 embeds a whole batch to find the memories it already holds, and the plugin's batches can pass the
    embedder's context: that lookup then embeds the start and end of the batch. The LLM still reads all of it."""
    embed = mem.embedding_model.embed

    def fitted(text, memory_action=None):
        while True:
            try:
                return embed(text, memory_action)
            except openai.BadRequestError as e:
                info = e.body.get("error", e.body) if isinstance(e.body, dict) else {}
                if info.get("type") != "exceed_context_size_error":
                    raise
                keep = int(len(text) * info["n_ctx"] / info["n_prompt_tokens"] * 0.9) // 2
                text = f"{text[:keep]} {text[-keep:]}"
                counts["embed_cuts"] += 1

    mem.embedding_model.embed = fitted


def add(mem, messages, **kwargs):
    """mem0 calls the LLM before it writes anything, so an add whose call failed is safe to send again."""
    for attempt in range(1, 11):
        try:
            return mem.add(messages, **kwargs)["results"]
        except LLMError as e:
            if not isinstance(e.__cause__, TRANSIENT):
                raise
            wait = min(60 * attempt, 600)
            log(f"LLM call failed ({e}); retrying in {wait} s")
            time.sleep(wait)
    sys.exit("mem0: the LLM failed 10 times in a row; run again to resume from the last checkpoint")


def hosted_api(mem, counts):
    """The plugin's POST to mem0's hosted /v3/memories/add goes to open-source mem0 instead. The hosted service keeps
    shared project memory under agent_id and personal memory under user_id, each from its own instructions, both
    scoped to the repo by app_id: here that is one add per kind. OSS mem0 has no app_id or categories, so app_id goes in
    metadata, where search can filter on it, and the categories are dropped. The session id goes in metadata too: as
    run_id, OSS mem0 would stop showing earlier sessions' memories to the extraction."""
    added = {}

    def request_json(url, key, payload, timeout):
        metadata = {**payload["metadata"], "app_id": payload["app_id"], "session_id": payload["run_id"]}
        metadata["created_at"] = current["now"].isoformat()
        messages = payload["messages"]
        project = add(mem, messages, agent_id=payload["agent_id"], metadata=metadata, prompt=payload["agent_custom_instructions"])
        personal = add(mem, messages, user_id=payload["user_id"], metadata=metadata, prompt=payload["custom_instructions"])
        counts["requests"] += 1
        event = f"local-{counts['requests']}"
        added[event] = len(project) + len(personal)
        return {"event_id": event}, 0, 0

    def wait_for_event(api_url, key, event_id):
        return "SUCCEEDED", 0, added.pop(event_id, 0)

    plugin._request_json = request_json
    plugin._wait_for_event = wait_for_event


def text_of(content):
    if isinstance(content, str):
        return content.strip()
    return "\n".join(b.get("text", "") for b in content or [] if isinstance(b, dict) and b.get("type") == "text").strip()


def slash_command(text):
    """A slash command as the user typed it, from Claude Code's <command-name> record."""
    name = re.search(r"<command-name>(.*?)</command-name>", text, re.DOTALL)
    args = re.search(r"<command-args>(.*?)</command-args>", text, re.DOTALL)
    return " ".join(x for x in (name.group(1).strip() if name else "", args.group(1).strip() if args else "") if x)


class Session:
    """One session through the plugin, called as Claude Code's hooks call it (the plugin's hooks.json and hook_runner):
    SessionStart, again after a compaction; UserPromptSubmit for each prompt typed; PostToolUse or PostToolUseFailure
    for each tool result; Stop when a reply ends, then a checkpoint when one is due, or an idle flush 5 minutes later
    unless another Stop comes first; PreCompact before a compaction; SessionEnd at the end. Each step is one event, so
    sessions open at the same time interleave by time. The transcript file grows line by line, so each hook reads what
    Claude Code had written by then."""

    def __init__(self, repo, path, store, counts):
        self.name, self.store, self.counts = f"{repo}/{path.name}", store, counts
        self.lines = [line if line.endswith("\n") else line + "\n" for line in path.read_text().splitlines()]
        self.entries = [json.loads(line) for line in self.lines]
        self.times, when = [], None
        for entry in self.entries:
            when = stamp(entry) or when
            self.times.append(when)
        first_time = next(t for t in self.times if t)
        self.times = [t or first_time for t in self.times]
        first = next(e for e in self.entries if e.get("sessionId"))
        self.sid, self.cwd = first["sessionId"], first.get("cwd") or "/"
        branch = first.get("gitBranch")
        self.git = {"remote": f"https://github.com/{repo.replace('__', '/', 1)}", "branch": "" if branch in (None, "HEAD") else branch}
        self.model = next(((e.get("message") or {}).get("model", "") for e in self.entries if e.get("type") == "assistant"), "")
        self.transcript = LIVE / "transcripts" / f"{self.sid}.jsonl"
        self.transcript.parent.mkdir(parents=True, exist_ok=True)
        self.out = self.transcript.open("w")
        self.turn, self.idle_at, self.tools, self.index, self.ended = None, None, {}, -1, False

    def due(self):
        return self.times[min(max(self.index, 0), len(self.entries) - 1)]

    def next_time(self):
        return min(self.due(), self.idle_at) if self.idle_at else self.due()

    def hook(self, **kwargs):
        return {"session_id": self.sid, "cwd": self.cwd, "transcript_path": str(self.transcript), **kwargs}

    def flush(self, reason):
        result = plugin.checkpoint_session(self.store, self.hook(), reason)
        if result["status"] not in FLUSHED:
            sys.exit(f"mem0: the plugin's {reason} flush of {self.name} ended {result['status']}: {result.get('error', '')}")
        self.counts[f"flush_{reason}"] += 1

    def stop(self):
        turn = self.turn
        if not turn or turn["stopped"] or not turn["reply"] or turn["interrupted"]:
            return
        turn["stopped"] = True
        self.counts["stops"] += 1
        repo, _ = plugin_transcript.record_stop(self.store, self.hook(last_assistant_message=turn["reply"]))
        if self.store.checkpoint_due(repo.identity, self.sid):
            self.flush("periodic")
        elif self.store.has_unflushed_events(repo.identity, self.sid):
            self.idle_at = current["now"] + IDLE

    def step(self):
        current.update(self.git)
        plugin._resolve_repo_cached.cache_clear()
        if self.index < 0:
            current["now"], self.index = self.times[0], 0
            plugin.record_session_start(self.store, self.hook(source="startup", model=self.model))
        elif self.idle_at and self.idle_at <= self.due():
            current["now"], self.idle_at = self.idle_at, None
            self.flush("idle")
        elif self.index < len(self.entries):
            current["now"] = self.times[self.index]
            self.event(self.lines[self.index], self.entries[self.index])
            self.index += 1
        else:
            current["now"] = self.times[-1]
            self.stop()
            plugin_transcript.record_stop(self.store, self.hook())
            self.flush("session-end")
            self.out.close()
            self.transcript.unlink()
            self.ended = True

    def event(self, line, entry):
        kind, subtype, sidechain = entry.get("type"), entry.get("subtype"), entry.get("isSidechain")
        content = (entry.get("message") or {}).get("content")
        blocks = content if isinstance(content, list) else []
        results = [b for b in blocks if isinstance(b, dict) and b.get("type") == "tool_result"]

        if kind == "system" and subtype in ("stop_hook_summary", "turn_duration"):
            self.stop()
        elif kind == "system" and subtype == "compact_boundary":
            self.stop()
            self.flush("pre-compact")
        elif kind == "user" and not (sidechain or results or entry.get("isMeta") or entry.get("isCompactSummary")):
            text = text_of(content)
            if text.startswith("[Request interrupted"):
                if self.turn:
                    self.turn["interrupted"] = True
            elif text and not text.startswith(NOTICES):
                self.stop()
                self.turn = {"reply": "", "interrupted": False, "stopped": False, "prompt": None}
                if "<command-name>" in text:
                    self.turn["prompt"] = slash_command(text)  # sent once the model answers: a local command sends none
                elif not text.startswith("<task-notification>"):
                    plugin.record_user_prompt(self.store, self.hook(prompt=text))
                    self.counts["prompts"] += 1

        self.out.write(line)
        self.out.flush()
        if sidechain:
            return
        if kind == "assistant":
            if self.turn and self.turn["prompt"]:
                plugin.record_user_prompt(self.store, self.hook(prompt=self.turn["prompt"]))
                self.counts["prompts"] += 1
                self.turn["prompt"] = None
            for b in blocks:
                if isinstance(b, dict) and b.get("type") == "tool_use":
                    self.tools[b.get("id")] = (b.get("name"), b.get("input") or {})
            reply = text_of(content)
            if self.turn and reply:
                self.turn["reply"], self.turn["interrupted"] = reply, False
        elif kind == "user" and results:
            for b in results:
                name, tool_input = self.tools.get(b.get("tool_use_id"), ("unknown", {}))
                hook = self.hook(tool_name=name, tool_input=tool_input, tool_use_id=b.get("tool_use_id"), tool_response=entry.get("toolUseResult"))
                if b.get("is_error"):
                    plugin.record_tool(self.store, {**hook, "error": text_of(b.get("content"))}, failed=True)
                else:
                    plugin.record_tool(self.store, hook)
                self.counts["tool_events"] += 1
        elif kind == "system" and subtype == "compact_boundary":
            plugin.record_session_start(self.store, self.hook(source="compact", model=self.model))


def save(client, mem, progress):
    """Write the store to the checkpoint: the swap is two renames, and a run that finds only checkpoint.old uses it."""
    tmp, old = WORK / "checkpoint.tmp", WORK / "checkpoint.old"
    shutil.rmtree(tmp, ignore_errors=True)
    (tmp / "qdrant").mkdir(parents=True)
    for c in client.get_collections().collections:
        snap = client.create_snapshot(c.name, wait=True)
        with httpx.stream("GET", f"{QDRANT}/collections/{c.name}/snapshots/{snap.name}", timeout=600) as r, open(tmp / "qdrant" / f"{c.name}.snapshot", "wb") as f:
            r.raise_for_status()
            for chunk in r.iter_bytes():
                f.write(chunk)
        client.delete_snapshot(c.name, snap.name, wait=True)
    db = sqlite3.connect(tmp / "history.db")
    mem.db.connection.backup(db)
    db.close()
    (tmp / "progress.json").write_text(json.dumps(progress, indent=2) + "\n")
    shutil.rmtree(old, ignore_errors=True)
    if CHECKPOINT.exists():
        CHECKPOINT.rename(old)
    tmp.rename(CHECKPOINT)
    shutil.rmtree(old, ignore_errors=True)


def restore(client):
    for f in sorted((CHECKPOINT / "qdrant").glob("*.snapshot")):
        with f.open("rb") as fh:
            r = httpx.post(f"{QDRANT}/collections/{f.stem}/snapshots/upload", params={"priority": "snapshot", "wait": "true"}, files={"snapshot": (f.name, fh)}, timeout=600)
        r.raise_for_status()
    shutil.copy(CHECKPOINT / "history.db", LIVE / "history.db")
    return json.loads((CHECKPOINT / "progress.json").read_text())


def manifest(client, progress, skipped):
    inputs = (WORK / "key.txt").read_text().splitlines()
    memories, offset = Counter(), None
    while True:
        points, offset = client.scroll(COLLECTION, limit=256, offset=offset, with_payload=True)
        for p in points:
            memories[f"{p.payload.get('app_id', '')}/{'project' if p.payload.get('agent_id') else 'personal'}"] += 1
        if offset is None:
            break
    counts = progress["counts"]
    usage = ("llm_calls", "prompt_tokens", "completion_tokens")
    entities = f"{COLLECTION}_entities"
    return {
        "arm": "mem0",
        "key": inputs[0],
        "inputs": inputs[1:],
        "device": os.environ["BILBO_EVAL_DEVICE"],
        "versions": {p: version(p) for p in ("mem0ai", "qdrant-client", "fastembed", "spacy", "en-core-web-sm", "openai")}
        | {"claude-code-plugin": plugin.PLUGIN_VERSION, "qdrant": httpx.get(QDRANT).json()["version"]},
        "sessions": len(progress["done"]),
        "skipped": skipped,
        "memories": {"total": sum(memories.values()), "by_repo": dict(sorted(memories.items()))},
        "entities": client.count(entities, exact=True).count if client.collection_exists(entities) else 0,
        "replay": {k: v for k, v in sorted(counts.items()) if k not in usage},
        "llm": {"model": LLM_MODEL} | {k: counts.get(k, 0) for k in usage},
        "ingest_seconds": round(progress["ingest_seconds"]),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }


def main():
    try:
        httpx.get(f"{LLM_URL}/models", headers={"Authorization": f"Bearer {LLM_KEY}"} | LLM_HEADERS, timeout=30).raise_for_status()
    except httpx.HTTPError as e:
        sys.exit(f"mem0: the LLM endpoint {LLM_URL} does not answer: {e}")

    LIVE.mkdir(parents=True, exist_ok=True)
    if not CHECKPOINT.exists() and (WORK / "checkpoint.old").exists():
        (WORK / "checkpoint.old").rename(CHECKPOINT)
    client = QdrantClient(url=QDRANT)
    if (CHECKPOINT / "progress.json").exists():
        progress = restore(client)
        log(f"resuming after {len(progress['done'])} sessions")
    else:
        progress = {"done": [], "counts": {}, "ingest_seconds": 0}
    counts = Counter(progress["counts"])
    mem = memory()
    hybrid_search_on(client)
    count_usage(mem, counts)
    fit_embedder(mem, counts)
    hosted_api(mem, counts)

    plan, skipped = sessions()
    done = set(progress["done"])
    queue = [(start, n, None, repo, path) for n, (start, repo, path) in enumerate(plan) if f"{repo}/{path.name}" not in done]
    heapq.heapify(queue)
    store, open_now, started = plugin.EvidenceStore(), 0, time.monotonic()
    while queue:
        _, n, session, repo, path = heapq.heappop(queue)
        if session is None:
            session, open_now = Session(repo, path, store, counts), open_now + 1
        session.step()
        if not session.ended:
            heapq.heappush(queue, (session.next_time(), n, session, repo, path))
            continue
        open_now -= 1
        progress["done"].append(session.name)
        if open_now == 0:
            progress["counts"] = dict(counts)
            progress["ingest_seconds"] += time.monotonic() - started
            save(client, mem, progress)
            started = time.monotonic()
            log(f"{len(progress['done'])}/{len(plan)} sessions: {counts['requests']} plugin requests, {counts['llm_calls']} LLM calls so far")
    store.close()

    out = WORK / "store"
    shutil.rmtree(out, ignore_errors=True)
    shutil.copytree(CHECKPOINT, out)
    (out / "progress.json").unlink()
    (out / "manifest.json").write_text(json.dumps(manifest(client, progress, skipped), indent=2) + "\n")
    log(f"done: {out}")


if __name__ == "__main__":
    main()
