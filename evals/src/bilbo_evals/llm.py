"""Model calls through `claude -p` and `codex exec`: clean sessions, preflight, the call log, the budget and rate-limit stops."""

from __future__ import annotations

import atexit
import concurrent.futures
import json
import os
import shutil
import tempfile
import threading
import time
import tomllib
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Literal

from bilbo_evals import common, schema
from bilbo_evals.common import Refused, append_jsonl, sha256_bytes, write_json

CLAUDE_TIMEOUT = 900
CODEX_TIMEOUT = 600
ENV_STRIP = ("CLAUDE", "ANTHROPIC_", "CODEX_", "BILBO_")
UTILIZATION_STOP = 0.95
LIMIT_WORDS = ("usage limit", "rate limit", "rate_limit", "quota")
# Codex adds cache files of its own to the home; these are the names that would carry memory in.
CODEX_MEMORY = ("AGENTS.md", "AGENTS.override.md", "config.toml", "hooks.json", "rules", "plugins", "prompts")
PROBE_MARK = "bilbo-evals preflight probe"
CODEX_PROBE = (
    f"This is the {PROBE_MARK} (codex). Do not use any tool. Answer only from what is already in this session.\n"
    "1. If the session holds instructions from a user or a project file (such as AGENTS.md), give the text of their "
    "first heading; if it holds none, answer exactly NONE.\n"
    "2. List the names of the MCP tools you can call; if you can call none, answer an empty list."
)
CLAUDE_PROBE = f"This is the {PROBE_MARK} (claude). Do not use any tool. Reply with the JSON object {{\"ok\": true}}."
CLAUDE_PROBE_SCHEMA = {
    "type": "object", "required": ["ok"], "additionalProperties": False, "properties": {"ok": {"type": "boolean"}},
}
CODEX_PROBE_SCHEMA = json.loads((Path(__file__).resolve().parent / "generate/schemas/probe.json").read_text(encoding="utf-8"))


class BudgetExhausted(Refused):
    """The next call would pass `max_calls`."""


class RateLimited(Refused):
    """A CLI reports its usage window is used up; the message names the reset time when known."""


class SessionLeak(Refused):
    """A session loaded a plugin, hook, skill, MCP server or tool that could carry memory in."""


class CallFailed(Exception):
    """A call failed twice for a reason that is not a stop."""


@dataclass
class GenConfig:
    seed: int
    max_calls: int
    concurrency: int
    renderer_model: str
    query_model: str
    reasoning_effort: str
    world: dict = field(default_factory=dict)
    library: dict = field(default_factory=dict)
    strata: dict = field(default_factory=dict)
    prompts: dict = field(default_factory=dict)


@dataclass
class Call:
    step: str
    item: str
    cli: Literal["claude", "codex"]
    prompt: str
    schema: dict
    attempt: int = 1


# --- config -----------------------------------------------------------------------------------------------------


def require_unfrozen(ds_dir: Path) -> None:
    if (Path(ds_dir) / "FROZEN").exists():
        raise Refused(f"{ds_dir} is frozen: a change to its contents is a new version in a new folder")


def _section(raw: dict, name: str) -> dict:
    value = raw.get(name)
    if not isinstance(value, dict):
        raise Refused(f"generation/config.toml has no [{name}] table")
    return value


