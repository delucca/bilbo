"""Check a MemPalace store unpacked in /work/store: the palace opens with MemPalace's OpenAI-compatible embedding
function, never chromadb's default, with unit vectors of 1,024 dimensions; the counts by kind, wing and room match the
manifest; every drawer carries the metadata MemPalace writes; each session has the chunks MemPalace's own normalizer
and chunker make of its corpus file, dated with its latest timestamp and filed during the session, and the sweep
drawers of its messages (one per message uuid); the sessions are the first N of the corpus; up to five transcript
drawers, one diary drawer and two sweep drawers (one of them cut, when the build cut any) embedded again point the way
their stored vectors do; and four searches in the sessions wing, in each diary wing and in none find drawers. The
hallways are what MemPalace's hallway count makes of the transcript drawers, and the write log holds every diary write.
A store built in lanes (manifest `lanes`) holds each lane's drawers, as many of each kind as the manifest says, and each
lane's hallways, which only that lane's drawers make. build-store.sh --check runs it in the build service, with the
embedder and no LLM."""

import http.client
import json
import os
import re
import sys
from collections import Counter, defaultdict
from datetime import datetime, timedelta
from itertools import combinations
from pathlib import Path
from urllib.parse import urlparse

import numpy as np

STORE = Path("/work/store")
CORPUS = Path("/corpus")
API_MODEL = "embedder"
EF_NAME = f"openai_compat_emb_{API_MODEL}"  # mempalace/embedding.py:632-636
DIM = 1024
MIN_COSINE = 0.99  # as for dense and ai-memory; dense measured 0.9976 between a CPU and a GPU embedding
ROOMS = {"technical", "architecture", "planning", "decisions", "problems", "general"}  # convo_miner.py:417-494
QUERIES = ("what was decided about the build", "which bug was fixed", "how are tests run", "what did the user prefer")
TRANSCRIPT = re.compile(r"^/(Users|home)/[^/]+/\.claude/projects/[^/]+/([0-9a-f-]{36})\.jsonl$")
CHECKPOINT = re.compile(r"^CHECKPOINT:(\d{4}-\d{2}-\d{2})\|session:([0-9a-f-]{36})\|msgs:\d+\|recent:")
EMBEDDER = urlparse(os.environ["EMBEDDER_URL"])
# The settings the build ran with (build.py CONFIG); the store opens only under the same embedder.
os.environ.update({"MEMPALACE_CONFIG_DIR": str(STORE / "config"), "MEMPALACE_PALACE_PATH": str(STORE / "palace"),
                   "MEMPALACE_EMBEDDING_MODEL": "openai-compat", "MEMPALACE_EMBEDDING_API_URL": os.environ["EMBEDDER_URL"],
                   "MEMPALACE_EMBEDDING_API_MODEL": API_MODEL, "ANONYMIZED_TELEMETRY": "False"})
Path(os.environ["HOME"]).mkdir(parents=True, exist_ok=True)

from mempalace.config import MempalaceConfig, strip_lone_surrogates, strip_nul_bytes  # noqa: E402
from mempalace.convo_miner import MIN_CHUNK_SIZE, chunk_exchanges  # noqa: E402
from mempalace.hallways import _hallway_id, _parse_entities  # noqa: E402
from mempalace.ids import make_convo_drawer_id  # noqa: E402
from mempalace.normalize import normalize_conversations  # noqa: E402
from mempalace.palace import get_collection  # noqa: E402
from mempalace.searcher import search_memories  # noqa: E402
from mempalace.sweeper import parse_claude_jsonl  # noqa: E402

manifest = json.loads((STORE / "manifest.json").read_text())
if manifest["arm"] != "mempalace" or manifest["embedder"]["api_model"] != API_MODEL or manifest["embedder"]["ef_name"] != EF_NAME:
    sys.exit(f"check: the manifest is not a mempalace store built with {EF_NAME}")
