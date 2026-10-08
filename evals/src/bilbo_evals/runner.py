"""`l1 run`: one sandbox, one embedder, every requested arm ranked over a split and written as a run folder."""

from __future__ import annotations

import argparse
import importlib.metadata
import platform
import subprocess
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path

from bilbo_evals import arms as arms_pkg
from bilbo_evals import common, dataset, embedder, results, retrieval, sandbox
from bilbo_evals.arms import ARMS, PROCESS_ARMS, Context, Result
from bilbo_evals.common import Refused, UsageError, err, out, tool_version

EMBEDDER_ARMS = {"dense-ref", "bilbo-full"}
NO_ANSWER = "no-answer"


def parse_arms(spec: str) -> list[str]:
    if spec == "all":
        return list(ARMS)
    names = [n.strip() for n in spec.split(",") if n.strip()]
    if not names:
        raise UsageError("--arms needs `all` or a comma list of arms")
    unknown = [n for n in names if n not in ARMS]
    if unknown:
        raise UsageError(f"unknown arm {unknown[0]!r}; the arms are {', '.join(ARMS)}")
    return [n for n in ARMS if n in names]


def _stamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def bilbo_identity(exe: Path) -> dict:
    """Version, commit, SHA-256 and a path that holds no user name: relative to the checkout, else the basename."""
    exe = Path(exe).resolve()
    line = tool_version([str(exe)])
    version = line.split()[-1]
    commit, shown = None, exe.name
    try:
        rel = exe.relative_to(common.REPO_ROOT.resolve())
    except ValueError:
        rel = None
    if rel is not None:
        shown = rel.as_posix()
        p = subprocess.run(["git", "-C", str(common.REPO_ROOT), "rev-parse", "HEAD"], capture_output=True, text=True)
        commit = p.stdout.strip() or None if p.returncode == 0 else None
    return {"version": version, "commit": commit, "sha256": common.sha256_file(exe), "path": shown}


def _cpu() -> str:
    if sys.platform == "darwin":
        p = subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True)
        if p.returncode == 0 and p.stdout.strip():
            return p.stdout.strip()
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or platform.machine()


def host() -> dict:
    return {"platform": platform.platform(), "machine": platform.machine(), "cpu": _cpu()}


def _dist(name: str) -> str | None:
    try:
        return importlib.metadata.version(name)
    except importlib.metadata.PackageNotFoundError:
        return None


def versions(server: embedder.Server | None) -> dict:
    return {
        "python": platform.python_version(),
        "bilbo_evals": _dist("bilbo-evals"),
        "ir_measures": _dist("ir-measures"),
        "bm25s": _dist("bm25s"),
        "PyStemmer": _dist("PyStemmer"),
        "numpy": _dist("numpy"),
        "ripgrep": None,
        "llama_server": server.version if server is not None else None,
    }


def embedder_record(server: embedder.Server) -> dict:
    rec = embedder.record(server)
    rec["llama_server"] = Path(rec["llama_server"]).name
    return rec


class Session:
    """The sandbox, the copy of the store, the embedder and the arms' shared context for one run."""

    def __init__(self, ds: dataset.Dataset, split: str, bilbo: Path, gguf: Path | None, llama_server: str | None,
                 server_url: str | None, need_embedder: bool, tag: str, keep_root: bool = False) -> None:  # fmt: skip
        self.ds, self.split, self.bilbo = ds, split, Path(bilbo)
        self.gguf, self.llama_server, self.server_url = gguf, llama_server or "llama-server", server_url
        self.need_embedder, self.tag, self.keep_root = need_embedder, tag, keep_root
        self.sb: sandbox.Sandbox | None = None
        self.server: embedder.Server | None = None
        self.ctx: Context | None = None
        self.folders: dict[str, str] = {}

    def __enter__(self) -> "Session":
        self.sb = sandbox.create(self.tag)
        try:
            self.folders = sandbox.guard(self.sb)
            mapping = dataset.materialize(self.ds, self.sb.store)
            sandbox.write_config(self.sb, None)
            if self.need_embedder:
                if self.server_url:
                    self.server = embedder.Server.attach(self.server_url)
                else:
                    self.server = embedder.Server.start(self.gguf or embedder.default_gguf(), self.llama_server)
                sandbox.write_config(self.sb, self.server.url)
            self.ctx = Context(self.ds, self.split, self.sb, self.bilbo, self.server, mapping)
        except BaseException:
            self.__exit__(None, None, None)
            raise
        return self

    def __exit__(self, *exc) -> None:
        if self.server is not None:
            self.server.stop()
        if self.sb is not None and not self.keep_root:
            sandbox.destroy(self.sb)


