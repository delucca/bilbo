"""Build the dense store: render every session's main-thread messages as text, cut each session into chunks of at most
512 tokens of the embedder's own tokenizer, embed every chunk once with the eval's embedder, and save the unit vectors
as a float16 matrix with one metadata row per chunk in /work/store. After every session the same files go to
/work/checkpoint, and a run that finds them resumes from there."""

import http.client
import json
import os
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from importlib.metadata import version
from pathlib import Path
from urllib.parse import urlparse

import numpy as np

BUDGET = 512  # tokens per chunk
OVERLAP = 64  # tokens shared by consecutive windows of one long message
REQUEST = 16  # chunks per embeddings request
DIMS = 1024
# The server reuses the KV entries of the prefix a request shares with the previous one, so a vector would depend on
# the text embedded before it. One token that no chunk starts with, sent before every chunk, leaves no prefix to reuse.
SENTINEL = "\u0000"

CORPUS = Path("/corpus")
WORK = Path("/work")
CHECKPOINT = WORK / "checkpoint"


def log(msg):
    print(f"dense: {msg}", file=sys.stderr, flush=True)


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def sessions():
    """Every session as (repo, path), in the order they started across all repos, as for mem0. A transcript with no
    timestamp (Cursor's) comes last, by path."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        start = next((t for t in map(stamp, map(json.loads, path.open())) if t), None)
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    limit = os.environ.get("BILBO_EVAL_SESSIONS")
    return [(repo, path) for _, repo, path in found + other][: int(limit) if limit else None]


def media(block):
    return f"[{block['type']} {block.get('source', {}).get('media_type', '')}]".replace(" ]", "]")


def render_input(value):
    if not isinstance(value, dict):
        return json.dumps(value, ensure_ascii=False)
    return "\n".join(f"{k}: {v if isinstance(v, str) else json.dumps(v, ensure_ascii=False)}" for k, v in value.items())


def render_result(content):
    if isinstance(content, str):
        return content
    parts = []
    for item in content or []:
        kind = item.get("type") if isinstance(item, dict) else None
        if kind == "text":
            parts.append(item.get("text", ""))
        elif kind == "image":
            parts.append(media(item))
        elif kind == "tool_reference":
            parts.append(f"[tool_reference {item.get('tool_name', '')}]")
        elif kind:
            parts.append(f"[{kind}]")
    return "\n".join(parts)


def render_entry(entry, role, tools, blocks_skipped):
    """The text of one user or assistant entry, a labelled piece per block (thinking is left out), and where each
    piece starts in the text, in bytes, with its label line."""
    content = (entry.get("message") or {}).get("content")
    blocks = [{"type": "text", "text": content}] if isinstance(content, str) else content
    if not isinstance(blocks, list):
        return "", []
    label = "compact summary" if entry.get("isCompactSummary") else role
    pieces = []
    for b in blocks:
        kind = b.get("type") if isinstance(b, dict) else None
        if kind == "text":
            if text := b.get("text", "").strip():
                pieces.append(f"{label}: {text}")
        elif kind == "tool_use":
            tools[b.get("id")] = b.get("name", "")
            args = render_input(b.get("input") or {})
            pieces.append(f"tool_use {b.get('name', '')}:\n{args}" if args else f"tool_use {b.get('name', '')}:")
        elif kind == "tool_result":
            name = tools.get(b.get("tool_use_id"), "unknown")
            head = f"{'tool_error' if b.get('is_error') else 'tool_result'} {name}:"
            body = render_result(b.get("content")).strip()
            pieces.append(f"{head}\n{body}" if body else head)
        elif kind in ("image", "document"):
            pieces.append(f"{label}: {media(b)}")
        else:
            blocks_skipped[kind] += 1
    starts, at = [], 0
    for p in pieces:
        starts.append((at, head_of(p)))
        at += len(p.encode()) + 1
    return "\n".join(pieces), starts


def render_session(path):
    """The messages of one session as {line, text, heads}, and what was left out, by kind: entries (no message of the main
    thread, an isMeta injection, or nothing to say) and blocks (thinking). Cursor's entries carry role, not type."""
    tools, entries, blocks = {}, Counter(), Counter()
    messages = []
    for n, raw in enumerate(path.open(), 1):
        if not raw.strip():
            continue
        entry = json.loads(raw)
        kind = entry.get("type")
        role = entry.get("role") if kind is None else kind
        if role not in ("user", "assistant"):
            entries[f"{kind}/{entry['subtype']}" if entry.get("subtype") else f"{kind}"] += 1
        elif entry.get("isSidechain"):
            entries["sidechain"] += 1
        elif entry.get("isMeta"):
            entries["meta"] += 1
        elif (rendered := render_entry(entry, role, tools, blocks))[0]:
            messages.append({"line": n, "text": rendered[0], "heads": rendered[1]})
        else:
            entries["empty"] += 1
    return messages, {"entries": entries, "blocks": blocks}