config = json.loads((STORE / "config" / "config.json").read_text())
listed = {e["key"].removeprefix("config.json "): e["value"] for e in manifest["config"] if e["key"].startswith("config.json ")}
config_ok = config == listed and config.get("embedding_model") == "openai-compat" and config.get("embedding_api_model") == API_MODEL
identity_ok = (STORE / "palace" / "mempalace_embedder.json").exists() and manifest["embedder"]["identity_recorded"] is True
print("config.json matches the manifest", config_ok, "| embedder identity recorded", (STORE / "palace" / "mempalace_embedder.json").exists(),
      "manifest", manifest["embedder"]["identity_recorded"])
cutting = manifest["embedder"]["cut"]
NONE = "(none)"  # build.py's name for a missing wing or room


def post(path, body):
    conn = http.client.HTTPConnection(EMBEDDER.netloc, timeout=3600)
    conn.request("POST", path, json.dumps(body), {"Content-Type": "application/json"})
    r = conn.getresponse()
    data = r.read()
    if r.status != 200:
        sys.exit(f"check: POST {path}: {r.status} {data[:300]!r}")
    return json.loads(data)


def tokens(text):
    return post(EMBEDDER.path.removesuffix("/v1") + "/tokenize", {"content": text, "add_special": False})["tokens"]


def cut(text):
    """What the build sent for a sweep message (build.py, Cut.cut): the text, or its first tokens. A copy, since this
    file is outside the key."""
    if len(text.encode()) <= cutting["limit_tokens"]:
        return text, False
    ids = tokens(text)
    if len(ids) <= cutting["limit_tokens"]:
        return text, False
    ids = ids[: cutting["limit_tokens"]]
    if cutting["sent_as"] == "token ids":
        return ids, True
    head = post(EMBEDDER.path.removesuffix("/v1") + "/detokenize", {"tokens": ids})["content"]
    while len(tokens(head)) > cutting["limit_tokens"]:
        ids = ids[:-1]
        head = post(EMBEDDER.path.removesuffix("/v1") + "/detokenize", {"tokens": ids})["content"]
    return head, True


def embedded(text):
    """The embedder's vector for an input, sent as MemPalace sends it (embedding.py:646-667), one input a request; an
    input is a text or the token ids of a cut one."""
    data = post(EMBEDDER.path.rstrip("/") + "/embeddings", {"model": API_MODEL, "input": [text], "encoding_format": "float"})
    v = np.asarray(data["data"][0]["embedding"], dtype=np.float32)
    return v / np.linalg.norm(v)


def ef_name(col):
    """The name of the embedding function the opened collection carries; chromadb 1.x keeps none on disk, so a reader
    that resolves none gets the library default (backends/chroma.py:2939-2952)."""
    for obj in (col, getattr(col, "_inner", None), getattr(col, "_collection", None), getattr(col, "collection", None)):
        f = getattr(obj, "_embedding_function", None)
        if f is not None:
            return f.name() if hasattr(f, "name") else type(f).__name__
    return None


col = get_collection(str(STORE / "palace"), create=False, read_only=True)  # raises on a recorded identity mismatch
name = ef_name(col)
# Paged by 1,000 as MemPalace's own scans are (palace/mined.py:169-171): chromadb 1.5.9 fails an unbounded get past
# 32,766 rows. Each page's embeddings become a float32 array at once, since MemPalace hands them back as Python lists.
ids, metas, docs, pages = [], [], [], []
for offset in range(0, col.count(), 1000):
    page = col.get(limit=1000, offset=offset, include=["metadatas", "documents", "embeddings"])
    ids += page["ids"]
    metas += page["metadatas"]
    docs += page["documents"]
    pages.append(np.asarray(page["embeddings"], dtype=np.float32))
n = len(ids)
if not n:
    sys.exit("check: the store has no drawers")
vectors = np.concatenate(pages)
norms = np.linalg.norm(vectors, axis=1)
vectors_ok = vectors.shape == (n, DIM) and bool(np.isfinite(norms).all()) and float(np.abs(norms - 1).max()) <= 1e-3
ef_ok = name == EF_NAME
print("embedding function", name, "| drawers", n, "| vectors", vectors.shape, f"| norms {norms.min():.5f} to {norms.max():.5f}")


