"""llm.py with the fake claude and codex: clean sessions, leaks, budget, limits, the codex login."""

from __future__ import annotations

import json
import os
import stat
from pathlib import Path

import pytest
from gen_e_helpers import CLAUDE_CLEAN, CLEAN_PROBE, cfg, login, make_ds, rows, use_cache

from bilbo_evals import common, llm
from bilbo_evals.common import Refused

SCHEMA = {"type": "object", "required": ["title"], "properties": {"title": {"type": "string"}}}
FAKES = Path(__file__).resolve().parent / "fakes"


@pytest.fixture(autouse=True)
def _cache(tmp_path, monkeypatch):
    use_cache(tmp_path, monkeypatch)
    login(tmp_path)


def claude_call(item="n1", attempt=1, prompt="WRITE note n1"):
    return llm.Call("notes", item, "claude", prompt, SCHEMA, attempt)


def codex_call(item="q1", attempt=1, prompt="WORD query q1"):
    return llm.Call("queries", item, "codex", prompt, SCHEMA, attempt)


def test_load_config(tmp_path):
    ds = make_ds(tmp_path)
    c = llm.load_config(ds)
    assert (c.seed, c.max_calls, c.renderer_model, c.query_model, c.reasoning_effort) == (
        20261007, 200, "claude-sonnet-5-5", "gpt-6.1-sol", "low")
    assert c.prompts["test"] == c.prompts["dev"] and c.strata["dev"]["no-answer"] == 1


def test_config_missing_and_frozen(tmp_path):
    ds = tmp_path / "ds"
    ds.mkdir()
    with pytest.raises(Refused, match="config.toml is missing"):
        llm.load_config(ds)
    ds = make_ds(tmp_path, "v2")
    (ds / "FROZEN").write_text("x\n")
    with pytest.raises(Refused, match="is frozen"):
        llm.load_config(ds)


def test_claude_call_is_clean_and_logged(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "WRITE", "output": {"title": "T"}}])
    out = llm.call(claude_call(), ds, cfg(ds))
    assert out == {"title": "T"}
    (logged,) = fake_llm.calls()
    argv = logged["argv"]
    assert argv[:3] == ["-p", "--model", "claude-sonnet-5-5"]
    for flag in ("--strict-mcp-config", "--disable-slash-commands", "--no-session-persistence", "--verbose"):
        assert flag in argv
    assert argv[argv.index("--tools") + 1] == "" and argv[argv.index("--setting-sources") + 1] == "project"
    assert json.loads(argv[argv.index("--json-schema") + 1]) == SCHEMA
    assert logged["env"]["CLAUDE_CODE_DISABLE_AUTO_MEMORY"] == "1"
    (row,) = rows(ds)
    assert row["call_id"] == "notes/n1/1" and row["status"] == "ok" and row["cli"] == "claude"
    assert row["cli_version"] == "fake 0.0.0" and row["model"] == "claude-sonnet-5-5" and row["params"] == {}
    assert (ds / "generation" / row["prompt_file"]).read_text() == "WRITE note n1"
    assert row["prompt_sha256"] == common.sha256_bytes(b"WRITE note n1")
    assert json.loads((ds / "generation" / row["output_file"]).read_text()) == {"title": "T"}
    assert (ds / "generation/schemas" / f"{row['schema_sha256']}.json").is_file()
    assert (row["tokens_in"], row["tokens_out"]) == (10, 5)
    assert llm.output_path(ds, "notes", "n1", 1) == ds / "generation/outputs/notes/n1.1.json"


def test_log_and_dataset_hold_no_home_path(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "WRITE", "output": {"title": "T"}}])
    llm.call(claude_call(), ds, cfg(ds))
    home = os.environ["HOME"]
    for p in ds.rglob("*"):
        if p.is_file():
            assert home not in p.read_text(), p
    with pytest.raises(Refused, match="home path"):
        llm.call(claude_call("n2", prompt=f"WRITE in {home}/x"), ds, cfg(ds))


def test_output_with_a_home_path_is_invalid(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "WRITE", "output": {"title": f"see {os.environ['HOME']}/x"}}])
    with pytest.raises(llm.CallFailed):
        llm.call(claude_call(), ds, cfg(ds))
    assert [r["status"] for r in rows(ds)] == ["invalid", "invalid"]
    assert not llm.output_path(ds, "notes", "n1", 1).exists()


