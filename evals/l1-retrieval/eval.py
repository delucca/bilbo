"""L1 retrieval eval of bilbo: `bilbo recall` and `bilbo digest` against baselines on a frozen dataset.

    uv --directory evals/l1-retrieval run eval.py run --bilbo /abs/path/to/bilbo --split dev
    uv --directory evals/l1-retrieval run eval.py diff OLD.json NEW.json

A guard that fires aborts the run with exit 1 and writes nothing: scores never gate, guards do.
"""

from __future__ import annotations

import argparse
import atexit
import hashlib
import json
import os
import platform
import random
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path

import bm25s
import ir_measures
import numpy as np
import Stemmer
from bm25s.stopwords import STOPWORDS_EN
from ir_measures import RR, Judged, Qrel, R, ScoredDoc, Success, nDCG

HERE = Path(__file__).resolve().parent

# --- constants -------------------------------------------------------------------------------------------------------

MODEL = "qwen3-embedding-0.6b"
GGUF_SHA256 = "06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439"
GGUF_FILE = "Qwen3-Embedding-0.6B-Q8_0.gguf"
QUERY_PREFIX = "Instruct: Given a question, retrieve notes that answer it\nQuery: "
THRESHOLD = 0.55
LIMIT = 100
MIN_EFFECT = 0.10  # the preregistered minimum effect of the primary comparison
EXACT_FAMILIES = 13  # up to this many fact families the sign-flip test enumerates every flip
MIN_FAMILIES = 10  # fewer fact families than this and a bootstrap interval is not worth printing
SEEDS = range(20)
ARMS = ["oracle", "random", "ripgrep", "bm25", "bilbo-keyword", "bilbo-full"]
PROCESS_ARMS = {"ripgrep", "bilbo-keyword", "bilbo-full"}
STRATA = ["known-item", "paraphrase", "pt-en", "alias", "supersession", "multi-hop", "kind-filter", "no-answer", "library"]
METRICS = {"success@5": Success @ 5, "rr": RR, "ndcg@10": nDCG @ 10, "r@10": R @ 10, "judged@10": Judged @ 10}
STOPWORDS = set(STOPWORDS_EN) | set("de do da dos das em no na nos nas um uma uns umas para por com que qual quais como quando onde foi era ser são os as ao aos se ou".split())
NEGATIVE = ("noise", "off-topic", "near-miss")
NOTE_HEADER = re.compile(r"^(?P<path>.+\.md):\d+\t[a-z]+\t")
SOURCE_HEADER = re.compile(r"^(?P<path>.+\.md):\d+\t(?:source|guide)\t(?P<ref>[^\t]+)\t")
EMPTY = ("no notes match", "no sources match")
FALLBACK = ("embedder unavailable", "not indexed")
INDEX_LINE = re.compile(r"embedded (\d+), kept (\d+), dropped (\d+)")
BIG = ("per_query", "per_prompt", "queries")  # written one entry per line so git diffs stay readable
DIRECT = urllib.request.build_opener(urllib.request.ProxyHandler({}))  # bypasses http_proxy for localhost


class Abort(Exception):
    """A guard fired: the run is invalid."""


# --- dataset ---------------------------------------------------------------------------------------------------------


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def frontmatter(text: str) -> tuple[dict[str, str], str]:
    """Top-level `key: value` pairs of a leading `---` block, and the text after it."""
    lines = text.split("\n")
    if lines[0].strip() != "---":
        return {}, text
    for end in range(1, len(lines)):
        if lines[end].strip() == "---":
            meta = {}
            for line in lines[1:end]:
                if line and line[0] not in " \t-" and ":" in line:
                    key, value = line.split(":", 1)
                    meta[key.strip()] = value.strip().strip("\"'")
            return meta, "\n".join(lines[end + 1:])
    return {}, text


def jsonl(p: Path) -> list[dict]:
    return [json.loads(line) for line in p.read_text(encoding="utf-8").splitlines() if line.strip()]


@dataclass
class Dataset:
    dir: Path
    name: str
    sha: str
    notes: dict[str, dict]  # id -> file, kind, text
    library: dict[str, str]  # id -> text; a source's id is corpus/name, a guide's the corpus name
    queries: list[dict]
    prompts: list[dict]
    qrels: dict[str, dict[str, int]]


def dataset_id(listed: dict[str, str]) -> str:
    """Hash of the data files only: the datasheet can be corrected without invalidating a baseline."""
    return hashlib.sha256("".join(f"{h}  {rel}\n" for rel, h in sorted(listed.items()) if rel != "README.md").encode()).hexdigest()