def kind_of(m):
    return m.get("ingest_mode") or ("diary" if m.get("type") == "diary_entry" else "other")


kinds = Counter(kind_of(m) for m in metas)
by_wing, by_room = Counter(m.get("wing") or NONE for m in metas), Counter(m.get("room") or NONE for m in metas)
d = manifest["drawers"]
counts_ok = n == d["total"] and dict(kinds) == d["by_kind"] and dict(by_wing) == d["by_wing"] and dict(by_room) == d["by_room"]
print("by kind", dict(kinds), "| counts match the manifest", counts_ok)

# The corpus session each transcript path names, by the session id in its file name, and its first and latest time.
# The latest is the maximum over every line, as MemPalace takes authored_at (convo_miner.py:595-625), not the last line's.
spans, files, firsts = {}, {}, {}
for path in CORPUS.glob("*/*.jsonl"):
    sid = path.stem.split("--")[-1]
    entries = list(map(json.loads, path.open()))
    times = [e["timestamp"] for e in entries if isinstance(e.get("timestamp"), str)]
    if times:
        spans[sid] = (min(times), max(times))
        # The build orders by the session's own first record, the first with the file name's session id (build.py own_start).
        own = next((i for i, e in enumerate(entries) if e.get("sessionId") == sid), 0)
        firsts[sid] = datetime.fromisoformat(next(e["timestamp"] for e in entries[own:] if isinstance(e.get("timestamp"), str)))
        files[sid] = path
# The sessions a build of N replays: the first N by their own start, ties as build.py sorts them (repository, then file
# name); the Cursor transcripts (no timestamp) left out.
limit = next(line.split()[1] for line in manifest["inputs"] if line.startswith("sessions "))
order = sorted((first, files[sid].parent.name, files[sid].name, sid) for sid, first in firsts.items())
if ":" in limit:  # repo:N,repo:N: the first N sessions of each repository named
    per, taken, expected_sids = {r: int(k) for r, k in (part.split(":") for part in limit.split(","))}, Counter(), set()
    for _, repo, _, sid in order:
        if taken[repo] < per.get(repo, 0):
            taken[repo] += 1
            expected_sids.add(sid)
else:
    expected_sids = {row[-1] for row in order[: None if limit == "all" else int(limit)]}
# What MemPalace's own normalizer and chunker make of a corpus file, as its mine does (convo_miner.py:1128-1150).
chunk_size = MempalaceConfig(palace_path=str(STORE / "palace")).chunk_size


def expected_chunks(path):
    conversations = [c for c in normalize_conversations(str(path)) if c]
    return len(chunk_exchanges("\n\n".join(conversations), chunk_size=chunk_size, min_chunk_size=MIN_CHUNK_SIZE))

per_source, bad = defaultdict(list), Counter()
for i, m in zip(ids, metas):
    if kind_of(m) != "convos":
        continue
    src = m.get("source_file") or ""
    found = TRANSCRIPT.match(src)
    fields = (m.get("wing") == "sessions" and m.get("room") in ROOMS and isinstance(m.get("hall"), str) and found
              and isinstance(m.get("chunk_index"), int) and isinstance(m.get("chunk_total"), int) and 0 <= m["chunk_index"] < m["chunk_total"]
              and m.get("extract_mode") == "exchange" and m.get("normalize_version") == 2 and m.get("id_recipe") == "v3"
              and m.get("added_by") == "mempalace" and m.get("filed_at") and m.get("authored_at") and m.get("source_mtime"))
    if not fields:
        bad["fields"] += 1
        continue
    if i != make_convo_drawer_id("sessions", m["room"], src, "exchange", m["chunk_index"]):
        bad["id"] += 1
    per_source[(found.group(2), src)].append(m)