@pytest.mark.parametrize("leak,word", [("plugin", "bilbo"), ("hook", "hook"), ("mcp", "MCP"), ("tool_use", "tools")])
def test_a_leak_refuses_the_step_before_any_generation_call(tmp_path, fake_llm, leak, word):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "preflight probe", "output": CLAUDE_CLEAN, "leak": leak}])
    with pytest.raises(Refused, match=word):
        llm.preflight("claude", ds, cfg(ds), "notes")
    assert len(fake_llm.calls()) == 1
    assert rows(ds, "preflight.jsonl")[0]["status"] == "refused"
    assert [r["status"] for r in rows(ds)] == ["refused"]
    assert not list((ds / "generation/outputs").rglob("*.json"))


def test_a_claude_probe_that_reports_instructions_refuses_before_any_generation_call(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([
        {"match": "preflight probe", "output": {"user_instructions_first_heading": "Global Agent Instructions"}},
        {"match": "WRITE", "output": {"title": "T"}},
    ])
    with pytest.raises(Refused, match="Global Agent Instructions"):
        llm.preflight("claude", ds, cfg(ds), "notes")
    assert len(fake_llm.calls()) == 1
    assert rows(ds, "preflight.jsonl")[0]["status"] == "refused"


def test_a_leak_on_a_later_call_stops_the_step(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([
        {"match": "preflight probe", "output": CLAUDE_CLEAN},
        {"match": "WRITE note n2", "output": {"title": "x"}, "leak": "plugin"},
        {"match": "WRITE", "output": {"title": "ok"}},
    ])
    llm.preflight("claude", ds, cfg(ds), "notes")
    assert rows(ds, "preflight.jsonl")[0]["status"] == "ok"
    calls = [claude_call("n1", prompt="WRITE note n1"), claude_call("n2", prompt="WRITE note n2")]
    results = llm.run_many(calls, ds, _serial(ds))
    assert results["n1"] == {"title": "ok"}
    assert isinstance(results["n2"], llm.SessionLeak) and "bilbo" in str(results["n2"])


def _serial(ds):
    c = cfg(ds)
    c.concurrency = 1
    return c


def test_missing_cli(tmp_path, monkeypatch):
    empty = tmp_path / "empty"
    empty.mkdir()
    monkeypatch.setenv("PATH", str(empty))
    with pytest.raises(Refused, match="`codex` is missing"):
        llm.require_cli("codex")


def test_budget_stops_before_the_call_over_it(tmp_path, fake_llm):
    ds = make_ds(tmp_path, max_calls=3)
    fake_llm.set_script([{"match": "WRITE", "output": {"title": "T"}}])
    c = cfg(ds)
    for i in range(3):
        llm.call(claude_call(f"n{i}", prompt=f"WRITE {i}"), ds, c)
    with pytest.raises(llm.BudgetExhausted, match="budget of 3"):
        llm.call(claude_call("n3", prompt="WRITE 3"), ds, c)
    assert len(rows(ds)) == 3 and len(fake_llm.calls()) == 3


def test_budget_stops_run_many_and_keeps_what_it_wrote(tmp_path, fake_llm):
    ds = make_ds(tmp_path, max_calls=2)
    fake_llm.set_script([{"match": "WRITE", "output": {"title": "T"}}])
    c = _serial(ds)
    calls = [claude_call(f"n{i}", prompt=f"WRITE {i}") for i in range(5)]
    results = llm.run_many(calls, ds, c)
    done = [k for k, v in results.items() if isinstance(v, dict)]
    assert done == ["n0", "n1"]
    assert isinstance(llm.fatal(results), llm.BudgetExhausted)
    assert llm.failed(results) == ["n2", "n3", "n4"]
    assert len(rows(ds)) == 2
    again = llm.run_many(calls, ds, c)
    assert [k for k, v in again.items() if isinstance(v, dict)] == ["n0", "n1"]
    assert len(fake_llm.calls()) == 2


def test_run_many_skips_finished_items_and_runs_in_parallel(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "WRITE", "output": {"title": "T"}}])
    calls = [claude_call(f"n{i}", prompt=f"WRITE {i}") for i in range(6)]
    first = llm.run_many(calls[:3], ds, cfg(ds))
    assert set(first) == {"n0", "n1", "n2"}
    second = llm.run_many(calls, ds, cfg(ds))
    assert all(v == {"title": "T"} for v in second.values()) and len(second) == 6
    assert len(fake_llm.calls()) == 6


