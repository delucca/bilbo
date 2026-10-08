"""A temporary root for bilbo, its cleared environment, its config and the guard that keeps a run out of the user's folders."""

from __future__ import annotations

import os
import shutil
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping

from bilbo_evals.common import Proc, Refused, run

ROOT_PREFIX = "bilbo-evals-"


@dataclass
class Sandbox:
    root: Path
    home: Path
    store: Path
    config: Path
    cache: Path
    data: Path
    state: Path
    env: dict[str, str]


def _tmpdir() -> str:
    return os.environ.get("TMPDIR") or tempfile.gettempdir()


def create(run_id: str) -> Sandbox:
    root = Path(_tmpdir()) / f"{ROOT_PREFIX}{run_id}"
    if root.exists():
        raise Refused(f"{root} exists already; pick another run id")
    home, store, cache, data, state = (root / n for n in ("home", "store", "cache", "data", "state"))
    config_home = root / "config"
    config = config_home / "bilbo/config"
    for d in (home, store, cache, data, state, config.parent):
        d.mkdir(parents=True)
    env = {
        "HOME": str(home),
        "BILBO_HOME": str(store),
        "BILBO_CONFIG": str(config),
        "XDG_CONFIG_HOME": str(config_home),
        "XDG_DATA_HOME": str(data),
        "XDG_CACHE_HOME": str(cache),
        "XDG_STATE_HOME": str(state),
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "LANG": os.environ.get("LANG", "en_US.UTF-8"),
        "LC_ALL": os.environ.get("LC_ALL", os.environ.get("LANG", "en_US.UTF-8")),
        "TMPDIR": _tmpdir(),
    }
    return Sandbox(root, home, store, config, cache, data, state, env)


def quote(value: str) -> str:
    """bilbo's config quoting (shared::config::quote)."""
    edge = " \t"
    if not (value == "" or value[0] in edge or value[-1] in edge or value[0] == '"' or "\n" in value or "\r" in value):
        return value.replace("\\", "\\\\")
    body = value.replace("\\", "\\\\").replace("\n", "\\n").replace('"', '\\"')
    return f'"{body}"'


def write_config(sb: Sandbox, embedder_url: str | None, extra: Mapping[str, str] = {}) -> None:
    from bilbo_evals import embedder

    settings: list[tuple[str, str]] = []
    if embedder_url:
        settings += [
            ("embedder.url", embedder_url),
            ("embedder.model", embedder.MODEL),
            ("embedder.query_prefix", embedder.QUERY_PREFIX),
        ]
    settings += list(extra.items())
    sb.config.parent.mkdir(parents=True, exist_ok=True)
    sb.config.write_text("# written by bilbo-evals\n" + "".join(f"{k} = {quote(v)}\n" for k, v in settings), encoding="utf-8")


def _absolute(value: str | None) -> Path | None:
    return Path(value) if value and os.path.isabs(value) else None


def folders(env: Mapping[str, str]) -> dict[str, Path | None]:
    """The store root, config file, cache and state folders bilbo resolves for `env` (docs/reference/configuration.md#folders)."""
    home = _absolute(env.get("HOME"))
    under = lambda var, tail: (base / tail) if (base := _absolute(env.get(var))) else None
    from_home = lambda tail: (home / tail) if home else None

    store = _absolute(env.get("BILBO_HOME")) if env.get("BILBO_HOME") else (
        under("XDG_DATA_HOME", "bilbo") or from_home(".local/share/bilbo")
    )
    config = _absolute(env.get("BILBO_CONFIG")) if env.get("BILBO_CONFIG") else (
        under("XDG_CONFIG_HOME", "bilbo/config") or from_home(".config/bilbo/config")
    )
    cache = under("XDG_CACHE_HOME", "bilbo") or from_home(".cache/bilbo")
    state = under("XDG_STATE_HOME", "bilbo") or from_home(".local/state/bilbo")
    return {"store": store, "config": config, "cache": cache, "state": state}


def _real(p: Path) -> Path:
    return Path(os.path.realpath(p))


def guard(sb: Sandbox) -> dict[str, str]:
    root = _real(sb.root)
    mine = folders(sb.env)
    theirs = folders(os.environ)
    resolved: dict[str, str] = {}
    for name, path in mine.items():
        if path is None:
            raise Refused(f"the run's {name} folder does not resolve; the sandbox environment is incomplete")
        real = _real(path)
        if real != root and root not in real.parents:
            raise Refused(f"the run's {name} folder {path} is outside the run root {sb.root}")
        user = theirs[name]
        if user is not None and _real(user) == real:
            raise Refused(f"the run's {name} folder {path} is the invoking user's {name} folder")
        resolved[name] = str(real)
    return resolved


def portable(sb: Sandbox, value):
    """`value` with the temporary root, as written and as resolved, replaced by `$TMPDIR/<root name>` in every string."""
    shown = f"$TMPDIR/{sb.root.name}"
    roots = sorted({str(sb.root), os.path.realpath(sb.root)}, key=len, reverse=True)
    if isinstance(value, str):
        for root in roots:
            value = value.replace(root, shown)
        return value
    if isinstance(value, dict):
        return {k: portable(sb, v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [portable(sb, v) for v in value]
    return value


def bilbo(sb: Sandbox, exe: Path, args: list[str], stdin: str | None = None, timeout: float = 60) -> Proc:
    guard(sb)
    return run([str(exe), *args], env=sb.env, stdin=stdin, cwd=sb.root, timeout=timeout)


def destroy(sb: Sandbox) -> None:
    if sb.root.name.startswith(ROOT_PREFIX):
        shutil.rmtree(sb.root, ignore_errors=True)
