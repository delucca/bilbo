"""Search a Graphiti store, whose dump build-store.sh --search has loaded into the fresh Neo4j this reaches, for the
questions in /work/questions.jsonl, the way the Graphiti MCP server's two search tools do: search_memory_facts
(Graphiti.search, hybrid RRF over facts) and search_nodes (hybrid RRF over entities), with the embedder and no LLM.
The store holds every project in the one group `main`; the loaded copy is first split into one group per repository,
and each question searches its own. Appends one row per question and form to /work/search.jsonl, facts and entities
alternating by rank. build-store.sh --search runs it in the build service."""

import asyncio
import json
import os
import re
import sys
import uuid
from collections import defaultdict
from pathlib import Path

from graphiti_core import Graphiti
from graphiti_core.cross_encoder.client import CrossEncoderClient
from graphiti_core.embedder.openai import OpenAIEmbedder, OpenAIEmbedderConfig
from graphiti_core.llm_client.client import LLMClient
from graphiti_core.search.search_config_recipes import NODE_HYBRID_SEARCH_RRF
from graphiti_core.search.search_filters import SearchFilters
from neo4j import AsyncGraphDatabase
from openai import AsyncOpenAI

NEO4J_URI = os.environ["NEO4J_URI"]
NEO4J_PASSWORD = os.environ["NEO4J_PASSWORD"]
GROUP_ID = "main"
LIMIT = 100  # per tool; the MCP tools default to 10, and pack.py cuts at the budget
EPISODE_SOURCE = "Claude Code session in "
COPY = uuid.UUID("5d0f3a52-61b4-4c1e-9a43-0c6f1d6b2a10")  # namespace of the copies' uuids, so a split is repeatable
BATCH = 2000


class NoLLM(LLMClient):
    async def _generate_response(self, *args, **kwargs):
        raise RuntimeError("graphiti: the search never calls the LLM")


class NoReranker(CrossEncoderClient):
    async def rank(self, query, passages):
        raise RuntimeError("graphiti: the search reranks with RRF, never a cross-encoder")


def one_line(text):
    return re.sub(r"\s+", " ", text or "").strip()


def fact_text(edge):
    """The fact with the validity the MCP server returns beside it, so an agent can tell a superseded fact."""
    span = [f"valid from {edge.valid_at.date()}" if edge.valid_at else "",
            f"invalid since {edge.invalid_at.date()}" if edge.invalid_at else ""]
    span = "; ".join(s for s in span if s)
    return f"{one_line(edge.fact)} [{span}]" if span else one_line(edge.fact)


def entity_text(node):
    kinds = [label for label in node.labels if label != "Entity"]
    head = f"{one_line(node.name)} ({', '.join(kinds)})" if kinds else one_line(node.name)
    return f"{head}: {one_line(node.summary)}" if one_line(node.summary) else head


def group_of(source_description):
    """A repository's group: its corpus key (owner__name), which Graphiti's group-id check accepts."""
    if not source_description.startswith(EPISODE_SOURCE):
        sys.exit(f"graphiti: an episode's source is not a session: {source_description!r}")
    return source_description.removeprefix(EPISODE_SOURCE).replace("/", "__", 1)


def copy_uuid(original, group):
    return str(uuid.uuid5(COPY, f"{original}/{group}"))


async def write(driver, query, rows):
    for i in range(0, len(rows), BATCH):
        await driver.execute_query(query, rows=rows[i:i + BATCH])