def test_an_invalid_output_is_retried_once(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([
        {"match": "WRITE", "output": {"nope": 1}, "times": 1},
        {"match": "WRITE", "output": {"title": "fixed"}},
    ])
    assert llm.call(claude_call(), ds, cfg(ds)) == {"title": "fixed"}
    assert [(r["status"], r["try"]) for r in rows(ds)] == [("invalid", 1), ("ok", 2)]
    assert rows(ds)[0]["output_file"] is None


def test_a_failing_call_is_retried_once_then_fails(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "WRITE", "output": {"title": "x"}, "exit": 1}])
    with pytest.raises(llm.CallFailed, match="notes/n1"):
        llm.call(claude_call(), ds, cfg(ds))
    assert [r["status"] for r in rows(ds)] == ["error", "error"]


def test_a_rejected_claude_window_stops_everything(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([
        {"match": "WRITE 0", "output": {"title": "x"}, "rate_limit": True},
        {"match": "WRITE", "output": {"title": "T"}},
    ])
    c = _serial(ds)
    with pytest.raises(llm.RateLimited, match="2026-10-"):
        llm.call(claude_call("n0", prompt="WRITE 0"), ds, c)
    with pytest.raises(llm.RateLimited):
        llm.call(claude_call("n1", prompt="WRITE 1"), ds, c)
    assert len(fake_llm.calls()) == 1
    assert [r["status"] for r in rows(ds)] == ["rate_limited"]


def test_codex_call_uses_a_throwaway_home_with_only_the_login(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    real = Path(os.environ["HOME"]) / ".codex/auth.json"
    fake_llm.set_script([{"match": "WORD", "output": {"title": "q"}}])
    assert llm.call(codex_call(), ds, cfg(ds)) == {"title": "q"}
    (logged,) = fake_llm.calls()
    argv = logged["argv"]
    assert argv[:5] == ["exec", "-m", "gpt-6.1-sol", "-c", 'model_reasoning_effort="low"']
    for flag in ("--ignore-user-config", "--ignore-rules", "--ephemeral", "--skip-git-repo-check", "--json"):
        assert flag in argv
    assert argv[argv.index("-s") + 1] == "read-only" and argv[-1] == "-"
    assert [argv[i + 1] for i, a in enumerate(argv) if a == "--disable"] == ["plugins", "hooks", "apps"]
    home = Path(logged["env"]["CODEX_HOME"])
    assert home != real.parent and home.name == "codex-home"
    assert [p.name for p in home.iterdir()] == ["auth.json"] and (home / "auth.json").is_symlink()
    assert (home / "auth.json").resolve() == real.resolve()
    (row,) = rows(ds)
    assert row["cli"] == "codex" and row["model"] == "gpt-6.1-sol" and row["params"] == {"reasoning_effort": "low"}


def test_codex_preflight_probe(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "preflight probe", "output": CLEAN_PROBE}])
    llm.preflight("codex", ds, cfg(ds), "queries")
    assert len(rows(ds)) == 1 and rows(ds)[0]["step"] == "preflight"
    assert rows(ds, "preflight.jsonl")[0]["status"] == "ok"


@pytest.mark.parametrize("answer", [
    {"user_instructions_first_heading": "Global Agent Instructions", "mcp_tools": []},
    {"user_instructions_first_heading": "NONE", "mcp_tools": ["bilbo_recall"]},
])
def test_codex_probe_that_reports_memory_refuses(tmp_path, fake_llm, answer):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "preflight probe", "output": answer}])
    with pytest.raises(Refused, match="probe reports"):
        llm.preflight("codex", ds, cfg(ds), "queries")
    assert rows(ds, "preflight.jsonl")[0]["status"] == "refused"


def test_codex_home_with_memory_refuses(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    home = llm._codex_home(ds)
    (home / "AGENTS.md").write_text("# mine")
    fake_llm.set_script([{"match": "preflight probe", "output": CLEAN_PROBE}])
    with pytest.raises(Refused, match="AGENTS.md"):
        llm.preflight("codex", ds, cfg(ds), "queries")
    assert fake_llm.calls() == []


def test_codex_without_a_login_refuses(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    (Path(os.environ["HOME"]) / ".codex/auth.json").unlink()
    fake_llm.set_script([{"match": "WORD", "output": {"title": "q"}}])
    with pytest.raises(Refused, match="not logged in"):
        llm.call(codex_call(), ds, cfg(ds))


def test_codex_tool_use_fails_the_call_and_discards_the_output(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "WORD", "output": {"title": "q"}, "leak": "tool_use"}])
    with pytest.raises(llm.CallFailed, match="command_execution"):
        llm.call(codex_call(), ds, cfg(ds))
    assert [r["status"] for r in rows(ds)] == ["tool_use_refused", "tool_use_refused"]
    assert not llm.output_path(ds, "queries", "q1", 1).exists()