def load_dataset(d: Path) -> Dataset:
    sums = d / "SHA256SUMS"
    if not sums.is_file():
        raise Abort(f"no SHA256SUMS in {d}")
    listed = {}
    for line in sums.read_text(encoding="utf-8").splitlines():
        digest, _, rel = line.partition("  ")
        listed[rel] = digest
    found = {p.relative_to(d).as_posix() for p in d.rglob("*") if p.is_file()} - {"SHA256SUMS"}
    if found != set(listed):
        raise Abort(f"{d} and its SHA256SUMS list different files: {sorted(found ^ set(listed))[:5]}")
    bad = [rel for rel, digest in listed.items() if sha256_file(d / rel) != digest]
    if bad:
        raise Abort(f"{d} does not match its SHA256SUMS: {bad[:5]}")
    notes, library = {}, {}
    for p in sorted((d / "store/notes").glob("*.md")):
        meta, body = frontmatter(p.read_text(encoding="utf-8"))
        title = next((l[2:].strip() for l in body.split("\n") if l.startswith("# ")), "")
        notes[meta["id"]] = {"file": p.name, "kind": p.stem.split("-", 1)[0], "text": f"{title}\n{body}"}
    for p in sorted((d / "store/library").glob("*/*.md")):
        ident = p.parent.name if p.name == "guide.md" else f"{p.parent.name}/{p.stem}"
        library[ident] = frontmatter(p.read_text(encoding="utf-8"))[1]
    qrels: dict[str, dict[str, int]] = defaultdict(dict)
    for line in (d / "qrels.txt").read_text(encoding="utf-8").splitlines():
        if line.strip():
            qid, _, doc, grade = line.split()
            qrels[qid][doc] = int(grade)
    ds = Dataset(d, (d / "README.md").read_text(encoding="utf-8").splitlines()[0].lstrip("# ").strip(),
                 dataset_id(listed), notes, library, jsonl(d / "queries.jsonl"), jsonl(d / "prompts.jsonl"), dict(qrels))
    ids = set(notes) | set(library)
    refs = [(q["id"], i) for q in ds.queries for i in [*q["decoys"], *(i for s in q["evidence_sets"] for i in s)]]
    refs += [(qid, doc) for qid, docs in qrels.items() for doc in docs] + [(p["id"], i) for p in ds.prompts for i in p["gold"]]
    unknown = [(owner, i) for owner, i in refs if i not in ids]
    if unknown:
        raise Abort(f"{len(unknown)} ids outside the dataset, the first {unknown[0]}")
    return ds


# --- sandbox ---------------------------------------------------------------------------------------------------------


@dataclass
class Sandbox:
    root: Path
    store: Path
    config: Path
    state: Path
    env: dict[str, str]
    exe: Path
    path_to_id: dict[str, str]


def quote(value: str) -> str:
    """bilbo's config quoting (shared::config::quote)."""
    if not (value == "" or value[0] in " \t" or value[-1] in " \t" or value[0] == '"' or "\n" in value or "\r" in value):
        return value.replace("\\", "\\\\")
    return '"' + value.replace("\\", "\\\\").replace("\n", "\\n").replace('"', '\\"') + '"'


def folders(env: dict[str, str]) -> dict[str, Path | None]:
    """The store, config file, cache and state folders bilbo resolves for `env` (docs/reference/configuration.md#folders)."""
    absolute = lambda v: Path(v) if v and os.path.isabs(v) else None
    home = absolute(env.get("HOME"))
    under = lambda var, tail: (b / tail) if (b := absolute(env.get(var))) else None
    from_home = lambda tail: (home / tail) if home else None
    return {
        "store": absolute(env.get("BILBO_HOME")) or under("XDG_DATA_HOME", "bilbo") or from_home(".local/share/bilbo"),
        "config": absolute(env.get("BILBO_CONFIG")) or under("XDG_CONFIG_HOME", "bilbo/config") or from_home(".config/bilbo/config"),
        "cache": under("XDG_CACHE_HOME", "bilbo") or from_home(".cache/bilbo"),
        "state": under("XDG_STATE_HOME", "bilbo") or from_home(".local/state/bilbo"),
    }


def guard(sb: Sandbox) -> None:
    """Refuse to run bilbo unless every folder it resolves is inside the temp root and none is the invoking user's."""
    real = lambda p: Path(os.path.realpath(p))
    theirs = folders(dict(os.environ))
    for name, path in folders(sb.env).items():
        if path is None or sb.root not in real(path).parents and real(path) != sb.root:
            raise Abort(f"the run's {name} folder {path} is outside the run root {sb.root}")
        if theirs[name] is not None and real(theirs[name]) == real(path):
            raise Abort(f"the run's {name} folder {path} is the invoking user's {name} folder")


def make_sandbox(ds: Dataset, exe: Path) -> Sandbox:
    root = Path(os.path.realpath(tempfile.mkdtemp(prefix="bilbo-eval-")))
    for n in ("home", "cache", "data", "state", "config/bilbo"):
        (root / n).mkdir(parents=True)
    store, config = root / "store", root / "config/bilbo/config"
    shutil.copytree(ds.dir / "store", store)
    env = {
        "HOME": str(root / "home"), "BILBO_HOME": str(store), "BILBO_CONFIG": str(config),
        "XDG_CONFIG_HOME": str(root / "config"), "XDG_DATA_HOME": str(root / "data"),
        "XDG_CACHE_HOME": str(root / "cache"), "XDG_STATE_HOME": str(root / "state"),
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"), "TMPDIR": os.path.dirname(root),
        "LANG": os.environ.get("LANG", "en_US.UTF-8"), "LC_ALL": os.environ.get("LC_ALL", os.environ.get("LANG", "en_US.UTF-8")),
    }  # fmt: skip
    mapping = {str(store / "notes" / n["file"]): i for i, n in ds.notes.items()}
    mapping |= {str(store / "library" / (f"{i}.md" if "/" in i else f"{i}/guide.md")): i for i in ds.library}
    sb = Sandbox(root, store, config, root / "state/bilbo", env, exe, mapping)
    try:
        guard(sb)
    except Abort:
        shutil.rmtree(root, ignore_errors=True)
        raise
    return sb


def write_config(sb: Sandbox, embedder_url: str | None, extra: dict[str, str] | None = None) -> None:
    settings = [("embedder.url", embedder_url), ("embedder.model", MODEL), ("embedder.query_prefix", QUERY_PREFIX)] if embedder_url else []
    settings += list((extra or {}).items())
    sb.config.write_text("".join(f"{k} = {quote(v)}\n" for k, v in settings), encoding="utf-8")


@dataclass
class Proc:
    exit: int
    stdout: str
    stderr: str
    ms: float


