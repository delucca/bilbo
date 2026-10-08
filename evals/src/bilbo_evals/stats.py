"""Paired statistics over fact families and the preregistered power sizing (no scipy)."""

from __future__ import annotations

import argparse
import json
import math
from collections import defaultdict
from pathlib import Path
from typing import Sequence

import numpy as np

from bilbo_evals import common
from bilbo_evals.common import Refused

NOTE_STRATA = ["known-item", "paraphrase", "pt-en", "alias", "supersession", "multi-hop", "kind-filter"]
OTHER_STRATA = ["library", "no-answer"]
STRATUM_FLOOR = 20
N_CAP = 5000
EXACT_MAX_CLUSTERS = 13
PRIMARY = "success@5"
DEFAULTS = {
    "primary_metric": PRIMARY,
    "principal": ["bilbo-full", "bm25-ref"],
    "secondary": ["bilbo-keyword", "ripgrep", "dense-ref", "random"],
    "min_effect": 0.10,
    "alpha": 0.05,
    "power": 0.80,
    "correction": "holm (secondary only)",
    "test": "paired sign-flip by fact family, 10000 resamples",
    "seed": 0,
    "resamples": 10000,
}


def _cluster_index(clusters: Sequence[str]) -> np.ndarray:
    ids: dict[str, int] = {}
    return np.array([ids.setdefault(c, len(ids)) for c in clusters], dtype=np.int64)


def _cluster_sums(diffs: np.ndarray, clusters: Sequence[str]) -> tuple[np.ndarray, np.ndarray]:
    idx = _cluster_index(clusters)
    return np.bincount(idx, weights=diffs), np.bincount(idx).astype(float)


def sign_flip_p(diffs: np.ndarray, clusters: Sequence[str], n: int = 10000, seed: int = 0) -> float:
    """Two-sided p of mean(diffs) when each cluster's summed difference flips sign as one.

    With at most 13 clusters (2^C <= n) all 2^C sign vectors are enumerated and p is the exact share
    with |sum| >= |observed|. Otherwise n seeded random sign vectors are drawn and p = (hits+1)/(n+1).
    """
    diffs = np.asarray(diffs, dtype=float)
    if len(diffs) != len(clusters):
        raise ValueError("diffs and clusters differ in length")
    if len(diffs) == 0:
        return 1.0
    sums, _ = _cluster_sums(diffs, clusters)
    observed = abs(sums.sum())
    eps = 1e-9 * max(1.0, float(np.abs(diffs).sum()))
    if observed <= eps:
        return 1.0
    c = len(sums)
    if c <= EXACT_MAX_CLUSTERS and 2**c <= n:
        bits = (np.arange(2**c)[:, None] >> np.arange(c)) & 1
        totals = np.abs((bits * 2 - 1) @ sums)
        return float((totals >= observed - eps).mean())
    rng = np.random.default_rng(seed)
    hits, done = 0, 0
    while done < n:
        chunk = min(1000, n - done)
        signs = rng.integers(0, 2, size=(chunk, c)) * 2 - 1
        hits += int((np.abs(signs @ sums) >= observed - eps).sum())
        done += chunk
    return (hits + 1) / (n + 1)


def _boot_means(diffs: np.ndarray, clusters: Sequence[str], n: int, seed: int) -> np.ndarray:
    """Mean difference of n cluster-bootstrap resamples (clusters drawn with replacement)."""
    sums, counts = _cluster_sums(diffs, clusters)
    rng = np.random.default_rng(seed)
    c = len(sums)
    out = np.empty(n)
    for start in range(0, n, 1000):
        chunk = min(1000, n - start)
        pick = rng.integers(0, c, size=(chunk, c))
        out[start:start + chunk] = sums[pick].sum(axis=1) / counts[pick].sum(axis=1)
    return out


def bootstrap_ci(a: np.ndarray, b: np.ndarray, clusters: Sequence[str], n: int = 10000, seed: int = 0,
                 level: float = 0.95) -> tuple[float, float, float]:
    """(mean a-b, lo, hi): percentile interval over a paired bootstrap that resamples clusters."""
    diffs = np.asarray(a, dtype=float) - np.asarray(b, dtype=float)
    if len(diffs) == 0:
        return 0.0, 0.0, 0.0
    means = _boot_means(diffs, clusters, n, seed)
    tail = (1 - level) / 2 * 100
    lo, hi = np.percentile(means, [tail, 100 - tail])
    return float(diffs.mean()), float(lo), float(hi)


