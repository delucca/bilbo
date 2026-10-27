"""Build the Graphiti store: add every turn of every session to Graphiti as a message episode, one after another in the
order they happened across all repositories, the way Graphiti's authors say episodes are added ("each episode is added
sequentially and awaited before adding the next one"). The graph is the checkpoint: an episode is written in one
transaction, so a run that finds the first N episodes of the plan in the graph continues with the next. The database is
dumped into /work/store by build-store.sh once this exits; the manifest is written there. Graphiti writes an episode's
saga links after that transaction, so a run stopped between the two leaves the last episode partly linked: repair_saga
writes the rest."""

import asyncio
import contextvars
import json
import logging
import os
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from importlib.metadata import version
from pathlib import Path

sys.path.insert(0, "/app/mcp")

import httpx
import openai
from entity_types import ENTITY_TYPES as MCP_TYPES
from episodes import plan
from graphiti_core import Graphiti
from graphiti_core.cross_encoder.client import CrossEncoderClient
from graphiti_core.edges import HasEpisodeEdge, NextEpisodeEdge
from graphiti_core.embedder.openai import OpenAIEmbedder, OpenAIEmbedderConfig
from graphiti_core.llm_client.config import DEFAULT_MAX_TOKENS, LLMConfig, ModelSize
from graphiti_core.llm_client.errors import EmptyResponseError
from graphiti_core.llm_client.openai_generic_client import OpenAIGenericClient
from graphiti_core.nodes import EpisodeType
from graphiti_core.search.search_utils import RELEVANT_SCHEMA_LIMIT
from openai import AsyncOpenAI

# The MCP server's types in the order of mcp_server/config/config.yaml, which is the order its type_config.py passes them.
ENTITY_TYPES = {name: MCP_TYPES[name] for name in ("Preference", "Requirement", "Procedure", "Location", "Event", "Organization", "Document", "Topic", "Person", "Object")}
GROUP_ID = "main"  # the default group of Graphiti's MCP server (mcp_server/config/config.yaml)
INDEXES = ("episode_content", "node_name_and_summary", "community_name", "edge_name_and_fact")
ATTEMPTS = 10
TRANSIENT = (openai.RateLimitError, openai.InternalServerError, openai.APIConnectionError)
FALLBACK = "gpt-6.1-sol"  # a call the model's safeguards refuse goes here at once: every retry is one more flag on the account
ROUTED = ("MoonshotAI/kimi-cli", "malkoG/hackerspub-android", "ngngardner/readaloud")  # the repos whose text drew repeated refusals: the main-model calls of an episode whose prompts carry it go to FALLBACK, never to Claude; small calls stay (the user, 2026-10-22 and 2026-10-24)

CORPUS = Path("/corpus")
WORK = Path("/work")
STORE = WORK / "store"
PROGRESS = WORK / "progress.json"
NEO4J_URI = os.environ["NEO4J_URI"]
NEO4J_PASSWORD = os.environ["NEO4J_PASSWORD"]
EMBEDDER_URL = os.environ["EMBEDDER_URL"]
LLM_URL = os.environ["BILBO_EVAL_LLM_URL"].rstrip("/")
LLM_KEY = os.environ["BILBO_EVAL_LLM_KEY"]
LLM_MODEL = os.environ["BILBO_EVAL_LLM_MODEL"]
LLM_SMALL = os.environ.get("BILBO_EVAL_LLM_SMALL_MODEL") or LLM_MODEL
LLM_HEADERS = json.loads(os.environ.get("BILBO_EVAL_LLM_HEADERS") or "{}")

logging.basicConfig(level=logging.WARNING, stream=sys.stderr, format="graphiti: %(name)s: %(message)s")
logging.getLogger("neo4j.notifications").setLevel(logging.ERROR)  # Graphiti's queries warn about unknown properties on an empty graph

PROMPT = contextvars.ContextVar("prompt", default=None)
ROUTE = contextvars.ContextVar("route", default=None)  # the repo of the episode being added, when its prompts carry a ROUTED repo's episodes


def log(msg):
    print(f"graphiti: {msg}", file=sys.stderr, flush=True)


def retry_after(error):
    try:
        return int(float(error.response.headers.get("retry-after", 0)))
    except ValueError:  # an HTTP date: back off as usual
        return 0


def refused(error):
    """Claude's safeguards refusing a call, which the L3 proxy answers with a 502."""
    return isinstance(error, openai.InternalServerError) and "safeguards flagged this message" in str(error)