def sh(argv: list[str], env: dict[str, str], stdin: str | None = None, cwd: Path | None = None, timeout: float = 120) -> Proc:
    start = time.monotonic()
    try:
        p = subprocess.run(argv, env=env, input=stdin, cwd=cwd, timeout=timeout, capture_output=True, text=True,
                           encoding="utf-8", errors="replace")  # fmt: skip
    except subprocess.TimeoutExpired:
        raise Abort(f"{Path(argv[0]).name} {argv[1:2]} timed out after {timeout}s") from None
    return Proc(p.returncode, p.stdout, p.stderr, (time.monotonic() - start) * 1000)


def bilbo(sb: Sandbox, args: list[str], stdin: str | None = None, timeout: float = 60) -> Proc:
    """Run bilbo in the sandbox. Any error exit or fallback to keywords aborts; `no notes match` is an empty answer."""
    guard(sb)
    p = sh([str(sb.exe), *args], sb.env, stdin, sb.root, timeout)
    if any(f in p.stderr for f in FALLBACK):
        raise Abort(f"bilbo {args[0]} fell back: {p.stderr.strip()}")
    if p.exit == 1 and any(e in p.stderr for e in EMPTY):
        return p
    if p.exit != 0:
        raise Abort(f"bilbo {' '.join(args[:3])} exited {p.exit}: {p.stderr.strip()}")
    return p


# --- llama-server ----------------------------------------------------------------------------------------------------


def http(url: str, body: dict | None = None, timeout: float = 300) -> bytes:
    req = urllib.request.Request(url, data=json.dumps(body).encode() if body else None,
                                 headers={"Content-Type": "application/json"}, method="POST" if body else "GET")  # fmt: skip
    with DIRECT.open(req, timeout=timeout) as r:
        return r.read()


@dataclass
class Server:
    url: str
    proc: subprocess.Popen
    info: dict

    def stop(self) -> None:
        self.proc.terminate()
        try:
            self.proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()


def start_server(gguf: Path, llama_server: str) -> Server:
    if not gguf.is_file():
        raise Abort(f"no embedding model at {gguf}; pass --model")
    found = sha256_file(gguf)
    if found != GGUF_SHA256:
        raise Abort(f"{gguf} is not the pinned model\n  pinned: {GGUF_SHA256}\n  found:  {found}")
    if shutil.which(llama_server) is None:
        raise Abort(f"`{llama_server}` is missing; put llama-server on PATH or pass --llama-server")
    probe = lambda flag: subprocess.run([llama_server, flag], capture_output=True, text=True, errors="replace", timeout=60)
    out = probe("--version")
    version = next((l.strip() for l in (out.stdout + out.stderr).splitlines() if l.strip().startswith("version:")), "unknown")
    listing = probe("--list-devices")
    device = re.search(r"^\s+(\w+?)\d+: ", listing.stdout + listing.stderr, re.M)
    backend = {"MTL": "Metal", "CUDA": "CUDA", "Vulkan": "Vulkan", "ROCm": "ROCm"}.get(device[1] if device else "", "CPU")
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    proc = subprocess.Popen(
        [llama_server, "--model", str(gguf), "--alias", MODEL, "--embedding", "--pooling", "last", "--host", "127.0.0.1",
         "--port", str(port), "--ctx-size", "4096", "--batch-size", "4096", "--ubatch-size", "4096", "--parallel", "1"],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )  # fmt: skip
    server = Server(f"http://127.0.0.1:{port}", proc, {"model": MODEL, "gguf_sha256": found, "llama_server": version, "backend": backend})
    atexit.register(server.stop)  # the last resort when the finally in cmd_run cannot run
    deadline = time.monotonic() + 180
    try:
        while True:
            if proc.poll() is not None:
                raise Abort(f"llama-server exited with {proc.returncode} before it was ready")
            try:
                http(f"{server.url}/health", timeout=5)
                break
            except (urllib.error.URLError, OSError):
                if time.monotonic() > deadline:
                    raise Abort("llama-server did not answer /health in 180s") from None
                time.sleep(0.5)
        vector = json.loads(http(f"{server.url}/v1/embeddings", {"model": MODEL, "input": ["probe"]}))["data"][0]["embedding"]
        if not vector:
            raise Abort("llama-server answered no vector for a probe")
    except BaseException:
        server.stop()
        raise
    return server


def index(sb: Sandbox, url: str) -> dict:
    """`bilbo index` twice: the second pass must embed and drop nothing, which shows the first one indexed every note."""
    write_config(sb, url)
    counts = []
    for _ in range(2):
        log = (p := bilbo(sb, ["index"], timeout=3600)).stdout + p.stderr
        if "withheld" in log:
            raise Abort(next(l for l in log.splitlines() if "withheld" in l))
        m = INDEX_LINE.search(log)
        if not m:
            raise Abort(f"bilbo index printed no `embedded` line: {log.strip()}")
        counts.append((int(m[1]), int(m[3])))
    if counts[1] != (0, 0):
        raise Abort(f"a second bilbo index embedded {counts[1][0]} and dropped {counts[1][1]}")
    return {"embedded": counts[0][0], "second_pass_embedded": counts[1][0]}


# --- arms ------------------------------------------------------------------------------------------------------------
# An arm maps a query to a list of rankings (one, except random's seeds) and the milliseconds it took, if timed.

Ranked = tuple[list[list[str]], float | None]


def candidates(ds: Dataset, q: dict) -> dict[str, str]:
    if q["stratum"] == "library":
        return ds.library
    return {i: n["text"] for i, n in ds.notes.items() if not q["kind"] or n["kind"] == q["kind"]}