def design_effect(diffs, clusters: Sequence[str], n: int = 10000, seed: int = 0) -> float:
    """var(cluster-bootstrap mean) / var(query-bootstrap mean), floored at 1."""
    diffs = np.asarray(diffs, dtype=float)
    if len(diffs) < 2:
        return 1.0
    cluster_var = float(np.var(_boot_means(diffs, clusters, n, seed)))
    rng = np.random.default_rng(seed + 1)
    iid = np.empty(n)
    for start in range(0, n, 1000):
        chunk = min(1000, n - start)
        iid[start:start + chunk] = diffs[rng.integers(0, len(diffs), size=(chunk, len(diffs)))].mean(axis=1)
    iid_var = float(np.var(iid))
    if iid_var == 0.0:
        return 1.0
    return max(1.0, cluster_var / iid_var)


def holm(p: dict[str, float]) -> dict[str, float]:
    """Holm step-down adjusted p-values: (m-i) * p(i) in ascending order, made monotone, capped at 1."""
    order = sorted(p, key=lambda k: p[k])
    adjusted: dict[str, float] = {}
    running = 0.0
    for i, key in enumerate(order):
        running = max(running, min(1.0, (len(order) - i) * p[key]))
        adjusted[key] = running
    return {k: adjusted[k] for k in p}


def _log_pmf(n: int, k: int, p: float) -> float:
    if k < 0 or k > n:
        return -math.inf
    if p <= 0.0:
        return 0.0 if k == 0 else -math.inf
    if p >= 1.0:
        return 0.0 if k == n else -math.inf
    return math.log(math.comb(n, k)) + k * math.log(p) + (n - k) * math.log1p(-p)


def _mcnemar_crit(m: int, alpha: float) -> int:
    """Largest c with two-sided exact p = 2 P(X <= c) <= alpha, X ~ Binomial(m, 1/2); -1 if none."""
    c, cum = -1, 0
    while c + 1 < (m + 1) // 2:
        cum += math.comb(m, c + 1)
        if 2 * cum / 2**m > alpha:
            break
        c += 1
    return c


def _reject_prob(m: int, q: float, alpha: float) -> float:
    """P(the exact McNemar test rejects | m discordant, P(k10) = q): k10 <= c or k10 >= m - c."""
    c = _mcnemar_crit(m, alpha)
    if c < 0:
        return 0.0
    tail = sum(math.exp(_log_pmf(m, k, q)) for k in range(c + 1))
    upper = sum(math.exp(_log_pmf(m, k, q)) for k in range(m - c, m + 1))
    return tail + upper


def _power(n: int, psi: float, delta: float, alpha: float, cache: dict[int, float]) -> float:
    q = (psi + delta) / 2 / psi
    mean, sd = n * psi, math.sqrt(n * psi * (1 - psi))
    lo, hi = max(0, int(mean - 10 * sd) - 1), min(n, int(mean + 10 * sd) + 1)
    total = 0.0
    for m in range(lo, hi + 1):
        w = math.exp(_log_pmf(n, m, psi))
        if w == 0.0:
            continue
        if m not in cache:
            cache[m] = _reject_prob(m, q, alpha)
        total += w * cache[m]
    return total


def mcnemar_n(psi: float, delta: float, alpha: float = 0.05, power: float = 0.80) -> int:
    """Smallest n >= 10 whose exact conditional McNemar test rejects with probability >= power.

    Discordant pairs m ~ Binomial(n, psi'), psi' = max(psi, delta); k10 | m ~ Binomial(m, p10 / psi') with
    p10 = (psi' + delta) / 2 and p01 = (psi' - delta) / 2; the test is the two-sided exact binomial test of
    k10 against m/2 at alpha. Power is not monotone in n (a sawtooth), so this is the first n that reaches it.
    """
    if not 0 < delta < 1:
        raise Refused(f"minimum effect {delta} must be between 0 and 1")
    psi = max(psi, delta)
    if psi > 1:
        raise Refused(f"discordance {psi} exceeds 1")
    cache: dict[int, float] = {}
    for n in range(10, N_CAP + 1):
        if _power(n, psi, delta, alpha, cache) >= power:
            return n
    raise Refused(f"no test size up to {N_CAP} queries reaches power {power} at discordance {psi} and effect {delta}")


def per_query(rows: list[dict], metric: str) -> dict[str, float]:
    """Mean of a metric over an item's trials (random's 20 seeds); items with no value are left out."""
    values: dict[str, list[float]] = defaultdict(list)
    for row in rows:
        v = row["metrics"].get(metric)
        if v is not None:
            values[row["item"]].append(float(v))
    return {item: sum(v) / len(v) for item, v in values.items()}