def agy_failed(error, model):
    """A call on an agy model that the L3 proxy failed with a 502: its refusals have no known text, so it goes to FALLBACK
    at once as a refusal does, never again to the same account; a usage limit (429) and an outage (503) still wait."""
    return model.startswith("agy/") and isinstance(error, openai.InternalServerError) and error.status_code == 502


def repo(episode):
    return episode.source_description.removeprefix("Claude Code session in ")


def tally(counts, table, key, **amounts):
    row = counts.setdefault(table, {}).setdefault(key, {})
    for name, amount in amounts.items():
        row[name] = row.get(name, 0) + amount


class LLMFailed(BaseException):
    """The LLM endpoint refused a call for good or failed it every attempt. A BaseException, not an Exception:
    graphiti-core catches Exception around some of its LLM calls (edge_operations.py `_extract_edge_timestamps` logs
    it and goes on), which would commit the episode without what the call adds; this passes those handlers, fails the
    episode, and a rerun resumes before it."""


class Client(OpenAIGenericClient):
    """Graphiti's client for OpenAI-compatible endpoints, with what the build needs: the small model on Graphiti's
    small calls, FALLBACK on the other calls of an episode that ROUTE names, the token counts of every call, and a wait for
    the endpoint on a usage limit or a server error."""

    def __init__(self, config, counts):
        client = AsyncOpenAI(api_key=LLM_KEY, base_url=LLM_URL, default_headers=LLM_HEADERS, max_retries=0, timeout=1800)
        super().__init__(config, client=client)
        self.counts = counts

    async def generate_response(self, *args, prompt_name=None, **kwargs):
        PROMPT.set(prompt_name)
        return await super().generate_response(*args, prompt_name=prompt_name, **kwargs)

    async def send(self, request):
        """The call and its retries: a refusal by the model's safeguards on FALLBACK at once, any other transient error again
        after a growing wait. Returns the response and the model that gave it."""
        failures = 0
        while True:
            try:
                return await self.client.chat.completions.create(**request), request["model"]
            except TRANSIENT as e:
                if (refused(e) or agy_failed(e, request["model"])) and request["model"] != FALLBACK:
                    why = f"refused by {request['model']}'s safeguards" if refused(e) else f"failed on {request['model']}"
                    log(f"LLM call {why}; sending it to {FALLBACK}")
                    tally(self.counts, "fallback", request["model"], calls=1)
                    request = request | {"model": FALLBACK}
                    failures = 0
                    continue
                failures += 1
                wait = min(60 * failures, 600)
                if isinstance(e, openai.RateLimitError):
                    wait = max(wait, retry_after(e))
                log(f"LLM call failed ({type(e).__name__}: {e}); retrying in {wait} s")
                await asyncio.sleep(wait)
                if failures == ATTEMPTS:
                    raise LLMFailed(f"the LLM failed {ATTEMPTS} times in a row")
            except openai.APIError as e:
                raise LLMFailed(f"the LLM endpoint refused the call ({type(e).__name__}: {e})") from e

    async def _generate_response(self, messages, response_model=None, max_tokens=DEFAULT_MAX_TOKENS, model_size=ModelSize.medium):
        """The parent's request, sent to the model for its size. Nothing is written before the LLM answers, so a call that
        failed is safe to send again; a usage limit can last hours, so a 429 waits as long as its Retry-After asks. An
        error no retry changes (a wrong key, model or request) and one that outlasts the attempts raise LLMFailed; an
        answer the model got wrong (empty, not JSON) is left to graphiti-core, as the published build left it."""
        model = self.small_model if model_size is ModelSize.small else self.model
        if model_size is not ModelSize.small and ROUTE.get():
            tally(self.counts, "routed", ROUTE.get(), calls=1)
            model = FALLBACK
        request = dict(
            model=model,
            messages=[{"role": m.role, "content": self._clean_input(m.content)} for m in messages if m.role in ("user", "system")],
            max_tokens=max_tokens,
            response_format=self._build_response_format(response_model),
        )
        if self.temperature is not None:
            request["temperature"] = self.temperature
        response, model = await self.send(request)
        used = response.usage.prompt_tokens or 0 if response.usage else 0
        amounts = dict(calls=1, prompt_tokens=used, completion_tokens=response.usage.completion_tokens or 0 if response.usage else 0)
        tally(self.counts, "model", model, **amounts)
        tally(self.counts, "prompt", PROMPT.get() or "unknown", **amounts)
        self.counts["max_prompt_tokens"] = max(self.counts["max_prompt_tokens"], used)
        content = response.choices[0].message.content or ""
        if not content:
            raise EmptyResponseError("LLM returned an empty response")
        return json.loads(self._strip_code_fences(content))