complete = 0
for (sid, src), rows in per_source.items():
    span = spans.get(sid)
    totals, rooms, authored = {m["chunk_total"] for m in rows}, {m["room"] for m in rows}, {m["authored_at"] for m in rows}
    filed = {m["filed_at"] for m in rows}
    if not span or len(totals) != 1 or sorted(m["chunk_index"] for m in rows) != list(range(rows[0]["chunk_total"])):
        bad["incomplete"] += 1
    elif rows[0]["chunk_total"] != expected_chunks(files[sid]):
        bad["chunks"] += 1  # not what MemPalace's chunker makes of the whole transcript
    elif len(rooms) != 1 or len(authored) != 1 or len(filed) != 1:
        bad["mixed"] += 1
    elif rows[0]["authored_at"] != span[1]:
        bad["authored_at"] += 1  # the last mine did not see the whole transcript
    elif not (span[0][:19] <= rows[0]["filed_at"][:19] <= (datetime.fromisoformat(span[1].replace("Z", "")) + timedelta(days=1)).isoformat()):
        bad["filed_at"] += 1  # filed outside the session's time: the clock was not the event's
    else:
        complete += 1
sids = {sid for sid, _ in per_source}
sessions_ok = (not bad and complete == len(per_source) == d["sessions_with_drawers"] == manifest["sessions"] and len(sids) == len(per_source)
               and sids == expected_sids)
print("sessions with drawers", len(per_source), "complete", complete, "manifest", manifest["sessions"], "expected", len(expected_sids), "| bad", dict(bad))

# A sweep drawer per message uuid of each session (sweeper.py:196-330): the ids MemPalace's own parser yields from the
# corpus files, no more and no fewer, each carrying its message's metadata, filed at or after its session's last day.
sweep_want = {}
for sid in expected_sids:
    for rec in parse_claude_jsonl(str(files[sid])):
        sweep_want[f"sweep_{rec['session_id']}_{rec['uuid']}"] = (sid, rec)
sweep_bad, sweep_got = Counter(), {}
for i, m, doc in zip(ids, metas, docs):
    if kind_of(m) != "sweep":
        continue
    sweep_got[i] = m
    want = sweep_want.get(i)
    found = TRANSCRIPT.match(m.get("source_file") or "")
    ok = (want and found and found.group(2) == want[0] and m.get("session_id") == want[1]["session_id"] and m.get("message_uuid") == want[1]["uuid"]
          and m.get("role") == want[1]["role"] and m.get("timestamp") == want[1]["timestamp"] and m.get("filed_at")
          and doc == strip_nul_bytes(strip_lone_surrogates(f"{want[1]['role'].upper()}: {want[1]['content']}")) and not m.get("wing") and not m.get("room")
          and m["filed_at"][:10] >= spans[want[0]][1][:10])
    sweep_bad["fields"] += not ok
sweep_bad["missing"] = len(set(sweep_want) - set(sweep_got))
sweep_ok = not any(sweep_bad.values()) and len(sweep_got) == len(sweep_want) == kinds["sweep"] == manifest["replay"]["sweeps"]["drawers_added"]
print("sweep drawers", len(sweep_got), "expected", len(sweep_want), "manifest", manifest["replay"]["sweeps"], "| bad", dict(sweep_bad))

# A diary entry over 800 characters is several drawers, chunk 0 holding its head (mcp_server/tools_diary.py:88-125).
diary_bad, diary_wings, entries = 0, set(), 0
for i, m, doc in zip(ids, metas, docs):
    if kind_of(m) != "diary":
        continue
    ok = (m.get("wing", "").startswith("wing_") and m.get("room") == "diary" and m.get("hall") == "hall_diary" and m.get("agent") == "claude"
          and m.get("topic") == "checkpoint" and (m.get("filed_at") or "")[:10] == m.get("date") and isinstance(m.get("chunk_index"), int))
    if ok and m["chunk_index"] == 0:
        found = CHECKPOINT.match(doc)
        ok = bool(found) and found.group(1) == m["date"] and found.group(2) in sids
        entries += 1
    elif ok:
        ok = m.get("parent_entry_id") and i.startswith(m["parent_entry_id"] + "_chunk_")
    diary_bad += not ok
    diary_wings.add(m.get("wing"))
diary_ok = diary_bad == 0 and entries == manifest["replay"]["diary_writes"] == d["diary_entries"]
print("diary drawers", kinds["diary"], "entries", entries, "manifest writes", manifest["replay"]["diary_writes"], "| wings", sorted(diary_wings), "| bad", diary_bad)

