"""`generate facts`: no model calls. A seeded script turns the world into facts, notes to render, aliases and noise."""

from __future__ import annotations

import json
import math
import random
import re
import unicodedata
from datetime import datetime, timedelta, timezone
from pathlib import Path

from bilbo_evals import common, llm
from bilbo_evals.common import Refused
from bilbo_evals.generate import NOTE_STRATA, needs, read_json
from bilbo_evals.schema import NOTE_KINDS

TZ = timezone(timedelta(hours=-3))
START = datetime(2025, 4, 1, 0, 0, tzinfo=TZ)
END = datetime(2026, 9, 30, 23, 59, tzinfo=TZ)
MIN0, MIN1 = int(START.timestamp() // 60), int(END.timestamp() // 60)
DAY = 24 * 60
MARGIN = 1.15
FAMILY_CAP = 6
OMIT_SHARE = 0.12
DUP_SHARE = 0.06
PT_CLAMP = (0.15, 0.50)
CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
OMISSIONS = [
    "the reason or rationale", "who made the choice", "the alternatives that were considered",
    "when it happened or how long it took", "how the result was verified",
]
STOP = frozenset("this that with from have when then than into over under used uses using each also does should will".split())
BRIDGE = {
    "old-name": "{alias} was the previous name of {canonical}; the component is called {canonical} now.",
    "codename": "{alias} is the internal codename of {canonical}.",
    "abbreviation": "{alias} is short for {canonical}.",
}
ACTIVITIES = {
    "plan": [
        ("next-steps", "outline the next steps for this component in general terms"),
        ("rollout-outline", "outline how a change to this component would be rolled out, with no numbers"),
        ("cleanup-ideas", "list ideas for tidying this component up"),
        ("onboarding-outline", "sketch what a new teammate should learn about this component first"),
        ("testing-outline", "outline how this component could be tested better"),
        ("handover-outline", "outline what to hand over when ownership of this component changes"),
    ],
    "spec": [
        ("expected-behavior", "describe the behavior people expect of this component, in general terms"),
        ("interface-overview", "give a loose overview of how callers use this component"),
        ("requirements-overview", "summarize the broad requirements behind this component"),
        ("compatibility-notes", "note compatibility concerns around this component in general"),
        ("scope-overview", "describe what this component is and is not responsible for"),
        ("quality-goals", "describe the general quality goals of this component"),
    ],
    "design": [
        ("structure-overview", "describe the overall structure of this component"),
        ("data-flow-overview", "describe how data flows through this component, loosely"),
        ("module-layout", "describe how the code of this component is organised"),
        ("dependency-overview", "describe what this component depends on, in general terms"),
        ("extension-points", "describe where this component can be extended"),
        ("tradeoff-overview", "discuss the broad design tradeoffs of this component"),
    ],
    "decision": [
        ("direction-chosen", "record a general direction the team chose for this component, without exact values"),
        ("tooling-choice", "record a tooling choice made around this component, with no versions"),
        ("process-choice", "record a working-process choice about this component"),
        ("ownership-choice", "record who looks after this component and why, loosely"),
        ("naming-choice", "record how the team agreed to name things in this component"),
        ("scope-choice", "record what the team agreed to leave out of this component for now"),
    ],
    "gotcha": [
        ("watch-outs", "list general things to watch out for when changing this component"),
        ("common-mistakes", "describe common mistakes people make with this component, with no exact errors"),
        ("surprising-behavior", "describe surprising behavior of this component in general terms"),
        ("local-setup-traps", "describe traps when running this component locally, without exact commands"),
        ("review-pitfalls", "describe pitfalls reviewers should keep in mind for this component"),
        ("upgrade-pitfalls", "describe pitfalls when upgrading what this component runs on, without versions"),
    ],
    "research": [
        ("options-survey", "survey the general options considered for this component"),
        ("reading-list", "collect what the team read about this component, without links"),
        ("experiment-ideas", "list experiments worth running on this component"),
        ("background-notes", "give background on the problem this component solves"),
        ("comparison-notes", "compare approaches to what this component does, loosely"),
        ("open-questions", "list open questions about this component"),
    ],
    "review": [
        ("review-themes", "summarize the themes of recent reviews of this component, without specifics"),
        ("retro-notes", "write loose retrospective notes about work on this component"),
        ("feedback-summary", "summarize feedback the team gave about this component"),
        ("quality-impressions", "give general impressions of the quality of this component"),
        ("audit-themes", "summarize the themes of an informal audit of this component"),
        ("followup-themes", "list follow-ups from reviews of this component, without specifics"),
    ],
    "report": [
        ("weekly-recap", "write a weekly recap of work on this component with no figures"),
        ("status-summary", "write a status summary of this component without numbers"),
        ("milestone-recap", "recap a milestone of this component in general terms"),
        ("incident-recap", "recap a small incident that touched this component, without exact errors"),
        ("progress-notes", "write progress notes on this component with no measurements"),
        ("handoff-report", "write a handoff report on this component without exact values"),
    ],
    "reference": [
        ("where-things-live", "point to where the pieces of this component live, in general terms"),
        ("glossary", "give a short glossary of terms used around this component"),
        ("how-to-navigate", "explain how to find your way around this component"),
        ("who-to-ask", "explain which role to ask about this component"),
        ("faq", "answer a few general questions about this component"),
        ("links-overview", "describe which kinds of documents exist about this component"),
    ],
}


class DSU:
    def __init__(self):
        self.parent: dict[str, str] = {}
        self.size: dict[str, int] = {}

    def find(self, x: str) -> str:
        if x not in self.parent:
            self.parent[x], self.size[x] = x, 1
        while self.parent[x] != x:
            self.parent[x] = self.parent[self.parent[x]]
            x = self.parent[x]
        return x

    def can_join(self, members: list[str]) -> bool:
        roots = {self.find(m) for m in members}
        return len(roots) == len(members) and sum(self.size[r] for r in roots) <= FAMILY_CAP

    def join(self, members: list[str]) -> None:
        roots = [self.find(m) for m in members]
        for r in roots[1:]:
            self.parent[r] = roots[0]
            self.size[roots[0]] += self.size[r]


def fold(text: str) -> str:
    return "".join(c for c in unicodedata.normalize("NFD", text.lower()) if not unicodedata.combining(c))


def stamp(minute: int) -> str:
    return datetime.fromtimestamp(minute * 60, TZ).isoformat(timespec="minutes")


def ulid(minute: int, rng: random.Random) -> str:
    ms = minute * 60000 + rng.randrange(60000)
    head = []
    for _ in range(10):
        head.append(CROCKFORD[ms % 32])
        ms //= 32
    tail = rng.getrandbits(80)
    return "".join(reversed(head)) + "".join(CROCKFORD[(tail >> (5 * i)) & 31] for i in range(16))


def spread(total: int, caps: dict[str, int], rng: random.Random) -> dict[str, int]:
    """Share `total` one at a time over keys in a seeded order, never beyond a key's cap."""
    order = sorted(caps)
    rng.shuffle(order)
    got = dict.fromkeys(order, 0)
    while total > 0 and any(got[k] < caps[k] for k in order):
        for k in order:
            if total > 0 and got[k] < caps[k]:
                got[k] += 1
                total -= 1
    return got


def largest_remainder(total: int, weights: dict[str, int]) -> dict[str, int]:
    whole = sum(weights.values()) or 1
    raw = {k: total * v / whole for k, v in weights.items()}
    out = {k: int(v) for k, v in raw.items()}
    for k in sorted(raw, key=lambda k: (-(raw[k] - out[k]), k))[: total - sum(out.values())]:
        out[k] += 1
    return out


def sample_quantiles(q: dict, rng: random.Random) -> float:
    """A draw from a distribution given by its p10..p90 quantiles, flat beyond them."""
    pts = [(0.0, q["p10"]), (0.10, q["p10"]), (0.25, q["p25"]), (0.50, q["p50"]), (0.75, q["p75"]), (0.90, q["p90"]), (1.0, q["p90"])]
    u = rng.random()
    for (a, x), (b, y) in zip(pts, pts[1:]):
        if u <= b:
            return x if b == a else x + (y - x) * (u - a) / (b - a)
    return pts[-1][1]


def topic_for(comp_slug: str, statement: str, used: set[str]) -> str:
    parts = comp_slug.split("-")[:2]
    words = []
    for w in re.findall(r"[a-z]{4,}", fold(statement)):
        if w not in STOP and w not in parts and w not in words:
            words.append(w)
    base = "-".join(parts + words[:2])
    topic, n = base, 1
    while topic in used:
        n += 1
        topic = f"{base}-{n}"
    used.add(topic)
    return topic


def weighted_order(items: list, weight, rng: random.Random) -> list:
    """Items in a seeded order where heavier ones tend to come first (Efraimidis-Spirakis keys)."""
    keyed = [(rng.random() ** (1 / max(weight(i), 0.02)), n, i) for n, i in enumerate(items)]
    return [i for _, _, i in sorted(keyed, key=lambda t: (-t[0], t[1]))]


class Planner:
    def __init__(self, world: dict, profile: dict, cfg: llm.GenConfig):
        self.world, self.profile, self.cfg = world, profile, cfg
        self.projects = {p["slug"]: p for p in world["projects"]}
        self.kind_w = {k: profile.get("kinds", {}).get(k, 0.0) for k in NOTE_KINDS}
        share = profile.get("shares", {}).get("portuguese", 0.0)
        self.pt = min(max(share, PT_CLAMP[0]), PT_CLAMP[1])
        self.rng = lambda name: random.Random(f"{cfg.seed}:{name}")
        self.used_topics: set[str] = set()
        self.facts: list[dict] = []
        self.notes: list[dict] = []
        self.dsu = DSU()
        self.warnings: list[str] = []

    # --- splits -----------------------------------------------------------------------------------------------

    def splits(self) -> dict:
        w = self.cfg.world
        slugs = sorted(self.projects)
        dev_n = int(w["dev_projects"])
        if len(slugs) < 2 or not 0 < dev_n < len(slugs):
            raise Refused(f"{len(slugs)} projects cannot split into {dev_n} dev projects and the rest for test")
        order = slugs[:]
        self.rng("splits").shuffle(order)
        dev, test = sorted(order[:dev_n]), sorted(order[dev_n:])
        self.split_of = {**dict.fromkeys(dev, "dev"), **dict.fromkeys(test, "test")}
        p = self.cfg.prompts
        none, i = {"dev": [], "test": []}, 0
        for s in ("dev", "test"):
            for _ in range(int(p[s].get("noise", 0)) + int(p[s].get("off-topic", 0))):
                i += 1
                none[s].append(f"p-none-{i:03d}")
        return {"seed": self.cfg.seed, "dev": dev, "test": test, "none_prompts": none}

    # --- choosing facts ---------------------------------------------------------------------------------------

    def choose(self) -> None:
        need = needs(self.cfg)
        w = self.cfg.world
        gold_total = round(int(w["notes"]) * (1 - float(w["filler_share"])))
        weight = {s: sum(need[s].values()) for s in need}
        gold = {"dev": round(gold_total * weight["dev"] / ((weight["dev"] + weight["test"]) or 1))}
        gold["test"] = gold_total - gold["dev"]
        self.chosen: dict[str, list[tuple[dict, str]]] = {}
        for s in ("dev", "test"):
            slugs = sorted(p for p, v in self.split_of.items() if v == s)
            rng = self.rng(f"choose-{s}")
            pairs_by, plain_by = {}, {}
            for slug in slugs:
                facts = self.projects[slug]["candidate_facts"]
                by_key = {f["key"]: f for f in facts}
                pairs = [(by_key[f["replaces"]], f) for f in facts if f.get("replaces") in by_key]
                paired = {c["key"] for pair in pairs for c in pair}
                rng.shuffle(pairs)
                pairs_by[slug] = pairs
                plain_by[slug] = self.clustered([f for f in facts if f["key"] not in paired], rng)
            want_pairs = min(math.ceil(MARGIN * need[s]["supersession"]), int(gold[s] * 0.6) // 2)
            take = spread(want_pairs, {k: len(v) for k, v in pairs_by.items()}, rng)
            picked = {slug: [] for slug in slugs}
            for slug in slugs:
                for old, new in pairs_by[slug][: take[slug]]:
                    picked[slug] += [(old, "old"), (new, "new")]
            left = gold[s] - 2 * sum(take.values())
            plain = spread(left, {k: len(v) for k, v in plain_by.items()}, rng)
            for slug in slugs:
                picked[slug] += [(f, "plain") for f in plain_by[slug][: plain[slug]]]
            self.chosen.update(picked)
            got = sum(len(v) for v in picked.values())
            if got < gold[s]:
                self.warnings.append(f"{s}: {got} of {gold[s]} fact notes planned; the world has too few candidate facts")

    def clustered(self, facts: list[dict], rng: random.Random) -> list[dict]:
        """Facts in the order they are taken: two per component at a time, kinds alternating inside a component.

        Taking facts in runs keeps same-component facts together, which kind-filter pairs need.
        """
        by_comp: dict[str, list[dict]] = {}
        for f in weighted_order(facts, lambda f: self.kind_w.get(f["kind"], 0.0), rng):
            by_comp.setdefault(f["component"], []).append(f)
        order = sorted(by_comp)
        rng.shuffle(order)
        lists = []
        for c in order:
            seen: dict[str, int] = {}
            ranked = []
            for n, f in enumerate(by_comp[c]):
                seen[f["kind"]] = seen.get(f["kind"], 0) + 1
                ranked.append((seen[f["kind"]], n, f))
            lists.append([f for _, _, f in sorted(ranked, key=lambda t: t[:2])])
        out = []
        for r in range(max((len(x) for x in lists), default=0)):
            for items in lists:
                out += items[2 * r: 2 * r + 2]
        return out

    def build_facts(self) -> None:
        """Facts, ids, languages, the timeline and one note per fact."""
        used = self.used_topics
        rng_lang, rng_time, rng_id = self.rng("lang"), self.rng("time"), self.rng("ulid")
        by_cand: dict[tuple[str, str], dict] = {}
        for slug in sorted(self.chosen):
            comps = {c["slug"]: c for c in self.projects[slug]["components"]}
            order = {f["key"]: n for n, f in enumerate(self.projects[slug]["candidate_facts"])}
            for n, (cand, role) in enumerate(sorted(self.chosen[slug], key=lambda t: order[t[0]["key"]]), 1):
                fact = {
                    "id": f"f-{slug}-{n:03d}", "project": slug, "component": cand["component"], "family": None,
                    "kind": cand["kind"], "statement": cand["statement"], "verbatim": cand["verbatim"],
                    "lang": "pt" if rng_lang.random() < self.pt else "en", "valid_from": None,
                    "supersedes": None, "superseded_by": None, "joins": [], "kind_pair": None, "bridge": None,
                    "note_id": None, "status": "planted", "source": cand.get("source"),
                }
                fact["_role"], fact["_cand"], fact["_name"] = role, cand, comps[cand["component"]]["name"]
                by_cand[(slug, cand["key"])] = fact
                self.facts.append(fact)
        for fact in self.facts:
            if fact["_role"] == "new":
                old = by_cand[(fact["project"], fact["_cand"]["replaces"])]
                fact["supersedes"], old["superseded_by"] = old["id"], fact["id"]
                fact["lang"] = old["lang"]
                self.dsu.join([old["id"], fact["id"]])
        minutes = {}
        for fact in self.facts:
            if fact["_role"] == "new":
                continue
            if fact["_role"] == "old":
                minutes[fact["id"]] = rng_time.randint(MIN0, MIN1 - 30 * DAY)
            else:
                minutes[fact["id"]] = rng_time.randint(MIN0, MIN1)
        for fact in self.facts:
            if fact["_role"] == "new":
                old_min = minutes[fact["supersedes"]]
                minutes[fact["id"]] = rng_time.randint(old_min + 7 * DAY, MIN1)
        topics: dict[str, str] = {}
        for fact in self.facts:
            if fact["_role"] == "new":
                continue
            topics[fact["id"]] = topic_for(fact["component"], fact["statement"], used)
        for fact in self.facts:
            if fact["_role"] == "new":
                topic = f"{topics[fact['supersedes']]}-revised"
                n = 1
                while topic in used:
                    n += 1
                    topic = f"{topics[fact['supersedes']]}-revised-{n}"
                used.add(topic)
                topics[fact["id"]] = topic
        for fact in sorted(self.facts, key=lambda f: minutes[f["id"]]):
            nid = ulid(minutes[fact["id"]], rng_id)
            fact["note_id"], fact["valid_from"] = nid, stamp(minutes[fact["id"]])
            self.notes.append({
                "id": nid, "file": f"{fact['kind']}-{topics[fact['id']]}.md", "kind": fact["kind"],
                "topic": topics[fact["id"]], "project": fact["project"], "component": fact["component"],
                "lang": fact["lang"], "created": fact["valid_from"], "facts": [fact["id"]], "filler": False,
                "noise": [], "near_duplicate_of": None, "render_attempts": 1, "status": "kept",
                "omit": None, "activity": None, "_min": minutes[fact["id"]],
            })
        self.note_of = {n["id"]: n for n in self.notes}

    # --- structure over the facts ---------------------------------------------------------------------------------

    def usable(self, f: dict) -> bool:
        return f["_role"] != "old" and not f["bridge"]

    def structure(self) -> None:
        need = needs(self.cfg)
        for s in ("dev", "test"):
            slugs = sorted(p for p, v in self.split_of.items() if v == s)
            self.group(s, slugs, math.ceil(MARGIN * need[s]["multi-hop"]), "join")
            self.group(s, slugs, math.ceil(MARGIN * need[s]["kind-filter"]), "kind")
        self.fams()

    def group(self, s: str, slugs: list[str], quota: int, what: str) -> None:
        rng = self.rng(f"{what}-{s}")
        pools = {}
        for slug in slugs:
            pool = [f for f in self.facts if f["project"] == slug and self.usable(f)]
            rng.shuffle(pool)
            pools[slug] = pool
        taken: set[str] = set()
        made, stuck = 0, False
        order = list(slugs)
        rng.shuffle(order)
        while made < quota and not stuck:
            stuck = True
            for slug in order:
                if made >= quota:
                    break
                members = self.try_group(pools[slug], taken, what, rng)
                if members:
                    made += 1
                    stuck = False
                    ids = [m["id"] for m in members]
                    self.dsu.join(ids)
                    taken.update(ids)
                    for m in members:
                        if what == "join":
                            m["joins"] = [i for i in ids if i != m["id"]]
                        else:
                            m["kind_pair"] = next(i for i in ids if i != m["id"])

    def try_group(self, pool: list[dict], taken: set[str], what: str, rng: random.Random) -> list[dict] | None:
        free = [f for f in pool if f["id"] not in taken]
        for anchor in free:
            want = 3 if what == "join" and rng.random() < 0.3 else 2
            near = [f for f in free if f is not anchor]
            near.sort(key=lambda f: f["component"] != anchor["component"])
            if what == "kind":
                near = [f for f in near if f["component"] == anchor["component"] and f["kind"] != anchor["kind"]]
            members = [anchor]
            for f in near:
                if len(members) == want:
                    break
                if self.dsu.can_join([m["id"] for m in members] + [f["id"]]):
                    members.append(f)
            if len(members) >= 2:
                return members
        return None

    def fams(self) -> None:
        count: dict[str, int] = {}
        names: dict[str, str] = {}
        for f in self.facts:
            root = self.dsu.find(f["id"])
            if root not in names:
                count[f["project"]] = count.get(f["project"], 0) + 1
                names[root] = f"fam-{f['project']}-{count[f['project']]:03d}"
            f["family"] = names[root]
        self.fam_count, self.fam_names = count, names

    # --- aliases and bridge facts ---------------------------------------------------------------------------------

    def bridges(self) -> list[dict]:
        rng = self.rng("bridges")
        rows = []
        gold_by_note = {n["id"]: n for n in self.notes}
        for slug in sorted(self.projects):
            p = self.projects[slug]
            mine = [n for n in self.notes if n["project"] == slug]
            seq = sum(1 for f in self.facts if f["project"] == slug)
            for comp in p["components"]:
                for a in comp["aliases"]:
                    same = [n for n in mine if n["component"] == comp["slug"]]
                    clean = [n for n in same if not self.roles(n)]
                    pool = clean or [n for n in same if not self.old(n)] or [n for n in mine if not self.old(n)]
                    if not pool:
                        self.warnings.append(f"{slug}: no note can carry the alias {a['alias']}; it is left out")
                        continue
                    host = rng.choice(pool)
                    seq += 1
                    fid = f"f-{slug}-{seq:03d}"
                    fact = {
                        "id": fid, "project": slug, "component": comp["slug"], "family": None, "kind": host["kind"],
                        "statement": BRIDGE[a["type"]].format(alias=a["alias"], canonical=comp["name"]),
                        "verbatim": [a["alias"], comp["name"]], "lang": host["lang"], "valid_from": host["created"],
                        "supersedes": None, "superseded_by": None, "joins": [], "kind_pair": None,
                        "bridge": {"alias": a["alias"], "canonical": comp["name"]}, "note_id": host["id"],
                        "status": "planted", "source": None, "_role": "bridge", "_name": comp["name"],
                    }
                    self.facts.append(fact)
                    gold_by_note[host["id"]]["facts"].append(fid)
                    rows.append({
                        "project": slug, "component": comp["slug"], "canonical": comp["name"], "alias": a["alias"],
                        "type": a["type"], "bridge_notes": [host["id"]],
                    })
        # a bridge fact is its own family
        for f in self.facts:
            if f["bridge"]:
                self.fam_count[f["project"]] = self.fam_count.get(f["project"], 0) + 1
                f["family"] = f"fam-{f['project']}-{self.fam_count[f['project']]:03d}"
        return rows

    def roles(self, note: dict) -> bool:
        f = next(x for x in self.facts if x["id"] == note["facts"][0])
        return bool(f["supersedes"] or f["superseded_by"] or f["joins"] or f["kind_pair"])

    def old(self, note: dict) -> bool:
        f = next(x for x in self.facts if x["id"] == note["facts"][0])
        return bool(f["superseded_by"])

    # --- fillers, near-duplicates, omissions --------------------------------------------------------------------

    def fillers(self) -> dict:
        rng, rng_id = self.rng("fillers"), self.rng("ulid-fillers")
        w = self.cfg.world
        total = max(0, int(w["notes"]) - len(self.notes))
        dups = min(round(DUP_SHARE * int(w["notes"])), total)
        gold = [n for n in self.notes if not self.old(n)]
        rng.shuffle(gold)
        noise = {"omission": [], "near-duplicate": [], "stale": []}
        for n in gold[:dups]:
            later = rng.randint(min(n["_min"] + DAY, MIN1), MIN1)
            topic = f"{n['topic']}-recap"
            while topic in self.used_topics:
                topic += "-2"
            self.used_topics.add(topic)
            nid = ulid(later, rng_id)
            self.add_note({
                "id": nid, "file": f"{n['kind']}-{topic}.md", "kind": n["kind"], "topic": topic,
                "project": n["project"], "component": n["component"], "lang": n["lang"], "created": stamp(later),
                "facts": [], "filler": True, "noise": ["near-duplicate"], "near_duplicate_of": n["id"],
                "render_attempts": 1, "status": "kept", "omit": None, "activity": None, "_min": later,
            })
            if "near-duplicate" not in n["noise"]:
                n["noise"].append("near-duplicate")
            noise["near-duplicate"].append([n["id"], nid])
        counts = {}
        for n in self.notes:
            if not n["filler"]:
                counts[n["project"]] = counts.get(n["project"], 0) + 1
        quota = largest_remainder(total - dups, counts)
        for slug in sorted(quota):
            comps = self.projects[slug]["components"]
            for _ in range(quota[slug]):
                comp = rng.choice(comps)
                kind = weighted_order(NOTE_KINDS, lambda k: self.kind_w.get(k, 0.0), rng)[0]
                options = [(c, a) for c in [comp, *comps] for a in ACTIVITIES[kind]]
                for c, (act_slug, act) in options:
                    topic = f"{'-'.join(c['slug'].split('-')[:2])}-{act_slug}"
                    if topic not in self.used_topics:
                        break
                else:
                    self.warnings.append(f"{slug}: no free filler topic left")
                    continue
                self.used_topics.add(topic)
                minute = rng.randint(MIN0, MIN1)
                self.add_note({
                    "id": ulid(minute, rng_id), "file": f"{kind}-{topic}.md", "kind": kind, "topic": topic,
                    "project": slug, "component": c["slug"], "lang": "pt" if rng.random() < self.pt else "en",
                    "created": stamp(minute), "facts": [], "filler": True, "noise": [], "near_duplicate_of": None,
                    "render_attempts": 1, "status": "kept", "omit": None, "activity": act, "_min": minute,
                })
        rng_o = self.rng("omission")
        for n in sorted(self.notes, key=lambda n: n["id"]):
            if n["filler"] or not n["facts"]:
                continue
            if rng_o.random() < OMIT_SHARE:
                n["omit"] = rng_o.choice(OMISSIONS)
                n["noise"].append("omission")
                noise["omission"].append(n["id"])
        for f in self.facts:
            if f["supersedes"]:
                noise["stale"].append([self.fact_by_id(f["supersedes"])["note_id"], f["note_id"]])
        return {k: sorted(v) for k, v in noise.items()}

    def add_note(self, note: dict) -> None:
        self.notes.append(note)
        self.note_of[note["id"]] = note

    def fact_by_id(self, fid: str) -> dict:
        return next(f for f in self.facts if f["id"] == fid)

    # --- style targets ------------------------------------------------------------------------------------------

    def styles(self) -> None:
        rng = self.rng("style")
        prof = self.profile
        shares = prof.get("shares", {})
        by_project: dict[str, list[dict]] = {}
        for n in self.notes:
            by_project.setdefault(n["project"], []).append(n)
        for n in sorted(self.notes, key=lambda n: n["id"]):
            chars = int(round(sample_quantiles(prof["length_chars"], rng) / 100) * 100) if "length_chars" in prof else 1500
            heads = int(round(sample_quantiles(prof["headings"], rng))) if "headings" in prof else 3
            sibling = None
            if rng.random() < shares.get("wiki_links", 0.0):
                others = [o["topic"] for o in by_project[n["project"]] if o["id"] != n["id"]]
                sibling = rng.choice(others) if others else None
            n["style"] = {
                "chars": max(chars, 400), "headings": max(heads, 1),
                "code_block": rng.random() < shares.get("code_blocks", 0.0), "wiki_link": sibling,
            }
            # most facts carry a source in the world; the profile's share decides how many notes show it
            n["use_source"] = rng.random() < min(1.0, shares.get("sources", 0.25) / 0.25)

    # --- capacity -----------------------------------------------------------------------------------------------

    def capacity(self, aliases: list[dict]) -> tuple[list[tuple[str, str, int, int]], list[str]]:
        need = needs(self.cfg)
        rows, short = [], []
        for s in ("dev", "test"):
            mine = [f for f in self.facts if self.split_of[f["project"]] == s]
            single = [f for f in mine if self.usable(f)]
            have = {
                "known-item": len(single), "paraphrase": len(single), "pt-en": len(single),
                "supersession": sum(1 for f in mine if f["supersedes"]),
                "multi-hop": len({self.dsu_group(f, "joins") for f in mine if f["joins"]}),
                "kind-filter": sum(1 for f in mine if f["kind_pair"]) // 2,
            }
            seats = 0
            for a in aliases:
                if self.split_of[a["project"]] != s:
                    continue
                seats += sum(1 for f in single if f["project"] == a["project"] and f["component"] == a["component"]
                             and f["note_id"] not in a["bridge_notes"])
            have["alias"] = seats
            for stratum in NOTE_STRATA:
                rows.append((s, stratum, need[s][stratum], have[stratum]))
                if have[stratum] < need[s][stratum]:
                    short.append(f"{s} {stratum}: {have[stratum]} < {need[s][stratum]}")
            gold = len([f for f in mine if not f["bridge"]])
            asked = max(need[s].values())
            if gold < asked:
                self.warnings.append(
                    f"{s}: {gold} facts for {asked} queries of one stratum; a fact serves one query per stratum, so the largest stratum will fall short")
        return rows, short

    def dsu_group(self, f: dict, key: str) -> str:
        return min([f["id"], *f[key]])


def render_rows(p: Planner) -> tuple[list[dict], list[dict]]:
    facts = []
    for f in sorted(p.facts, key=lambda f: f["id"]):
        facts.append({k: v for k, v in f.items() if not k.startswith("_")})
    notes = []
    for n in sorted(p.notes, key=lambda n: n["id"]):
        row = {k: v for k, v in n.items() if not k.startswith("_") and k != "use_source"}
        sources = [f["source"] for f in (p.fact_by_id(i) for i in n["facts"]) if f.get("source")] if n.get("use_source") else []
        row["sources"] = sources
        notes.append(row)
    return facts, notes


def cmd(args) -> int:
    ds = Path(args.dataset)
    cfg = llm.load_config(ds)
    world = read_json(ds / "world/world.json", "run `generate world` first")
    profile = read_json(ds / "world/profile.json", "run `generate profile` first")
    p = Planner(world, profile, cfg)
    splits = p.splits()
    p.choose()
    p.build_facts()
    p.structure()
    aliases = p.bridges()
    noise = p.fillers()
    p.styles()
    rows, short = p.capacity(aliases)
    common.out("split  stratum         need  capacity")
    for s, stratum, need, have in rows:
        common.out(f"{s:<6} {stratum:<14} {need:>5} {have:>9}")
    for w in p.warnings:
        common.err(w)
    if short:
        raise Refused("the facts are short of the strata: " + "; ".join(short)
                      + " (raise world.notes or ask the world for more facts, then rerun world and facts)")
    old = ds / "world/splits.json"
    if old.is_file():
        splits["library"] = json.loads(old.read_text(encoding="utf-8")).get("library", {"dev": [], "test": []})
    facts, notes = render_rows(p)
    w = ds / "world"
    common.write_jsonl(w / "facts.jsonl", facts)
    common.write_jsonl(w / "notes.jsonl", notes)
    common.write_json(w / "aliases.json", aliases)
    common.write_json(w / "noise.json", noise)
    common.write_json(w / "splits.json", splits)
    common.out(f"facts: {len(facts)} facts in {len(notes)} notes ({sum(1 for n in notes if n['filler'])} fillers)")
    return 0
