"""The embedder module against the fake embedder: GGUF check, proxy, vector cache and the index check."""

from __future__ import annotations

import hashlib
import json
import urllib.request
from pathlib import Path

import numpy as np
import pytest

from bilbo_evals import embedder, passages, sandbox
from bilbo_evals.common import Refused
from bilbo_evals.embedder import Proxy, Server, VectorCache

FRONT = "---\nid: 01J{n:023d}\ncreated: 2025-06-02T10:14-03:00\n---\n\n"


def write_notes(store: Path, notes: dict[str, str]) -> None:
    (store / "notes").mkdir(parents=True, exist_ok=True)
    for n, (name, body) in enumerate(notes.items()):
        (store / "notes" / name).write_text(FRONT.format(n=n) + body)


NOTES = {
    "decision-retry-limit.md": "# Retry limit\n\nThe client retries five times.\n\n## Why\n\nBackoff storms hurt the edge cache.\n",
    "gotcha-wal-locks.md": "# WAL locks\n\n## Symptom\n\nWriters block while a reader holds a snapshot.\n\n## Fix\n\nCheckpoint more often.\n",
    # same title, same section and same text as the one above: one input, two passages
    "gotcha-wal-locks-copy.md": "# WAL locks\n\n## Fix\n\nCheckpoint more often.\n",
}


@pytest.fixture
def box(tmp_path, monkeypatch):
    monkeypatch.setenv("TMPDIR", str(tmp_path))
    for var in ("BILBO_HOME", "BILBO_CONFIG", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"):
        monkeypatch.delenv(var, raising=False)
    sb = sandbox.create("emb")
    yield sb
    sandbox.destroy(sb)


@pytest.fixture
def server(fake_embedder):
    s = Server.attach(fake_embedder.url)
    yield s
    s.stop()


def fake_bilbo(tmp_path: Path, stdout: str = "", stderr: str = "", code: int = 0) -> Path:
    script = tmp_path / "fake-bilbo"
    script.write_text(f"#!/bin/sh\nprintf '%s' '{stdout}'\nprintf '%s' '{stderr}' >&2\nexit {code}\n")
    script.chmod(0o755)
    return script


def test_server_args_are_bilbos():
    assert embedder.server_args(Path("/c/m.gguf"), 9100) == [
        "--model", "/c/m.gguf", "--alias", "qwen3-embedding-0.6b", "--embedding", "--pooling", "last",
        "--host", "127.0.0.1", "--port", "9100", "--ctx-size", "4096", "--batch-size", "4096",
        "--ubatch-size", "4096", "--parallel", "1",
    ]  # fmt: skip


def test_default_gguf_follows_the_cache_folder(monkeypatch, tmp_path):
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path / "x"))
    assert embedder.default_gguf() == tmp_path / "x/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf"
    monkeypatch.delenv("XDG_CACHE_HOME")
    assert embedder.default_gguf() == tmp_path / "home/.cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf"


def test_the_pin_is_the_one_bilbo_carries():
    source = (Path(__file__).resolve().parents[2] / "src/host/model.rs").read_text()
    assert f'sha256: "{embedder.GGUF_SHA256}"' in source
    assert f'pub const NAME: &str = "{embedder.MODEL}"' in source
    config = (Path(__file__).resolve().parents[2] / "src/shared/config.rs").read_text()
    assert json.dumps(embedder.QUERY_PREFIX)[1:-1] in config


def test_a_gguf_with_another_hash_is_refused_naming_both(tmp_path):
    gguf = tmp_path / "other.gguf"
    gguf.write_bytes(b"not the model")
    with pytest.raises(Refused) as e:
        Server.start(gguf, "llama-server-that-does-not-exist")
    assert embedder.GGUF_SHA256 in str(e.value)
    assert hashlib.sha256(b"not the model").hexdigest() in str(e.value)


def test_a_gguf_with_the_pinned_hash_passes(tmp_path, monkeypatch):
    gguf = tmp_path / "m.gguf"
    gguf.write_bytes(b"tiny")
    monkeypatch.setattr(embedder, "GGUF_SHA256", hashlib.sha256(b"tiny").hexdigest())
    assert embedder.check_gguf(gguf) == embedder.GGUF_SHA256


