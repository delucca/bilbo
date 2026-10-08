"""`l1 digest`: run `bilbo digest` on every prompt of a split and summarise what it injected."""

from __future__ import annotations

import argparse
import json
import re
import uuid
from collections import defaultdict
from pathlib import Path

from bilbo_evals import common, dataset, embedder, results, runner, sandbox
from bilbo_evals.common import Refused, out

THRESHOLD = 0.55
SWEEP = [round(0.30 + i * 0.025, 3) for i in range(21)]
NEGATIVE = ("noise", "off-topic", "near-miss")
SHOWN_LINE = re.compile(r"^- (?P<path>.+?):\d+ \(")


def config_name(threshold: float | None) -> str:
    return "keywords" if threshold is None else f"meaning@{threshold:g}"


def _log_rows(sb: sandbox.Sandbox) -> list[dict]:
    path = sb.state / "bilbo" / "digest.jsonl"
    if not path.is_file():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def run_prompt(sb: sandbox.Sandbox, exe: Path, prompt: dict, config: str, path_to_id: dict[str, str]) -> dict:
    before = len(_log_rows(sb))
    stdin = json.dumps({"session_id": str(uuid.uuid4()), "prompt": prompt["prompt"]})
    p = sandbox.bilbo(sb, exe, ["digest"], stdin=stdin)
    logged = _log_rows(sb)
    entry = logged[-1] if len(logged) > before else None
    paths = entry["shown"] if entry else [m["path"] for m in map(SHOWN_LINE.match, p.stdout.splitlines()) if m]
    shown = [path_to_id.get(x, x) for x in paths]
    error = entry.get("error") if entry else (p.stderr.strip() or "no digest.jsonl row")
    injected = bool(shown)
    hit = bool(set(shown) & set(prompt["gold"])) if prompt["label"] == "positive" and injected else None
    return {
        "item": prompt["id"], "label": prompt["label"], "split": prompt["split"], "config": config,
        "injected": injected, "shown": shown, "hit": hit, "ranking": entry["ranking"] if entry else None,
        "passed": entry["passed"] if entry else None, "error": error, "latency_ms": p.ms,
    }  # fmt: skip


def run_config(sess: runner.Session, prompts: list[dict], threshold: float | None) -> list[dict]:
    extra = {"digest.log": "on"}
    if threshold is None:
        sandbox.write_config(sess.sb, None, extra)
    else:
        sandbox.write_config(sess.sb, sess.server.url, {**extra, "digest.min_similarity": f"{threshold:g}"})
    name = config_name(threshold)
    return [run_prompt(sess.sb, sess.bilbo, p, name, sess.ctx.path_to_id) for p in prompts]


def stratum(row: dict) -> str:
    ranking = row["ranking"] or "unlogged"
    return f"{ranking}, error" if row["error"] else ranking


def summarize(rows: list[dict]) -> dict:
    by_label: dict[str, list[dict]] = defaultdict(list)
    for r in rows:
        by_label[r["label"]].append(r)
    share = lambda hit, n: hit / n if n else None
    negatives = [r for label in NEGATIVE for r in by_label[label]]
    positives = by_label["positive"]
    shown = [r for r in positives if r["injected"]]
    return {
        "n": len(rows),
        "fir": {label: share(sum(r["injected"] for r in by_label[label]), len(by_label[label])) for label in NEGATIVE},
        "n_label": {label: len(by_label[label]) for label in ("positive", *NEGATIVE)},
        "fir_all": share(sum(r["injected"] for r in negatives), len(negatives)),
        "coverage": share(len(shown), len(positives)),
        "hit_given_inject": share(sum(bool(r["hit"]) for r in shown), len(shown)),
        "injected": sum(r["injected"] for r in rows),
    }


def by_stratum(rows: list[dict]) -> dict[str, dict]:
    groups: dict[str, list[dict]] = defaultdict(list)
    for r in rows:
        groups[stratum(r)].append(r)
    return {k: summarize(v) for k, v in sorted(groups.items())}


def _pct(v: float | None) -> str:
    return "-" if v is None else f"{v * 100:.1f}%"


def _table(header: list[str], body: list[list[str]]) -> list[str]:
    return ["| " + " | ".join(header) + " |", "|" + "|".join(["---"] * len(header)) + "|"] + [
        "| " + " | ".join(r) + " |" for r in body
    ]


def _rates(label: str, s: dict) -> list[str]:
    return [label, str(s["n"]), *[_pct(s["fir"][k]) for k in NEGATIVE], _pct(s["coverage"]), _pct(s["hit_given_inject"])]


RATE_HEADER = ["", "prompts", "false injection: noise", "off-topic", "near-miss", "coverage", "hit given inject"]