# The repository each drawer came from: a transcript or sweep drawer by its session, a diary drawer by the session its
# checkpoint names (a chunked entry's chunks by their first chunk's, through parent_entry_id).
lane_of, entry_session = {sid: path.parent.name for sid, path in files.items()}, {}
for i, m, doc in zip(ids, metas, docs):
    if kind_of(m) == "diary" and m.get("chunk_index") == 0 and (found := CHECKPOINT.match(doc)):
        entry_session[m.get("parent_entry_id") or i] = found.group(2)
repo_of = []
for i, m in zip(ids, metas):
    if kind_of(m) == "diary":
        repo_of.append(lane_of.get(entry_session.get(m.get("parent_entry_id") or i), "?"))
    else:
        found = TRANSCRIPT.match(m.get("source_file") or "")
        repo_of.append(lane_of.get(found.group(2), "?") if found else "?")


def hallways_of(rows):
    """MemPalace's hallway count over some transcript drawers (hallways.py compute_hallways_for_wing, min_count 2):
    every pair of distinct entities a drawer carries is one co-occurrence. As a mine leaves them: id, pair, count and
    rooms."""
    counts, rooms = Counter(), defaultdict(set)
    for m in rows:
        entities = _parse_entities(m.get("entities"))
        if m.get("is_sentinel") or len(entities) < 2:
            continue
        for a, b in combinations(entities, 2):
            if a != b:
                key = tuple(sorted([a, b]))
                counts[key] += 1
                if isinstance(m.get("room"), str) and m["room"].strip():
                    rooms[key].add(m["room"])
    return {_hallway_id("sessions", a, b): (a, b, n, sorted(rooms[(a, b)])) for (a, b), n in counts.items() if n >= 2}


def as_left(records):
    return {h["id"]: (h["entity_a"], h["entity_b"], h["co_occurrence_count"], h["rooms"]) for h in records if h.get("wing") == "sessions"}


def wal_findings(writes, entries):
    """What is wrong with the write log against the diary. MemPalace writes one diary_write per logical entry
    (tools_diary.py, wal.py; a chunked entry once, under its parent id), so every entry id the log holds must be a
    diary entry's, none twice, and none of them missing."""
    got, bad = Counter((w.get("params") or {}).get("entry_id") for w in writes), []
    if any(w.get("operation") != "diary_write" for w in writes):
        bad.append("operations other than diary_write")
    if twice := sorted(i for i, k in got.items() if k > 1):
        bad.append(f"entry ids written twice {twice[:3]}")
    if unknown := sorted(set(got) - set(entries), key=str):
        bad.append(f"entry ids of no diary entry {unknown[:3]}")
    if missing := sorted(set(entries) - set(got)):
        bad.append(f"diary entries never written {missing[:3]}")
    return bad


def slice_findings(lanes, total):
    """The lanes' hallway slices, in the manifest's lane order, must follow one another from 0 with no gap and no
    overlap, and end at the end of the file."""
    at, bad = 0, []
    for lane, facts in lanes.items():
        if facts["hallways"]["first"] != at or facts["hallways"]["count"] < 0:
            bad.append(lane)
        at = facts["hallways"]["first"] + facts["hallways"]["count"]
    return bad + ([] if at == total else [f"the slices end at {at}, the file at {total}"])


