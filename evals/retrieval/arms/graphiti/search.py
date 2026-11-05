"""Search a Graphiti store, whose dump build-store.sh --search has loaded into the fresh Neo4j this reaches, for the
questions in /work/questions.jsonl, the way the Graphiti MCP server's two search tools do: search_memory_facts
(Graphiti.search, hybrid RRF over facts) and search_nodes (hybrid RRF over entities), both in the one group `main`, with
the embedder and no LLM. Appends one row per question and form to /work/search.jsonl, facts and entities alternating
by rank. build-store.sh --search runs it in the build service."""

import asyncio
import json
import os
import re
import sys
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


async def search(graphiti, text):
    edges = await graphiti.search(text, group_ids=[GROUP_ID], num_results=LIMIT)
    nodes = (await graphiti.search_(text, config=NODE_HYBRID_SEARCH_RRF.model_copy(update={"limit": LIMIT}),
                                    group_ids=[GROUP_ID], search_filter=SearchFilters())).nodes[:LIMIT]
    facts = [{"text": fact_text(e), "ref": e.uuid, "kind": "fact", "rank": i} for i, e in enumerate(edges, 1)]
    entities = [{"text": entity_text(n), "ref": n.uuid, "kind": "entity", "rank": i} for i, n in enumerate(nodes, 1)]
    return sorted(facts + entities, key=lambda r: (r["rank"], r["kind"] != "fact"))


async def main():
    if os.environ.get("GRAPHITI_TELEMETRY_ENABLED") != "false":  # before Graphiti(), whose __init__ may send it
        sys.exit("graphiti: GRAPHITI_TELEMETRY_ENABLED is not false, so Graphiti would send telemetry")
    # A search before the restored indexes come online finds nothing and does not fail.
    async with AsyncGraphDatabase.driver(NEO4J_URI, auth=("neo4j", NEO4J_PASSWORD)) as driver:
        await driver.execute_query("CALL db.awaitIndexes(600)")
    embedder = OpenAIEmbedder(OpenAIEmbedderConfig(embedding_model="embedder", embedding_dim=1024),
                              client=AsyncOpenAI(base_url=os.environ["EMBEDDER_URL"], api_key="none"))
    graphiti = Graphiti(NEO4J_URI, "neo4j", NEO4J_PASSWORD, llm_client=NoLLM(None),
                        embedder=embedder, cross_encoder=NoReranker())
    rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
    try:
        with open("/work/search.jsonl", "a") as out:
            for i, q in enumerate(rows, 1):
                for form in ("query", "prompt"):
                    hits = await search(graphiti, q[form])
                    if not hits:
                        sys.exit(f"search: {q['id']} {form}: Graphiti found nothing")
                    out.write(json.dumps({"id": q["id"], "form": form, "results": hits}, ensure_ascii=False) + "\n")
                    out.flush()
                print(f"search: {i}/{len(rows)} {q['id']}", file=sys.stderr)
    finally:
        await graphiti.close()


asyncio.run(main())
