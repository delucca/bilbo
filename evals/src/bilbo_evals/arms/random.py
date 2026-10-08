"""random: a seeded shuffle of the candidates, one ranking per seed."""

from __future__ import annotations

import random

from bilbo_evals.arms import LIMIT, Context, Result, candidates


class Random:
    name = "random"

    def prepare(self, ctx: Context) -> dict:
        return {"parity": None, "index": None, "embedder": None, "bilbo_config": {}, "versions": {}}

    def rank(self, item: dict, ctx: Context) -> list[Result]:
        ids = sorted(candidates(item, ctx))
        results = []
        for seed in ctx.seeds:
            shuffled = list(ids)
            random.Random(seed).shuffle(shuffled)
            results.append(Result(shuffled[:LIMIT]))
        return results


arm = Random()