def restrict(ds: Dataset, q: dict, ranking: list[str]) -> list[str]:
    if q["kind"] and q["stratum"] != "library":
        ranking = [i for i in ranking if ds.notes[i]["kind"] == q["kind"]]
    return ranking[:LIMIT]


def positives(ds: Dataset, qid: str) -> list[str]:
    docs = ds.qrels.get(qid, {})
    return sorted((d for d, g in docs.items() if g > 0), key=lambda d: (-docs[d], d))


def oracle_arm(ds: Dataset):
    return lambda q: ([positives(ds, q["id"])], None)


def random_arm(ds: Dataset):
    def rank(q: dict) -> Ranked:
        ids = sorted(candidates(ds, q))
        return [random.Random(s).sample(ids, len(ids))[:LIMIT] for s in SEEDS], None

    return rank


def ripgrep_arm(ds: Dataset, sb: Sandbox):
    exe = shutil.which("rg")
    if exe is None:
        raise Abort("`rg` is missing; put ripgrep on PATH")

    def rank(q: dict) -> Ranked:
        if q["stratum"] == "library":
            targets = [str(d) for d in sorted((sb.store / "library").iterdir()) if d.is_dir() and not d.name.startswith(".")]
        else:
            targets = [str(sb.store / "notes")]
        found, total, wall = Counter(), Counter(), 0.0
        for word in sorted({w for w in re.findall(r"\w+", q["text"].lower()) if w not in STOPWORDS and len(w) > 1}):
            p = sh([exe, "--ignore-case", "--word-regexp", "--fixed-strings", "--count-matches", "--no-ignore",
                    "--with-filename", "-e", word, "--", *targets], sb.env)  # fmt: skip
            wall += p.ms
            if p.exit not in (0, 1):
                raise Abort(f"rg exited {p.exit}: {p.stderr.strip()}")
            for line in p.stdout.splitlines():
                path, _, count = line.rpartition(":")
                if path not in sb.path_to_id:
                    raise Abort(f"rg printed a path no dataset id maps to: {path}")
                found[path] += 1
                total[path] += int(count)
        ranking = [sb.path_to_id[p] for p in sorted(found, key=lambda p: (-found[p], -total[p], p))]
        return [restrict(ds, q, ranking)], wall

    return rank


def bm25_arm(ds: Dataset):
    """Lucene BM25 (k1 1.2, b 0.75) with English stemming over whole notes; documents scoring zero are dropped."""
    stemmer = Stemmer.Stemmer("english")
    tokens = lambda texts, ids: bm25s.tokenize(texts, stopwords="en", stemmer=stemmer, show_progress=False, return_ids=ids)
    indexes = {}
    for library, docs in ((False, {i: n["text"] for i, n in ds.notes.items()}), (True, ds.library)):
        ids = sorted(docs)
        retriever = bm25s.BM25(method="lucene", k1=1.2, b=0.75)
        retriever.index(tokens([docs[i] for i in ids], True), show_progress=False)
        indexes[library] = (ids, retriever)

    def rank(q: dict) -> Ranked:
        ids, retriever = indexes[q["stratum"] == "library"]
        query = tokens([q["text"]], False)
        if not query or not query[0]:
            return [[]], None
        docs, scores = retriever.retrieve(query, k=len(ids), show_progress=False)
        return [restrict(ds, q, [ids[int(d)] for d, s in zip(docs[0], scores[0]) if s > 0])], None

    return rank


def bilbo_arm(ds: Dataset, sb: Sandbox):
    """`bilbo recall --limit 100` in the sandbox; keyword-only or hybrid is whatever config the sandbox holds."""

    def rank(q: dict) -> Ranked:
        library = q["stratum"] == "library"
        args = ["recall", "--limit", str(LIMIT)] + (["--library"] if library else ["--kind", q["kind"]] if q["kind"] else [])
        p = bilbo(sb, [*args, "--", q["text"]])
        ranking: list[str] = []
        for line in p.stdout.splitlines():
            m = (SOURCE_HEADER if library else NOTE_HEADER).match(line)
            if not m:
                continue
            ident = m["ref"] if library else sb.path_to_id.get(m["path"])
            if ident is None or ident not in (ds.library if library else ds.notes):
                raise Abort(f"bilbo recall printed {m['path']}, which maps to no dataset id (query {q['id']})")
            if ident not in ranking:
                ranking.append(ident)
        if p.exit == 0 and not ranking:
            raise Abort(f"bilbo recall exited 0 but printed no hit header (query {q['id']}): its output format changed")
        return [ranking[:LIMIT]], p.ms

    return rank


# --- scoring ---------------------------------------------------------------------------------------------------------


def score(ds: Dataset, rankings: dict[str, list[str]]) -> dict[str, dict[str, float]]:
    """Every queried id gets every metric; an absent or empty ranking scores 0."""
    qrels = [Qrel(qid, d, g) for qid in rankings for d, g in ds.qrels.get(qid, {}).items()]
    run = [ScoredDoc(qid, d, float(1000 - r)) for qid, docs in rankings.items() for r, d in enumerate(docs, 1)]
    out = {qid: {m: 0.0 for m in METRICS} for qid in rankings}
    names = {str(measure): key for key, measure in METRICS.items()}
    for m in ir_measures.iter_calc(list(METRICS.values()), qrels, run):
        out[m.query_id][names[str(m.measure)]] = float(m.value)
    for qid, docs in rankings.items():
        if not docs:
            del out[qid]["judged@10"]  # judged@10 is of what came back: a silent arm must not read as "unjudged"
    return out