async def split(driver):
    """Split the one group into one per repository, as if each had its own graph. An episode goes to its session's
    repository; an entity to every repository whose episodes mention it or whose facts touch it, one copy each
    (an entity merged across repositories at build time keeps its one summary in every copy); a fact to the
    repositories of the episodes it cites, between that repository's copies of its entities, citing only its episodes.
    Returns how many entities and facts several repositories share."""
    records, _, _ = await driver.execute_query(
        "MATCH (e:Episodic) RETURN e.uuid AS uuid, e.source_description AS source, e.valid_at AS at, e.group_id AS g")
    if {r["g"] for r in records} != {GROUP_ID}:
        sys.exit(f"graphiti: the store's episodes are not all in the group {GROUP_ID}")
    episode = {r["uuid"]: group_of(r["source"]) for r in records}
    at = {r["uuid"]: r["at"] for r in records}
    mentions, _, _ = await driver.execute_query(
        "MATCH (e:Episodic)-[m:MENTIONS]->(n:Entity) RETURN m.uuid AS uuid, e.uuid AS episode, n.uuid AS node")
    facts, _, _ = await driver.execute_query(
        "MATCH (a:Entity)-[r:RELATES_TO]->(b:Entity) RETURN r.uuid AS uuid, a.uuid AS a, b.uuid AS b, r.episodes AS episodes")
    entities, _, _ = await driver.execute_query("MATCH (n:Entity) RETURN n.uuid AS uuid, labels(n) AS labels")

    # The first repository to mention an entity keeps the original; the others get copies.
    first = {}
    for m in sorted(mentions, key=lambda m: (at[m["episode"]], m["episode"])):
        first.setdefault(m["node"], episode[m["episode"]])
    groups = defaultdict(set)
    for m in mentions:
        groups[m["node"]].add(episode[m["episode"]])
    fact_groups = {}
    for f in facts:
        cited = sorted({episode[e] for e in f["episodes"] or [] if e in episode})
        if not cited:
            sys.exit(f"graphiti: the fact {f['uuid']} cites no episode of the store")
        fact_groups[f["uuid"]] = cited
        for g in cited:
            groups[f["a"]].add(g)
            groups[f["b"]].add(g)
    unplaced = [e["uuid"] for e in entities if not groups[e["uuid"]]]
    if unplaced:
        sys.exit(f"graphiti: {len(unplaced)} entities are mentioned by no episode and touched by no fact, e.g. {unplaced[0]}")
    owner = {n: first.get(n, min(gs)) for n, gs in groups.items()}

    def node_in(n, g):
        return n if owner[n] == g else copy_uuid(n, g)

    await write(driver, "UNWIND $rows AS r MATCH (e:Episodic {uuid: r.uuid}) SET e.group_id = r.g",
                [{"uuid": u, "g": g} for u, g in episode.items()])
    await write(driver, "UNWIND $rows AS r MATCH (n:Entity {uuid: r.uuid}) SET n.group_id = r.g",
                [{"uuid": n, "g": g} for n, g in owner.items()])
    by_labels = defaultdict(list)
    for e in entities:
        for g in sorted(groups[e["uuid"]] - {owner[e["uuid"]]}):
            by_labels[tuple(sorted(e["labels"]))].append({"src": e["uuid"], "uuid": copy_uuid(e["uuid"], g), "g": g})
    for labels, rows in by_labels.items():
        if "Entity" not in labels or not all(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", label) for label in labels):
            sys.exit(f"graphiti: unexpected entity labels {labels}")
        await write(driver, f"UNWIND $rows AS r MATCH (n:Entity {{uuid: r.src}}) CREATE (c:{':'.join(labels)}) "
                            "SET c = properties(n), c.uuid = r.uuid, c.group_id = r.g", rows)

    keep, move = [], []
    for f in facts:
        cited = fact_groups[f["uuid"]]
        ends = [(g, node_in(f["a"], g), node_in(f["b"], g)) for g in cited]
        if len(cited) == 1 and ends[0][1:] == (f["a"], f["b"]):
            keep.append({"uuid": f["uuid"], "g": cited[0]})
            continue
        for i, (g, a, b) in enumerate(ends):
            mine = [e for e in f["episodes"] if episode.get(e) == g]
            move.append({"src": f["uuid"], "uuid": f["uuid"] if i == 0 else copy_uuid(f["uuid"], g), "g": g,
                         "a": a, "b": b, "episodes": mine})
    await write(driver, "UNWIND $rows AS r MATCH ()-[f:RELATES_TO {uuid: r.uuid}]->() SET f.group_id = r.g", keep)
    # Create every new fact before removing an original, since the first copy reuses its uuid.
    await write(driver, "UNWIND $rows AS r MATCH ()-[o:RELATES_TO {uuid: r.src}]->() "
                        "MATCH (a:Entity {uuid: r.a}), (b:Entity {uuid: r.b}) CREATE (a)-[f:RELATES_TO]->(b) "
                        "SET f = properties(o), f.uuid = '_' + r.uuid, f.group_id = r.g, f.episodes = r.episodes", move)
    await write(driver, "UNWIND $rows AS r MATCH ()-[o:RELATES_TO {uuid: r.uuid}]->() DELETE o",
                [{"uuid": u} for u in sorted({m["src"] for m in move})])
    await write(driver, "UNWIND $rows AS r MATCH ()-[f:RELATES_TO {uuid: '_' + r.uuid}]->() SET f.uuid = r.uuid",
                [{"uuid": m["uuid"]} for m in move])

    keep, move = [], []
    for m in mentions:
        g = episode[m["episode"]]
        target = node_in(m["node"], g)
        (keep if target == m["node"] else move).append({"uuid": m["uuid"], "g": g, "ep": m["episode"], "n": target})
    await write(driver, "UNWIND $rows AS r MATCH ()-[m:MENTIONS {uuid: r.uuid}]->() SET m.group_id = r.g", keep)
    await write(driver, "UNWIND $rows AS r MATCH (e:Episodic)-[o:MENTIONS {uuid: r.uuid}]->() "
                        "MATCH (n:Entity {uuid: r.n}) CREATE (e)-[m:MENTIONS]->(n) SET m = properties(o), m.group_id = r.g "
                        "DELETE o", move)
    await driver.execute_query(
        "MATCH (s:Saga)-[:HAS_EPISODE]->(e:Episodic) WITH s, collect(DISTINCT e.group_id) AS gs "
        "WHERE size(gs) = 1 SET s.group_id = gs[0]")
    await driver.execute_query("MATCH ()-[r:HAS_EPISODE|NEXT_EPISODE]->(e:Episodic) SET r.group_id = e.group_id")

    left, _, _ = await driver.execute_query(
        "CALL () { MATCH (n) WHERE n.group_id = $g RETURN labels(n) AS kind UNION ALL "
        "MATCH ()-[r]->() WHERE r.group_id = $g RETURN [type(r)] AS kind } RETURN kind, count(*) AS n", g=GROUP_ID)
    if left:
        sys.exit(f"graphiti: the split left items in {GROUP_ID}: {[(r['kind'], r['n']) for r in left]}")
    crossed, _, _ = await driver.execute_query(
        "MATCH (a)-[r:RELATES_TO|MENTIONS|HAS_EPISODE|NEXT_EPISODE]->(b) "
        "WHERE NOT (a.group_id = r.group_id AND b.group_id = r.group_id) RETURN count(r) AS n")
    if crossed[0]["n"]:
        sys.exit(f"graphiti: {crossed[0]['n']} relationships join two groups after the split")
    await driver.execute_query("CALL db.awaitIndexes(600)")
    return sum(len(gs) > 1 for gs in groups.values()), sum(len(gs) > 1 for gs in fact_groups.values())


async def search(graphiti, text, group):
    edges = await graphiti.search(text, group_ids=[group], num_results=LIMIT)
    nodes = (await graphiti.search_(text, config=NODE_HYBRID_SEARCH_RRF.model_copy(update={"limit": LIMIT}),
                                    group_ids=[group], search_filter=SearchFilters())).nodes[:LIMIT]
    facts = [{"text": fact_text(e), "ref": e.uuid, "kind": "fact", "rank": i} for i, e in enumerate(edges, 1)]
    entities = [{"text": entity_text(n), "ref": n.uuid, "kind": "entity", "rank": i} for i, n in enumerate(nodes, 1)]
    return sorted(facts + entities, key=lambda r: (r["rank"], r["kind"] != "fact"))


async def main():
    if os.environ.get("GRAPHITI_TELEMETRY_ENABLED") != "false":  # before Graphiti(), whose __init__ may send it
        sys.exit("graphiti: GRAPHITI_TELEMETRY_ENABLED is not false, so Graphiti would send telemetry")
    # A search before the restored indexes come online finds nothing and does not fail.
    async with AsyncGraphDatabase.driver(NEO4J_URI, auth=("neo4j", NEO4J_PASSWORD)) as driver:
        await driver.execute_query("CALL db.awaitIndexes(600)")
        entities, facts = await split(driver)
        groups = {r["g"] for r in (await driver.execute_query("MATCH (e:Episodic) RETURN DISTINCT e.group_id AS g"))[0]}
    print(f"search: split into {len(groups)} groups; {entities} entities and {facts} facts are in several", file=sys.stderr)
    embedder = OpenAIEmbedder(OpenAIEmbedderConfig(embedding_model="embedder", embedding_dim=1024),
                              client=AsyncOpenAI(base_url=os.environ["EMBEDDER_URL"], api_key="none"))
    graphiti = Graphiti(NEO4J_URI, "neo4j", NEO4J_PASSWORD, llm_client=NoLLM(None),
                        embedder=embedder, cross_encoder=NoReranker())
    rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
    missing = sorted({q["repo"] for q in rows} - groups)
    if missing:
        sys.exit(f"graphiti: no group for {', '.join(missing)}")
    try:
        with open("/work/search.jsonl", "a") as out:
            for i, q in enumerate(rows, 1):
                for form in ("query", "prompt"):
                    hits = await search(graphiti, q[form], q["repo"])
                    if not hits:
                        sys.exit(f"search: {q['id']} {form}: Graphiti found nothing")
                    out.write(json.dumps({"id": q["id"], "form": form, "results": hits}, ensure_ascii=False) + "\n")
                    out.flush()
                print(f"search: {i}/{len(rows)} {q['id']}", file=sys.stderr)
    finally:
        await graphiti.close()


asyncio.run(main())