def load_config(ds_dir: Path) -> GenConfig:
    ds_dir = Path(ds_dir)
    require_unfrozen(ds_dir)
    path = ds_dir / "generation/config.toml"
    if not path.is_file():
        raise Refused(f"{path} is missing: copy generate/config.example.toml there and edit it")
    try:
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as e:
        raise Refused(f"{path} is not valid TOML: {e}") from e
    for key in ("seed", "max_calls", "concurrency"):
        if not isinstance(raw.get(key), int) or isinstance(raw.get(key), bool):
            raise Refused(f"generation/config.toml: {key} must be an integer")
    if raw["max_calls"] < 1 or raw["concurrency"] < 1:
        raise Refused("generation/config.toml: max_calls and concurrency must be at least 1")
    renderer, query = _section(raw, "renderer"), _section(raw, "query_model")
    for table, keys in ((renderer, ("model",)), (query, ("model", "reasoning_effort"))):
        for key in keys:
            if not isinstance(table.get(key), str) or not table[key]:
                raise Refused(f"generation/config.toml: {key} must be a non-empty string")
    prompts = dict(_section(raw, "prompts"))
    if "dev" not in prompts:
        raise Refused("generation/config.toml has no [prompts.dev] table")
    prompts.setdefault("test", dict(prompts["dev"]))
    strata = _section(raw, "strata")
    if "dev" not in strata:
        raise Refused("generation/config.toml has no [strata.dev] table")
    return GenConfig(
        seed=raw["seed"], max_calls=raw["max_calls"], concurrency=raw["concurrency"],
        renderer_model=renderer["model"], query_model=query["model"], reasoning_effort=query["reasoning_effort"],
        world=_section(raw, "world"), library=_section(raw, "library"), strata=strata, prompts=prompts,
    )


# --- paths and process state --------------------------------------------------------------------------------------


def require_cli(name: str) -> str:
    found = shutil.which(name)
    if not found:
        raise Refused(f"`{name}` is missing: it is not on PATH")
    return found


def output_path(ds_dir: Path, step: str, item: str, attempt: int) -> Path:
    return Path(ds_dir) / "generation/outputs" / step / f"{item.replace('/', '--')}.{attempt}.json"


def _raw_path(ds_dir: Path, call_id: str, tries: int) -> Path:
    ds = Path(ds_dir).resolve()
    return common.CACHE_DIR / "generation" / ds.parent.name / ds.name / "raw" / f"{call_id.replace('/', '--')}.try{tries}.jsonl"


@dataclass
class _Run:
    lock: threading.Lock = field(default_factory=threading.Lock)
    inflight: int = 0
    limited: RateLimited | None = None
    tmp: Path | None = None
    codex_home: Path | None = None
    codex_real: Path | None = None
    preflighted: set = field(default_factory=set)


_RUNS: dict[str, _Run] = {}
_RUNS_LOCK = threading.Lock()
_AUTH_LOCK = threading.Lock()
_VERSIONS: dict[str, str] = {}


def _run_state(ds_dir: Path) -> _Run:
    key = str(Path(ds_dir).resolve())
    with _RUNS_LOCK:
        return _RUNS.setdefault(key, _Run())


def _cleanup() -> None:
    for run in _RUNS.values():
        if run.tmp:
            shutil.rmtree(run.tmp, ignore_errors=True)


atexit.register(_cleanup)


def _cli_version(cli: str) -> str:
    if cli not in _VERSIONS:
        _VERSIONS[cli] = common.tool_version([cli])
    return _VERSIONS[cli]


def _rows(ds_dir: Path) -> int:
    p = Path(ds_dir) / "generation/calls.jsonl"
    if not p.is_file():
        return 0
    with open(p, encoding="utf-8") as f:
        return sum(1 for line in f if line.strip())


def _env(extra: dict[str, str] | None = None) -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if not k.startswith(ENV_STRIP)}
    env.update(extra or {})
    return env


def _write_once(path: Path, text: str) -> None:
    if path.is_file():
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(f".{path.name}.{os.getpid()}.{threading.get_ident()}.tmp")
    tmp.write_text(text, encoding="utf-8")
    os.replace(tmp, path)


def _events(stdout: str) -> list[dict]:
    out = []
    for line in stdout.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if isinstance(value, dict):
                out.append(value)
    return out