def compare_arms(a_rows: list[dict], b_rows: list[dict], families: dict[str, str], metric: str) -> dict:
    """Paired comparison a - b over the items both arms have: {"n", "a", "b", "diff", "lo", "hi", "p"}."""
    pa, pb = per_query(a_rows, metric), per_query(b_rows, metric)
    items = sorted(set(pa) & set(pb))
    missing = [i for i in items if i not in families]
    if missing:
        raise Refused(f"no fact family for {missing[0]} (and {len(missing) - 1} more)" if len(missing) > 1
                      else f"no fact family for {missing[0]}")
    if not items:
        return {"n": 0, "a": None, "b": None, "diff": None, "lo": None, "hi": None, "p": None}
    a = np.array([pa[i] for i in items])
    b = np.array([pb[i] for i in items])
    clusters = [families[i] for i in items]
    diff, lo, hi = bootstrap_ci(a, b, clusters)
    return {"n": len(items), "a": float(a.mean()), "b": float(b.mean()), "diff": diff, "lo": lo, "hi": hi,
            "p": sign_flip_p(a - b, clusters)}


def _allocate(total: int, counts: dict[str, int]) -> dict[str, int]:
    """Split total in the proportions of counts by largest remainder, each part at least STRATUM_FLOOR."""
    weight = sum(counts.values())
    if weight == 0:
        return {k: STRATUM_FLOOR for k in counts}
    exact = {k: total * v / weight for k, v in counts.items()}
    parts = {k: int(x) for k, x in exact.items()}
    for k in sorted(counts, key=lambda k: exact[k] - parts[k], reverse=True)[: total - sum(parts.values())]:
        parts[k] += 1
    return {k: max(STRATUM_FLOOR, v) for k, v in parts.items()}


def _arm_rows(run: Path, arm: str) -> tuple[dict, list[dict]]:
    folder = run / arm
    if not (folder / "run.json").is_file():
        raise Refused(f"{run} has no run for {arm}")
    return json.loads((folder / "run.json").read_text()), common.read_jsonl(folder / "per_item.jsonl")


def cmd_power(args: argparse.Namespace) -> int:
    run, ds = Path(args.run), Path(args.dataset)
    prereg_path = ds / "preregistration.json"
    if (ds / "FROZEN").exists():
        raise Refused(f"{ds} is frozen: preregistration.json cannot change")
    prereg = dict(DEFAULTS)
    if prereg_path.is_file():
        prereg.update(json.loads(prereg_path.read_text()))
    a_arm, b_arm = prereg["principal"]
    a_run, a_rows = _arm_rows(run, a_arm)
    b_run, b_rows = _arm_rows(run, b_arm)
    for meta in (a_run, b_run):
        if meta["split"] != "dev":
            raise Refused("power is sized on dev only: this run is of the " + f"{meta['split']} split")
    queries = {q["id"]: q for q in common.read_jsonl(ds / "queries.jsonl") if q["split"] == "dev"}
    families = {i: q["family"] for i, q in queries.items() if q["stratum"] in NOTE_STRATA}
    metric = prereg["primary_metric"]
    pa, pb = per_query(a_rows, metric), per_query(b_rows, metric)
    items = sorted(i for i in set(pa) & set(pb) if i in families)
    if len(items) < 2:
        raise Refused(f"the dev run has {len(items)} note queries scored by both {a_arm} and {b_arm}")
    diffs = np.array([pa[i] - pb[i] for i in items])
    clusters = [families[i] for i in items]
    delta = prereg["min_effect"]
    discordance = float(np.mean([pa[i] != pb[i] for i in items]))
    psi = max(discordance, delta)
    n_iid = mcnemar_n(psi, delta, prereg["alpha"], prereg["power"])
    de = design_effect(diffs, clusters, prereg["resamples"], prereg["seed"])
    n_test = math.ceil(n_iid * de)
    dev_counts = {s: sum(1 for q in queries.values() if q["stratum"] == s) for s in NOTE_STRATA + OTHER_STRATA}
    per_stratum = _allocate(n_test, {s: dev_counts[s] for s in NOTE_STRATA})
    note_dev = sum(dev_counts[s] for s in NOTE_STRATA)
    for s in OTHER_STRATA:
        per_stratum[s] = max(STRATUM_FLOOR, math.ceil(dev_counts[s] * n_test / note_dev)) if note_dev else STRATUM_FLOOR
    prereg.update({
        "dev_run": run.name,
        "dev_tree_hash": a_run.get("dataset", {}).get("tree_hash"),
        "discordance": discordance,
        "psi_used": psi,
        "design_effect": de,
        "n_iid": n_iid,
        "n_test": n_test,
        "per_stratum": per_stratum,
        "created": common.now(),
    })
    common.write_json(prereg_path, prereg)
    common.out(f"principal: {a_arm} vs {b_arm} on {metric}, {len(items)} dev note queries")
    common.out(f"discordance {discordance:.3f} (used {psi:.3f}), min effect {delta}")
    common.out(f"n_iid {n_iid}")
    common.out(f"design_effect {de:.3f}")
    common.out(f"n_test {n_test}")
    for stratum, count in per_stratum.items():
        common.out(f"  {stratum:<12} {count}")
    return 0
