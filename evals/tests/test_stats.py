"""Statistics checked against small cases worked by hand, then the power command on synthetic rows."""

from __future__ import annotations

import json
import math
from itertools import product
from math import comb

import numpy as np
import pytest

from bilbo_evals import common, stats
from bilbo_evals.common import Refused

DEV_COUNTS = {"known-item": 20, "paraphrase": 30, "pt-en": 25, "alias": 20, "supersession": 20,
              "multi-hop": 20, "kind-filter": 20, "library": 20, "no-answer": 20}


# sign_flip_p: exact enumeration, hand-worked

def test_sign_flip_all_positive_five_clusters():
    # 2 of the 2^5 sign vectors reach |sum| = 5
    assert stats.sign_flip_p(np.ones(5), list("abcde")) == pytest.approx(2 / 32)


def test_sign_flip_cluster_sums_flip_together():
    # sums A = 2, B = 1, observed 3: only ++ and -- reach it
    assert stats.sign_flip_p(np.ones(3), ["A", "A", "B"]) == pytest.approx(0.5)
    # sums A = 2, B = -1, observed 1: every vector reaches it
    assert stats.sign_flip_p(np.array([1.0, 1.0, -1.0]), ["A", "A", "B"]) == 1.0


def test_sign_flip_equals_exact_mcnemar():
    # 9 of 10 discordant pairs one way: p = 2 (C(10,0) + C(10,1)) / 2^10 = 22/1024
    diffs = np.array([1.0] * 9 + [-1.0])
    assert stats.sign_flip_p(diffs, [str(i) for i in range(10)]) == pytest.approx(22 / 1024)


def test_sign_flip_fractional_values():
    # sums 0.5, 0.25, 0.25: observed 1.0, reached by ppp and mmm only
    assert stats.sign_flip_p(np.array([0.5, 0.25, 0.25]), ["a", "b", "c"]) == pytest.approx(2 / 8)


def test_sign_flip_zero_and_empty():
    assert stats.sign_flip_p(np.zeros(4), list("abcd")) == 1.0
    assert stats.sign_flip_p(np.array([]), []) == 1.0