def _epoch(seconds) -> str | None:
    if isinstance(seconds, (int, float)) and not isinstance(seconds, bool):
        return datetime.fromtimestamp(seconds, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return None


# --- the claude session check ---------------------------------------------------------------------------------------


def _claude_problem(events: list[dict]) -> str | None:
    """What the session loaded that could carry memory in; None for a clean one. Fails closed on a missing key."""
    init = next((e for e in events if e.get("type") == "system" and e.get("subtype") == "init"), None)
    if init is None:
        return "the session reported no system/init event"
    for key in ("tools", "mcp_servers", "skills", "slash_commands", "plugins"):
        if not isinstance(init.get(key), list):
            return f"the session init lacks {key!r}"
    tools = [t for t in init["tools"] if t != "StructuredOutput"]
    if tools:
        return f"the session loads tools {tools}"
    if init["mcp_servers"]:
        names = [m.get("name", "?") if isinstance(m, dict) else str(m) for m in init["mcp_servers"]]
        return f"the session loads MCP servers {names}"
    if init["skills"]:
        return f"the session loads skills {init['skills']}"
    if init["slash_commands"]:
        return f"the session loads slash commands {init['slash_commands']}"
    for plugin in init["plugins"]:
        name = plugin.get("name", "?") if isinstance(plugin, dict) else str(plugin)
        if not isinstance(plugin, dict) or plugin.get("path") != "builtin" or "bilbo" in name.lower():
            return f"the session loads the plugin {name}"
    memory = init.get("memory_paths")
    if memory and any(memory.values() if isinstance(memory, dict) else memory):
        return "the session loads user memory"
    for e in events:
        if e.get("type") != "system" or not str(e.get("subtype", "")).startswith("hook_"):
            continue
        name = str(e.get("hook_name", "?"))
        if "bilbo" in name.lower():
            return f"the session runs the hook {name}"
        if e.get("subtype") == "hook_response":
            for key in ("stdout", "output"):
                if not isinstance(e.get(key), str):
                    return f"the hook {name} reported no {key}"
                if e[key].strip():
                    return f"the hook {name} printed into the session"
    return None


def _claude_limit(events: list[dict]) -> tuple[bool, bool, str | None]:
    """(rejected now, a window is nearly used, reset time) from the rate_limit_event rows."""
    rejected = near = False
    reset = None
    for e in events:
        if e.get("type") != "rate_limit_event":
            continue
        info = e.get("rate_limit_info", e)
        if not isinstance(info, dict):
            continue
        if info.get("status") not in (None, "allowed", "allowed_warning"):
            rejected = True
        reset = _epoch(info.get("resetsAt")) or reset
        windows = info.get("unifiedWindows") or info.get("windows") or {}
        for name in ("five_hour", "seven_day"):
            w = windows.get(name) if isinstance(windows, dict) else None
            u = w.get("utilization") if isinstance(w, dict) else None
            if isinstance(u, (int, float)) and not isinstance(u, bool):
                u = u / 100 if u > 1 else u
                if u >= UTILIZATION_STOP:
                    near = True
                    reset = _epoch(w.get("resetsAt")) or reset
    return rejected, near, reset


# --- executing one try -------------------------------------------------------------------------------------------


@dataclass
class _Outcome:
    status: str
    output: dict | None = None
    message: str = ""
    tokens_in: int | None = None
    tokens_out: int | None = None
    stdout: str = ""
    near_limit: str | None = None
    stop: Refused | None = None


def _int(value) -> int | None:
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def _run_claude(c: Call, cfg: GenConfig) -> _Outcome:
    argv = [
        "claude", "-p", "--model", cfg.renderer_model, "--output-format", "stream-json", "--verbose",
        "--json-schema", json.dumps(c.schema, sort_keys=True), "--tools", "", "--strict-mcp-config",
        "--setting-sources", "project", "--disable-slash-commands", "--no-session-persistence",
    ]
    env = _env({"CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1"})
    with tempfile.TemporaryDirectory(prefix="bilbo-evals-cwd-") as cwd:
        p = common.run(argv, env=env, stdin=c.prompt, cwd=Path(cwd), timeout=CLAUDE_TIMEOUT)
    events = _events(p.stdout)
    rejected, near, reset = _claude_limit(events)
    out = _Outcome("error", stdout=p.stdout)
    if rejected:
        out.status, out.message = "rate_limited", f"claude reports its usage window is used up (resets {reset or 'at an unknown time'})"
        out.stop = RateLimited(out.message)
        return out
    if not events and p.exit != 0:
        out.message = f"claude exit {p.exit}: {p.stderr.strip()[:200] or 'no output'}"
        return out
    problem = _claude_problem(events)
    if problem:
        out.status, out.message = "refused", problem
        out.stop = SessionLeak(f"{problem}; no generation call is made until it is fixed")
        return out
    if near:
        out.near_limit = reset or "an unknown time"
    result = next((e for e in reversed(events) if e.get("type") == "result"), None)
    if p.exit != 0 or result is None or result.get("is_error"):
        detail = str(result.get("result", "")) if result else p.stderr
        out.message = f"claude exit {p.exit}: {detail.strip()[:200] or 'no result event'}"
        return out
    usage = result.get("usage") if isinstance(result.get("usage"), dict) else {}
    parts = [_int(usage.get(k)) or 0 for k in ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens")]
    out.tokens_in, out.tokens_out = sum(parts), _int(usage.get("output_tokens"))
    structured = result.get("structured_output")
    if not isinstance(structured, dict):
        out.message = "the result holds no structured_output"
        return out
    out.output = structured
    return out


ALLOWED_CODEX_EVENTS = {"thread.started", "turn.started", "turn.completed"}
ALLOWED_CODEX_ITEMS = {"agent_message", "reasoning"}


def _codex_problem(events: list[dict]) -> str | None:
    for e in events:
        kind = e.get("type")
        if kind in ALLOWED_CODEX_EVENTS or kind == "error" or kind == "turn.failed":
            continue
        if kind in ("item.completed", "item.started", "item.updated"):
            item_type = (e.get("item") or {}).get("type")
            if item_type not in ALLOWED_CODEX_ITEMS:
                return f"codex used a {item_type} item"
            continue
        return f"codex emitted an unexpected {kind} event"
    return None


def _limit_text(p: common.Proc, events: list[dict]) -> str | None:
    text = (p.stderr + "\n" + "\n".join(str(e.get("message", "")) for e in events if e.get("type") in ("error", "turn.failed"))).lower()
    for line in text.splitlines():
        if any(w in line for w in LIMIT_WORDS):
            return line.strip()[:200]
    return None


def _run_codex(c: Call, cfg: GenConfig, ds_dir: Path) -> _Outcome:
    run = _run_state(ds_dir)
    home = _codex_home(ds_dir)
    out = _Outcome("error")
    try:
        with tempfile.TemporaryDirectory(prefix="bilbo-evals-codex-") as tmp:
            tmp = Path(tmp)
            cwd = tmp / "cwd"
            cwd.mkdir()
            schema_file, out_file = tmp / "schema.json", tmp / "out.json"
            schema_file.write_text(json.dumps(c.schema, sort_keys=True), encoding="utf-8")
            argv = [
                "codex", "exec", "-m", cfg.query_model, "-c", f'model_reasoning_effort="{cfg.reasoning_effort}"',
                "--ignore-user-config", "--ignore-rules", "--ephemeral", "--skip-git-repo-check", "-s", "read-only",
                "--disable", "plugins", "--disable", "hooks", "--disable", "apps",
                "--output-schema", str(schema_file), "-o", str(out_file), "-C", str(cwd), "--json", "-",
            ]
            p = common.run(argv, env=_env({"CODEX_HOME": str(home)}), stdin=c.prompt, cwd=cwd, timeout=CODEX_TIMEOUT)
            written = out_file.read_text(encoding="utf-8") if out_file.is_file() else ""
    finally:
        _sync_auth(home, run.codex_real)
    out.stdout = p.stdout
    events = _events(p.stdout)
    limited = _limit_text(p, events) if p.exit != 0 else None
    if limited:
        out.status, out.message = "rate_limited", f"codex reports a usage limit: {limited}"
        out.stop = RateLimited(out.message)
        return out
    problem = _codex_problem(events)
    if problem:
        out.status, out.message = "tool_use_refused", problem
        return out
    if p.exit != 0:
        out.message = f"codex exit {p.exit}: {p.stderr.strip()[:200]}"
        return out
    usage = next((e.get("usage") for e in reversed(events) if e.get("type") == "turn.completed"), None) or {}
    out.tokens_in, out.tokens_out = _int(usage.get("input_tokens")), _int(usage.get("output_tokens"))
    text = written.strip()
    if not text:
        messages = [e["item"].get("text", "") for e in events
                    if e.get("type") == "item.completed" and e["item"].get("type") == "agent_message"]
        text = messages[-1].strip() if messages else ""
    try:
        value = json.loads(text)
    except ValueError:
        out.message = "codex wrote no JSON output"
        return out
    if not isinstance(value, dict):
        out.message = "codex output is not an object"
        return out
    out.output = value
    return out


# --- the codex home ---------------------------------------------------------------------------------------------------


def _real_auth() -> Path:
    base = os.environ.get("CODEX_HOME")
    return Path(base).expanduser() / "auth.json" if base else Path.home() / ".codex/auth.json"


def _codex_home(ds_dir: Path) -> Path:
    """A throwaway CODEX_HOME holding only a symlink to the invoking user's auth.json."""
    run = _run_state(ds_dir)
    with run.lock:
        if run.codex_home is not None:
            return run.codex_home
        real = _real_auth()
        if not real.is_file():
            raise Refused(f"codex is not logged in: no auth.json at {real}")
        if run.tmp is None:
            run.tmp = Path(tempfile.mkdtemp(prefix="bilbo-evals-gen-"))
        home = run.tmp / "codex-home"
        home.mkdir()
        os.symlink(real, home / "auth.json")
        run.codex_home, run.codex_real = home, real
        return home


def _sync_auth(home: Path, real: Path | None) -> str | None:
    """Codex may replace the link with a refreshed token; copy a valid newer file back and restore the link."""
    if real is None:
        return None
    link = home / "auth.json"
    with _AUTH_LOCK:
        if link.is_symlink():
            return None
        note = None
        if link.is_file():
            data = link.read_bytes()
            try:
                valid = bool(json.loads(data)) and isinstance(json.loads(data), dict)
            except ValueError:
                valid = False
            target = Path(os.path.realpath(real))
            if not valid:
                note = "codex left an auth.json that is not a JSON object; the real one is untouched"
            elif data != target.read_bytes():
                mode = target.stat().st_mode & 0o777 if target.exists() else 0o600
                tmp = target.with_name(f".{target.name}.{os.getpid()}.tmp")
                fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, mode)
                with os.fdopen(fd, "wb") as f:
                    f.write(data)
                os.replace(tmp, target)
                note = "codex refreshed its login; the new auth.json was copied back"
            link.unlink()
        elif link.exists():
            link.unlink()
        os.symlink(real, link)
        if note:
            common.err(note)
        return note


# --- the log, the budget, call ----------------------------------------------------------------------------------


def _model(c: Call, cfg: GenConfig) -> tuple[str, dict]:
    if c.cli == "claude":
        return cfg.renderer_model, {}
    return cfg.query_model, {"reasoning_effort": cfg.reasoning_effort}


def _try(c: Call, ds_dir: Path, cfg: GenConfig, tries: int) -> tuple[str, dict | None, str]:
    """One CLI run with its budget slot and its log row: (status, output, message). Stops raise."""
    ds_dir = Path(ds_dir)
    gen = ds_dir / "generation"
    run = _run_state(ds_dir)
    home = str(Path.home())
    if home and home != "/" and home in c.prompt:
        raise Refused(f"the prompt of {c.step}/{c.item} holds the invoking user's home path")
    prompt_sha = sha256_bytes(c.prompt.encode("utf-8"))
    schema_text = json.dumps(c.schema, sort_keys=True)
    schema_sha = sha256_bytes(schema_text.encode("utf-8"))
    with run.lock:
        if run.limited:
            raise run.limited
        used = _rows(ds_dir)
        if used + run.inflight + 1 > cfg.max_calls:
            raise BudgetExhausted(f"the call budget of {cfg.max_calls} is spent ({used} calls made)")
        run.inflight += 1
    call_id = f"{c.step}/{c.item}/{c.attempt}"
    model, params = _model(c, cfg)
    outcome = _Outcome("error", message="the call raised before it finished")
    try:
        _write_once(gen / "prompts" / f"{prompt_sha}.txt", c.prompt)
        _write_once(gen / "schemas" / f"{schema_sha}.json", json.dumps(c.schema, indent=2, sort_keys=True) + "\n")
        start = time.monotonic()
        try:
            outcome = _run_claude(c, cfg) if c.cli == "claude" else _run_codex(c, cfg, ds_dir)
        except Refused:
            outcome = _Outcome("refused", message="refused before the CLI ran")
            raise
        except OSError as e:
            outcome = _Outcome("error", message=f"cannot run {c.cli}: {e}")
        duration = int((time.monotonic() - start) * 1000)
        if outcome.output is not None:
            problems = schema.check_output(c.schema, outcome.output)
            blob = json.dumps(outcome.output, ensure_ascii=False)
            if problems:
                outcome.status, outcome.message, outcome.output = "invalid", problems[0], None
            elif home and home != "/" and home in blob:
                outcome.status, outcome.message, outcome.output = "invalid", "the output holds the invoking user's home path", None
            else:
                outcome.status = "ok"
        if outcome.stdout:
            raw = _raw_path(ds_dir, call_id, tries)
            raw.parent.mkdir(parents=True, exist_ok=True)
            raw.write_text(outcome.stdout, encoding="utf-8")
        out_file = None
        if outcome.status == "ok":
            path = output_path(ds_dir, c.step, c.item, c.attempt)
            write_json(path, outcome.output)
            out_file = str(path.relative_to(gen))
        append_jsonl(gen / "calls.jsonl", {
            "call_id": call_id, "step": c.step, "item": c.item, "attempt": c.attempt, "cli": c.cli,
            "cli_version": _cli_version(c.cli), "model": model, "params": params, "prompt_sha256": prompt_sha,
            "prompt_file": f"prompts/{prompt_sha}.txt", "schema_sha256": schema_sha, "output_file": out_file,
            "status": outcome.status, "duration_ms": duration, "tokens_in": outcome.tokens_in,
            "tokens_out": outcome.tokens_out, "time": common.now(), "try": tries,
        })
    finally:
        with run.lock:
            run.inflight -= 1
    if outcome.near_limit:
        with run.lock:
            run.limited = RateLimited(f"a usage window is nearly used up (resets {outcome.near_limit})")
    if outcome.stop is not None:
        if isinstance(outcome.stop, RateLimited):
            with run.lock:
                run.limited = outcome.stop
        raise outcome.stop
    return outcome.status, outcome.output, outcome.message


def call(c: Call, ds_dir: Path, cfg: GenConfig) -> dict:
    """Run one call; retry once on an invalid or failed one. Raises BudgetExhausted, RateLimited, SessionLeak or CallFailed."""
    message = ""
    for tries in (1, 2):
        status, output, message = _try(c, ds_dir, cfg, tries)
        if status == "ok":
            return output
    raise CallFailed(f"{c.step}/{c.item}: {message}")


def run_many(calls: list[Call], ds_dir: Path, cfg: GenConfig) -> dict[str, dict | Exception]:
    """Run the calls whose output is missing on a pool; one stop (budget, limit, leak) ends them all."""
    results: dict[str, dict | Exception] = {}
    pending = []
    for c in calls:
        path = output_path(ds_dir, c.step, c.item, c.attempt)
        if path.is_file():
            results[c.item] = json.loads(path.read_text(encoding="utf-8"))
        else:
            pending.append(c)
    stop = threading.Event()
    fatal: list[Exception] = []

    def work(c: Call):
        if stop.is_set():
            return fatal[0]
        try:
            return call(c, ds_dir, cfg)
        except Refused as e:
            fatal.append(e)
            stop.set()
            return e
        except Exception as e:
            return e

    with concurrent.futures.ThreadPoolExecutor(max_workers=cfg.concurrency) as pool:
        for c, value in zip(pending, pool.map(work, pending)):
            results[c.item] = value
    return results


def fatal(results: dict) -> Exception | None:
    """The first stop (a Refused: budget, limit or leak) among the results of run_many."""
    return next((v for v in results.values() if isinstance(v, Refused)), None)


def failed(results: dict) -> list[str]:
    """Items whose result is an exception."""
    return sorted(k for k, v in results.items() if isinstance(v, Exception))


# --- preflight ---------------------------------------------------------------------------------------------------------


def _preflight_row(ds_dir: Path, cli: str, step: str, status: str, reason: str | None, checks: list[str]) -> None:
    append_jsonl(Path(ds_dir) / "generation/preflight.jsonl", {
        "cli": cli, "cli_version": _cli_version(cli), "step": step, "status": status, "reason": reason,
        "checks": checks, "time": common.now(),
    })


def preflight(cli: str, ds_dir: Path, cfg: GenConfig, step: str = "") -> None:
    """Check the session a CLI opens before a step's first call (once per step and process); refuses on anything that could carry memory in."""
    ds_dir = Path(ds_dir)
    require_cli(cli)
    run = _run_state(ds_dir)
    if (cli, step) in run.preflighted:
        return
    item = f"{step or 'step'}-{cli}"
    checks: list[str] = []
    try:
        if cli == "claude":
            call(Call("preflight", item, "claude", CLAUDE_PROBE, CLAUDE_PROBE_SCHEMA), ds_dir, cfg)
            checks.append("session init, hooks and rate limit read from the probe call")
        else:
            home = _codex_home(ds_dir)
            for entry in sorted(home.iterdir()):
                if entry.name in CODEX_MEMORY or (entry.name == "skills" and any(s.name != ".system" for s in entry.iterdir())):
                    raise SessionLeak(f"the codex home holds {entry.name}")
            checks.append("codex home holds no instructions, config, hooks, rules, plugins or user skills")
            answer = call(Call("preflight", item, "codex", CODEX_PROBE, CODEX_PROBE_SCHEMA), ds_dir, cfg)
            heading = answer["user_instructions_first_heading"].strip().strip("`'\".").upper()
            if heading != "NONE" or answer["mcp_tools"]:
                raise SessionLeak(
                    f"the codex probe reports user instructions {answer['user_instructions_first_heading']!r} "
                    f"and MCP tools {answer['mcp_tools']}"
                )
            checks.append("probe call reports no user instructions and no MCP tools")
    except Refused as e:
        _preflight_row(ds_dir, cli, step, "refused", str(e)[:200], checks)
        raise
    except CallFailed as e:
        _preflight_row(ds_dir, cli, step, "refused", str(e)[:200], checks)
        raise Refused(f"the {cli} preflight call failed: {e}") from e
    _preflight_row(ds_dir, cli, step, "ok", None, checks)
    run.preflighted.add((cli, step))
