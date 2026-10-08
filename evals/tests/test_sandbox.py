"""The sandbox gives bilbo a closed environment and the guard refuses any folder that is not the run's own."""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from bilbo_evals import embedder, sandbox
from bilbo_evals.common import Refused

BILBO_VARS = ("BILBO_HOME", "BILBO_CONFIG", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME")


@pytest.fixture
def sb(tmp_path, monkeypatch):
    monkeypatch.setenv("TMPDIR", str(tmp_path / "tmp"))
    (tmp_path / "tmp").mkdir()
    for var in BILBO_VARS:
        monkeypatch.delenv(var, raising=False)
    box = sandbox.create("t1")
    yield box
    sandbox.destroy(box)


def test_create_lays_out_the_root(sb):
    assert sb.root.name == "bilbo-evals-t1"
    for d in (sb.home, sb.store, sb.cache, sb.data, sb.state, sb.config.parent):
        assert d.is_dir() and sb.root in d.parents
    assert sb.config == sb.root / "config/bilbo/config"


def test_env_holds_only_the_allowed_variables(sb):
    assert set(sb.env) == {"HOME", *BILBO_VARS, "PATH", "LANG", "LC_ALL", "TMPDIR"}
    assert sb.env["BILBO_HOME"] == str(sb.store) and sb.env["BILBO_CONFIG"] == str(sb.config)
    assert sb.env["XDG_CACHE_HOME"] == str(sb.cache) and sb.env["HOME"] == str(sb.home)


def test_bilbo_runs_with_exactly_that_environment(sb, monkeypatch):
    monkeypatch.setenv("ANTHROPIC_API_KEY", "secret")
    p = sandbox.bilbo(sb, Path("/usr/bin/env"), [])
    assert p.exit == 0
    assert dict(line.split("=", 1) for line in p.stdout.splitlines()) == sb.env


def test_create_refuses_an_existing_root(sb):
    with pytest.raises(Refused):
        sandbox.create("t1")


def test_config_without_an_embedder_holds_only_the_extras(sb):
    sandbox.write_config(sb, None, {"digest.log": "on"})
    assert sb.config.read_text() == "# written by bilbo-evals\ndigest.log = on\n"


def test_config_quotes_the_query_prefix(sb):
    sandbox.write_config(sb, "http://127.0.0.1:9", {"digest.min_similarity": "0.55"})
    assert sb.config.read_text().splitlines() == [
        "# written by bilbo-evals",
        "embedder.url = http://127.0.0.1:9",
        f"embedder.model = {embedder.MODEL}",
        'embedder.query_prefix = "Instruct: Given a question, retrieve notes that answer it\\nQuery: "',
        "digest.min_similarity = 0.55",
    ]


@pytest.mark.parametrize(
    "value, quoted",
    [("plain", "plain"), ("a\\b", "a\\\\b"), (" lead", '" lead"'), ("trail\t", '"trail\t"'), ('"q', '"\\"q"'),
     ("two\nlines", '"two\\nlines"'), ("", '""')],
)  # fmt: skip
def test_quote_follows_bilbo(value, quoted):
    assert sandbox.quote(value) == quoted


def test_guard_resolves_the_four_folders_inside_the_root(sb):
    got = sandbox.guard(sb)
    root = os.path.realpath(sb.root)
    assert set(got) == {"store", "config", "cache", "state"}
    assert got["store"] == os.path.join(root, "store")
    assert got["config"] == os.path.join(root, "config/bilbo/config")
    assert got["cache"] == os.path.join(root, "cache/bilbo")
    assert got["state"] == os.path.join(root, "state/bilbo")


def test_guard_refuses_a_folder_equal_to_the_users_store(sb, monkeypatch):
    monkeypatch.setenv("BILBO_HOME", str(sb.store))
    with pytest.raises(Refused, match=str(sb.store)):
        sandbox.guard(sb)


def test_guard_refuses_the_users_default_store(sb, monkeypatch, tmp_path):
    user_store = Path(os.environ["HOME"]) / ".local/share/bilbo"
    sb.env["BILBO_HOME"] = str(user_store)
    with pytest.raises(Refused, match=str(user_store)):
        sandbox.guard(sb)


def test_guard_refuses_a_folder_outside_the_root(sb, tmp_path):
    sb.env["XDG_CACHE_HOME"] = str(tmp_path / "elsewhere")
    with pytest.raises(Refused, match="outside the run root"):
        sandbox.guard(sb)


def test_guard_compares_real_paths(sb, tmp_path):
    outside = tmp_path / "outside"
    outside.mkdir()
    (sb.root / "link").symlink_to(outside)
    sb.env["BILBO_HOME"] = str(sb.root / "link/store")
    with pytest.raises(Refused, match="outside the run root"):
        sandbox.guard(sb)


def test_guard_sees_the_same_folder_through_a_symlink(sb, monkeypatch, tmp_path):
    alias = tmp_path / "alias"
    alias.symlink_to(sb.root)
    monkeypatch.setenv("BILBO_HOME", str(alias / "store"))
    with pytest.raises(Refused, match="invoking user's store"):
        sandbox.guard(sb)


def test_guard_ignores_an_unresolvable_user_folder(sb, monkeypatch):
    monkeypatch.delenv("HOME")
    assert sandbox.guard(sb)["store"].endswith("/store")


def test_bilbo_is_never_started_when_the_guard_refuses(sb, monkeypatch, tmp_path):
    marker = tmp_path / "ran"
    script = tmp_path / "bilbo"
    script.write_text(f"#!/bin/sh\ntouch {marker}\n")
    script.chmod(0o755)
    monkeypatch.setenv("BILBO_HOME", str(sb.store))
    with pytest.raises(Refused):
        sandbox.bilbo(sb, script, ["index"])
    assert not marker.exists()


def test_destroy_removes_the_root_only_when_it_is_one(sb, tmp_path):
    sandbox.destroy(sb)
    assert not sb.root.exists()
    stranger = sandbox.Sandbox(tmp_path / "keep", *(tmp_path,) * 6, {})
    (tmp_path / "keep").mkdir()
    sandbox.destroy(stranger)
    assert (tmp_path / "keep").is_dir()