def test_a_missing_gguf_is_refused(tmp_path):
    with pytest.raises(Refused, match="no embedding model"):
        embedder.check_gguf(tmp_path / "nope.gguf")


def test_a_missing_llama_server_is_refused_after_the_hash(tmp_path, monkeypatch):
    gguf = tmp_path / "m.gguf"
    gguf.write_bytes(b"tiny")
    monkeypatch.setattr(embedder, "GGUF_SHA256", hashlib.sha256(b"tiny").hexdigest())
    with pytest.raises(Refused, match="llama-server-that-does-not-exist"):
        Server.start(gguf, "llama-server-that-does-not-exist")


def test_attach_embeds_and_records(server, fake_embedder):
    vectors = server.embed(["alpha", "beta"])
    assert len(vectors) == 2 and len(vectors[0]) == 64
    assert server.version == "attached"
    assert embedder.record(server) == {
        "model": "qwen3-embedding-0.6b", "gguf_sha256": None, "llama_server": "attached",
        "llama_server_version": "attached", "query_prefix": embedder.QUERY_PREFIX,
    }  # fmt: skip


def test_embed_batches_in_order(server, fake_embedder):
    texts = [f"word{i}" for i in range(40)]
    vectors = server.embed(texts)
    assert vectors[17] == server.embed(["word17"])[0]
    assert fake_embedder.inputs[1:41] == texts


def test_attach_to_nothing_is_refused(monkeypatch):
    monkeypatch.setattr(embedder, "_get", lambda *a, **k: (_ for _ in ()).throw(OSError("down")))
    with pytest.raises(Refused, match="/health"):
        embedder.Server("http://127.0.0.1:1", "attached", None, None, "attached")._ready(0)


def test_vector_cache_persists_and_ignores_a_torn_tail(tmp_path):
    cache = VectorCache(tmp_path / "v")
    cache.put("one", [0.5, 0.25])
    cache.put("two", [1.0, 2.0])
    cache.put("one", [9.0, 9.0])
    assert len(cache) == 2
    with open(tmp_path / "v/vectors.bin", "ab") as f:
        f.write(b"\x01" * 20)
    again = VectorCache(tmp_path / "v")
    assert len(again) == 2
    assert again.get("one").tolist() == [0.5, 0.25]
    assert again.get("missing") is None


def post(url: str, inputs: list[str]) -> dict:
    req = urllib.request.Request(
        f"{url}/v1/embeddings", data=json.dumps({"model": "m", "input": inputs}).encode(),
        headers={"Content-Type": "application/json"},
    )  # fmt: skip
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read())


def test_proxy_records_everything_and_forwards_once(fake_embedder, tmp_path):
    proxy = Proxy.start(fake_embedder.url, tmp_path / "cache")
    try:
        first = post(proxy.url, ["a b", "c d"])
        second = post(proxy.url, ["c d", "e f"])
    finally:
        proxy.stop()
    assert proxy.inputs() == ["a b", "c d", "c d", "e f"]
    assert fake_embedder.inputs == ["a b", "c d", "e f"]
    assert [d["index"] for d in second["data"]] == [0, 1]
    assert second["data"][0]["embedding"] == first["data"][1]["embedding"]
    assert first["data"][0]["embedding"] == pytest.approx(embedder_fake(["a b"])[0], abs=1e-6)


def embedder_fake(texts):
    from fake_embedder import embed

    return [embed(t) for t in texts]


def test_proxy_serves_a_later_proxy_from_the_cache(fake_embedder, tmp_path):
    for _ in range(2):
        proxy = Proxy.start(fake_embedder.url, tmp_path / "cache")
        post(proxy.url, ["only once"])
        proxy.stop()
    assert fake_embedder.inputs == ["only once"]
    assert VectorCache(tmp_path / "cache").get("only once").dtype == np.dtype("<f4")


def test_proxy_passes_an_upstream_failure_on(tmp_path):
    proxy = Proxy.start("http://127.0.0.1:1", tmp_path / "cache")
    try:
        with pytest.raises(urllib.error.HTTPError) as e:
            post(proxy.url, ["x"])
        assert e.value.code == 502
    finally:
        proxy.stop()


