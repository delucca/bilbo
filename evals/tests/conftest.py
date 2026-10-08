"""Shared fixtures: the bilbo binary, the fixture dataset, the fake embedder and the fake LLM CLIs."""

from __future__ import annotations

import json
import os
import shutil
import sys
from pathlib import Path

import pytest

TESTS = Path(__file__).resolve().parent
sys.path.insert(0, str(TESTS))

from fake_embedder import FakeEmbedder  # noqa: E402

REPO_ROOT = TESTS.parents[1]


@pytest.fixture(autouse=True)
def _home(tmp_path, monkeypatch):
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))


@pytest.fixture(scope="session")
def bilbo_bin() -> Path:
    path = Path(os.environ.get("BILBO_BIN") or REPO_ROOT / "target/debug/bilbo")
    if not path.is_file():
        pytest.fail(f"no bilbo binary at {path}: build it with `cargo build --locked` or set BILBO_BIN")
    return path


@pytest.fixture(scope="session")
def fixture_dir() -> Path:
    return TESTS / "fixtures/notes-fixture"


@pytest.fixture
def fixture_copy(fixture_dir, tmp_path) -> Path:
    dest = tmp_path / "notes-fixture"
    shutil.copytree(fixture_dir, dest)
    return dest


@pytest.fixture
def fake_embedder():
    server = FakeEmbedder().start()
    try:
        yield server
    finally:
        server.stop()


class FakeLLM:
    def __init__(self, script: Path, log: Path) -> None:
        self.script, self.log = script, log

    def set_script(self, rules: list[dict]) -> None:
        self.script.write_text(json.dumps(rules), encoding="utf-8")
        for suffix in (".state", ".lock"):
            Path(str(self.script) + suffix).unlink(missing_ok=True)

    def calls(self) -> list[dict]:
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text(encoding="utf-8").splitlines() if line]


@pytest.fixture
def fake_llm(tmp_path, monkeypatch) -> FakeLLM:
    script, log = tmp_path / "fake-llm-script.json", tmp_path / "fake-llm-log.jsonl"
    script.write_text("[]", encoding="utf-8")
    monkeypatch.setenv("PATH", f"{TESTS / 'fakes'}{os.pathsep}{os.environ['PATH']}")
    monkeypatch.setenv("FAKE_LLM_SCRIPT", str(script))
    monkeypatch.setenv("FAKE_LLM_LOG", str(log))
    return FakeLLM(script, log)