def exceeded(error):
    """llama.cpp's body for an input longer than the embedder's context, else None."""
    info = error.body.get("error", error.body) if isinstance(error.body, dict) else {}
    return info if info.get("type") == "exceed_context_size_error" else None


class Embedder(OpenAIEmbedder):
    """Counts its calls. llama.cpp rejects an input over the embedder's context, and the same fact would come back on
    every rerun: embed the start and end of it instead, as the mem0 arm does."""

    def __init__(self, counts):
        config = OpenAIEmbedderConfig(embedding_model="embedder", embedding_dim=1024)
        super().__init__(config, client=AsyncOpenAI(base_url=EMBEDDER_URL, api_key="none"))
        self.counts = counts

    async def fit(self, text):
        while True:
            try:
                return await super().create(text)
            except openai.BadRequestError as e:
                if not (info := exceeded(e)):
                    raise
                keep = int(len(text) * info["n_ctx"] / info["n_prompt_tokens"] * 0.9) // 2
                text = f"{text[:keep]} {text[-keep:]}"
                tally(self.counts, "embedder", "all", cuts=1)

    async def create(self, input_data):
        tally(self.counts, "embedder", "all", calls=1, texts=1)
        return await self.fit(input_data)

    async def create_batch(self, input_data_list):
        tally(self.counts, "embedder", "all", calls=1, texts=len(input_data_list))
        try:
            return await super().create_batch(input_data_list)
        except openai.BadRequestError as e:
            if not exceeded(e):
                raise
            return [await self.fit(text) for text in input_data_list]  # the error does not say which input is too long


class NoReranker(CrossEncoderClient):
    """Adding episodes never reranks; Graphiti's default reranker would need an OpenAI key."""

    async def rank(self, query, passages):
        raise RuntimeError("graphiti: the build never reranks")


def clients(counts):
    config = LLMConfig(api_key=LLM_KEY, model=LLM_MODEL, small_model=LLM_SMALL, base_url=LLM_URL, temperature=None)  # as the MCP server (mcp_server/src/config/schema.py), max_tokens as graphiti-core
    return Graphiti(NEO4J_URI, "neo4j", NEO4J_PASSWORD, llm_client=Client(config, counts), embedder=Embedder(counts), cross_encoder=NoReranker())


async def preflight(graphiti):
    """Refuse a build whose endpoint or embedder is wrong, then create the indexes once."""
    try:
        httpx.get(f"{LLM_URL}/models", headers={"Authorization": f"Bearer {LLM_KEY}"} | LLM_HEADERS, timeout=30).raise_for_status()
    except httpx.HTTPError as e:
        sys.exit(f"graphiti: the LLM endpoint {LLM_URL} does not answer: {e}")
    probe = await graphiti.embedder.client.embeddings.create(input="probe", model="embedder")
    if len(probe.data[0].embedding) != 1024:
        sys.exit(f"graphiti: the embedder answers {len(probe.data[0].embedding)} dimensions, not 1024")
    await graphiti.build_indices_and_constraints()


async def present(driver):
    """The episodes in the graph as {name: (uuid, session, valid_at)}."""
    records, _, _ = await driver.execute_query(
        "MATCH (e:Episodic {group_id: $group_id}) OPTIONAL MATCH (s:Saga)-[:HAS_EPISODE]->(e) "
        "RETURN e.name AS name, e.uuid AS uuid, s.name AS saga, e.valid_at AS valid_at",
        group_id=GROUP_ID,
    )
    return {r["name"]: (r["uuid"], r["saga"], r["valid_at"].to_native()) for r in records}


def resume_point(episodes, found):
    """How many episodes of the plan the graph holds. One written at a time, they are a prefix of the plan."""
    done = 0
    while done < len(episodes) and episodes[done].name in found:
        if found[episodes[done].name][2] != episodes[done].reference_time:
            sys.exit(f"graphiti: {episodes[done].name} is in the graph at another time than the plan's; --rebuild")
        done += 1
    stray = set(found) - {e.name for e in episodes[:done]}
    if stray:
        sys.exit(f"graphiti: the graph holds {min(stray)} but the plan has not reached it, or lacks the episode before it; --rebuild")
    return done