def _scored(items: list[dict], trials: list[dict[str, Result]], ds_dir: Path, split: str) -> list[dict[str, dict]]:
    """Per trial, retrieval scores for every item that has qrels (no-answer items have none)."""
    scores: list[dict[str, dict]] = [{} for _ in trials]
    for library in (False, True):
        qs = [q for q in items if (q["stratum"] == "library") == library and q["stratum"] != NO_ANSWER]
        if not qs:
            continue
        qrels = retrieval.read_qrels(ds_dir, split, library)
        for t, got in enumerate(trials):
            scores[t].update(retrieval.score({q["id"]: got[q["id"]].ranking for q in qs}, qrels))
    return scores


def build_rows(arm_name: str, items: list[dict], raw: dict[str, list[Result]], ds_dir: Path, split: str) -> list[dict]:
    n_trials = max((len(r) for r in raw.values()), default=1)
    trials = [{i: r[min(t, len(r) - 1)] for i, r in raw.items()} for t in range(n_trials)]
    scores = _scored(items, trials, ds_dir, split)
    random_arm = arm_name == "random"
    rows: list[dict] = []
    for q in items:
        for t, got in enumerate(trials):
            res = got[q["id"]]
            extra = retrieval.extras(q, res.ranking)
            if q["stratum"] == NO_ANSWER:
                metrics = {"empty": extra["empty"]}
            else:
                metrics = dict(scores[t][q["id"]])
                metrics.update({"evidence@10": None, "new_above_old": None, "empty": extra["empty"]})
                metrics.update(extra)
            rows.append({
                "item": q["id"], "stratum": q["stratum"], "split": q["split"], "trial": t,
                "ranking": None if random_arm else res.ranking, "metrics": metrics,
                "latency_ms": res.latency_ms if arm_name in PROCESS_ARMS else None, "exit": res.exit,
                "warnings": res.warnings, "fallback": res.fallback, "error": res.error,
                "tokens_in": None, "tokens_out": None, "cost_usd": None,
            })  # fmt: skip
    return rows


def _rank_items(arm, items: list[dict], ctx: Context) -> dict[str, list[Result]]:
    raw: dict[str, list[Result]] = {}
    for q in items:
        r = arm.rank(q, ctx)
        raw[q["id"]] = r if isinstance(r, list) else [r]
    return raw


def dataset_path(ds_dir: Path) -> str:
    """The dataset folder relative to evals/, POSIX form; a run on a folder outside evals/ is refused."""
    try:
        return Path(ds_dir).resolve().relative_to(common.EVALS_ROOT.resolve()).as_posix()
    except ValueError:
        raise Refused("a run's dataset must be inside evals/") from None


def _folders(session: Session) -> dict[str, str]:
    return {k: session.folders[k] for k in ("store", "config", "cache", "state")}