def test_codex_usage_limit_is_a_rate_limit(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "WORD", "rate_limit": True, "exit": 1}])
    with pytest.raises(llm.RateLimited, match="usage limit"):
        llm.call(codex_call(), ds, cfg(ds))
    with pytest.raises(llm.RateLimited):
        llm.call(codex_call("q2", prompt="WORD q2"), ds, cfg(ds))
    assert len(fake_llm.calls()) == 1


def refreshing_codex(tmp_path: Path, monkeypatch, content: str) -> None:
    """A `codex` that replaces the login link with a file, as Codex does when it refreshes a token."""
    bin_dir = tmp_path / "refresh-bin"
    bin_dir.mkdir()
    script = bin_dir / "codex"
    script.write_text(
        "#!/usr/bin/env python3\n"
        "import os, sys\n"
        "if '--version' not in sys.argv:\n"
        "    link = os.path.join(os.environ['CODEX_HOME'], 'auth.json')\n"
        "    os.unlink(link)\n"
        f"    open(link, 'w').write({content!r})\n"
        f"os.execv({str(FAKES / 'codex')!r}, [{str(FAKES / 'codex')!r}] + sys.argv[1:])\n",
        encoding="utf-8",
    )
    script.chmod(script.stat().st_mode | stat.S_IXUSR)
    monkeypatch.setenv("PATH", f"{bin_dir}{os.pathsep}{os.environ['PATH']}")


def test_a_refreshed_login_is_copied_back_and_the_link_restored(tmp_path, monkeypatch, fake_llm, capsys):
    ds = make_ds(tmp_path)
    real = Path(os.environ["HOME"]) / ".codex/auth.json"
    real.chmod(0o600)
    real.write_text(json.dumps({"tokens": {"access_token": "old"}, "last_refresh": "2026-10-07T11:00:00Z"}))
    fresh = json.dumps({"tokens": {"access_token": "new"}, "last_refresh": "2026-10-07T12:00:00Z"})
    refreshing_codex(tmp_path, monkeypatch, fresh)
    fake_llm.set_script([{"match": "WORD", "output": {"title": "q"}}])
    llm.call(codex_call(), ds, cfg(ds))
    assert json.loads(real.read_text())["tokens"] == {"access_token": "new"}
    assert stat.S_IMODE(real.stat().st_mode) == 0o600
    link = llm._codex_home(ds) / "auth.json"
    assert link.is_symlink() and link.resolve() == real.resolve()
    assert "refreshed its login" in capsys.readouterr().err
    assert not [p for p in real.parent.iterdir() if p.name != "auth.json"]


def test_an_older_login_never_replaces_a_newer_one(tmp_path, monkeypatch, fake_llm, capsys):
    ds = make_ds(tmp_path)
    real = Path(os.environ["HOME"]) / ".codex/auth.json"
    real.write_text(json.dumps({"tokens": {"access_token": "mine"}, "last_refresh": "2026-10-07T11:00:00Z"}))
    before = real.read_bytes()
    refreshing_codex(tmp_path, monkeypatch, json.dumps({"tokens": {"access_token": "stale"}, "last_refresh": "2026-10-07T10:00:00Z"}))
    fake_llm.set_script([{"match": "WORD", "output": {"title": "q"}}])
    llm.call(codex_call(), ds, cfg(ds))
    assert real.read_bytes() == before
    link = llm._codex_home(ds) / "auth.json"
    assert link.is_symlink() and link.resolve() == real.resolve()
    assert "older auth.json" in capsys.readouterr().err


def test_a_broken_login_file_never_overwrites_the_real_one(tmp_path, monkeypatch, fake_llm):
    ds = make_ds(tmp_path)
    real = Path(os.environ["HOME"]) / ".codex/auth.json"
    before = real.read_text()
    refreshing_codex(tmp_path, monkeypatch, "not json")
    fake_llm.set_script([{"match": "WORD", "output": {"title": "q"}}])
    llm.call(codex_call(), ds, cfg(ds))
    assert real.read_text() == before
    assert (llm._codex_home(ds) / "auth.json").is_symlink()