async def repair_saga(graphiti, episodes, found, done):
    """After an episode's transaction, Graphiti writes its saga links one by one: the saga if new, NEXT_EPISODE,
    HAS_EPISODE, then the saga's first and last episode. A run that stopped in between leaves the last episode with only
    some of them, and its saga's last episode is not yet it. Write the rest as Graphiti would have (graphiti.py
    `_process_episode_data`), keeping a NEXT_EPISODE already there."""
    if not done:
        return
    last = episodes[done - 1]
    uuid, saga, _ = found[last.name]
    node = await graphiti._get_or_create_saga(last.session, GROUP_ID, last.reference_time)
    if node.last_episode_uuid == uuid:
        return
    log(f"repair: {last.name} lacks some of its saga links")
    driver, now = graphiti.driver, datetime.now(timezone.utc)
    if not saga:
        previous = await graphiti._saga_get_previous_episode_uuid(node.uuid, uuid)
        linked, _, _ = await driver.execute_query("MATCH ()-[r:NEXT_EPISODE]->(:Episodic {uuid: $uuid}) RETURN r", uuid=uuid)
        if previous and not linked:
            await NextEpisodeEdge(source_node_uuid=previous, target_node_uuid=uuid, group_id=GROUP_ID, created_at=now).save(driver)
        await HasEpisodeEdge(source_node_uuid=node.uuid, target_node_uuid=uuid, group_id=GROUP_ID, created_at=now).save(driver)
    node.first_episode_uuid = node.first_episode_uuid or uuid
    node.last_episode_uuid = uuid
    await node.save(driver)


def save_progress(progress):
    """Replace progress.json whole and durably: the file and the rename are on disk before the build goes on."""
    tmp = PROGRESS.with_suffix(".tmp")
    with tmp.open("w") as f:
        f.write(json.dumps(progress, indent=2) + "\n")
        f.flush()
        os.fsync(f.fileno())
    tmp.rename(PROGRESS)
    folder = os.open(PROGRESS.parent, os.O_RDONLY)
    try:
        os.fsync(folder)
    finally:
        os.close(folder)


async def ingest(graphiti, episodes, done, progress):
    last, started = {}, time.monotonic()  # a session's latest uuid; after a resume Graphiti looks it up
    for n, episode in enumerate(episodes[done:], done + 1):
        # add_episode puts the group's last RELEVANT_SCHEMA_LIMIT episodes in its prompts: the plan's, as it is in time order
        context = episodes[max(0, n - 1 - RELEVANT_SCHEMA_LIMIT):n]
        ROUTE.set(repo(episode) if any(repo(e) in ROUTED for e in context) else None)
        try:
            result = await graphiti.add_episode(
                name=episode.name,
                episode_body=episode.body,
                source_description=episode.source_description,
                reference_time=episode.reference_time,
                source=EpisodeType.message,
                group_id=GROUP_ID,
                entity_types=ENTITY_TYPES,
                saga=episode.session,
                saga_previous_episode_uuid=last.get(episode.session),
            )
        except (Exception, LLMFailed) as e:
            sys.exit(f"graphiti: {episode.name} failed after {n - 1} episodes ({type(e).__name__}: {e}); run again to resume")
        last[episode.session] = result.episode.uuid
        progress["episodes"] = n
        progress["ingest_seconds"] += time.monotonic() - started
        started = time.monotonic()
        save_progress(progress)
        if n % 10 == 0 or n == len(episodes):
            calls = sum(row["calls"] for row in progress["counts"]["model"].values())
            log(f"{n}/{len(episodes)} episodes, {calls} LLM calls so far")


async def check(driver, episodes):
    """Refuse a graph that does not hold exactly the plan, or whose search indexes are not ready."""
    records, _, _ = await driver.execute_query(
        "MATCH (e:Episodic) WITH e.group_id AS g, e.name AS name, count(*) AS n "
        "RETURN g, count(name) AS names, sum(n) AS episodes"
    )
    held = [(r["g"], r["names"], r["episodes"]) for r in records]
    if held != [(GROUP_ID, len(episodes), len(episodes))]:
        sys.exit(f"graphiti: the graph holds {held} (group, distinct names, episodes), the plan {len(episodes)} episodes in {GROUP_ID}")
    records, _, _ = await driver.execute_query("SHOW INDEXES YIELD name, state RETURN name, state")
    states = {r["name"]: r["state"] for r in records}
    if bad := {name: states.get(name) for name in INDEXES if states.get(name) != "ONLINE"}:
        sys.exit(f"graphiti: search indexes are not online: {bad}")


async def rows(driver, query):
    records, _, _ = await driver.execute_query(query)
    return records