def lane_findings(lanes, manifest, actual):
    """Each lane's manifest facts against what the store holds for it, and their sums against the store's own fields.
    actual[lane] holds what the store says: the lane's sessions (the corpus sessions its transcript drawers come from),
    its drawers by kind, its diary entries and its write-log lines."""
    bad = []
    if set(lanes) != set(actual):
        bad.append(f"lanes the manifest and the store do not share: {sorted(set(lanes) ^ set(actual))}")
    for lane, facts in lanes.items():
        a = actual.get(lane, {"sessions": set(), "by_kind": {}, "diary_entries": 0, "write_log": 0})
        if not (facts["sessions"] == len(a["sessions"]) == manifest["by_repo"].get(lane) == facts.get("sweeps")):
            bad.append(f"{lane}: sessions")
        if facts["drawers"]["by_kind"] != a["by_kind"] or facts["drawers"]["total"] != sum(a["by_kind"].values()):
            bad.append(f"{lane}: drawers")
        if not (facts["diary_writes"] == a["diary_entries"] == facts["write_log"] == a["write_log"]):
            bad.append(f"{lane}: diary writes")
    total = lambda key: sum(f.get(key, 0) for f in lanes.values())  # noqa: E731
    sums = {"sessions": manifest["sessions"], "diary_writes": manifest["replay"]["diary_writes"],
            "sweeps": manifest["replay"]["sweeps"]["runs"], "mines": manifest["replay"]["mines"],
            "stop": manifest["events"]["stop"], "precompact": manifest["events"]["precompact"],
            "session-end": manifest["events"]["session_end"]}
    bad += [f"the lanes' {key} add up to {total(key)}, the store's to {want}" for key, want in sums.items() if total(key) != want]
    return bad


hallways = json.loads((STORE / "hallways.json").read_text()).get("hallways", []) if (STORE / "hallways.json").exists() else []
lanes = manifest.get("lanes")
sessions_rows = [(r, m) for r, m in zip(repo_of, metas) if m.get("wing") == "sessions"]
if lanes:  # each lane's hallways, lane after lane, made by its own drawers alone
    hallways_bad = slice_findings(lanes, len(hallways))
    for lane, facts in lanes.items():
        part = hallways[facts["hallways"]["first"] : facts["hallways"]["first"] + facts["hallways"]["count"]]
        if as_left(part) != hallways_of([m for r, m in sessions_rows if r == lane]) or len(as_left(part)) != len(part):
            hallways_bad.append(lane)
else:
    hallways_bad = [] if as_left(hallways) == hallways_of([m for _, m in sessions_rows]) and len(as_left(hallways)) == len(hallways) else ["the store"]
hallways_ok = not hallways_bad and len(hallways) == manifest["hallways"]
print("hallways", len(hallways), "manifest", manifest["hallways"], "| not what MemPalace's count makes of the drawers:", hallways_bad)

# Each logical diary entry (a chunked one once, by parent_entry_id) and its lane; the write log against them.
entries = {m.get("parent_entry_id") or i: r for i, m, r in zip(ids, metas, repo_of) if kind_of(m) == "diary" and m.get("chunk_index") == 0}
log_file = STORE / "config" / "wal" / "write_log.jsonl"
writes = [json.loads(line) for line in log_file.read_text().splitlines()] if log_file.exists() else []
log_bad = wal_findings(writes, entries)
log_ok = not log_bad and len(writes) == manifest["replay"]["diary_writes"]
print("write log", len(writes), "diary writes, manifest", manifest["replay"]["diary_writes"], "| bad", log_bad)

lanes_ok = True
if lanes:  # every lane's sessions, drawers and diary, from the store and the corpus, against the manifest
    actual = defaultdict(lambda: {"sessions": set(), "by_kind": Counter(), "diary_entries": 0, "write_log": 0})
    for i, r, m in zip(ids, repo_of, metas):
        actual[r]["by_kind"][kind_of(m)] += 1
        if kind_of(m) == "convos" and (found := TRANSCRIPT.match(m.get("source_file") or "")):
            actual[r]["sessions"].add(found.group(2))
    for r in entries.values():
        actual[r]["diary_entries"] += 1
    for w in writes:
        actual[entries.get((w.get("params") or {}).get("entry_id"), "?")]["write_log"] += 1
    actual = {r: a | {"by_kind": dict(a["by_kind"])} for r, a in actual.items()}
    lanes_bad = lane_findings(lanes, manifest, actual)
    lanes_ok = not lanes_bad and sum(f["drawers"]["total"] for f in lanes.values()) == n
    print("lanes", len(lanes), "| sessions by lane", {r: len(a["sessions"]) for r, a in sorted(actual.items())}, "| bad", lanes_bad)