def section(digest: dict) -> list[str]:
    """Report lines for a loaded digest folder: {"run", "rows", "sweep"}."""
    rows = digest["rows"]
    configs = sorted({r["config"] for r in rows}, key=lambda c: (c == "keywords", c))
    lines = ["## Digest", ""]
    lines += _table(RATE_HEADER, [_rates(c, summarize([r for r in rows if r["config"] == c])) for c in configs])
    lines += ["", "False injection is the share of that label's prompts that got any digest; coverage is the share of "
              "positive prompts that got one; hit given inject is the share of those that list a gold note.", ""]
    lines += ["### By ranking and error", ""]
    body = []
    for c in configs:
        for name, s in by_stratum([r for r in rows if r["config"] == c]).items():
            body.append(_rates(f"{c}: {name}", s))
    lines += _table(RATE_HEADER, body)
    if digest.get("sweep"):
        lines += ["", "### Approximate operating curve", "",
                  "One indexed root, one pass per threshold; the points are approximate and `*` marks the "
                  f"default {THRESHOLD:g}.", ""]
        body = []
        for t in SWEEP:
            pts = [r for r in digest["sweep"] if r["config"] == config_name(t)]
            if not pts:
                continue
            s = summarize(pts)
            fell = sum(1 for r in pts if r["error"])
            mark = " *" if abs(t - THRESHOLD) < 1e-9 else ""
            body.append([f"{t:g}{mark}", _pct(s["fir_all"]), *[_pct(s["fir"][k]) for k in NEGATIVE], _pct(s["coverage"]),
                         _pct(s["hit_given_inject"]), str(fell)])  # fmt: skip
        lines += _table(["min_similarity", "false injection", "noise", "off-topic", "near-miss", "coverage",
                         "hit given inject", "prompts with an error"], body)  # fmt: skip
    return lines


def cmd(args: argparse.Namespace) -> int:
    split, draft, ds_dir = args.split, bool(args.draft), Path(args.dataset)
    rel_path = runner.dataset_path(ds_dir)
    tree_hash = dataset.require_ready(ds_dir, draft, split)
    ds = dataset.load(ds_dir)
    prompts = [p for p in ds.prompts if p["split"] == split]
    if not prompts:
        raise Refused(f"the {split} split has no digest prompts")
    bilbo_exe = Path(args.bilbo)
    if not bilbo_exe.is_file():
        raise Refused(f"no bilbo binary at {bilbo_exe}")
    identity = runner.bilbo_identity(bilbo_exe)
    if args.run:
        run = Path(args.run)
        existing = results.load_run(run)
        for name, arm in existing["arms"].items():
            seen = (arm["run"]["split"], arm["run"]["dataset"]["tree_hash"])
            if seen != (split, tree_hash):
                raise Refused(f"{run} holds {name} for split {seen[0]} and tree hash {seen[1]}, not {split} and {tree_hash}")
            if arm["run"]["bilbo"]["sha256"] != identity["sha256"]:
                raise Refused(f"{run} holds {name} made by bilbo sha256 {arm['run']['bilbo']['sha256']}, not {identity['sha256']}")
        if existing["digest"] is not None:
            raise Refused(f"{run} holds a digest run already")
        run_id = run.name
    else:
        run_id = f"{runner._stamp()}-{split}{'-draft' if draft else ''}"
        run = results.run_dir(run_id)
        if run.exists():
            raise Refused(f"{run} exists already; pick another run id")
    started = common.now()
    from bilbo_evals.arms.dense_ref import ensure_index

    with runner.Session(ds, split, bilbo_exe, args.model, args.llama_server, getattr(args, "embedder_url", None),
                        True, f"{run_id}-digest") as sess:  # fmt: skip
        if args.run:
            mine = runner.embedder_record(sess.server)
            for name, arm in existing["arms"].items():
                theirs = arm["run"].get("embedder")
                for field in ("model", "gguf_sha256", "query_prefix"):
                    if theirs and theirs.get(field) != mine.get(field):
                        raise Refused(f"{run} holds {name} with embedder {field} {theirs.get(field)!r}, not {mine.get(field)!r}")
        index = ensure_index(sess.ctx)
        rows = run_config(sess, prompts, THRESHOLD) + run_config(sess, prompts, None)
        sweep: list[dict] = []
        if args.sweep:
            for t in SWEEP:
                sweep += run_config(sess, prompts, t)
        meta = {
            "schema_version": 1, "run_id": run_id, "layer": "L1", "arm": "digest", "split": split, "draft": draft,
            "dataset": {"name": ds.name, "version": ds.version, "tree_hash": tree_hash, "path": rel_path}, "bilbo": identity,
            "embedder": runner.embedder_record(sess.server),
            "bilbo_config": {"digest.log": "on", "digest.min_similarity": f"{THRESHOLD:g}", "embedder.model": embedder.MODEL},
            "parity": "ok", "index": {"embedded": index["embedded"], "inputs": index["inputs"]},
            "versions": runner.versions(sess.server), "seeds": [], "host": runner.host(), "root": str(sess.sb.root),
            "folders": runner._folders(sess),
            "sweep": bool(args.sweep), "started": started, "ended": common.now(),
        }  # fmt: skip
        results.write_digest(run, sandbox.portable(sess.sb, meta), sandbox.portable(sess.sb, rows),
                             sandbox.portable(sess.sb, sweep) or None)
    if split == "test" and not draft and not args.run:
        results.log_test_run(run_id, tree_hash, identity["version"])
    text = results.report(run)
    (run / "report.md").write_text(text + "\n", encoding="utf-8")
    out(text)
    return 0