async def manifest(driver, counts, episodes, skipped, drops, progress):
    inputs = (WORK / "key.txt").read_text().splitlines()
    labels = {r["label"]: r["n"] for r in await rows(driver, "MATCH (n) UNWIND labels(n) AS label RETURN label, count(*) AS n ORDER BY label")}
    [nodes] = await rows(driver, "MATCH (n) RETURN count(n) AS n")
    [edges] = await rows(driver, "MATCH ()-[r:RELATES_TO]->() RETURN count(r) AS total, count(r.invalid_at) AS invalidated")
    [bare] = await rows(driver, "MATCH (e:Episodic) WHERE NOT (e)-[:MENTIONS]->() RETURN count(e) AS n")
    [server] = await rows(driver, "CALL dbms.components() YIELD versions RETURN versions[0] AS version")
    usage = ("calls", "prompt_tokens", "completion_tokens")
    models = counts["model"]
    return {
        "arm": "graphiti",
        "key": inputs[0],
        "inputs": inputs[1:],
        "device": os.environ["BILBO_EVAL_DEVICE"],
        "versions": {p: version(p) for p in ("graphiti-core", "openai", "neo4j", "pydantic")} | {"neo4j-server": server["version"]},
        "sessions": len({e.session for e in episodes}),
        "skipped": skipped,
        "episodes": {
            "total": len(episodes),
            "by_repo": dict(sorted(Counter(e.source_description.removeprefix("Claude Code session in ") for e in episodes).items())),
            "without_entities": bare["n"],
            "dropped_entries": dict(sorted(drops.items())),
        },
        "nodes": {"total": nodes["n"], "by_label": labels},
        "edges": {"total": edges["total"], "invalidated": edges["invalidated"]},
        "sagas": labels.get("Saga", 0),
        "llm": {"model": LLM_MODEL, "small_model": LLM_SMALL}
        | {k: sum(row.get(k, 0) for row in models.values()) for k in usage}
        | {"max_prompt_tokens": counts["max_prompt_tokens"], "unrecorded_episodes": progress.get("unrecorded_episodes", 0)}
        | {"fallback": {"model": FALLBACK, "from": dict(sorted(counts.get("fallback", {}).items()))}}
        | {"routed": {"model": FALLBACK, "repos": list(ROUTED), "context": RELEVANT_SCHEMA_LIMIT, "calls": dict(sorted(counts.get("routed", {}).items()))}}
        | {"by_model": dict(sorted(models.items())), "by_prompt": dict(sorted(counts["prompt"].items()))},
        "carried_over": progress.get("carried_over"),  # the keys the graph was begun under, oldest first: episodes, key and models
        "embedder": {"calls": 0, "texts": 0, "cuts": 0} | counts["embedder"].get("all", {}),
        "ingest_seconds": round(progress["ingest_seconds"]),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }


async def main():
    episodes, skipped, drops = plan(CORPUS, int(os.environ["BILBO_EVAL_SESSIONS"]) if os.environ.get("BILBO_EVAL_SESSIONS") else None)
    progress = json.loads(PROGRESS.read_text()) if PROGRESS.exists() else {"episodes": 0, "ingest_seconds": 0, "counts": {"model": {}, "prompt": {}, "max_prompt_tokens": 0, "embedder": {}}}
    if os.environ.get("GRAPHITI_TELEMETRY_ENABLED") != "false":  # before Graphiti(), whose __init__ may send it
        sys.exit("graphiti: GRAPHITI_TELEMETRY_ENABLED is not false, so Graphiti would send telemetry")
    graphiti = clients(progress["counts"])
    try:
        await preflight(graphiti)
        found = await present(graphiti.driver)
        done = resume_point(episodes, found)
        if done not in (progress["episodes"], progress["episodes"] + 1):  # the last episode may be written but not yet recorded
            sys.exit(f"graphiti: progress.json counts {progress['episodes']} episodes and the graph holds {done}; --rebuild")
        if done == progress["episodes"] + 1:
            # The last run stopped between this episode's write and its record. Record it before another episode is
            # written, so a second stop at the same point still resumes; its LLM calls are not in the counts.
            progress["episodes"] = done
            progress["unrecorded_episodes"] = progress.get("unrecorded_episodes", 0) + 1
            save_progress(progress)
            log(f"recorded episode {done}, written before the last stop; its LLM calls are not counted")
        if done:
            log(f"resuming after {done} episodes")
        await repair_saga(graphiti, episodes, found, done)
        await ingest(graphiti, episodes, done, progress)
        await check(graphiti.driver, episodes)
        STORE.mkdir(exist_ok=True)
        (STORE / "manifest.json").write_text(json.dumps(await manifest(graphiti.driver, progress["counts"], episodes, skipped, drops, progress), indent=2) + "\n")
    finally:
        await graphiti.close()
    log(f"done: {STORE}")


if __name__ == "__main__":
    asyncio.run(main())