# Up to five transcript drawers spread over the store, one diary drawer, and two sweep drawers (the longest message,
# which the build cut when it cut any, and the shortest), embedded again; a sweep drawer through the build's cut.
convos = sorted(i for i, m in zip(range(n), metas) if kind_of(m) == "convos")
picks = sorted({convos[len(convos) * k // 4] for k in range(4)} | {convos[-1]})
picks += [next(i for i, m in zip(range(n), metas) if kind_of(m) == "diary")] if kinds["diary"] else []
sweeps_at = sorted((i for i, m in zip(range(n), metas) if kind_of(m) == "sweep"), key=lambda i: len(docs[i].encode()))
# The shortest sweep drawer, and the longest one the build cut (a long message of few tokens is not cut).
picks += sweeps_at[:1]
for i in reversed(sweeps_at):
    if len(docs[i].encode()) <= cutting["limit_tokens"]:
        break
    if cut(docs[i])[1]:
        picks.append(i)
        break
cosines, cut_seen = [], 0
for i in picks:
    sent, was_cut = cut(docs[i]) if kind_of(metas[i]) == "sweep" else (docs[i], False)
    cut_seen += was_cut
    cosines.append(float(embedded(sent) @ vectors[i] / np.linalg.norm(vectors[i])))
    print(f"drawer {i} ({kind_of(metas[i])}, {len(docs[i])} chars{', cut' if was_cut else ''}): cosine {cosines[-1]:.6f}")
embedded_ok = bool(cosines) and all(c >= MIN_COSINE for c in cosines) and (cut_seen > 0 or cutting["cut"] == 0)
print("cut inputs", cutting, "| cut drawers embedded again", cut_seen)

found, sweep_hits = {}, 0
for wing in ["sessions", *sorted(diary_wings), None]:
    for q in QUERIES:
        hits = search_memories(q, str(STORE / "palace"), wing=wing, n_results=5)["results"]
        found[f"{wing or 'all'}: {q}"] = (len(hits), round(max((h.get("similarity") or 0) for h in hits), 3) if hits else None)
        sweep_hits += wing is None and sum(str(h.get("drawer_id")).startswith("sweep_") for h in hits)
empty = sorted(k for k, (hits, _) in found.items() if not hits)
# A diary wing whose few drawers MemPalace's approximate index cannot reach comes back empty. It passes only when no
# drawer of it is found by its own stored vector under the wing's filter; the unfiltered and the transcript searches
# must always find something, and the sweep drawers must be reachable.
unreached = set()
for wing in {k.split(": ", 1)[0] for k in empty} - {"all", "sessions"}:
    members = [i for i in range(n) if metas[i].get("wing") == wing]
    answers = [col.query(query_embeddings=[vectors[i].tolist()], n_results=5, where={"wing": wing}).ids for i in members]
    reached = [ids[i] in sum(found_ids, []) for i, found_ids in zip(members, answers)]
    if members and not any(reached):
        unreached.add(wing)
searches_ok = sweep_hits > 0 and all(k.split(": ", 1)[0] in unreached for k in empty)
print("unfiltered search hits that are sweep drawers:", sweep_hits)
print("searches with no hits:", empty, "| wings none of whose drawers its own vector finds:", sorted(unreached))
print("searches (hits, best similarity):", found)

# How much of the palace its vector index reaches (DIAG-mempalace.md), printed, not judged: each drawer's own vector as
# the query of an unfiltered search for 3, found when it or an equal vector comes back.
missed = 0
for b in range(0, n, 100):
    res = col.query(query_embeddings=[v.tolist() for v in vectors[b : b + 100]], n_results=3, include=["distances"])
    missed += sum(i not in hits and not (dist and dist[0] < 1e-4) for i, hits, dist in zip(ids[b : b + 100], res["ids"], res["distances"]))
print(f"index reach: {missed} of {n} drawers not found by their own vector at k=3", "| the merge's:", (manifest.get("merge") or {}).get("reach"))

ok = (identity_ok and sweep_ok and config_ok and ef_ok and vectors_ok and counts_ok and sessions_ok and diary_ok and hallways_ok
      and embedded_ok and searches_ok and log_ok and lanes_ok)
print("OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
