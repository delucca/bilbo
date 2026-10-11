"""Check a Graphiti store unpacked in /work/store, whose dump build-store.sh --check has loaded into the fresh Neo4j this
reaches: the search indexes come online, the graph matches the manifest's counts, every episode sits in one saga, whose
first and last episode are its earliest and latest, the stored vectors of a sample of entities and facts match what the
embedder gives for their text, and four searches through graphiti-core, with the embedder and no LLM, find facts and
entities. build-store.sh --check runs it in the build service."""

import asyncio
import json
import os
import sys
from collections import Counter
from pathlib import Path

import numpy as np
from graphiti_core import Graphiti
from graphiti_core.cross_encoder.client import CrossEncoderClient
from graphiti_core.embedder.openai import OpenAIEmbedder, OpenAIEmbedderConfig
from graphiti_core.llm_client.client import LLMClient
from graphiti_core.search.search_config_recipes import COMBINED_HYBRID_SEARCH_RRF
from neo4j import AsyncGraphDatabase
from openai import AsyncOpenAI

STORE = Path("/work/store")
NEO4J_URI = os.environ["NEO4J_URI"]
NEO4J_PASSWORD = os.environ["NEO4J_PASSWORD"]
GROUP_ID = "main"
INDEXES = ("episode_content", "node_name_and_summary", "community_name", "edge_name_and_fact")
QUERIES = ("what was decided about the build", "which bug was fixed", "how are tests run", "what did the user prefer")
SAMPLE = 20  # entities and facts whose vectors are embedded again
manifest = json.loads((STORE / "manifest.json").read_text())


class NoLLM(LLMClient):
    """Searching never calls the LLM; Graphiti's default client would need an OpenAI key."""

    async def _generate_response(self, *args, **kwargs):
        raise RuntimeError("graphiti: the check never calls the LLM")


class NoReranker(CrossEncoderClient):
    """The searches rerank with RRF; Graphiti's default reranker would need an OpenAI key."""

    async def rank(self, query, passages):
        raise RuntimeError("graphiti: the check never reranks with a cross-encoder")


async def rows(driver, query, **params):
    records, _, _ = await driver.execute_query(query, **params)
    return records


async def restored_indexes():
    """The indexes the dump brought back, read before graphiti-core connects: it creates any missing one."""
    async with AsyncGraphDatabase.driver(NEO4J_URI, auth=("neo4j", NEO4J_PASSWORD)) as driver:
        await driver.execute_query("CALL db.awaitIndexes(600)")
        records, _, _ = await driver.execute_query("SHOW INDEXES YIELD name, state RETURN name, state")
    return {r["name"]: r["state"] for r in records}


async def vectors(graphiti, match, text, vector):
    """The lowest cosine between a sample's stored vectors and the embedder's for the same text."""
    sample = await rows(graphiti.driver, f"{match} RETURN x.{text} AS text, x.{vector} AS v ORDER BY x.uuid LIMIT $n", n=SAMPLE)
    cosines = []
    for r in sample:
        a, b = np.array(r["v"]), np.array(await graphiti.embedder.create(input_data=[r["text"]]))
        cosines.append(a @ b / np.linalg.norm(a) / np.linalg.norm(b))
    return len(sample), float(np.min(cosines, initial=1.0))