def test_a_preflight_failure_row_holds_no_home_path(tmp_path, fake_llm):
    from bilbo_evals import dataset

    ds = make_ds(tmp_path)
    (Path(os.environ["HOME"]) / ".codex/auth.json").unlink()
    with pytest.raises(Refused, match="not logged in"):
        llm.preflight("codex", ds, cfg(ds), "queries")
    (row,) = rows(ds, "preflight.jsonl")
    assert "~/.codex/auth.json" in row["reason"] and os.environ["HOME"] not in row["reason"]
    assert dataset._home_problems(ds) == []


def test_the_log_row_is_written_before_the_output(tmp_path, fake_llm, monkeypatch):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "WRITE", "output": {"title": "T"}}])
    real_write = llm.write_json

    def interrupted(path, value):
        if "outputs" in Path(path).parts:
            raise KeyboardInterrupt
        return real_write(path, value)

    monkeypatch.setattr(llm, "write_json", interrupted)
    with pytest.raises(KeyboardInterrupt):
        llm.call(claude_call(), ds, cfg(ds))
    (row,) = rows(ds)
    assert row["output_file"] == "outputs/notes/n1.1.json" and row["status"] == "ok"
    assert not llm.output_path(ds, "notes", "n1", 1).exists()
    monkeypatch.setattr(llm, "write_json", real_write)
    assert llm.call(claude_call(), ds, cfg(ds)) == {"title": "T"}
    assert len(fake_llm.calls()) == 2


def test_a_codex_login_that_did_not_change_is_left_alone(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    real = Path(os.environ["HOME"]) / ".codex/auth.json"
    mtime = real.stat().st_mtime_ns
    fake_llm.set_script([{"match": "WORD", "output": {"title": "q"}}])
    llm.call(codex_call(), ds, cfg(ds))
    assert real.stat().st_mtime_ns == mtime


def test_the_cli_env_drops_memory_variables(monkeypatch):
    for name in ("CLAUDECODE", "CLAUDE_CONFIG_DIR", "ANTHROPIC_API_KEY", "CODEX_HOME", "BILBO_HOME"):
        monkeypatch.setenv(name, "x")
    monkeypatch.setenv("KEEP_ME", "1")
    env = llm._env({"CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1"})
    assert not [k for k in env if k.startswith(llm.ENV_STRIP) and k != "CLAUDE_CODE_DISABLE_AUTO_MEMORY"]
    assert env["KEEP_ME"] == "1" and env["CLAUDE_CODE_DISABLE_AUTO_MEMORY"] == "1"


def test_a_pool_call_keeps_its_prompt_in_the_cache_not_in_the_dataset(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "WORD", "output": {"title": "q"}}])
    c = llm.Call("pool", "q1", "codex", "WORD pool prompt", SCHEMA, 1)
    assert c.prompt_in_cache and not codex_call().prompt_in_cache
    assert llm.call(c, ds, cfg(ds)) == {"title": "q"}
    assert not (ds / "generation/prompts").exists()
    (row,) = rows(ds)
    assert row["prompt_file"] is None and row["prompt_sha256"] == common.sha256_bytes(b"WORD pool prompt")
    cached = llm.cached_prompt_path(ds, row["prompt_sha256"])
    assert common.CACHE_DIR in cached.parents
    assert common.sha256_bytes(cached.read_bytes()) == row["prompt_sha256"]


def test_codex_builtin_tools_are_allowed_in_the_probe(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    tools = ["clock__curr_time", "clock.sleep", "image_gen__imagegen", "web__run"]
    fake_llm.set_script([{"match": "preflight probe", "output": {"user_instructions_first_heading": "NONE", "mcp_tools": tools}}])
    llm.preflight("codex", ds, cfg(ds), "queries")
    assert rows(ds, "preflight.jsonl")[0]["status"] == "ok"


def test_any_other_codex_tool_still_fails_closed(tmp_path, fake_llm):
    ds = make_ds(tmp_path)
    fake_llm.set_script([{"match": "preflight probe", "output": {"user_instructions_first_heading": "NONE", "mcp_tools": ["web__run", "bilbo_recall"]}}])
    with pytest.raises(Refused, match="bilbo_recall"):
        llm.preflight("codex", ds, cfg(ds), "queries")