def extras(ds: Dataset, q: dict, ranking: list[str]) -> dict[str, float]:
    """evidence@10 for multi-hop, new-above-old for supersession, and an empty answer for no-answer."""
    if q["stratum"] == "multi-hop":
        top = set(ranking[:10])
        return {"evidence@10": float(any(ev and set(ev) <= top for ev in q["evidence_sets"]))}
    if q["stratum"] == "supersession":
        pos = {d: i for i, d in enumerate(ranking)}
        gold = min((pos[g] for g in positives(ds, q["id"]) if g in pos), default=None)
        return {"new_above_old": float(gold is not None and all(d not in pos or pos[d] > gold for d in q["decoys"]))}
    return {"empty": float(not ranking)} if q["stratum"] == "no-answer" else {}


def evaluate(ds: Dataset, queries: list[dict], rank) -> tuple[dict[str, dict[str, float]], list[float]]:
    """Per-query metrics (a mean over trials) and the latencies of the timed calls."""
    got = {q["id"]: rank(q) for q in queries}
    trials = max(len(r) for r, _ in got.values())
    scored = [q for q in queries if q["stratum"] != "no-answer"]
    sums: dict[str, Counter] = {q["id"]: Counter() for q in queries}
    for t in range(trials):
        pick = lambda q: got[q["id"]][0][min(t, len(got[q["id"]][0]) - 1)]
        for qid, metrics in score(ds, {q["id"]: pick(q) for q in scored}).items():
            sums[qid].update(metrics)
        for q in queries:
            sums[q["id"]].update(extras(ds, q, pick(q)))
    metrics = {qid: {k: round(v / trials, 4) for k, v in c.items()} for qid, c in sums.items()}
    return metrics, [ms for _, ms in got.values() if ms is not None]


def check_oracle(ds: Dataset, queries: list[dict], metrics: dict[str, dict[str, float]]) -> None:
    """The oracle ranks exactly the labelled notes, so every metric it reports must be 1: it proves qrels, ids and scoring."""
    bad = []
    for q in queries:
        for k, v in metrics[q["id"]].items():
            if v != 1.0 and not (k == "r@10" and len(positives(ds, q["id"])) > 10):
                bad.append(f"{q['id']} {k}={v}")
    if bad:
        raise Abort(f"the oracle arm does not score 1.0: {', '.join(bad[:8])}")


def mean_by_stratum(queries: list[dict], metrics: dict[str, dict[str, float]]) -> dict[str, dict]:
    groups: dict[str, list[dict]] = defaultdict(list)
    for q in queries:
        groups[q["stratum"]].append(q)
        if q["stratum"] != "no-answer":
            groups["all"].append(q)
    out = {}
    for name, qs in groups.items():
        keys = sorted({k for q in qs for k in metrics[q["id"]] if name != "all" or k in METRICS})
        out[name] = {"n": len(qs)} | {k: round(float(np.mean(v)), 4) for k in keys if (v := [metrics[q["id"]][k] for q in qs if k in metrics[q["id"]]])}
    return out


# --- stats -----------------------------------------------------------------------------------------------------------


def bootstrap_ci(a: np.ndarray, b: np.ndarray, clusters: list[str], n: int = 10000, seed: int = 0) -> tuple[float, float, float]:
    """(mean a-b, lo, hi): a 95% percentile interval of a paired bootstrap that resamples whole fact families."""
    diffs = np.asarray(a, dtype=float) - np.asarray(b, dtype=float)
    if len(diffs) == 0:
        return 0.0, 0.0, 0.0
    ids: dict[str, int] = {}
    idx = np.array([ids.setdefault(c, len(ids)) for c in clusters], dtype=np.int64)
    sums, counts = np.bincount(idx, weights=diffs), np.bincount(idx).astype(float)
    rng, c = np.random.default_rng(seed), len(sums)
    means = np.empty(n)
    for start in range(0, n, 1000):
        pick = rng.integers(0, c, size=(min(1000, n - start), c))
        means[start:start + len(pick)] = sums[pick].sum(axis=1) / counts[pick].sum(axis=1)
    lo, hi = np.percentile(means, [2.5, 97.5])
    return float(diffs.mean()), float(lo), float(hi)


def paired(qs: list[dict], a: dict, b: dict, clusters: dict[str, str]) -> dict:
    if len(set(clusters[q["id"]] for q in qs)) < MIN_FAMILIES:
        return {"n": len(qs), "diff": round(float(np.mean([a[q["id"]] - b[q["id"]] for q in qs])), 4), "lo": None, "hi": None}
    d, lo, hi = bootstrap_ci(np.array([a[q["id"]] for q in qs]), np.array([b[q["id"]] for q in qs]), [clusters[q["id"]] for q in qs])
    return {"n": len(qs), "diff": round(d, 4), "lo": round(lo, 4), "hi": round(hi, 4)}