async def main():
    if os.environ.get("GRAPHITI_TELEMETRY_ENABLED") != "false":  # before Graphiti(), whose __init__ may send it
        sys.exit("graphiti: GRAPHITI_TELEMETRY_ENABLED is not false, so Graphiti would send telemetry")
    states = await restored_indexes()
    embedder = OpenAIEmbedder(OpenAIEmbedderConfig(embedding_model="embedder", embedding_dim=1024),
                              client=AsyncOpenAI(base_url=os.environ["EMBEDDER_URL"], api_key="none"))
    graphiti = Graphiti(NEO4J_URI, "neo4j", NEO4J_PASSWORD, llm_client=NoLLM(None),
                        embedder=embedder, cross_encoder=NoReranker())
    driver = graphiti.driver
    try:
        [server] = await rows(driver, "CALL dbms.components() YIELD versions RETURN versions[0] AS version")
        print("indexes", len(states), "| online", sum(s == "ONLINE" for s in states.values()), "| search indexes",
              {name: states.get(name) for name in INDEXES}, "| neo4j", server["version"], "manifest", manifest["versions"]["neo4j-server"])

        labels = {r["label"]: r["n"] for r in await rows(driver, "MATCH (n) UNWIND labels(n) AS label RETURN label, count(*) AS n")}
        [edges] = await rows(driver, "MATCH ()-[r:RELATES_TO]->() RETURN count(r) AS total, count(r.invalid_at) AS invalidated")
        episodes = await rows(driver, "MATCH (e:Episodic) RETURN e.name AS name, e.source_description AS source")
        by_repo = dict(Counter(r["source"].removeprefix("Claude Code session in ") for r in episodes))
        [bare] = await rows(driver, "MATCH (e:Episodic) WHERE NOT (e)-[:MENTIONS]->() RETURN count(e) AS n")
        groups = {r["g"] for r in await rows(driver, "MATCH (n) WHERE n.group_id IS NOT NULL RETURN DISTINCT n.group_id AS g")}
        groups |= {r["g"] for r in await rows(driver, "MATCH ()-[r]->() WHERE r.group_id IS NOT NULL RETURN DISTINCT r.group_id AS g")}
        print("episodes", len(episodes), "manifest", manifest["episodes"]["total"], "| distinct names", len({r["name"] for r in episodes}),
              "| by repo matches", by_repo == manifest["episodes"]["by_repo"], "| without entities", bare["n"],
              "manifest", manifest["episodes"]["without_entities"], "| groups", sorted(groups))
        [nodes] = await rows(driver, "MATCH (n) RETURN count(n) AS n")
        print("nodes", nodes["n"], "manifest", manifest["nodes"]["total"], "| labels match", labels == manifest["nodes"]["by_label"], "| edges", edges.data(), "manifest", manifest["edges"])

        [sagas] = await rows(driver, "MATCH (s:Saga) RETURN count(s) AS n")
        [unsaga] = await rows(driver, "MATCH (e:Episodic) OPTIONAL MATCH (s:Saga)-[:HAS_EPISODE]->(e) WITH e, count(s) AS n WHERE n <> 1 RETURN count(e) AS n")
        [chain] = await rows(driver, "MATCH (:Episodic)-[r:NEXT_EPISODE]->(:Episodic) RETURN count(r) AS n")
        [ends] = await rows(driver, "MATCH (s:Saga)-[:HAS_EPISODE]->(e:Episodic) WITH s, e ORDER BY e.valid_at, e.created_at WITH s, collect(e.uuid) AS ids "
                            "WHERE coalesce(s.first_episode_uuid, '') <> ids[0] OR coalesce(s.last_episode_uuid, '') <> ids[-1] RETURN count(s) AS n")
        print("sagas", sagas["n"], "manifest", manifest["sagas"], "sessions", manifest["sessions"], "| episodes not in one saga", unsaga["n"],
              "| next-episode links", chain["n"], "expected", len(episodes) - sagas["n"], "| sagas with another first or last episode", ends["n"])

        names = await vectors(graphiti, "MATCH (x:Entity)", "name", "name_embedding")
        facts = await vectors(graphiti, "MATCH ()-[x:RELATES_TO]->()", "fact", "fact_embedding")
        print(f"vectors embedded again: {names[0]} entities, lowest cosine {names[1]:.4f} | {facts[0]} facts, lowest cosine {facts[1]:.4f}")

        found = {}
        for q in QUERIES:
            results = await graphiti.search_(q, config=COMBINED_HYBRID_SEARCH_RRF, group_ids=[GROUP_ID])
            found[q] = (len(results.edges), len(results.nodes), len(results.episodes))
        print("searches (facts, entities, episodes):", found)
    finally:
        await graphiti.close()

    ok = (all(s == "ONLINE" for s in states.values()) and all(states.get(name) == "ONLINE" for name in INDEXES)
          and server["version"] == manifest["versions"]["neo4j-server"]
          and len(episodes) == manifest["episodes"]["total"] == len({r["name"] for r in episodes})
          and by_repo == manifest["episodes"]["by_repo"] and bare["n"] == manifest["episodes"]["without_entities"]
          and groups == {GROUP_ID} and nodes["n"] == manifest["nodes"]["total"] and labels == manifest["nodes"]["by_label"] and edges.data() == manifest["edges"]
          and sagas["n"] == manifest["sagas"] == manifest["sessions"] and unsaga["n"] == 0 and chain["n"] == len(episodes) - sagas["n"] and ends["n"] == 0
          and names[0] == min(SAMPLE, labels.get("Entity", 0)) and facts[0] == min(SAMPLE, edges["total"])
          and names[1] > 0.99 and facts[1] > 0.99 and all(e and n for e, n, _ in found.values()))
    print("OK" if ok else "FAILED")
    sys.exit(0 if ok else 1)


asyncio.run(main())