class Server:
    """llama-server's HTTP API, over one connection."""

    def __init__(self, url):
        self.conn = http.client.HTTPConnection(urlparse(url).netloc, timeout=3600)

    def request(self, method, path, body=None):
        headers = {"Content-Type": "application/json"} if body is not None else {}
        for attempt in (1, 2):
            try:  # the server closes a connection that idles for a few seconds; every request can be sent again
                self.conn.request(method, path, body=json.dumps(body) if body is not None else None, headers=headers)
                r = self.conn.getresponse()
                raw = r.read()
                break
            except (http.client.RemoteDisconnected, ConnectionError):
                self.conn.close()
                if attempt == 2:
                    raise
        if r.status != 200:
            sys.exit(f"dense: {method} {path}: {r.status} {raw[:300]!r}")
        return json.loads(raw)

    def tokens(self, text):
        """Token ids of a text as the embedder reads it, without the EOS the server appends to every input."""
        return self.request("POST", "/tokenize", {"content": text, "add_special": False})["tokens"]

    def pieces(self, text):
        """The bytes of every token of a text, in order: they join back to the text."""
        found = self.request("POST", "/tokenize", {"content": text, "add_special": False, "with_pieces": True})["tokens"]
        return [bytes(p["piece"]) if isinstance(p["piece"], list) else p["piece"].encode() for p in found]

    def embed(self, texts):
        """Unit vectors of the texts and the tokens served, each text after a sentinel so that it stands alone."""
        data = self.request("POST", "/v1/embeddings", {"input": [t for text in texts for t in (SENTINEL, text)]})
        if len(data["data"]) != 2 * len(texts):
            sys.exit(f"dense: asked for {2 * len(texts)} embeddings, got {len(data['data'])}")
        found = sorted(data["data"], key=lambda d: d["index"])[1::2]
        if any(len(d["embedding"]) != DIMS for d in found):
            sys.exit(f"dense: the embedder returns vectors of another size than {DIMS}")
        return np.array([d["embedding"] for d in found], dtype="<f2"), data["usage"]["prompt_tokens"]


def head_of(text):
    """The label line of a message up to its first colon: `user:`, `tool_use Edit:`, `tool_result Bash:`."""
    first = text.split("\n", 1)[0]
    return first[: first.index(":") + 1] if ":" in first else first


def windows(server, text, heads):
    """A message over the budget as bodies of its tokens, each OVERLAP tokens before the end of the previous one. Every
    window but the first, which starts with the message's own label line, gets the label line of the piece it starts
    in. An edge inside a multi-byte character moves out to the character, and a window that the extra bytes push over
    the budget loses its last token until it fits."""
    width = BUDGET - len(server.tokens(heads[0][1] + "\n"))
    data, pieces = text.encode(), server.pieces(text)
    offsets = [0]
    for p in pieces:
        offsets.append(offsets[-1] + len(p))
    if offsets[-1] != len(data):
        sys.exit("dense: the tokens do not join back to the text")
    out, start = [], 0
    while True:
        end = min(start + width, len(pieces))
        while True:
            lo, hi = offsets[start], offsets[end]
            while lo > 0 and data[lo] & 0xC0 == 0x80:
                lo -= 1
            while hi < len(data) and data[hi] & 0xC0 == 0x80:
                hi += 1
            body = data[lo:hi].decode()
            if lo:
                body = next(h for at, h in reversed(heads) if at <= lo) + "\n" + body
            n = len(server.tokens(body))
            if n <= BUDGET:
                break
            end -= 1
        out.append((body, n))
        if end == len(pieces):
            return out
        start = end - OVERLAP


def chunk_session(server, repo, session, messages):
    """The session's messages as chunks, in line order: consecutive messages packed while they fit the budget, a
    message over it as windows of its own. A chunk keeps its first and last line."""
    chunks, current, stats = [], None, Counter()

    def close():
        nonlocal current
        if current:
            chunks.append(current)
            stats["packed"] += 1
        current = None

    for m in messages:
        n = len(server.tokens(m["text"]))
        if n > BUDGET:
            close()
            stats["split_messages"] += 1
            parts = windows(server, m["text"], m["heads"])
            chunks += [
                {"lines": [m["line"], m["line"]], "part": [k, len(parts)], "tokens": t, "text": text}
                for k, (text, t) in enumerate(parts)
            ]
            stats["windows"] += len(parts)
            continue
        if current:
            joined = current["text"] + "\n" + m["text"]
            t = len(server.tokens(joined))
            if t <= BUDGET:
                current.update(text=joined, tokens=t, lines=[current["lines"][0], m["line"]])
                continue
            close()
        current = {"lines": [m["line"], m["line"]], "part": None, "tokens": n, "text": m["text"]}
    close()
    return [{"repo": repo, "session": session} | c for c in chunks], stats