def signflip(qs: list[dict], a: dict, b: dict, clusters: dict[str, str]) -> dict:
    """The preregistered test: paired sign-flip permutation over fact families, two-sided on the mean difference."""
    by_family: dict[str, float] = defaultdict(float)
    for q in qs:
        by_family[clusters[q["id"]]] += a[q["id"]] - b[q["id"]]
    sums = np.array([by_family[k] for k in sorted(by_family)])
    k, observed = len(sums), abs(sums.sum())
    if k <= EXACT_FAMILIES:
        signs = ((np.arange(2**k)[:, None] >> np.arange(k)) & 1) * 2 - 1
        return {"p": round(float(np.mean(np.abs(signs @ sums) >= observed - 1e-9)), 6), "families": k, "draws": None}
    rng, hits, draws = np.random.default_rng(0), 0, 100000
    for _ in range(draws // 10000):
        flips = rng.integers(0, 2, size=(10000, k), dtype=np.int8) * 2 - 1
        hits += int(np.sum(np.abs(flips @ sums) >= observed - 1e-9))
    return {"p": round((hits + 1) / (draws + 1), 6), "families": k, "draws": draws}


def strata_of(queries: list[dict]) -> dict[str, list[dict]]:
    scored = [q for q in queries if q["stratum"] != "no-answer"]
    return {"all": scored} | {s: [q for q in scored if q["stratum"] == s] for s in STRATA if s != "no-answer"}


# --- digest ----------------------------------------------------------------------------------------------------------


def digest_rows(sb: Sandbox) -> list[dict]:
    p = sb.state / "digest.jsonl"
    return [json.loads(l) for l in p.read_text(encoding="utf-8").splitlines() if l.strip()] if p.is_file() else []


def digest_config(sb: Sandbox, prompts: list[dict], url: str | None) -> dict[str, dict]:
    extra = {"digest.log": "on"} | ({"digest.min_similarity": f"{THRESHOLD:g}"} if url else {})
    write_config(sb, url, extra)
    out = {}
    for pr in prompts:
        before = len(digest_rows(sb))
        p = bilbo(sb, ["digest"], stdin=json.dumps({"session_id": str(uuid.uuid4()), "prompt": pr["prompt"]}))
        rows = digest_rows(sb)
        if len(rows) != before + 1:
            raise Abort(f"bilbo digest wrote no digest.jsonl row for {pr['id']}")
        if rows[-1].get("error"):
            raise Abort(f"bilbo digest reported an error for {pr['id']}: {rows[-1]['error']}")
        want, got = "meaning" if url else "keywords", rows[-1].get("ranking")
        if got in ("meaning", "keywords") and got != want:
            raise Abort(f"bilbo digest ranked by {got} where the config asked for {want} (prompt {pr['id']})")
        unknown = [x for x in rows[-1]["shown"] if x not in sb.path_to_id]
        if unknown:
            raise Abort(f"bilbo digest showed {unknown[0]}, which maps to no dataset id (prompt {pr['id']})")
        shown = [sb.path_to_id[x] for x in rows[-1]["shown"]]
        out[pr["id"]] = {"injected": bool(shown), "hit": bool(set(shown) & set(pr["gold"])) if shown and pr["label"] == "positive" else None}
    return out


def digest_summary(prompts: list[dict], rows: dict[str, dict]) -> dict:
    by = {label: [rows[p["id"]] for p in prompts if p["label"] == label] for label in ("positive", *NEGATIVE)}
    share = lambda k, n: round(k / n, 4) if n else None
    shown = [r for r in by["positive"] if r["injected"]]
    return {
        "fir": {k: share(sum(r["injected"] for r in by[k]), len(by[k])) for k in NEGATIVE},
        "coverage": share(len(shown), len(by["positive"])),
        "hit_given_inject": share(sum(bool(r["hit"]) for r in shown), len(shown)),
        "n": {k: len(v) for k, v in by.items()},
    }


# --- report, results.json, diff --------------------------------------------------------------------------------------


def table(header: list[str], body: list[list[str]]) -> list[str]:
    return ["| " + " | ".join(header) + " |", "|" + "|".join(["---"] * len(header)) + "|"] + ["| " + " | ".join(r) + " |" for r in body]


def interval(c: dict) -> str:
    if c["lo"] is None:
        return f"{c['diff']:+.3f} [few families]"
    return f"{c['diff']:+.3f} [{c['lo']:+.3f}, {c['hi']:+.3f}]" + (" *" if c["lo"] > 0 or c["hi"] < 0 else "")


def primary_line(p: dict) -> str:
    how = "exact" if p["draws"] is None else f"{p['draws']} draws"
    return f"p = {p['p']:.4g} ({how}, {p['families']} families); minimum effect {p['min_effect']:.2f} {'met' if abs(p['diff']) >= p['min_effect'] else 'not met'}"


def report(res: dict) -> str:
    arms, vs = res["arms"], res["vs"]
    lines = [f"# L1 retrieval: {res['dataset']['name']} {res['split']}, bilbo {res['bilbo']['version']}", ""]
    if res["bilbo"]["build"] == "debug":
        lines += ["WARNING: bilbo is a debug build; its latencies are about 6x a release build and are not for a baseline.", ""]
    if "bm25" in vs and "all" in vs["bm25"]:
        lines += [f"Headline, success@5 over all {vs['bm25']['all']['n']} scored queries: bilbo-full minus bm25 = {interval(vs['bm25']['all'])}",
                  "", f"Preregistered test (paired sign-flip over fact families): {primary_line(res['primary'])}" if "primary" in res else "", "",
                  "`*` marks an interval that excludes 0, uncorrected across strata. Everything below is secondary.", ""]  # fmt: skip
    strata = [s for s in ["all", *STRATA] if any(s in a["by_stratum"] for a in arms.values())]
    lines += ["## success@5 by stratum", ""] + table(["arm", *strata], [
        [name, *[f"{a['by_stratum'][s]['success@5']:.3f}" if s in a["by_stratum"] and "success@5" in a["by_stratum"][s] else "-" for s in strata]]
        for name, a in arms.items()])  # fmt: skip
    lines += ["", "## All scored queries", ""] + table(["arm", *METRICS, "p50 ms", "p95 ms"], [
        [name, *[f"{a['by_stratum']['all'].get(m, float('nan')):.3f}" for m in METRICS], *[str(a["latency_ms"][k]) if a.get("latency_ms") else "-" for k in ("p50", "p95")]]
        for name, a in arms.items()])  # fmt: skip
    ex = [(name, s, k, v) for name, a in arms.items() for s, c in a["by_stratum"].items() for k, v in c.items() if k in ("evidence@10", "new_above_old", "empty")]
    if ex:
        lines += ["", "## Per-stratum metrics", ""] + table(["arm", "stratum", "metric", "value"], [[n, s, k, f"{v:.3f}"] for n, s, k, v in ex])
    for other, by in vs.items():
        lines += ["", f"## bilbo-full minus {other}, success@5, 95% interval over fact families", ""]
        lines += table(["stratum", "n", "diff"], [[s, str(c["n"]), interval(c)] for s, c in by.items()])
    for name, d in (res.get("digest") or {}).items():
        lines += ["", f"## digest, {name}", "", f"prompts {d['n']}; false injection {d['fir']}; coverage {d['coverage']}; hit given inject {d['hit_given_inject']}"]
    if res["index"]:
        lines += ["", f"index: embedded {res['index']['embedded']}, second pass embedded {res['index']['second_pass_embedded']}"]
    return "\n".join(lines)


def dump(res: dict) -> str:
    """JSON with the bulky per-item sections written one entry per line."""
    text = json.dumps({k: v for k, v in res.items() if k not in BIG}, indent=2, sort_keys=True, ensure_ascii=False)
    for key in BIG:
        if key in res:
            rows = ",\n".join(f"    {json.dumps(k)}: {json.dumps(v, sort_keys=True, ensure_ascii=False)}" for k, v in sorted(res[key].items()))
            text = text[:-2] + f',\n  "{key}": {{\n{rows}\n  }}\n}}'
    return text + "\n"


def diff(old: dict, new: dict) -> str:
    """success@5 of every shared arm and stratum, new minus old, paired by query with an interval over fact families."""
    if old["split"] != new["split"]:
        raise Abort(f"the runs differ in split: {old['split']} and {new['split']}")
    if old["dataset"]["sha256"] != new["dataset"]["sha256"]:
        raise Abort("the runs used different datasets; their scores do not compare")
    lines = [f"# diff: bilbo {old['bilbo']['version']} -> {new['bilbo']['version']}, {new['dataset']['name']} {new['split']}"]
    for what in ("embedder", "host"):
        if old[what] != new[what]:
            lines += [f"note: {what} differs: {old[what]} -> {new[what]}"]
    queries = new["queries"]
    body = []
    for arm in [a for a in new["arms"] if a in old["arms"]]:
        both = [qid for qid in new["per_query"] if arm in new["per_query"][qid] and arm in old["per_query"].get(qid, {})]
        for s in ["all", *STRATA]:
            qs = [{"id": qid, "stratum": queries[qid]["stratum"]} for qid in both if s in ("all", queries[qid]["stratum"]) and queries[qid]["stratum"] != "no-answer"]
            if qs:
                a, b = ({q["id"]: r["per_query"][q["id"]][arm]["success@5"] for q in qs} for r in (new, old))
                fam = {k: v["family"] for k, v in queries.items()}
                c = paired(qs, a, b, fam)
                body.append([arm, s, str(c["n"]), interval(c), f"{signflip(qs, a, b, fam)['p']:.4g}" if s == "all" else "-"])
    lines += ["", "## success@5, new minus old", ""] + table(["arm", "stratum", "n", "diff", "sign-flip p"], body)
    for name, r in (("old", old), ("new", new)):
        if "primary" in r:
            lines += ["", f"primary, {name}: bilbo-full minus bm25 = {r['primary']['diff']:+.3f}, {primary_line(r['primary'])}"]
    for cfg in [c for c in (new.get("digest") or {}) if c in (old.get("digest") or {})]:
        o, n = old["digest"][cfg], new["digest"][cfg]
        lines += ["", f"digest {cfg}: coverage {o['coverage']} -> {n['coverage']}, hit given inject {o['hit_given_inject']} -> {n['hit_given_inject']}, "
                  f"false injection {o['fir']} -> {n['fir']}"]  # fmt: skip
    return "\n".join(lines)


# --- run -------------------------------------------------------------------------------------------------------------


def cpu() -> str:
    if sys.platform == "darwin":
        p = subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True)
        if p.stdout.strip():
            return p.stdout.strip()
    try:
        return next(l.split(":", 1)[1].strip() for l in Path("/proc/cpuinfo").read_text().splitlines() if l.startswith("model name"))
    except (OSError, StopIteration):
        return platform.processor() or platform.machine()