def cmd_run(args: argparse.Namespace) -> int:
    split, draft, ds_dir = args.split, bool(args.draft), Path(args.dataset)
    names = parse_arms(args.arms)
    rel_path = dataset_path(ds_dir)
    tree_hash = dataset.require_ready(ds_dir, draft, split)
    ds = dataset.load(ds_dir)
    items = ds.queries_for(split)
    if not items:
        raise Refused(f"the {split} split has no queries")
    run_id = args.run_id or f"{_stamp()}-{split}{'-draft' if draft else ''}"
    run = results.run_dir(run_id)
    if run.exists():
        raise Refused(f"{run} exists already; pick another run id")
    bilbo_exe = Path(args.bilbo)
    if not bilbo_exe.is_file():
        raise Refused(f"no bilbo binary at {bilbo_exe}")
    identity = bilbo_identity(bilbo_exe)
    refused: list[str] = []
    started = common.now()
    with Session(ds, split, bilbo_exe, args.model, args.llama_server, getattr(args, "embedder_url", None),
                 bool(EMBEDDER_ARMS & set(names)), run_id, args.keep_root) as session:  # fmt: skip
        ctx = session.ctx
        tool_versions = versions(session.server)
        for name in names:
            arm = arms_pkg.get(name)
            try:
                entry = arm.prepare(ctx)
            except Refused as e:
                err(f"{name} not scored: {e}")
                refused.append(name)
                continue
            arm_started = common.now()
            raw = _rank_items(arm, items, ctx)
            rows = build_rows(name, items, raw, ds_dir, split)
            trec = None
            if name != "random":
                trec = {q["id"]: raw[q["id"]][0].ranking for q in items if q["stratum"] != NO_ANSWER}
            meta = {
                "schema_version": 1, "run_id": run_id, "layer": "L1", "arm": name, "split": split, "draft": draft,
                "dataset": {"name": ds.name, "version": ds.version, "tree_hash": tree_hash, "path": rel_path}, "bilbo": identity,
                "embedder": embedder_record(session.server) if entry.get("embedder") else None,
                "bilbo_config": entry.get("bilbo_config", {}), "parity": entry.get("parity"), "index": entry.get("index"),
                "versions": {**tool_versions, **entry.get("versions", {})},
                "seeds": list(ctx.seeds) if name == "random" else [], "host": host(),
                "root": str(session.sb.root), "folders": _folders(session),
                "started": arm_started, "ended": common.now(),
            }  # fmt: skip
            results.write_arm(run, name, sandbox.portable(session.sb, meta), sandbox.portable(session.sb, rows), trec)
    if not run.exists():
        raise Refused("no arm was scored")
    if split == "test" and not draft:
        results.log_test_run(run_id, tree_hash, identity["version"])
    text = results.report(run)
    (run / "report.md").write_text(text + "\n", encoding="utf-8")
    out(text)
    return 1 if refused else 0


def rank_all(ds_dir: Path, split: str, arm_names: list[str], bilbo: Path, gguf: Path | None, llama_server: str | None,
             server_url: str | None = None, hits: dict | None = None) -> dict[str, dict[str, list[str]]]:  # fmt: skip
    """Top-100 ranking of every query and digest prompt of a split per arm (random: seed 0); nothing is written.

    When `hits` is given it is filled with `hits[arm][item] = {note id: file line of the passage that ranked it}`.
    """
    ds = dataset.load(Path(ds_dir))
    items = list(ds.queries_for(split))
    items += [
        {"id": p["id"], "text": p["prompt"], "stratum": "prompt", "split": p["split"], "kind": None}
        for p in ds.prompts if split == "all" or p["split"] == split
    ]  # fmt: skip
    names = [n for n in ARMS if n in arm_names]
    tag = f"rank-{uuid.uuid4().hex[:8]}"
    with Session(ds, split, bilbo, gguf, llama_server, server_url, bool(EMBEDDER_ARMS & set(names)), tag) as session:
        found: dict[str, dict[str, list[str]]] = {}
        failed: list[str] = []
        for name in names:
            arm = arms_pkg.get(name)
            arm.prepare(session.ctx)
            ranked: dict[str, list[str]] = {}
            for q in items:
                r = arm.rank(q, session.ctx)
                got = r if isinstance(r, list) else [r]
                for res in got:
                    if res.error or res.fallback:
                        failed.append(f"{name} {q['id']}: {res.error or 'fallback'}")
                        break
                ranked[q["id"]] = got[0].ranking
                if hits is not None and got[0].lines:
                    hits.setdefault(name, {})[q["id"]] = got[0].lines
            found[name] = ranked
    if failed:
        shown = "\n".join(failed[:10])
        raise Refused(f"{len(failed)} ranking(s) failed or fell back to keywords; pooling needs none:\n{shown}")
    return found
