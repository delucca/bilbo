"""Shared by the fake claude and codex: log the call, pick the first matching rule."""

import fcntl
import json
import os
import sys


def log_call(prompt: str) -> None:
    path = os.environ.get("FAKE_LLM_LOG")
    if not path:
        return
    row = {
        "argv": sys.argv[1:],
        "prompt": prompt,
        "env": {k: os.environ.get(k) for k in ("CODEX_HOME", "CLAUDE_CODE_DISABLE_AUTO_MEMORY")},
    }
    with open(path, "a", encoding="utf-8") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        f.write(json.dumps(row) + "\n")


def pick_rule(prompt: str) -> dict | None:
    """First rule whose `match` is in the prompt; a rule with `times` n is consumed after n uses."""
    script = os.environ.get("FAKE_LLM_SCRIPT")
    if not script:
        return None
    with open(script + ".lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        with open(script, encoding="utf-8") as f:
            rules = json.load(f)
        try:
            with open(script + ".state", encoding="utf-8") as f:
                used = json.load(f)
        except FileNotFoundError:
            used = {}
        for i, rule in enumerate(rules):
            if rule.get("match", "") not in prompt:
                continue
            n = used.get(str(i), 0)
            if "times" in rule and n >= rule["times"]:
                continue
            used[str(i)] = n + 1
            with open(script + ".state", "w", encoding="utf-8") as f:
                json.dump(used, f)
            return rule
    return None