def bilbo_identity(sb: Sandbox) -> dict:
    p = sh([str(sb.exe), "--version"], sb.env, cwd=sb.root, timeout=60)
    if p.exit != 0 or not p.stdout.split():
        raise Abort(f"`bilbo --version` failed: {p.stderr.strip()}")
    return {"version": p.stdout.split()[-1], "sha256": sha256_file(sb.exe), "build": "debug" if "debug" in sb.exe.parts else "release"}


def cmd_run(a: argparse.Namespace) -> int:
    names = ARMS if a.arms == "all" else [n.strip() for n in a.arms.split(",") if n.strip()]
    if unknown := [n for n in names if n not in ARMS]:
        raise Abort(f"no arm named {unknown[0]!r}; the arms are {', '.join(ARMS)}")
    exe = Path(a.bilbo).resolve()
    if not exe.is_file():
        raise Abort(f"no bilbo binary at {exe}")
    ds = load_dataset(Path(a.dataset))
    queries = [q for q in ds.queries if q["split"] == a.split]
    prompts = [p for p in ds.prompts if p["split"] == a.split]
    if not queries:
        raise Abort(f"the {a.split} split has no queries")
    base = json.loads(Path(a.baseline).read_text(encoding="utf-8")) if a.baseline else None
    if base and (base["split"], base["dataset"]["sha256"]) != (a.split, ds.sha):
        raise Abort(f"{a.baseline} is of another split or dataset than this run; nothing was run")
    sb = server = None
    try:
        sb = make_sandbox(ds, exe)
        res: dict = {"schema": 1, "dataset": {"name": ds.name, "sha256": ds.sha}, "split": a.split, "bilbo": bilbo_identity(sb),
                     "embedder": None, "host": {"platform": platform.platform(), "cpu": cpu()}, "index": None, "arms": {}, "vs": {},
                     "digest": {}, "queries": {q["id"]: {"stratum": q["stratum"], "family": q["family"]} for q in queries},
                     "per_query": defaultdict(dict), "per_prompt": defaultdict(dict)}  # fmt: skip
        if "bilbo-full" in names:
            server = start_server(Path(a.model), a.llama_server)
            res["embedder"] = server.info
        metrics: dict[str, dict] = {}
        for name in dict.fromkeys(["oracle", *(n for n in ARMS if n in names)]):
            if name == "bilbo-full":
                res["index"] = index(sb, server.url)
            if name == "bilbo-keyword":
                write_config(sb, None)
            rank = {"oracle": lambda: oracle_arm(ds), "random": lambda: random_arm(ds), "ripgrep": lambda: ripgrep_arm(ds, sb),
                    "bm25": lambda: bm25_arm(ds), "bilbo-keyword": lambda: bilbo_arm(ds, sb),
                    "bilbo-full": lambda: bilbo_arm(ds, sb)}[name]()  # fmt: skip
            metrics[name], ms = evaluate(ds, queries, rank)
            if name == "oracle":
                check_oracle(ds, queries, metrics[name])
            if name not in names:
                continue
            res["arms"][name] = {"by_stratum": mean_by_stratum(queries, metrics[name])}
            if name in PROCESS_ARMS:
                res["arms"][name]["latency_ms"] = {"p50": round(float(np.percentile(ms, 50))), "p95": round(float(np.percentile(ms, 95)))}
            for qid, m in metrics[name].items():
                res["per_query"][qid][name] = m
        if "bilbo-full" in metrics:
            clusters = {q["id"]: q["family"] for q in queries}
            for other in [n for n in ("bm25", "bilbo-keyword") if n in metrics]:
                res["vs"][other] = {s: paired(qs, {q["id"]: metrics["bilbo-full"][q["id"]]["success@5"] for q in qs},
                                               {q["id"]: metrics[other][q["id"]]["success@5"] for q in qs}, clusters)
                                    for s, qs in strata_of(queries).items() if qs}  # fmt: skip
            if "bm25" in metrics:
                qs = strata_of(queries)["all"]
                ours, theirs = ({q["id"]: metrics[n][q["id"]]["success@5"] for q in qs} for n in ("bilbo-full", "bm25"))
                res["primary"] = {"comparison": "bilbo-full minus bm25", "metric": "success@5", "min_effect": MIN_EFFECT,
                                  "diff": res["vs"]["bm25"]["all"]["diff"], "n": len(qs), **signflip(qs, ours, theirs, clusters)}  # fmt: skip
        if prompts and not a.no_digest:
            configs = ([f"{THRESHOLD:g}"] if server else []) + ["keywords"]
            for cfg in configs:
                rows = digest_config(sb, prompts, server.url if cfg != "keywords" else None)
                res["digest"][cfg] = digest_summary(prompts, rows)
                for pid, r in rows.items():
                    res["per_prompt"][pid][cfg] = r
        text = dump(res)
        for secret in {str(sb.root), os.path.realpath(sb.root), os.path.expanduser("~"), os.environ.get("HOME", "\0")}:
            if secret in text:
                raise Abort(f"the results contain {secret}; refusing to write them")
        Path(a.out).write_text(text, encoding="utf-8")
        print(report(res))
        if base:
            print("\n" + diff(base, json.loads(text)))
    finally:
        if server:
            server.stop()
        if sb and a.keep_root:
            print(f"eval: kept {sb.root}", file=sys.stderr)
        elif sb:
            shutil.rmtree(sb.root, ignore_errors=True)
    return 0