def save(vectors, rows, progress):
    """Append the session to the checkpoint and record it last, so a run that stopped half way drops the session."""
    CHECKPOINT.mkdir(exist_ok=True)
    with open(CHECKPOINT / "vectors.f16", "ab") as f:
        f.write(vectors.tobytes())
        f.flush()
        os.fsync(f.fileno())
        progress["vector_bytes"] = f.tell()
    with open(CHECKPOINT / "chunks.jsonl", "ab") as f:
        f.write("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows).encode())
        f.flush()
        os.fsync(f.fileno())
        progress["chunk_bytes"] = f.tell()
    tmp = CHECKPOINT / "progress.json.tmp"
    tmp.write_text(json.dumps(progress, indent=2) + "\n")
    tmp.rename(CHECKPOINT / "progress.json")


def restore():
    progress = json.loads((CHECKPOINT / "progress.json").read_text())
    os.truncate(CHECKPOINT / "vectors.f16", progress["vector_bytes"])
    os.truncate(CHECKPOINT / "chunks.jsonl", progress["chunk_bytes"])
    return progress


def manifest(server, progress):
    inputs = (WORK / "key.txt").read_text().splitlines()
    counts = Counter(progress["counts"])
    return {
        "arm": "dense",
        "key": inputs[0],
        "inputs": inputs[1:],
        "device": os.environ["BILBO_EVAL_DEVICE"],
        "versions": {"python": sys.version.split()[0], "numpy": version("numpy")}
        | {"llama_cpp": server.request("GET", "/props")["build_info"]},
        "embedder": {"model": "Qwen3-Embedding-0.6B-Q8_0.gguf", "pooling": "last", "n_ctx": 8192, "dims": DIMS}
        | {"normalized": True, "dtype": "float16"},
        "chunking": {"budget": BUDGET, "overlap": OVERLAP, "sentinel": SENTINEL, "chunks_per_request": REQUEST},
        "sessions": len(progress["done"]),
        "skipped": progress["skipped"],
        "messages": counts["messages"],
        "chunks": {k: counts[k] for k in ("total", "packed", "windows", "split_messages")}
        | {"by_repo": dict(sorted(progress["by_repo"].items()))},
        "tokens": {"content": counts["content_tokens"], "served": counts["served_tokens"]},
        "embed_requests": counts["requests"],
        "ingest_seconds": round(progress["ingest_seconds"]),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }


def main():
    server = Server(os.environ["EMBEDDER_URL"])
    if (CHECKPOINT / "progress.json").exists():
        progress = restore()
        log(f"resuming after {len(progress['done'])} sessions")
    else:
        for f in CHECKPOINT.glob("*"):
            f.unlink()
        progress = {"done": [], "counts": {}, "skipped": {"entries": {}, "blocks": {}}, "by_repo": {}, "ingest_seconds": 0}
    counts, by_repo = Counter(progress["counts"]), Counter(progress["by_repo"])
    skipped = {k: Counter(v) for k, v in progress["skipped"].items()}

    plan = sessions()
    started = time.monotonic()
    for repo, path in plan:
        if f"{repo}/{path.name}" in progress["done"]:
            continue
        messages, left_out = render_session(path)
        chunks, stats = chunk_session(server, repo, path.name, messages)
        vectors = []
        for i in range(0, len(chunks), REQUEST):
            batch, served = server.embed([c["text"] for c in chunks[i : i + REQUEST]])
            vectors.append(batch)
            counts["requests"] += 1
            counts["served_tokens"] += served
        rows = [{"id": counts["total"] + i} | c for i, c in enumerate(chunks)]
        counts.update(stats)
        counts.update(messages=len(messages), total=len(chunks), content_tokens=sum(c["tokens"] for c in chunks))
        by_repo[repo] += len(chunks)
        for kind, found in left_out.items():
            skipped[kind].update(found)
        progress["done"].append(f"{repo}/{path.name}")
        progress |= {"counts": dict(counts), "by_repo": dict(by_repo)}
        progress["skipped"] = {k: dict(sorted(v.items())) for k, v in skipped.items()}
        progress["ingest_seconds"] += time.monotonic() - started
        started = time.monotonic()
        save(np.concatenate(vectors) if vectors else np.empty((0, DIMS), "<f2"), rows, progress)
        log(f"{len(progress['done'])}/{len(plan)} sessions: {counts['total']} chunks so far")

    out = WORK / "store"
    out.mkdir(exist_ok=True)
    matrix = np.fromfile(CHECKPOINT / "vectors.f16", dtype="<f2").reshape(-1, DIMS)
    np.save(out / "vectors.npy", matrix)
    (out / "chunks.jsonl").write_bytes((CHECKPOINT / "chunks.jsonl").read_bytes())
    (out / "manifest.json").write_text(json.dumps(manifest(server, progress), indent=2) + "\n")
    log(f"done: {out}")


if __name__ == "__main__":
    main()