def test_sign_flip_sampled_matches_brute_force_and_is_seeded():
    rng = np.random.default_rng(3)
    diffs = rng.choice([-1.0, 0.0, 1.0], size=40, p=[0.2, 0.3, 0.5])
    clusters = [str(i // 3) for i in range(40)]
    sums = np.array([diffs[[i for i in range(40) if i // 3 == c]].sum() for c in range(14)])
    obs = abs(sums.sum())
    exact = np.mean([abs(np.dot(s, sums)) >= obs - 1e-9 for s in product([-1, 1], repeat=14)])
    p = stats.sign_flip_p(diffs, clusters, n=20000, seed=1)
    assert p == pytest.approx(exact, abs=0.02)
    assert stats.sign_flip_p(diffs, clusters, n=2000, seed=5) == stats.sign_flip_p(diffs, clusters, n=2000, seed=5)


def test_sign_flip_clustering_widens_p():
    diffs = np.ones(20)
    iid = stats.sign_flip_p(diffs, [str(i) for i in range(20)], n=5000)
    one_family = stats.sign_flip_p(diffs, ["f"] * 10 + ["g"] * 10, n=5000)
    assert iid < 0.001 and one_family == pytest.approx(0.5)


# holm

def test_holm_hand_worked():
    adj = stats.holm({"a": 0.01, "b": 0.04, "c": 0.03, "d": 0.5})
    # ascending 0.01*4 = 0.04, 0.03*3 = 0.09, 0.04*2 = 0.08 -> monotone 0.09, 0.5*1
    assert adj == {"a": pytest.approx(0.04), "c": pytest.approx(0.09), "b": pytest.approx(0.09), "d": pytest.approx(0.5)}


def test_holm_caps_at_one_and_keeps_keys():
    assert stats.holm({"a": 0.6, "b": 0.7}) == {"a": 1.0, "b": 1.0}
    assert stats.holm({}) == {}


# bootstrap and design effect

def test_bootstrap_ci_constant_difference_is_degenerate():
    a, b = np.full(30, 0.75), np.full(30, 0.25)
    mean, lo, hi = stats.bootstrap_ci(a, b, [str(i % 6) for i in range(30)], n=500)
    assert (mean, lo, hi) == (0.5, 0.5, 0.5)


def test_bootstrap_ci_contains_mean_and_is_seeded():
    rng = np.random.default_rng(0)
    a = rng.integers(0, 2, 120).astype(float)
    b = rng.integers(0, 2, 120).astype(float)
    clusters = [str(i // 4) for i in range(120)]
    mean, lo, hi = stats.bootstrap_ci(a, b, clusters, n=2000, seed=2)
    assert lo < mean < hi
    assert (mean, lo, hi) == stats.bootstrap_ci(a, b, clusters, n=2000, seed=2)
    assert mean == pytest.approx(float((a - b).mean()))


def test_bootstrap_two_clusters_hand_worked():
    # clusters of sums 2 (n=2) and 0 (n=2): resample means are 1.0, 0.5, 0.5, 0.0 -> every draw in [0, 1]
    mean, lo, hi = stats.bootstrap_ci(np.array([1.0, 1.0, 0.0, 0.0]), np.zeros(4), ["x", "x", "y", "y"], n=4000)
    assert mean == 0.5 and 0.0 <= lo <= 0.5 <= hi <= 1.0


def test_design_effect_floor_and_clustering():
    rng = np.random.default_rng(1)
    iid_diffs = rng.choice([-1.0, 0.0, 1.0], 300)
    assert stats.design_effect(iid_diffs, [str(i) for i in range(300)], n=3000) == pytest.approx(1.0, abs=0.25)
    # perfectly correlated within families of 10: variance is inflated by about the family size
    fam = rng.choice([-1.0, 1.0], 30)
    diffs = np.repeat(fam, 10)
    de = stats.design_effect(diffs, [str(i // 10) for i in range(300)], n=3000)
    assert 6 < de < 14
    assert stats.design_effect(np.zeros(10), list("aabbccddee"), n=200) == 1.0
    assert stats.design_effect(np.array([1.0]), ["a"]) == 1.0


# mcnemar_n

def test_mcnemar_critical_values_hand_worked():
    assert stats._mcnemar_crit(5, 0.05) == -1       # 2/32 = 0.0625 > 0.05
    assert stats._mcnemar_crit(6, 0.05) == 0        # 2/64 = 0.031; 2*7/64 = 0.22
    assert stats._mcnemar_crit(10, 0.05) == 1       # 2*11/1024 = 0.0215; 2*56/1024 = 0.109
    assert stats._mcnemar_crit(0, 0.05) == -1


def test_mcnemar_power_hand_worked():
    # psi = 1, delta = 0.5: m = n = 6, q = 0.75, reject at k <= 0 or k >= 6
    expected = 0.75**6 + 0.25**6
    assert stats._power(6, 1.0, 0.5, 0.05, {}) == pytest.approx(expected)
    # n = 3 (psi = 1): no m = 3 outcome can reject
    assert stats._power(3, 1.0, 0.5, 0.05, {}) == 0.0


def test_mcnemar_power_against_direct_enumeration():
    n, psi, delta, alpha = 40, 0.3, 0.15, 0.05
    q = (psi + delta) / 2 / psi
    total = 0.0
    for m in range(n + 1):
        pm = comb(n, m) * psi**m * (1 - psi) ** (n - m)
        for k in range(m + 1):
            tail = min(sum(comb(m, j) for j in range(k + 1)), sum(comb(m, j) for j in range(k, m + 1)))
            if min(1.0, 2 * tail / 2**m) <= alpha:
                total += pm * comb(m, k) * q**k * (1 - q) ** (m - k)
    assert stats._power(n, psi, delta, alpha, {}) == pytest.approx(total, rel=1e-9)


def test_mcnemar_n_matches_normal_approximation():
    psi, delta = 0.24, 0.10
    z_a, z_b = 1.959964, 0.841621
    approx = ((z_a * math.sqrt(psi) + z_b * math.sqrt(psi - delta**2)) / delta) ** 2
    assert 180 < approx < 200
    n = stats.mcnemar_n(psi, delta)
    assert abs(n - approx) / approx <= 0.15
    cache: dict[int, float] = {}
    assert stats._power(n, psi, delta, 0.05, cache) >= 0.80
    assert all(stats._power(k, psi, delta, 0.05, cache) < 0.80 for k in range(10, n))


def test_mcnemar_n_grows_when_effect_shrinks_and_clamps_psi():
    assert stats.mcnemar_n(0.24, 0.15) < stats.mcnemar_n(0.24, 0.10)
    assert stats.mcnemar_n(0.02, 0.10) == stats.mcnemar_n(0.10, 0.10)


def test_mcnemar_n_refuses_beyond_cap(monkeypatch):
    monkeypatch.setattr(stats, "N_CAP", 60)
    with pytest.raises(Refused):
        stats.mcnemar_n(0.5, 0.005)
    with pytest.raises(Refused):
        stats.mcnemar_n(0.2, 0.0)


# per_query and compare_arms

def _row(item, trial, value, metric="success@5"):
    return {"item": item, "trial": trial, "metrics": {metric: value}}


def test_per_query_averages_trials_and_skips_none():
    rows = [_row("a", 0, 1.0), _row("a", 1, 0.0), _row("b", 0, None), _row("c", 0, 0.5)]
    assert stats.per_query(rows, "success@5") == {"a": 0.5, "c": 0.5}


def test_compare_arms_pairs_on_common_items():
    fams = {f"q{i}": f"f{i // 2}" for i in range(12)}
    a = [_row(f"q{i}", 0, 1.0) for i in range(12)]
    b = [_row(f"q{i}", 0, 0.0) for i in range(11)]
    res = stats.compare_arms(a, b, fams, "success@5")
    assert res["n"] == 11 and res["diff"] == 1.0 and res["lo"] == 1.0 and res["hi"] == 1.0
    assert res["a"] == 1.0 and res["b"] == 0.0
    assert 0 < res["p"] < 0.05


def test_compare_arms_random_averaged_over_seeds():
    fams = {f"q{i}": f"f{i}" for i in range(30)}
    a = [_row(f"q{i}", 0, 1.0) for i in range(30)]
    rand = [_row(f"q{i}", s, float(s % 4 == 0)) for i in range(30) for s in range(20)]
    res = stats.compare_arms(a, rand, fams, "success@5")
    assert res["n"] == 30 and res["b"] == pytest.approx(0.25) and res["diff"] == pytest.approx(0.75)


def test_compare_arms_unknown_family_and_empty():
    with pytest.raises(Refused):
        stats.compare_arms([_row("q", 0, 1.0)], [_row("q", 0, 0.0)], {}, "success@5")
    assert stats.compare_arms([], [], {}, "success@5")["n"] == 0


# cmd_power on synthetic runs

def _world(tmp_path, split="dev", frozen=False, discordant=37):
    ds = tmp_path / "ds"
    ds.mkdir()
    queries, a_rows, b_rows = [], [], []
    k = 0
    for stratum, count in DEV_COUNTS.items():
        for i in range(count):
            qid = f"q-{stratum}-{i:03d}"
            queries.append({"id": qid, "stratum": stratum, "split": "dev", "family": f"fam-{k // 2}"})
            if stratum not in stats.NOTE_STRATA:
                continue
            # discordant items alternate who wins; the rest are both right or both wrong
            if k < discordant:
                a, b = (1.0, 0.0) if k % 3 else (0.0, 1.0)
            else:
                a = b = float(k % 2)
            a_rows.append({"item": qid, "stratum": stratum, "split": "dev", "trial": 0, "metrics": {"success@5": a}})
            b_rows.append({"item": qid, "stratum": stratum, "split": "dev", "trial": 0, "metrics": {"success@5": b}})
            k += 1
    common.write_jsonl(ds / "queries.jsonl", queries)
    run = tmp_path / "runs" / "20261007T000000Z-dev"
    for arm, rows in (("bilbo-full", a_rows), ("bm25-ref", b_rows)):
        (run / arm).mkdir(parents=True)
        (run / arm / "run.json").write_text(json.dumps(
            {"arm": arm, "split": split, "dataset": {"tree_hash": "abc"}}))
        common.write_jsonl(run / arm / "per_item.jsonl", rows)
    if frozen:
        (ds / "FROZEN").write_text("x")
    return ds, run


class Args:
    def __init__(self, run, dataset):
        self.run, self.dataset = run, dataset


def test_power_reproduces_spec_scenario(tmp_path, monkeypatch, capsys):
    ds, run = _world(tmp_path)
    monkeypatch.setattr(stats, "design_effect", lambda *a, **k: 1.3)
    assert stats.cmd_power(Args(run, ds)) == 0
    pre = json.loads((ds / "preregistration.json").read_text())
    assert pre["discordance"] == pytest.approx(37 / 155)
    assert 0.23 < pre["discordance"] < 0.25
    assert pre["psi_used"] == pytest.approx(37 / 155)
    assert pre["n_iid"] == stats.mcnemar_n(37 / 155, 0.10)
    assert pre["design_effect"] == 1.3
    assert pre["n_test"] == math.ceil(pre["n_iid"] * 1.3)
    assert pre["dev_run"] == run.name and pre["dev_tree_hash"] == "abc"
    assert pre["principal"] == ["bilbo-full", "bm25-ref"] and pre["primary_metric"] == "success@5"
    assert pre["secondary"] == ["bilbo-keyword", "ripgrep", "dense-ref", "random"]
    per = pre["per_stratum"]
    assert set(per) == set(stats.NOTE_STRATA + stats.OTHER_STRATA)
    assert all(v >= 20 for v in per.values())
    notes = sum(per[s] for s in stats.NOTE_STRATA)
    assert notes >= pre["n_test"]
    assert per["paraphrase"] > per["known-item"] >= per["pt-en"] or per["paraphrase"] >= per["pt-en"]
    out = capsys.readouterr().out
    assert f"n_iid {pre['n_iid']}" in out and f"n_test {pre['n_test']}" in out and "design_effect 1.300" in out


def test_power_proportional_allocation_when_large(tmp_path, monkeypatch):
    ds, run = _world(tmp_path)
    monkeypatch.setattr(stats, "design_effect", lambda *a, **k: 3.0)
    stats.cmd_power(Args(run, ds))
    pre = json.loads((ds / "preregistration.json").read_text())
    per = pre["per_stratum"]
    assert pre["n_test"] == math.ceil(pre["n_iid"] * 3.0)
    assert sum(per[s] for s in stats.NOTE_STRATA) == pre["n_test"]
    assert per["paraphrase"] == pytest.approx(pre["n_test"] * 30 / 155, abs=1)
    assert per["library"] == math.ceil(20 * pre["n_test"] / 155)


def test_power_real_design_effect_is_at_least_one(tmp_path):
    ds, run = _world(tmp_path)
    stats.cmd_power(Args(run, ds))
    pre = json.loads((ds / "preregistration.json").read_text())
    assert pre["design_effect"] >= 1.0 and pre["n_test"] >= pre["n_iid"]


def test_power_refuses_a_test_run(tmp_path):
    ds, run = _world(tmp_path, split="test")
    with pytest.raises(Refused, match="dev only"):
        stats.cmd_power(Args(run, ds))
    assert not (ds / "preregistration.json").exists()


def test_power_refuses_a_frozen_dataset(tmp_path):
    ds, run = _world(tmp_path, frozen=True)
    with pytest.raises(Refused, match="frozen"):
        stats.cmd_power(Args(run, ds))
    assert not (ds / "preregistration.json").exists()


def test_power_needs_the_principal_arms(tmp_path):
    ds, run = _world(tmp_path)
    (run / "bm25-ref" / "run.json").unlink()
    with pytest.raises(Refused, match="bm25-ref"):
        stats.cmd_power(Args(run, ds))