def cmd_diff(a: argparse.Namespace) -> int:
    print(diff(*(json.loads(Path(p).read_text(encoding="utf-8")) for p in (a.old, a.new))))
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(prog="eval.py", description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run", help="rank a split with the arms, score it, write results.json")
    r.add_argument("--dataset", default=str(HERE / "dataset"))
    r.add_argument("--bilbo", required=True, help="absolute path of the bilbo binary")
    r.add_argument("--split", choices=["dev", "test"], default="dev", help="tune on dev, run test once per release")
    r.add_argument("--arms", default="all", help=f"`all` or a comma list of {', '.join(ARMS)}")
    r.add_argument("--model", default=str(Path.home() / ".cache/bilbo/models" / GGUF_FILE), help="the pinned embedding GGUF")
    r.add_argument("--llama-server", default="llama-server")
    r.add_argument("--out", default="results.json")
    r.add_argument("--baseline", help="a results.json to diff the run against")
    r.add_argument("--no-digest", action="store_true")
    r.add_argument("--keep-root", action="store_true", help="keep the temp root to inspect")
    r.set_defaults(fn=cmd_run)
    d = sub.add_parser("diff", help="success@5 of two results.json, paired by query")
    d.add_argument("old")
    d.add_argument("new")
    d.set_defaults(fn=cmd_diff)
    args = ap.parse_args()
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))  # unwinds through the finally blocks and atexit
    try:
        return args.fn(args)
    except Abort as e:
        print(f"eval: aborted: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
