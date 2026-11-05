"""The agent's PreToolUse hook: allow a call only while what the agent has read stays under the budget, and only on the
arm's memory. The state file ($BILBO_AGENT_STATE) gives the budget, the tokens read before the first call (a memory
Claude Code loads on its own), the tools that read memory, the folders Read, Glob and Grep may touch and the commands
Bash may run. Tokens are estimated at 4 bytes each: pack.py counts them exactly afterwards and cuts there."""

import json
import os
import re
import sys
from pathlib import Path

BYTES_PER_TOKEN = 4
UNSAFE = re.compile(r"[;&|`$<>\n]")


def text_of(content):
    if isinstance(content, str):
        return content
    return "\n".join(c.get("text", "") for c in content or [] if c.get("type") == "text")


def spent(transcript, reads):
    """Estimated tokens of every tool result so far whose call reads memory."""
    uses, total = {}, 0
    for line in Path(transcript).read_text().splitlines():
        try:
            entry = json.loads(line)
        except ValueError:
            continue
        for c in (entry.get("message") or {}).get("content") or []:
            if not isinstance(c, dict):
                continue
            if c.get("type") == "tool_use":
                uses[c["id"]] = c
            elif c.get("type") == "tool_result" and not c.get("is_error") and \
                    reads_memory(uses.get(c["tool_use_id"], {}), reads):
                total += len(text_of(c.get("content")).encode()) / BYTES_PER_TOKEN
    return total


def reads_memory(use, reads):
    """Whether a call reads memory: a tool in reads (a name ending in __ covers an MCP server's tools); for Bash, only a
    search, not the skill's check that bilbo is installed."""
    name = use.get("name")
    if name == "Bash":
        return "Bash" in reads and (use.get("input") or {}).get("command", "").startswith("bilbo recall")
    return bool(name) and (name in reads or any(name.startswith(r) for r in reads if r.endswith("__")))


def deny(reason):
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny",
                                             "permissionDecisionReason": reason}}))
    sys.exit(0)


def main():
    call = json.load(sys.stdin)
    state = json.loads(Path(os.environ["BILBO_AGENT_STATE"]).read_text())
    tool, args = call["tool_name"], call.get("tool_input") or {}
    if tool == "Skill":
        sys.exit(0)
    if tool in ("Read", "Glob", "Grep"):
        target = args.get("file_path") or args.get("path") or ""
        real = os.path.realpath(target) if target else ""
        if not real or not any(real == r or real.startswith(r.rstrip("/") + "/") for r in state["roots"]):
            deny(f"{tool} reaches only the memory: {', '.join(state['roots'])}")
    elif tool == "Bash":
        command = args.get("command", "").strip()
        if UNSAFE.search(command) or not any(re.fullmatch(p, command) for p in state["commands"]):
            deny("Bash runs only the memory tool's own commands, one at a time")
    elif not reads_memory({"name": tool}, state["reads"]):
        deny(f"{tool} is not available in this session")
    used = state["initial"] + spent(call["transcript_path"], state["reads"])
    if used >= state["budget"]:
        deny(f"The {state['budget']:,}-token reading budget is spent; answer from what you have read.")


if __name__ == "__main__":
    main()