def test_index_and_check_on_a_real_bilbo(box, server, bilbo_bin, fake_embedder):
    write_notes(box.store, NOTES)
    expected = passages.inputs(box.store)
    assert len(expected) > len(set(expected))
    result = embedder.index_and_check(box, bilbo_bin, server, None)
    assert result["parity"] == "ok" and result["differences"] == []
    # `embedded` counts distinct inputs, not passages: the duplicate passage is sent once
    assert result["embedded"] == result["inputs"] == len(set(expected))
    assert sorted(set(fake_embedder.inputs) - {"probe"}) == sorted(set(expected))
    assert f"embedder.url = {server.url}" in box.config.read_text()


def test_a_second_index_keeps_everything(box, server, bilbo_bin):
    write_notes(box.store, NOTES)
    first = embedder.index_and_check(box, bilbo_bin, server, None)
    again = embedder.index_and_check(box, bilbo_bin, server, None)
    assert again["embedded"] == 0 and again["parity"] == "ok" and again["inputs"] == first["inputs"]


def test_the_queries_can_use_the_indexed_vectors(box, server, bilbo_bin):
    write_notes(box.store, NOTES)
    embedder.index_and_check(box, bilbo_bin, server, None)
    p = sandbox.bilbo(box, bilbo_bin, ["recall", "checkpoint more often", "--limit", "3"])
    assert p.exit == 0 and "not indexed" not in p.stderr and "unavailable" not in p.stderr


def test_a_withheld_passage_is_refused_with_bilbos_line(box, server, tmp_path):
    line = "withheld 2 passages from http://x: their scope allows only a loopback embedder"
    with pytest.raises(Refused) as e:
        embedder.index_and_check(box, fake_bilbo(tmp_path, "embedded 1, kept 0, dropped 0\n", f"bilbo: {line}\n"), server, None)
    assert str(e.value) == line


def test_an_index_failure_is_refused(box, server, tmp_path):
    with pytest.raises(Refused, match="exited 1: bilbo: no store"):
        embedder.index_and_check(box, fake_bilbo(tmp_path, "", "bilbo: no store", 1), server, None)


def test_inputs_bilbo_never_sent_are_a_difference(box, server, tmp_path):
    write_notes(box.store, NOTES)
    n = len(set(passages.inputs(box.store)))
    result = embedder.index_and_check(box, fake_bilbo(tmp_path, f"embedded {n}, kept 0, dropped 0"), server, None)
    assert result["parity"] == "failed"
    assert {d["side"] for d in result["differences"]} == {"harness-only"}
    assert len(result["differences"]) == n


def test_a_count_that_disagrees_with_matching_inputs_is_refused(box, server, tmp_path):
    write_notes(box.store, NOTES)
    expected = sorted(set(passages.inputs(box.store)))
    script = tmp_path / "bilbo-sends-all"
    script.write_text(
        "#!/bin/sh\ncurl -s -o /dev/null -X POST -H 'Content-Type: application/json' "
        "--data @" + str(tmp_path / "body.json") + ' "$(sed -n \'s/^embedder.url = //p\' "$BILBO_CONFIG")/v1/embeddings"\n'
        "printf 'embedded 1, kept 0, dropped 0'\n"
    )
    script.chmod(0o755)
    (tmp_path / "body.json").write_text(json.dumps({"model": "m", "input": expected}))
    with pytest.raises(Refused, match="embedded 1 and kept 0, but the store has"):
        embedder.index_and_check(box, script, server, None)


def test_vector_cache_truncates_a_torn_tail_before_the_next_append(tmp_path):
    cache = VectorCache(tmp_path / "v")
    cache.put("one", [0.5, 0.25])
    cache.put("two", [1.0, 2.0])
    path = tmp_path / "v/vectors.bin"
    whole = path.stat().st_size
    with open(path, "ab") as f:
        f.write(b"\x01" * 20)
    again = VectorCache(tmp_path / "v")
    assert path.stat().st_size == whole
    again.put("three", [3.0, 4.0])
    final = VectorCache(tmp_path / "v")
    assert len(final) == 3
    assert final.get("one").tolist() == [0.5, 0.25]
    assert final.get("two").tolist() == [1.0, 2.0]
    assert final.get("three").tolist() == [3.0, 4.0]
    assert path.stat().st_size == whole + 32 + 4 + 8
