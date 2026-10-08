"""Run records on disk, the report over a run and the comparison of two runs."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np

from bilbo_evals import common, dataset, stats
from bilbo_evals.arms import ARMS, PROCESS_ARMS
from bilbo_evals.common import Refused, err, out

CLAIM = "These numbers measure retrieval over curated synthetic notes."
STRATA = ["known-item", "paraphrase", "pt-en", "alias", "supersession", "multi-hop", "kind-filter"]
NOTE_STRATA = set(STRATA)
METRIC_COLS = [("success@5", "success@5"), ("rr", "MRR"), ("ndcg@10", "nDCG@10"), ("r@10", "R@10")]
OVERALL = "all note queries"
DIGEST = "digest"


def run_dir(run_id: str) -> Path:
    return common.RUNS_DIR / run_id


def write_arm(run: Path, arm: str, run_json: dict, rows: list[dict], trec: dict[str, list[str]] | None) -> None:
    from bilbo_evals import retrieval

    folder = run / arm
    folder.mkdir(parents=True, exist_ok=True)
    common.write_jsonl(folder / "per_item.jsonl", rows)
    if trec is not None:
        retrieval.write_trec(folder / "run.trec", trec, arm)
    _seal(folder, run_json)


def write_digest(run: Path, run_json: dict, rows: list[dict], sweep: list[dict] | None) -> None:
    folder = run / DIGEST
    folder.mkdir(parents=True, exist_ok=True)
    common.write_jsonl(folder / "per_item.jsonl", rows)
    if sweep:
        common.write_jsonl(folder / "sweep.jsonl", sweep)
    _seal(folder, run_json)


def _seal(folder: Path, run_json: dict) -> None:
    run_json["files"] = {p.name: common.sha256_file(p) for p in sorted(folder.iterdir()) if p.is_file() and p.name != "run.json"}
    common.write_json(folder / "run.json", run_json)


def check_files(arm_dir: Path) -> list[str]:
    """Files of an arm folder that differ from, are missing from, or are absent in the hashes its run.json records."""
    arm_dir = Path(arm_dir)
    listed = json.loads((arm_dir / "run.json").read_text(encoding="utf-8")).get("files", {})
    problems = []
    for name, digest in sorted(listed.items()):
        path = arm_dir / name
        if not path.is_file():
            problems.append(f"{path}: missing")
        elif common.sha256_file(path) != digest:
            problems.append(f"{path}: differs from the hash in run.json")
    for path in sorted(arm_dir.iterdir()):
        if path.is_file() and path.name != "run.json" and path.name not in listed:
            problems.append(f"{path}: not listed in run.json")
    return problems


def _folders(run: Path) -> list[Path]:
    return sorted(d for d in Path(run).iterdir() if d.is_dir() and (d / "run.json").is_file()) if Path(run).is_dir() else []


def load_run(run: Path) -> dict:
    run = Path(run)
    folders = _folders(run)
    if not folders:
        raise Refused(f"{run} is not a run: no run.json under it")
    arms: dict[str, dict] = {}
    digest = None
    for d in folders:
        meta = json.loads((d / "run.json").read_text(encoding="utf-8"))
        rows = common.read_jsonl(d / "per_item.jsonl") if (d / "per_item.jsonl").is_file() else []
        if d.name == DIGEST:
            sweep = common.read_jsonl(d / "sweep.jsonl") if (d / "sweep.jsonl").is_file() else None
            digest = {"run": meta, "rows": rows, "sweep": sweep}
        else:
            arms[d.name] = {"run": meta, "rows": rows}
    order = {a: i for i, a in enumerate(ARMS)}
    return {"arms": dict(sorted(arms.items(), key=lambda kv: order.get(kv[0], len(order)))), "digest": digest}


def _read_log() -> list[dict]:
    return common.read_jsonl(common.TEST_RUNS_LOG) if common.TEST_RUNS_LOG.is_file() else []


def log_test_run(run_id: str, tree_hash: str, bilbo_version: str) -> int:
    earlier = sum(1 for r in _read_log() if r["tree_hash"] == tree_hash)
    common.append_jsonl(
        common.TEST_RUNS_LOG,
        {"run_id": run_id, "tree_hash": tree_hash, "bilbo_version": bilbo_version, "time": common.now()},
    )
    return earlier


def earlier_test_runs(run_id: str, tree_hash: str) -> int:
    rows = [r for r in _read_log() if r["tree_hash"] == tree_hash]
    for i, r in enumerate(rows):
        if r["run_id"] == run_id:
            return i
    return len(rows)


# --- the dataset a run was made on ----------------------------------------------------------------------------------

def find_dataset(meta: dict) -> dict:
    """Queries and preregistration of the dataset a run records, checked against its tree hash."""
    ds = meta["dataset"]
    path = common.EVALS_ROOT / ds["path"] if ds.get("path") else common.EVALS_ROOT / "datasets" / ds["name"] / ds["version"]
    if not path.is_dir():
        raise Refused(f"the dataset {ds['name']}/{ds['version']} is not at {path}")
    if ds["tree_hash"] is not None:
        actual, problems = dataset.verify(path)
        if problems or actual != ds["tree_hash"]:
            raise Refused(f"{path} no longer matches tree hash {ds['tree_hash']} recorded by the run")
    queries = {q["id"]: q for q in common.read_jsonl(path / "queries.jsonl")}
    prereg_path = path / "preregistration.json"
    prereg = json.loads(prereg_path.read_text(encoding="utf-8")) if prereg_path.is_file() else {}
    return {"queries": queries, "families": {i: q["family"] for i, q in queries.items()}, "prereg": prereg}


# --- numbers --------------------------------------------------------------------------------------------------------

def _rows(rows: list[dict], stratum: str | None) -> list[dict]:
    if stratum is None:
        return [r for r in rows if r["stratum"] in NOTE_STRATA]
    return [r for r in rows if r["stratum"] == stratum]


def _mean(rows: list[dict], metric: str) -> float | None:
    values = list(stats.per_query(rows, metric).values())
    return sum(values) / len(values) if values else None


def _n(rows: list[dict]) -> int:
    return len({r["item"] for r in rows})


def _f(v: float | None, digits: int = 3) -> str:
    return "-" if v is None else f"{v:.{digits}f}"


def _signed(v: float) -> str:
    return f"{v:+.3f}"


def _interval(diff: float, lo: float, hi: float) -> str:
    return f"{_signed(diff)} [{_signed(lo)}, {_signed(hi)}]"


def _table(header: list[str], body: list[list[str]]) -> list[str]:
    return ["| " + " | ".join(header) + " |", "|" + "|".join(["---"] * len(header)) + "|"] + [
        "| " + " | ".join(r) + " |" for r in body
    ]


def _metrics_table(arms: dict[str, dict], strata: list[str | None], zero_overlap: set[str] | None = None) -> list[str]:
    body = []
    for name, arm in arms.items():
        for s in strata:
            rows = _rows(arm["rows"], s)
            if zero_overlap is not None:
                rows = [r for r in rows if r["item"] in zero_overlap]
            if not rows:
                continue
            body.append([name, OVERALL if s is None else s, str(_n(rows)), *[_f(_mean(rows, m)) for m, _ in METRIC_COLS]])
    return _table(["arm", "stratum", "n", *[label for _, label in METRIC_COLS]], body)


def _paired(a: dict, b: dict, families: dict[str, str], metric: str, stratum: str | None):
    """Arrays of per-query values for two arms over the items both have: (a, b, clusters)."""
    pa = stats.per_query(_rows(a["rows"], stratum), metric)
    pb = stats.per_query(_rows(b["rows"], stratum), metric)
    items = sorted(set(pa) & set(pb))
    return np.array([pa[i] for i in items]), np.array([pb[i] for i in items]), [families[i] for i in items]


def _principal(prereg: dict, present: list[str]) -> str | None:
    pair = prereg.get("principal") or []
    if "bilbo-full" in pair:
        other = [x for x in pair if x != "bilbo-full"]
        if other and other[0] in present:
            return other[0]
    return None


def _statistics(loaded: dict, info: dict, split: str) -> list[str]:
    arms = loaded["arms"]
    if "bilbo-full" not in arms or len(arms) < 2:
        return ["Paired statistics need `bilbo-full` and one other arm in the run."]
    families = info["families"]
    full = arms["bilbo-full"]
    others = [n for n in arms if n != "bilbo-full"]
    principal = _principal(info["prereg"], others)
    tag = " exploratory" if split == "dev" else ""
    raw: dict[str, dict] = {n: stats.compare_arms(_rows(full["rows"], None), _rows(arms[n]["rows"], None), families, "success@5") for n in others}  # fmt: skip
    family = [n for n in info["prereg"].get("secondary", stats.DEFAULTS["secondary"]) if n != principal]
    holm = stats.holm({n: raw[n]["p"] if n in raw and raw[n]["p"] is not None else 1.0 for n in family})
    missing = [n for n in family if n not in raw or raw[n]["p"] is None]
    body = []
    for n in others:
        r = raw[n]
        if r["n"] == 0:
            continue
        role = "principal" if n == principal else "secondary"
        adj = "-" if n == principal else _f(holm.get(n), 4)
        body.append([n, role + (", exploratory" if tag else ""), str(r["n"]), _f(r["a"]), _f(r["b"]),
                     _interval(r["diff"], r["lo"], r["hi"]), _f(r["p"], 4) + tag, adj + (tag if adj != "-" else "")])  # fmt: skip
    lines = ["Primary metric: success@5 over note queries; difference is `bilbo-full` minus the arm, paired by query, "
             "resampled by fact family.", ""]
    lines += _table(["vs", "role", "n", "bilbo-full", "arm", "difference [95% CI]", "p", "p (Holm, secondary)"], body)
    if missing:
        lines += ["", f"Holm family: {', '.join(family)}; not in this run (p = 1): {', '.join(missing)}"]
    lines += ["", "Other metrics, same pairing:", ""]
    body = []
    for n in others:
        for metric, label in METRIC_COLS[1:]:
            a, b, c = _paired(full, arms[n], families, metric, None)
            if len(a):
                body.append([n, label, _interval(*stats.bootstrap_ci(a, b, c))])
    lines += _table(["vs", "metric", "difference [95% CI]"], body)
    lines += ["", "Per stratum, success@5 (descriptive):", ""]
    body = []
    for n in others:
        for s in [*STRATA, "library"]:
            a, b, c = _paired(full, arms[n], families, "success@5", s)
            if len(a):
                body.append([n, s, str(len(a)), _interval(*stats.bootstrap_ci(a, b, c))])
    lines += _table(["vs", "stratum", "n", "difference [95% CI]"], body)
    return lines


def _latency(arms: dict[str, dict]) -> list[str]:
    body = []
    for name, arm in arms.items():
        values = [r["latency_ms"] for r in arm["rows"] if r["latency_ms"] is not None] if name in PROCESS_ARMS else []
        if values:
            p50, p95 = np.percentile(values, [50, 95])
            body.append([name, f"{p50:.1f}", f"{p95:.1f}"])
        else:
            body.append([name, "-", "-"])
    return _table(["arm", "p50 ms", "p95 ms"], body)


def _extras(arms: dict[str, dict]) -> list[str]:
    body = []
    for name, arm in arms.items():
        rows = arm["rows"]
        body.append([
            name,
            _f(_mean(_rows(rows, "multi-hop"), "evidence@10")),
            _f(_mean(_rows(rows, "supersession"), "new_above_old")),
            _f(_mean(_rows(rows, "no-answer"), "empty")),
        ])  # fmt: skip
    return _table(["arm", "multi-hop: evidence@10", "supersession: gold above decoys", "no-answer: returned nothing"], body)


def _judged(arms: dict[str, dict]) -> list[str]:
    body = [[n, _f(_mean(_rows(a["rows"], None), "judged@10")), _f(_mean(_rows(a["rows"], "library"), "judged@10"))]
            for n, a in arms.items()]  # fmt: skip
    return _table(["arm", "note queries", "library"], body)


def _health(arms: dict[str, dict]) -> list[str]:
    body = []
    for name, arm in arms.items():
        rows = arm["rows"]
        body.append([name, str(len(rows)), str(sum(1 for r in rows if r["error"])), str(sum(1 for r in rows if r["fallback"]))])
    return _table(["arm", "records", "errors", "fallbacks"], body)


def _identity(loaded: dict, meta: dict) -> list[str]:
    ds, b, e = meta["dataset"], meta["bilbo"], None
    for arm in loaded["arms"].values():
        e = e or arm["run"].get("embedder")
    if loaded["digest"]:
        e = e or loaded["digest"]["run"].get("embedder")
    lines = [
        f"- run: {meta['run_id']} (split {meta['split']}{', draft' if meta['draft'] else ''})",
        f"- dataset: {ds['name']}/{ds['version']}, tree hash {ds['tree_hash'] or 'none (draft)'}",
        f"- bilbo: {b['version']}, commit {b.get('commit') or '-'}, binary sha256 {b['sha256']}",
    ]
    if e:
        lines.append(f"- embedder: {e['model']}, gguf sha256 {e['gguf_sha256']}, llama-server {e['llama_server_version']}")
    else:
        lines.append("- embedder: none")
    lines.append(f"- arms: {', '.join(loaded['arms']) or 'none'}; started {meta['started']}")
    return lines


def report(run: Path) -> str:
    from bilbo_evals import digest as digest_mod

    run = Path(run)
    loaded = load_run(run)
    arms = loaded["arms"]
    first = next(iter(arms.values()))["run"] if arms else loaded["digest"]["run"]
    split, draft = first["split"], first["draft"]
    info = find_dataset(first) if arms else None
    lines = [CLAIM, "", f"# L1 retrieval report{' (DRAFT)' if draft else ''}", ""]
    lines += _identity(loaded, first)
    if split == "test" and not draft:
        k = earlier_test_runs(first["run_id"], first["dataset"]["tree_hash"])
        lines.append(f"- {k} earlier test run{'' if k == 1 else 's'} on this dataset")
    if arms:
        zero = {i for i, q in info["queries"].items() if q.get("zero_overlap")}
        lines += ["", "## Metrics by stratum", ""] + _metrics_table(arms, [None, *STRATA])
        lines += ["", "## Zero-overlap queries", ""] + _metrics_table(arms, [None], zero)
        lines += ["", "## Library", ""] + _metrics_table(arms, ["library"])
        lines += ["", "## Judged@10", ""] + _judged(arms)
        lines += ["", "## Multi-hop, supersession and no-answer", ""] + _extras(arms)
        lines += ["", "## Paired statistics", ""] + _statistics(loaded, info, split)
        lines += ["", "## Latency", ""] + _latency(arms)
        lines += ["", "## Errors and fallbacks", ""] + _health(arms)
    if loaded["digest"]:
        lines += [""] + digest_mod.section(loaded["digest"])
    return "\n".join(lines)


def cmd_report(args: argparse.Namespace) -> int:
    run = Path(args.run)
    text = report(run)
    (run / "report.md").write_text(text + "\n", encoding="utf-8")
    out(text)
    return 0


# --- compare --------------------------------------------------------------------------------------------------------

def _embedder_of(loaded: dict) -> dict | None:
    for arm in loaded["arms"].values():
        if arm["run"].get("embedder"):
            return arm["run"]["embedder"]
    if loaded["digest"] and loaded["digest"]["run"].get("embedder"):
        return loaded["digest"]["run"]["embedder"]
    return None


def _tree(loaded: dict) -> str | None:
    metas = [a["run"] for a in loaded["arms"].values()] + ([loaded["digest"]["run"]] if loaded["digest"] else [])
    return metas[0]["dataset"]["tree_hash"]


def _check_embedders(base: dict, new: dict) -> None:
    eb, en = _embedder_of(base), _embedder_of(new)
    if (eb is None) != (en is None):
        raise Refused("one run used an embedder and the other did not")
    if eb is None:
        return
    for key in ("model", "gguf_sha256", "query_prefix"):
        if eb.get(key) != en.get(key):
            raise Refused(f"the embedders differ in {key}: {eb.get(key)!r} against {en.get(key)!r}")
    if eb.get("llama_server_version") != en.get("llama_server_version"):
        err(f"warning: llama-server versions differ: {eb.get('llama_server_version')} against {en.get('llama_server_version')}")


def _percentiles(arm: dict) -> tuple[float, float] | None:
    values = [r["latency_ms"] for r in arm["rows"] if r["latency_ms"] is not None]
    if not values:
        return None
    p50, p95 = np.percentile(values, [50, 95])
    return float(p50), float(p95)


def cmd_compare(args: argparse.Namespace) -> int:
    paths = [Path(args.baseline), Path(args.run)]
    loaded = [load_run(p) for p in paths]
    for path, run in zip(paths, loaded):
        folders = [path / a for a in run["arms"]] + ([path / DIGEST] if run["digest"] else [])
        problems = [p for f in folders for p in check_files(f)]
        if problems:
            raise Refused("\n".join(problems))
    base, new = loaded
    trees = (_tree(base), _tree(new))
    if trees[0] != trees[1] or trees[0] is None:
        raise Refused(f"the dataset tree hashes differ or are missing: {trees[0]} against {trees[1]}")
    _check_embedders(base, new)
    bm = next(iter(base["arms"].values()))["run"] if base["arms"] else None
    nm = next(iter(new["arms"].values()))["run"] if new["arms"] else None
    shared = [a for a in new["arms"] if a in base["arms"]]
    if not shared or bm["split"] != nm["split"]:
        raise Refused("the runs share no arm on the same split")
    info = find_dataset(nm)
    lines = [CLAIM, "", f"# L1 comparison: {paths[0].name} to {paths[1].name}", "",
             f"- bilbo: {bm['bilbo']['version']} to {nm['bilbo']['version']}",
             f"- dataset: {nm['dataset']['name']}/{nm['dataset']['version']}, tree hash {trees[0]}", "",
             "Differences are run minus baseline, paired by query id, with 95% intervals from a bootstrap over fact families.", ""]  # fmt: skip
    body = []
    for name in shared:
        for s in [None, *STRATA, "library"]:
            n_rows, b_rows = new["arms"][name], base["arms"][name]
            cells = []
            count = 0
            for metric, _ in METRIC_COLS:
                a, b, c = _paired(n_rows, b_rows, info["families"], metric, s)
                count = len(a)
                cells.append(_interval(*stats.bootstrap_ci(a, b, c)) if len(a) else "-")
            if count:
                body.append([name, OVERALL if s is None else s, str(count), *cells])
    lines += _table(["arm", "stratum", "n", *[f"{label}" for _, label in METRIC_COLS]], body)
    lines += [""]
    hb, hn = bm["host"], nm["host"]
    if (hb["platform"], hb["cpu"]) == (hn["platform"], hn["cpu"]):
        body = []
        for name in shared:
            if name not in PROCESS_ARMS:
                continue
            pb, pn = _percentiles(base["arms"][name]), _percentiles(new["arms"][name])
            if pb and pn:
                body.append([name, f"{pn[0] - pb[0]:+.1f}", f"{pn[1] - pb[1]:+.1f}"])
        lines += ["## Latency", ""] + _table(["arm", "p50 ms", "p95 ms"], body)
    else:
        lines.append("Latency deltas are omitted: the runs record different platforms or CPUs.")
    out("\n".join(lines))
    return 0
