"""The project pins every direct dependency with == and builds with a pinned backend."""

import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PYPROJECT = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
EXACT = re.compile(r"^[A-Za-z0-9_.\-\[\]]+==\d+(\.\d+)*$")


def direct_dependencies() -> list[str]:
    deps = list(PYPROJECT["project"]["dependencies"])
    for group in PYPROJECT.get("dependency-groups", {}).values():
        deps += group
    deps += PYPROJECT["build-system"]["requires"]
    return deps


def test_every_direct_dependency_is_pinned_with_double_equals():
    loose = [d for d in direct_dependencies() if not EXACT.match(d)]
    assert not loose, f"not pinned with ==: {loose}"


def test_python_is_pinned():
    assert (ROOT / ".python-version").read_text().strip() == "3.12.13"
    assert PYPROJECT["project"]["requires-python"] == "==3.12.*"


def test_lock_covers_every_pin():
    lock = (ROOT / "uv.lock").read_text(encoding="utf-8")
    for dep in direct_dependencies():
        if dep.startswith("uv_build"):
            continue
        name, version = dep.split("==")
        assert re.search(rf'name = "{re.escape(name.lower())}"\nversion = "{re.escape(version)}"', lock.replace("_", "-")), dep
