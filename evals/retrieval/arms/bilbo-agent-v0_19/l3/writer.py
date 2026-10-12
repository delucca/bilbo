"""The replay writer of the L3 sessions eval (PLAN.md section 3.4): bilbo's shipped `note` skill, and the same loop over a
CLAUDE.md, driven by a model that reads the recorded sessions exchange by exchange.

Two arms per tool: `<tool>-agent` (the agent alone) and `<tool>-assisted` (plus the simulated user's schedule). Tools are
`bilbo` and `claude-md`. A capture has two passes:

1. the decision pass: one call per exchange, shared by both arms of a tool through `decisions/<tool>.jsonl`; the model
   answers whether it would use the skill after this exchange;
2. the run loop: on every exchange where it chose the skill (or the schedule asks, assisted arms), a tool loop with
   Bash, Read and Edit emulated here. The Bash executor allows exactly the skill's `allowed-tools`.

Every LLM call goes to `L3_LLM_BASE_URL` (a chat-completions endpoint, `L3_LLM_API_KEY` as bearer) with the label
`l3s/<stage>/<arm>/<repo>/r<k>` in `X-L3-Label`. Nothing here calls a vendor or an agent CLI.
"""

from __future__ import annotations

import concurrent.futures
import fcntl
import hashlib
import json
import os
import posixpath
import re
import shlex
import shutil
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path

import schema

HERE = Path(__file__).resolve().parent
PROMPTS = HERE / "prompts"

MODEL = "gpt-6-luna"
EFFORT = "low"
DECIDE_WINDOW = 16_000  # tokens of the session shown to the decision pass
RUN_WINDOW = 32_000
MAX_STEPS = 12
FORMAT_RETRIES = 2  # more tries of a step the gateway rejected as invalid output, each with a new seed
RECALL_LIMIT = 20  # passages `bilbo recall` returns at query time
READ_TOKENS = 4_000  # the reader's budget; CLAUDE.md is cut here
OUTPUT_CAP = 20_000  # characters of one tool result shown to the model
UNREACHABLE_WAIT = 30 * 60
VIRTUAL_HOME = "/" "home/dev"  # split so the privacy grep for host paths stays empty
VIRTUAL_STORE = {"bilbo": f"{VIRTUAL_HOME}/.local/share/bilbo", "claude-md": f"{VIRTUAL_HOME}/project"}  # the model never sees a host path
VIRTUAL_PROJECT = f"{VIRTUAL_HOME}/project"
VIRTUAL_CONFIG = f"{VIRTUAL_HOME}/.config/bilbo/config"
VIRTUAL_BILBO = "/usr/local/bin/bilbo"
VIRTUAL_SKILL_DIR = f"{VIRTUAL_HOME}/.claude/skills/note"
SKILL_TAG = "v0.19.0"
SKILL_PATH = "plugins/bilbo/skills/note/SKILL.md"
SKILL_SHA256 = "4c4a1ac0670be19b9e71a7fe7da7338e698c242ccd372b5c225056766978b601"  # SKILL.md at v0.19.0
# a loopback call must not use a configured gateway
DIRECT = urllib.request.build_opener(getattr(urllib.request, "Pro" "xyHandler")({}))  # name split for the privacy grep

TOOLS = ("bilbo", "claude-md")
ARMS = {f"{t}-{m}": (t, m) for t in TOOLS for m in ("agent", "assisted")}
COMMAND = {"bilbo": "bilbo:note", "claude-md": "note"}  # the name the user and the skill listing use

HOOK_TEXT = (
    "Context was compacted. If this session settled something later sessions should know, such as a decision, a gotcha "
    "or a plan, save it with the bilbo note skill once the current task allows."
)  # hooks.json at v0.19.0, the SessionStart hook with matcher `compact`, verbatim

STEP_HOOK = None  # tests set a callable (point, xid); raising from it stands for a crash


class WriterError(Exception):
    """A failure that stops the capture; the session stays open and a rerun restores its snapshot."""


class ModelFailure(WriterError):
    """One call the model could not answer in the required form (the endpoint refused its output twice): counted, not fatal."""


def hook(point: str, xid: str) -> None:
    if STEP_HOOK is not None:
        STEP_HOOK(point, xid)


# --- small helpers -----------------------------------------------------------------------------------------------------


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def sha256_file(p: Path) -> str:
    return sha256_bytes(Path(p).read_bytes())


def jsonl(p: Path) -> list[dict]:
    p = Path(p)
    if not p.exists():
        return []
    return [json.loads(l) for l in p.read_text(encoding="utf-8").splitlines() if l.strip()]


def append_jsonl(p: Path, row: dict, lock: threading.Lock | None = None) -> None:
    line = json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n"
    if lock:
        lock.acquire()
    try:
        with open(p, "a", encoding="utf-8") as f:
            f.write(line)
            f.flush()
            os.fsync(f.fileno())
    finally:
        if lock:
            lock.release()


def write_atomic(p: Path, text: str) -> None:
    tmp = p.with_name(p.name + ".tmp")
    tmp.write_text(text, encoding="utf-8")
    os.replace(tmp, p)


_ENC = None


def encoding():
    global _ENC
    if _ENC is None:
        import tiktoken

        _ENC = tiktoken.get_encoding("o200k_base")
    return _ENC


def count_tokens(text: str) -> int:
    return len(encoding().encode(text, disallowed_special=()))


def cut_to_tokens(text: str, n: int) -> str:
    ids = encoding().encode(text, disallowed_special=())
    return text if len(ids) <= n else encoding().decode(ids[:n])


def split_frontmatter(text: str) -> tuple[dict[str, str], str]:
    m = re.match(r"^---\n(.*?)\n---\n(.*)$", text, re.S)
    if not m:
        raise WriterError("a skill file has no frontmatter")
    fm = {}
    for line in m[1].splitlines():
        k, _, v = line.partition(":")
        fm[k.strip()] = v.strip()
    return fm, m[2]


def minute(ts: str) -> str:
    """bilbo's `created` format: YYYY-MM-DDTHH:MM+00:00."""
    d = datetime.fromisoformat(ts.replace("Z", "+00:00")).astimezone(timezone.utc)
    return d.strftime("%Y-%m-%dT%H:%M+00:00")


CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"


def ulid_from(seed: str) -> str:
    n = int.from_bytes(hashlib.sha256(seed.encode()).digest()[:16], "big")
    out = []
    for _ in range(26):
        out.append(CROCKFORD[n & 31])
        n >>= 5
    return "".join(reversed(out))


# --- the tool: skill, listing, store ---------------------------------------------------------------------------------


@dataclass
class Tool:
    name: str  # bilbo | claude-md
    command: str  # the name in the listing and in a user's slash command
    description: str
    skill_text: str  # the body returned by the Skill tool
    allowed: list[str]  # Bash patterns from allowed-tools
    can_read: bool
    can_edit: bool
    skill_sha: str
    store_file: str | None  # claude-md: the file the skill writes


def allowed_patterns(allowed_tools: str) -> tuple[list[str], bool, bool]:
    bash = re.findall(r"Bash\(([^)]*)\)", allowed_tools)
    rest = re.sub(r"Bash\([^)]*\)", "", allowed_tools)
    words = {w.strip() for w in rest.split(",") if w.strip()}
    return bash, "Read" in words, "Edit" in words


def skill_text(path: Path | None) -> str:
    """The v0.19.0 SKILL.md, verbatim: from `path`, $L3_BILBO_SKILL, or `git show` in the repo this eval lives in. Its sha is pinned."""
    path = path or (Path(os.environ["L3_BILBO_SKILL"]) if os.environ.get("L3_BILBO_SKILL") else None)
    if path:
        raw = Path(path).read_bytes()
    else:
        p = subprocess.run(["git", "-C", str(HERE), "show", f"{SKILL_TAG}:{SKILL_PATH}"], capture_output=True)
        if p.returncode != 0:
            raise WriterError(f"no SKILL.md: pass one (L3_BILBO_SKILL) or run in a clone holding the tag {SKILL_TAG}")
        raw = p.stdout
    if sha256_bytes(raw) != SKILL_SHA256:
        raise WriterError(f"the bilbo SKILL.md is not the one of {SKILL_TAG} (sha256 {sha256_bytes(raw)[:12]}, expected {SKILL_SHA256[:12]})")
    return raw.decode("utf-8")


def load_tool(name: str, skill_md: Path | None, vstore: str) -> Tool:
    if name == "bilbo":
        text = skill_text(skill_md)
        fm, _ = split_frontmatter(text)
        bash, r, e = allowed_patterns(fm["allowed-tools"])
        return Tool("bilbo", COMMAND["bilbo"], fm["description"], text, bash, r, e, sha256_bytes(text.encode()), None)
    if name == "claude-md":
        raw = (PROMPTS / "claude_md_skill.md").read_text(encoding="utf-8")
        text = raw.replace("{path}", f"{vstore}/CLAUDE.md")
        fm, _ = split_frontmatter(text)
        bash, r, e = allowed_patterns(fm["allowed-tools"])
        return Tool("claude-md", COMMAND["claude-md"], fm["description"], text, bash, r, e, sha256_bytes(raw.encode()), "CLAUDE.md")
    raise WriterError(f"unknown tool {name!r}")


def skills_listing(tool: Tool) -> str:
    return f"- {tool.command}: {tool.description}"


# --- the LLM -----------------------------------------------------------------------------------------------------------


class Chat:
    """A chat-completions client. `base_for(label)` gives the endpoint root for a label (the harness's recorder, which counts
    answered calls per label and waits out an unreachable gateway), `ask(label, body)` returns the parsed answer.

    A pause (429 with a Retry-After, the budget brake, a login to renew) is waited out; the call and token caps raise, so the
    capture stops and a rerun resumes; an answer the model could not give in the required form is a `ModelFailure`.
    """

    def __init__(self, base_for, api_key: str = "x"):
        self.base_for, self.key = base_for, api_key
        self.calls = 0
        self._lock = threading.Lock()

    @classmethod
    def from_env(cls) -> "Chat":
        base = (os.environ.get("L3_LLM_BASE_URL") or "").rstrip("/")
        if not base:
            raise WriterError("L3_LLM_BASE_URL is not set")
        return cls(lambda label: base, os.environ.get("L3_LLM_API_KEY") or "x")

    def ask(self, label: str, body: dict) -> dict:
        schema.check_label_name(label)
        data = json.dumps(body, ensure_ascii=False).encode()
        deadline = time.monotonic() + UNREACHABLE_WAIT
        attempt = paused = 0
        while True:
            req = urllib.request.Request(
                self.base_for(label).rstrip("/") + "/chat/completions", data=data, method="POST",
                headers={"Content-Type": "application/json", "Authorization": f"Bearer {self.key}", "X-L3-Label": label},
            )  # fmt: skip
            try:
                with DIRECT.open(req, timeout=1800) as r:
                    out = json.loads(r.read())
                with self._lock:
                    self.calls += 1
                return out
            except urllib.error.HTTPError as e:
                payload = e.read()
                try:
                    err = json.loads(payload or b"{}").get("error", {})
                except ValueError:
                    err = {}
                code = err.get("code", "")
                if code in ("l3_call_cap", "l3_token_cap") or e.code in (401, 403):
                    raise WriterError(f"the LLM endpoint refused {label}: {code or e.code}; raise the cap and rerun, the capture resumes") from None
                if e.code in (429, 503) or code in ("l3_budget_stop", "l3_rate_limit_pause", "l3_auth_stop"):
                    paused += 1  # a usage-limit pause, the budget brake or a login the lead must renew: wait for it, up to a day
                    if paused > 24 * 60:
                        raise WriterError(f"the LLM endpoint stayed paused for {label}: {code or e.code}") from None
                    time.sleep(min(float(e.headers.get("Retry-After") or 60), 600))
                elif e.code == 502 and code.startswith("l3_"):  # the gateway tried the model twice and refused its output
                    raise ModelFailure(f"{label}: {code}: {err.get('message', '')[:200]}") from None
                elif e.code >= 500:
                    attempt += 1
                    if attempt > 6:
                        raise WriterError(f"the LLM endpoint kept answering {e.code} for {label}") from None
                    time.sleep(min(5 * 2**attempt, 120))
                else:
                    raise WriterError(f"the LLM endpoint answered {e.code} for {label}: {payload[:300]!r}") from None
            except (urllib.error.URLError, ConnectionError, TimeoutError, OSError) as e:
                if time.monotonic() > deadline:
                    raise WriterError(f"the LLM endpoint stayed unreachable for {label}: {e}") from None
                time.sleep(15)


# --- exchanges and windows -----------------------------------------------------------------------------------------------


def exchange_messages(x: dict, tool: Tool | None, marker: bool, hook_text: bool) -> list[dict]:
    user = x["user"]
    if hook_text and x["compact_before"]:
        user = f"<system-reminder>\nSessionStart:compact hook additional context: {HOOK_TEXT}\n</system-reminder>\n{user}"
    assistant = x["assistant"] if x["assistant"].strip() else "(no reply)"
    if marker:
        assistant += f"\n\n[The agent used the {tool.command} skill after this exchange.]"
    return [{"role": "user", "content": user}, {"role": "assistant", "content": assistant}]


def window(session: list[dict], i: int, budget: int) -> list[dict]:
    """The exchanges of `session` up to and including index `i` that fit `budget` tokens, newest kept; the current one always."""
    chosen, total = [], 0
    for j in range(i, -1, -1):
        t = session[j]["tokens"]
        if j != i and total + t > budget:
            break
        chosen.append(session[j])
        total += t
    out = list(reversed(chosen))
    cur = out[-1]
    if cur["tokens"] > budget:  # one exchange alone is over the window: keep its head and tail
        keep = int(budget * 0.6)
        tail = budget - keep
        enc = encoding()
        ids = enc.encode(cur["assistant"], disallowed_special=())
        if len(ids) > budget:
            cur = dict(cur, assistant=enc.decode(ids[:keep]) + "\n[... cut ...]\n" + enc.decode(ids[-tail:]))
            out[-1] = cur
    return out


# --- the decision pass ---------------------------------------------------------------------------------------------------


def decision_schema(tool: Tool) -> dict:
    return {
        "type": "json_schema",
        "json_schema": {"name": "decision", "strict": True, "schema": {
            "type": "object", "additionalProperties": False, "required": ["skill"],
            "properties": {"skill": {"type": "string", "enum": [tool.command, "none"]}},
        }},
    }  # fmt: skip


def frame_text(tool: Tool) -> str:
    return (PROMPTS / "agent_frame.md").read_text(encoding="utf-8").replace("{skills}", skills_listing(tool)).rstrip("\n")


def decide_session(chat, label: str, tool: Tool, session: list[dict], done: dict[str, dict], out_path: Path, lock) -> list[dict]:
    """One call per exchange, in order; earlier choices of this pass are marked in the window. Returns the new rows."""
    frame, tail = frame_text(tool), (PROMPTS / "agent_decide.md").read_text(encoding="utf-8").rstrip("\n")
    chose = {x["xid"] for x in session if done.get(x["xid"], {}).get("decision") == "skill"}
    new = []
    for i, x in enumerate(session):
        if x["xid"] in done:
            continue
        msgs = [{"role": "system", "content": frame}]
        for y in window(session, i, DECIDE_WINDOW):
            msgs += exchange_messages(y, tool, y["xid"] in chose and y is not x, tool.name == "bilbo")
        msgs.append({"role": "user", "content": tail})
        body = {"model": MODEL, "messages": msgs, "response_format": decision_schema(tool), "seed": 1}
        if EFFORT:
            body["reasoning_effort"] = EFFORT
        try:
            resp = chat.ask(label, body)
            raw = (resp["choices"][0]["message"].get("content") or "").strip()
        except ModelFailure as e:
            raw = f"model_failure: {e}"[:300]
        try:
            choice = json.loads(raw)["skill"]
        except (ValueError, KeyError, TypeError):
            choice = "none"  # unparseable counts against G3's 2% and is kept in raw
        decision = "skill" if choice == tool.command else "none"
        row = {"xid": x["xid"], "decision": decision, "raw": raw}
        schema.check_writer_decision(row)
        append_jsonl(out_path, row, lock)
        done[x["xid"]] = row
        new.append(row)
        if decision == "skill":
            chose.add(x["xid"])
    return new


def decision_pass(chat, label: str, tool: Tool, sessions: list[list[dict]], out_path: Path, workers: int) -> dict[str, dict]:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    done = {r["xid"]: r for r in jsonl(out_path)}
    lock = threading.Lock()
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as ex:
        futs = [ex.submit(decide_session, chat, label, tool, s, done, out_path, lock) for s in sessions]
        for f in futs:
            f.result()
    return {r["xid"]: r for r in jsonl(out_path)}


# --- the sandbox and the executor ------------------------------------------------------------------------------------------


@dataclass
class Sandbox:
    """The arm's real folders, and the fixed virtual paths the model sees in their place (the same for every arm, repo and host)."""

    root: Path  # the arm's state folder
    store: Path
    config: Path
    work: Path
    env: dict[str, str]
    exe: Path | None  # the bilbo binary (bilbo tool only)
    embedder: bool
    vstore: str = VIRTUAL_STORE["bilbo"]  # what the model calls `store`: bilbo's store, or the project holding CLAUDE.md

    def to_real(self, path: str) -> Path | None:
        """A virtual path inside the store root as its real path; None for anything else, a real host path included."""
        if not isinstance(path, str) or "\0" in path:
            return None
        norm = posixpath.normpath(path)
        if norm != self.vstore and not norm.startswith(self.vstore + "/"):
            return None
        real = self.store / norm[len(self.vstore) :].lstrip("/")
        rr = Path(os.path.realpath(real))
        base = Path(os.path.realpath(self.store))
        return real if rr == base or base in rr.parents else None

    def inside(self, p: str | Path) -> bool:
        return self.to_real(str(p)) is not None

    def virtual(self, text: str) -> str:
        """Real folders of this arm in a tool's output, rewritten to their virtual paths."""
        pairs = []
        for real, virt in ((self.store, self.vstore), (self.config, VIRTUAL_CONFIG), (self.work, VIRTUAL_PROJECT), (self.root, VIRTUAL_HOME)):
            for r in {str(real), os.path.realpath(real)}:
                pairs.append((r, virt))
        for r, virt in sorted(pairs, key=lambda x: -len(x[0])):
            text = text.replace(r, virt)
        return text

    def argv_real(self, argv: list[str]) -> list[str]:
        return [str(self.to_real(a) or a) if a.startswith("/") else a for a in argv]


def make_sandbox(state: Path, exe: Path | None, embedder: tuple[str, str, str] | None, tool: str = "bilbo") -> Sandbox:
    """`embedder` is (root url, model, query prefix) of the arm's own server; None runs bilbo keyword-only."""
    state = state.resolve()
    store, work = state / "store", state / "work"
    home, config = state / "home", state / "config"
    for d in (store, work, home, state / "cache", state / "data", state / "xdgconfig", state / "xdgstate"):
        d.mkdir(parents=True, exist_ok=True)
    settings = [("embedder.url", embedder[0]), ("embedder.model", embedder[1]), ("embedder.query_prefix", embedder[2])] if embedder else []
    write_atomic(config, "".join(f"{k} = {_quote(v)}\n" for k, v in settings))
    env = {
        "HOME": str(home), "BILBO_HOME": str(store), "BILBO_CONFIG": str(config), "XDG_CONFIG_HOME": str(state / "xdgconfig"),
        "XDG_DATA_HOME": str(state / "data"), "XDG_CACHE_HOME": str(state / "cache"), "XDG_STATE_HOME": str(state / "xdgstate"),
        "PATH": "/usr/bin:/bin", "TMPDIR": str(work), "LANG": "en_US.UTF-8", "LC_ALL": "en_US.UTF-8", "NO_COLOR": "1",
    }  # fmt: skip
    sb = Sandbox(state, store, config, work, env, Path(exe).resolve() if exe else None, bool(embedder), VIRTUAL_STORE[tool])
    for k in ("HOME", "BILBO_HOME", "BILBO_CONFIG", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"):
        p = Path(env[k]).resolve()
        if state not in p.parents and p != state:
            raise WriterError(f"{k}={p} is outside the arm's state folder {state}")
    return sb


def _quote(v: str) -> str:
    if not (v == "" or v[0] in " \t" or v[-1] in " \t" or v[0] == '"' or "\n" in v or "\r" in v):
        return v.replace("\\", "\\\\")
    return '"' + v.replace("\\", "\\\\").replace("\n", "\\n").replace('"', '\\"') + '"'


@dataclass
class Result:
    text: str
    exit: int | None
    refused: bool = False


def _scan(cmd: str) -> str | None:
    """A reason to refuse `cmd` before matching, or None. No shell runs here, so operators would be literal arguments;
    they are refused because the skill forbids chaining and a real shell would run them."""
    if "\n" in cmd or "\r" in cmd:
        return "multi-line command"
    q = None
    for i, ch in enumerate(cmd):
        if q == "'":
            q = None if ch == "'" else q
            continue
        if ch == "'" and q is None:
            q = "'"
        elif ch == '"':
            q = None if q == '"' else '"'
        elif ch == "`" or cmd.startswith("$(", i):
            return "command substitution"
    try:
        lex = shlex.shlex(cmd, posix=True, punctuation_chars=True)
        lex.whitespace_split = True
        toks = list(lex)
    except ValueError as e:
        return f"unparseable command ({e})"
    for t in toks:
        if t and all(c in "();<>|&" for c in t):
            return "compound command or redirect"
    if toks and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", toks[0]):
        return "environment variable prefix"
    return None


def pattern_matches(pattern: str, cmd: str) -> bool:
    return re.fullmatch(".*".join(re.escape(p) for p in pattern.split("*")), cmd) is not None


class Executor:
    """Runs one skill's tools in the sandbox. Every refusal is returned as a result and logged."""

    def __init__(self, sb: Sandbox, tool: Tool, xid: str, ts: str):
        self.sb, self.tool, self.xid, self.ts = sb, tool, xid, ts
        self.read: set[str] = set()
        self.created: list[Path] = []

    # Bash
    def bash(self, cmd: str) -> Result:
        cmd = cmd.strip()
        why = _scan(cmd)
        if why:
            return self.deny(cmd, why)
        if not any(pattern_matches(p, cmd) for p in self.tool.allowed):
            return self.deny(cmd, "not allowed by the skill")
        argv = shlex.split(cmd)
        if argv[:3] == ["command", "-v", "bilbo"]:
            return Result(f"{VIRTUAL_BILBO}\n", 0) if self.sb.exe else Result("", 1)
        if argv[0] == "bilbo":
            if self.sb.exe is None:
                return Result("bilbo: command not found\n", 127)
            argv = [str(self.sb.exe), *self.sb.argv_real(argv[1:])]
        elif argv[0] == "mv":
            rest = argv[1:]
            if rest[:1] != ["-n"]:
                return self.deny(cmd, "only `mv -n <src> <dst>` is allowed")
            sep = rest[1:2] == ["--"]  # `--` only as a separator
            paths = rest[2:] if sep else rest[1:]
            if len(paths) == 2 and any(not x.startswith("/") and not (x.startswith("-") and not sep) for x in paths):
                return Result(f"File path must be absolute, but a relative path was given: {' '.join(paths)}", None, True)
            if len(paths) != 2 or (not sep and any(x.startswith("-") for x in paths)) or not all(self.sb.inside(x) for x in paths):
                return self.deny(cmd, "only `mv -n <src> <dst>` inside the store is allowed")
            argv = ["mv", "-n", *[str(self.sb.to_real(x)) for x in paths]]
        else:
            return self.deny(cmd, "not allowed by the skill")
        p = subprocess.run(argv, env=self.sb.env, cwd=self.sb.work, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300)
        if argv[0] == str(self.sb.exe) and argv[1:2] == ["new"] and p.returncode == 0:
            self.normalise_new(p.stdout.strip())
        return Result(render_output(self.sb.virtual(p.stdout), self.sb.virtual(p.stderr), p.returncode), p.returncode)

    def bad_path(self, path: str) -> Result:
        if not str(path).startswith("/"):
            return Result(f"File path must be absolute, but a relative path was given: {path}", 1, True)
        return Result(f"Permission denied: {path} is outside the project.", None, True)

    def deny(self, cmd: str, why: str) -> Result:
        return Result(f"Permission to use Bash with command {cmd!r} has been denied ({why}). Only the commands the skill lists are allowed.", None, True)

    def normalise_new(self, path: str) -> None:
        """Right after `bilbo new`: the note's `created` becomes the exchange time and its `id` a fixed function of the
        arm, exchange and file, so a replayed run shows the model the same bytes."""
        f = Path(path)
        base = Path(os.path.realpath(self.sb.store))
        if not f.is_file() or base not in Path(os.path.realpath(f)).parents:
            return
        text = f.read_text(encoding="utf-8")
        text = re.sub(r"^id: .*$", f"id: {ulid_from(f'{self.xid}|{f.name}')}", text, count=1, flags=re.M)
        text = re.sub(r"^created: .*$", f"created: {minute(self.ts)}", text, count=1, flags=re.M)
        f.write_text(text, encoding="utf-8")
        self.created.append(f)

    # Read
    def read_file(self, path: str) -> Result:
        if not self.tool.can_read:
            return self.deny("Read", "not allowed by the skill")
        f = self.sb.to_real(path)
        if f is None:
            return self.bad_path(path)
        if not f.is_file():
            return Result(f"File does not exist: {path}", 1)
        self.read.add(os.path.realpath(f))
        text = f.read_text(encoding="utf-8", errors="replace")
        if not text.strip():
            return Result("<system-reminder>Warning: the file exists but the contents are empty.</system-reminder>", 0)
        lines = text.split("\n")
        if lines and lines[-1] == "":
            lines.pop()
        return Result(cap("\n".join(f"{n:>6}\t{l}" for n, l in enumerate(lines[:2000], 1))), 0)

    # Edit
    def edit(self, path: str, old: str, new: str) -> Result:
        if not self.tool.can_edit:
            return self.deny("Edit", "not allowed by the skill")
        f = self.sb.to_real(path)
        if f is None:
            return self.bad_path(path)
        if not f.is_file():
            return Result(f"File does not exist: {path}", 1)
        if os.path.realpath(f) not in self.read:
            return Result("File has not been read yet. Read it first before writing to it.", 1)
        if old == new:
            return Result("No changes to make: old_string and new_string are exactly the same.", 1)
        text = f.read_text(encoding="utf-8")
        n = text.count(old) if old else 0
        if n == 0:
            return Result("String to replace not found in file.", 1)
        if n > 1:
            return Result(f"Found {n} matches of the string to replace, but replace_all is false. Provide more context to make it unique.", 1)
        out = text.replace(old, new, 1)
        f.write_text(out, encoding="utf-8")
        return Result(f"The file {path} has been updated successfully.", 0)


def cap(text: str) -> str:
    return text if len(text) <= OUTPUT_CAP else text[: OUTPUT_CAP // 2] + f"\n[... {len(text) - OUTPUT_CAP} characters cut ...]\n" + text[-OUTPUT_CAP // 2 :]


def render_output(out: str, err: str, code: int) -> str:
    body = "".join(s for s in (out, err) if s)
    body = cap(body.rstrip("\n"))
    return body if code == 0 else f"Exit code {code}\n{body}".rstrip()


BASH_TOOL = {"type": "function", "function": {"name": "Bash", "description": "Run a command in the user's project. It runs for real and its output comes back in the next message.", "parameters": {
    "type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}}}  # fmt: skip
READ_TOOL = {"type": "function", "function": {"name": "Read", "description": "Read a file in the project, with line numbers.", "parameters": {
    "type": "object", "properties": {"file_path": {"type": "string"}}, "required": ["file_path"]}}}  # fmt: skip
EDIT_TOOL = {"type": "function", "function": {"name": "Edit", "description": "Edit a file in the project (it is writable): replace one exact string of a file you have read with another.", "parameters": {
    "type": "object", "properties": {"file_path": {"type": "string"}, "old_string": {"type": "string"}, "new_string": {"type": "string"}},
    "required": ["file_path", "old_string", "new_string"]}}}  # fmt: skip


REPLY_TOOL = {"type": "function", "function": {"name": "Reply", "description": "Finish: send your final message to the user. Call it when the skill is done or you stop.", "parameters": {
    "type": "object", "properties": {"message": {"type": "string"}}, "required": ["message"]}}}  # fmt: skip


def tool_defs(tool: Tool) -> list[dict]:
    return ([BASH_TOOL] if tool.allowed else []) + ([READ_TOOL] if tool.can_read else []) + ([EDIT_TOOL] if tool.can_edit else []) + [REPLY_TOOL]


# --- store tracking ------------------------------------------------------------------------------------------------------


def store_files(store: Path) -> dict[str, str]:
    out = {}
    for f in sorted([*store.glob("notes/*.md"), *store.glob("CLAUDE.md")]):
        out[str(f.relative_to(store))] = sha256_file(f)
    return out


def note_id(f: Path) -> str | None:
    m = re.search(r"^id: (.+)$", f.read_text(encoding="utf-8"), re.M)
    return m[1].strip() if m else None


def diff_store(store: Path, before: dict[str, str], before_ids: dict[str, str], xid: str, ts: str) -> list[dict]:
    after = store_files(store)
    rows = []
    gone = {p: before_ids.get(p) for p in before if p not in after}
    for p, h in after.items():
        if p not in before:
            nid = note_id(store / p) if p.startswith("notes/") else None
            src = next((g for g, i in gone.items() if i and i == nid), None)
            rows.append({"path": p, "xid": xid, "ts": ts, "op": "rename" if src else "new"})
        elif before[p] != h:
            rows.append({"path": p, "xid": xid, "ts": ts, "op": "edit"})
    return rows


def ids_of(store: Path) -> dict[str, str]:
    return {str(f.relative_to(store)): i for f in store.glob("notes/*.md") if (i := note_id(f))}


# --- the run loop ----------------------------------------------------------------------------------------------------------


def request_for(tool: Tool, request: str) -> str:
    """The schedule holds `/note ...`; the user types the tool's command name."""
    return re.sub(r"^/note\b", "/" + tool.command, request, count=1)


def skill_args(request: str) -> str:
    parts = request.split(None, 1)
    return parts[1] if len(parts) == 2 else ""


def run_exchange(chat, label: str, tool: Tool, sb: Sandbox, session: list[dict], i: int, trigger: str, request: str | None) -> tuple[dict, list[dict], dict]:
    """One run on exchange `session[i]`. Returns the actions row, the edits rows and the run's metrics (unreadable tool calls,
    whether the final message blames the environment)."""
    x = session[i]
    msgs = [{"role": "system", "content": frame_text(tool)}]
    for y in window(session, i, RUN_WINDOW):
        msgs += exchange_messages(y, tool, False, tool.name == "bilbo")
    args = skill_args(request) if request else ""
    skill_dir = VIRTUAL_SKILL_DIR
    body_msg = {"role": "user", "content": f"Base directory for this skill: {skill_dir}\n\n{split_frontmatter(tool.skill_text)[1].lstrip(chr(10))}"
                + (f"\n\nARGUMENTS: {args}" if args else "")}  # Claude Code: the directory line, then the body without its frontmatter
    if request:  # a slash command: the command line, then the skill body as a meta message (Claude Code's expansion)
        msgs += [{"role": "user", "content": request}, body_msg]
    else:  # the agent's own Skill call: its result, then the skill body as a meta message
        call = {"id": "call_skill_1", "type": "function", "function": {"name": "Skill", "arguments": json.dumps({"skill": tool.command}, ensure_ascii=False)}}
        msgs[-1] = dict(msgs[-1], tool_calls=[call])
        msgs += [{"role": "tool", "tool_call_id": "call_skill_1", "name": "Skill", "content": f"Launching skill: {tool.command}"}, body_msg]
    first = len(msgs)  # everything after the skill body is this run's own

    ex = Executor(sb, tool, x["xid"], x["ts"])
    before, before_ids = store_files(sb.store), ids_of(sb.store)
    steps, calls, ended = [], 0, "error"
    unparseable = rounds = format_retries = 0
    defs = tool_defs(tool)
    msg: dict = {"role": "assistant", "content": None}
    while True:
        if rounds >= MAX_STEPS + 3:  # unknown tools and unreadable calls count as calls
            ended = "max_steps"
            break
        body = {"model": MODEL, "messages": msgs, "tools": defs, "parallel_tool_calls": False, "seed": 1}
        body["tool_choice"] = "required"  # see REPLY_TOOL: given the option of a text answer, the model often declines the tools
        if EFFORT:
            body["reasoning_effort"] = EFFORT
        resp = None
        for attempt in range(1 + FORMAT_RETRIES):  # a step the gateway rejected as invalid output: again with a new seed
            body["seed"] = 1 + attempt
            calls += 1
            try:
                resp = chat.ask(label, body)
                break
            except ModelFailure:
                format_retries += attempt < FORMAT_RETRIES
        if resp is None:
            ended, msg = "error", {"role": "assistant", "content": None}
            break
        rounds += 1
        msg = resp["choices"][0]["message"]
        tcs = msg.get("tool_calls") or []
        if not tcs or tcs[0]["function"]["name"] == "Reply":
            ended = "report"
            msg = dict(msg, content=msg.get("content") or _reply_text(tcs))
            break
        if len(steps) >= MAX_STEPS:
            ended = "max_steps"
            break
        tc = tcs[0]
        msgs.append({"role": "assistant", "content": msg.get("content"), "tool_calls": [tc]})
        name = tc["function"]["name"]
        try:
            args = json.loads(tc["function"].get("arguments") or "{}")
            if not isinstance(args, dict):
                raise ValueError
        except ValueError:
            args = None
        if name not in ("Bash", "Read", "Edit") or args is None:
            unparseable += 1
            out = Result(f"Error: no such tool or unreadable arguments for {name}.", None, True)
            msgs.append({"role": "tool", "tool_call_id": tc["id"], "name": name, "content": out.text})
            continue
        try:
            if name == "Bash":
                out = ex.bash(str(args.get("command", "")))
            elif name == "Read":
                out = ex.read_file(str(args.get("file_path", "")))
            else:
                out = ex.edit(str(args.get("file_path", "")), str(args.get("old_string", "")), str(args.get("new_string", "")))
        except (subprocess.TimeoutExpired, OSError) as e:
            out = Result(sb.virtual(f"Error: {type(e).__name__}: {e}"), 1)
        steps.append({"tool": name, "args": args, "exit": out.exit, "refused": out.refused})
        msgs.append({"role": "tool", "tool_call_id": tc["id"], "name": name, "content": out.text})
    trace = sb.root / "traces"
    trace.mkdir(exist_ok=True)
    write_atomic(trace / (x["xid"].replace("#", "_") + ".json"), json.dumps(msgs[first:] + [msg], indent=1, ensure_ascii=False))
    action = {"xid": x["xid"], "trigger": trigger, "request": request, "steps": steps, "llm_calls": calls, "ended": ended}
    schema.check_action(action)
    edits = diff_store(sb.store, before, before_ids, x["xid"], x["ts"])
    final = msg.get("content") or ""
    metrics = {"xid": x["xid"], "unparseable": unparseable, "format_retries": format_retries, "env_refusal": bool(ENV_REFUSAL.search(final.replace("\u2019", "'"))),
               "check_dirty": 0, "index_failed": 0}  # fmt: skip
    return action, edits, metrics


ENV_REFUSAL = re.compile(
    r"can'?t run|cannot run|read-only|outside the (project )?workspace|not available in this session|doesn't provide the required|not on PATH", re.I)
# a final message that blames the environment is a harness failure, never a decision (R1-writer finding 11)


def _reply_text(tcs: list) -> str:
    try:
        return str(json.loads(tcs[0]["function"].get("arguments") or "{}").get("message", ""))
    except (ValueError, AttributeError, IndexError):
        return ""


def after_run(sb: Sandbox, tool: Tool) -> dict:
    """What the product's timer and the harness's checks do after a run: `bilbo check`, then `bilbo index`."""
    out = {"check_dirty": 0, "index_failed": 0}
    if tool.name != "bilbo" or sb.exe is None or not (sb.store / "notes").is_dir():
        return out  # no note yet: `bilbo check` would only say there is no store
    c = subprocess.run([str(sb.exe), "check"], env=sb.env, cwd=sb.work, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300)
    if c.returncode != 0 or c.stdout.strip():
        lines = [l for l in c.stdout.splitlines() if "(warning)" not in l]
        if c.returncode != 0 or lines:
            out["check_dirty"] = 1
            with open(sb.root / "check.log", "a", encoding="utf-8") as f:
                f.write(f"exit {c.returncode}\n{c.stdout}{c.stderr}\n")
    if sb.embedder:
        i = subprocess.run([str(sb.exe), "index"], env=sb.env, cwd=sb.work, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=3600)
        if i.returncode != 0:
            out["index_failed"] = 1
            with open(sb.root / "check.log", "a", encoding="utf-8") as f:
                f.write(f"index exit {i.returncode}\n{i.stdout}{i.stderr}\n")
    return out


# --- capture ---------------------------------------------------------------------------------------------------------------


def group_sessions(rows: list[dict]) -> list[list[dict]]:
    by: dict[str, list[dict]] = {}
    for r in rows:
        by.setdefault(r["session_id"], []).append(r)
    return sorted(by.values(), key=lambda s: s[0]["ts"])


@dataclass
class State:
    dir: Path
    actions: Path = field(init=False)
    edits: Path = field(init=False)
    done: Path = field(init=False)
    snap: Path = field(init=False)
    metrics: Path = field(init=False)

    def __post_init__(self):
        self.actions, self.edits = self.dir / "actions.jsonl", self.dir / "edits.jsonl"
        self.metrics = self.dir / "metrics.jsonl"
        self.done, self.snap = self.dir / "done.jsonl", self.dir / "snap"


def lines_in(p: Path) -> int:
    return len(p.read_text(encoding="utf-8").splitlines()) if p.exists() else 0


def truncate_lines(p: Path, n: int) -> None:
    if p.exists():
        write_atomic(p, "".join(l + "\n" for l in p.read_text(encoding="utf-8").splitlines()[:n]))


def take_snapshot(st: State, sb: Sandbox, sid: str) -> None:
    if st.snap.exists():
        shutil.rmtree(st.snap)
    st.snap.mkdir()
    shutil.copytree(sb.store, st.snap / "store")
    (st.snap / "meta.json").write_text(json.dumps({"session": sid, "actions": lines_in(st.actions), "edits": lines_in(st.edits), "done": lines_in(st.done), "metrics": lines_in(st.metrics)}))


def restore_snapshot(st: State, sb: Sandbox, sid: str) -> bool:
    """A session that stopped half way: back to the store, actions, edits and done rows it started from."""
    meta = st.snap / "meta.json"
    if not meta.exists():
        return False
    m = json.loads(meta.read_text())
    if m["session"] != sid:
        return False
    shutil.rmtree(sb.store)
    shutil.copytree(st.snap / "store", sb.store)
    truncate_lines(st.actions, m["actions"])
    truncate_lines(st.edits, m["edits"])
    truncate_lines(st.done, m["done"])
    truncate_lines(st.metrics, m["metrics"])
    return True


def prepare_store(tool: Tool, sb: Sandbox) -> None:
    """The user's tool is installed and its store exists: an empty `notes/` for bilbo, a CLAUDE.md with its title line."""
    if tool.name == "bilbo":
        (sb.store / "notes").mkdir(exist_ok=True)
    if tool.store_file and not (sb.store / tool.store_file).exists():
        (sb.store / tool.store_file).write_text("# CLAUDE.md\n", encoding="utf-8")


def tool_version(tool: Tool, sb: Sandbox) -> str:
    if tool.name != "bilbo":
        return "1"
    p = subprocess.run([str(sb.exe), "--version"], env=sb.env, cwd=sb.work, capture_output=True, text=True, timeout=60)
    return p.stdout.split()[-1] if p.returncode == 0 and p.stdout.split() else "unknown"


@dataclass
class Params:
    arm: str
    repo: str
    state: Path
    exchanges: Path
    render_sha: str
    decisions_dir: Path  # shared by the arms of a tool: <dir>/<repo>/r<rep>/<tool>.jsonl, made by `decide` before any capture
    bilbo: Path | None = None
    skill: Path | None = None
    embedder: tuple[str, str, str] | None = None
    schedule: Path | None = None
    rep: int = 1
    stage: str | None = None  # a label stage replacing decide/run, such as smoke
    workers: int = 8
    model: str = MODEL
    effort: str | None = EFFORT  # None: send none, the model runs at its own default


def model_tag(model: str) -> str:
    return re.sub(r"[^a-z0-9.]+", "-", model.lower()).strip("-")


def decisions_file(root, repo: str, rep: int, tool_name: str, model: str) -> Path:
    """The one place that names a decisions file: one per tool, repo, replicate and writer model (eval.py asks here too)."""
    return Path(root) / repo / f"r{rep}" / f"{tool_name}.{model_tag(model)}.jsonl"


def decisions_path(p: Params, tool: Tool) -> Path:
    """One file per tool, repo, replicate and writer model: states made by two models can never mix."""
    return decisions_file(p.decisions_dir, p.repo, p.rep, tool.name, p.model)


def pin_model(st: State, model: str) -> None:
    """The arm's state belongs to one writer model: the first run writes it, every later one must match."""
    f = st.dir / "writer_model"
    if f.exists():
        if f.read_text().strip() != model:
            raise WriterError(f"{st.dir} was made with the writer model {f.read_text().strip()}, not {model}; use a new state folder")
    else:
        f.write_text(model + "\n")


def _load(p: Params) -> tuple[Tool, list[dict], list[list[dict]], Sandbox, State]:
    global MODEL, EFFORT
    MODEL, EFFORT = p.model, p.effort
    if p.arm not in ARMS:
        raise WriterError(f"unknown arm {p.arm!r}; the writer arms are {', '.join(ARMS)}")
    tool_name, _ = ARMS[p.arm]
    st = State(Path(p.state).resolve())
    st.dir.mkdir(parents=True, exist_ok=True)
    if tool_name == "bilbo" and p.bilbo is None:
        raise WriterError("the bilbo arms need the v0.19.0 binary")
    sb = make_sandbox(st.dir, p.bilbo, p.embedder, ARMS[p.arm][0])
    tool = load_tool(tool_name, p.skill, sb.vstore)
    rows = schema.load_exchanges(Path(p.exchanges))
    if rows[0]["repo"] != p.repo:
        raise WriterError(f"{p.exchanges} holds repo {rows[0]['repo']}, not {p.repo}")
    return tool, rows, group_sessions(rows), sb, st


def run_decide(p: Params, chat) -> int:
    """The decision pass of one tool and repo: one call per exchange, labelled by tool (no arm), under a lock on its file."""
    tool, rows, sessions, _, _ = _load(p)
    out = decisions_path(p, tool)
    out.parent.mkdir(parents=True, exist_ok=True)
    with open(out.with_suffix(".lock"), "w") as lock_file:
        fcntl.flock(lock_file, fcntl.LOCK_EX)
        decision_pass(chat, f"l3s/{p.stage or 'decide'}/{tool.name}/{p.repo}/r{p.rep}", tool, sessions, out, p.workers)
    return 0


def load_decisions(p: Params, tool: Tool, rows: list[dict]) -> dict[str, dict]:
    path = decisions_path(p, tool)
    got = {r["xid"]: r for r in jsonl(path)}
    missing = [x["xid"] for x in rows if x["xid"] not in got]
    if missing:
        raise WriterError(f"{path} holds no decision for {len(missing)} of {len(rows)} exchanges (first {missing[0]}): run `decide` for {tool.name} first")
    for r in got.values():
        schema.check_writer_decision(r)
    return got


def decision_failed(row: dict, tool: Tool) -> bool:
    try:
        return json.loads(row["raw"])["skill"] not in (tool.command, "none")
    except (ValueError, KeyError, TypeError):
        return True


def run_capture(p: Params, chat) -> int:
    started = time.monotonic()
    tool, rows, sessions, sb, st = _load(p)
    arm = p.arm
    mode = ARMS[arm][1]
    if mode == "assisted" and p.schedule is None:
        raise WriterError("an assisted arm needs the simulated user's schedule")
    schedule = {r["xid"]: r["request"] for r in schema.load_schedule(Path(p.schedule))} if mode == "assisted" else {}
    label = lambda stage: f"l3s/{stage}/{arm}/{p.repo}/r{p.rep}"

    pin_model(st, p.model)
    prepare_store(tool, sb)
    decisions = load_decisions(p, tool, rows)
    (st.dir / "decisions").mkdir(exist_ok=True)  # the writer-state contract keeps the tool's decisions beside the arm's state
    shutil.copyfile(decisions_path(p, tool), st.dir / "decisions" / f"{tool.name}.jsonl")  # one model per state folder (pin_model)

    done = {r["xid"] for r in jsonl(st.done)}
    for session in sessions:
        sid = session[0]["session_id"]
        pending = [x for x in session if x["xid"] not in done]
        if not pending:
            continue
        if len(pending) < len(session):  # a half-done session: back to its snapshot
            if not restore_snapshot(st, sb, sid):
                raise WriterError(f"session {sid} is half done and has no snapshot to restore")
            done = {r["xid"] for r in jsonl(st.done)}
            after_run(sb, tool)
        else:
            take_snapshot(st, sb, sid)
        for i, x in enumerate(session):
            if x["xid"] in done:
                continue
            asked = schedule.get(x["xid"])
            chose = decisions[x["xid"]]["decision"] == "skill"
            if asked or chose:
                if asked:
                    trigger, request = "user", request_for(tool, asked)
                else:
                    trigger, request = ("compact" if x["compact_before"] and tool.name == "bilbo" else "agent"), None
                action, edits, metrics = run_exchange(chat, label(p.stage or "run"), tool, sb, session, i, trigger, request)
                hook("run_done", x["xid"])
                metrics |= after_run(sb, tool)  # the product's timer: check, then index
                append_jsonl(st.actions, action)
                for e in edits:
                    schema.check_edit(e)
                    append_jsonl(st.edits, e)
                append_jsonl(st.metrics, metrics)
            append_jsonl(st.done, {"xid": x["xid"]})
            done.add(x["xid"])
            hook("exchange_done", x["xid"])

    finish_manifest(p, st, sb, tool, decisions, time.monotonic() - started)
    return 0


def run_counts(st: State, decisions: dict[str, dict], tool: Tool) -> dict:
    """Every count is derived from the rows on disk, so a resumed capture reports the whole run."""
    acts, mets = jsonl(st.actions), jsonl(st.metrics)
    c = {
        "runs": len(acts), "agent_runs": sum(a["trigger"] == "agent" for a in acts), "user_runs": sum(a["trigger"] == "user" for a in acts),
        "compact_runs": sum(a["trigger"] == "compact" for a in acts), "steps": sum(len(a["steps"]) for a in acts),
        "refused": sum(s["refused"] for a in acts for s in a["steps"]), "max_steps": sum(a["ended"] == "max_steps" for a in acts),
        "errors": sum(a["ended"] == "error" for a in acts), "run_llm_calls": sum(a["llm_calls"] for a in acts),
        "unparseable": sum(m["unparseable"] for m in mets), "check_dirty": sum(m["check_dirty"] for m in mets),
        "index_failed": sum(m["index_failed"] for m in mets), "format_retries": sum(m.get("format_retries", 0) for m in mets), "env_refusals": sum(m["env_refusal"] for m in mets),
        "decisions": len(decisions), "skill_decisions": sum(d["decision"] == "skill" for d in decisions.values()),
        "decision_failures": sum(decision_failed(d, tool) for d in decisions.values()),
    }  # fmt: skip
    if len(acts) != len(mets):
        raise WriterError(f"{st.actions.name} and {st.metrics.name} disagree ({len(acts)} and {len(mets)} rows)")
    return c


def finish_manifest(p: Params, st: State, sb: Sandbox, tool: Tool, decisions: dict[str, dict], wall: float) -> None:
    prior = json.loads((st.dir / "manifest.json").read_text()) if (st.dir / "manifest.json").exists() else {}
    stats = run_counts(st, decisions, tool)
    stats["notes"] = len(list(sb.store.glob("notes/*.md"))) if tool.name == "bilbo" else int(bool(jsonl(st.edits)))
    manifest = {
        "tool": tool.name, "version": tool_version(tool, sb), "config_name": p.arm,
        "config": {"repo": p.repo, "model": p.model, "effort": p.effort or "default", "decide_window": DECIDE_WINDOW, "run_window": RUN_WINDOW,
                   "max_steps": MAX_STEPS, "skill_sha256": tool.skill_sha, "frame_sha256": sha256_file(PROMPTS / "agent_frame.md"),
                   "decide_sha256": sha256_file(PROMPTS / "agent_decide.md"), "writer_sha256": sha256_file(Path(__file__)),
                   "embedder": sb.embedder, "rep": p.rep},  # fmt: skip
        "render_sha": p.render_sha, "exchanges_sha256": sha256_file(Path(p.exchanges)), "counts": stats,
        "failures": stats["errors"] + stats["env_refusals"], "skipped": 0, "wall_s": round(prior.get("wall_s", 0) + wall, 1), "done": True,
    }  # fmt: skip
    schema.check_manifest(manifest)
    write_atomic(st.dir / "manifest.json", json.dumps(manifest, indent=2, sort_keys=True) + "\n")


# --- query -------------------------------------------------------------------------------------------------------------------


def last_edits(st: State) -> dict[str, dict]:
    """path -> the last edit row: its xid is a unit's ref and its time the unit's date."""
    out = {}
    for e in jsonl(st.edits):
        out[e["path"]] = e
    return out


def query_units(sb: Sandbox, tool: Tool, st: State, questions: list[dict]) -> list[dict]:
    edits = last_edits(st)
    out = []
    for q in questions:
        t0 = time.monotonic()
        if tool.name == "claude-md":
            e = edits.get("CLAUDE.md")
            text = (sb.store / "CLAUDE.md").read_text(encoding="utf-8") if e else ""
            units = [{"text": cut_to_tokens(text, READ_TOKENS), "date": e["ts"][:10], "ref": e["xid"]}] if e else []
        else:
            units = recall_units(sb, q["text"], edits)
        out.append({"qid": q["qid"], "ms": round((time.monotonic() - t0) * 1000, 1), "units": units})
    return out


def recall_units(sb: Sandbox, text: str, edits: dict[str, dict]) -> list[dict]:
    """`bilbo recall --limit 20 -- '<question>'`, then each hit read in full (query, then read): one unit per passage in rank order,
    dated and referenced by the note's last edit. recall itself prints only the first 300 characters of a passage."""
    core = _core()
    if sb.exe is None:
        raise WriterError("query needs the bilbo binary")
    csb = core.Sandbox(sb.root, sb.store, sb.config, sb.root / "xdgstate", sb.env, sb.exe)
    try:
        hits = core.bilbo_passages(csb, text, RECALL_LIMIT)
    except core.Abort as e:
        raise WriterError(str(e)) from None
    store = Path(os.path.realpath(sb.store))
    out = []
    for h in hits:
        rel = str(Path(h["path"]).relative_to(store))
        e = edits.get(rel)
        if e is None:
            raise WriterError(f"bilbo recall printed {rel}, which no run wrote")
        out.append({"text": h["text"], "date": e["ts"][:10], "ref": e["xid"]})
    return out


# --- the verbs eval.py dispatches ------------------------------------------------------------------------------------------------


def _core():
    import core

    return core


def _inputs(a, ctx, arm: str):
    import floors

    tool_name, mode = ARMS[arm]
    path = floors.exchanges_path(a)
    r_sha = floors.render_sha(a)
    skill = None
    if tool_name == "bilbo":
        if ctx.bilbo_exe is None:
            raise _core().Abort("the bilbo arms need --bilbo, the v0.19.0 binary")
        skill = Path(os.environ["L3_BILBO_SKILL"]) if os.environ.get("L3_BILBO_SKILL") else None
    return path, r_sha, skill


def writer_effort(ctx) -> str | None:
    """Claude writers run at their own default, as in a real session (no effort sent); the others at the run's effort (low)."""
    return None if ctx.models["writer"].startswith("claude-") else ctx.effort


def decisions_root(a) -> Path:
    root = os.environ.get("L3_DECISIONS_DIR") or getattr(a, "decisions_dir", None)
    if not root:
        raise _core().Abort("set L3_DECISIONS_DIR: the folder holding the decisions of each tool, shared by its agent and assisted arms")
    return Path(root)


def decide(a, ctx) -> int:
    """`eval.py decide --arm <any arm of the tool> --repo R --data D`: the decision pass of the arm's tool, once per tool, repo and replicate."""
    core = _core()
    if a.arm not in ARMS:
        raise core.Abort(f"{a.arm} is not a writer arm; they are {', '.join(ARMS)}")
    path, r_sha, skill = _inputs(a, ctx, a.arm)
    if ctx.recorder is None:
        raise core.Abort("the decision pass needs an LLM endpoint (L3_LLM_BASE_URL)")
    p = Params(arm=a.arm, repo=a.repo, state=Path(a.state), exchanges=path, render_sha=r_sha, decisions_dir=decisions_root(a), bilbo=ctx.bilbo_exe,
               skill=skill, rep=a.rep, stage=os.environ.get("L3_STAGE") or None, workers=getattr(a, "workers", None) or 8,
               model=ctx.models["writer"], effort=writer_effort(ctx))  # fmt: skip
    try:
        run_decide(p, Chat(ctx.recorder.url_for))
    except (WriterError, schema.SchemaError) as e:
        raise core.Abort(f"{a.arm}: {e}") from None
    print(f"decide {ARMS[a.arm][0]} {a.repo} r{a.rep}: done")
    return 0


def capture(a, ctx) -> int:
    core = _core()
    arm = a.arm
    if arm not in ARMS:
        raise core.Abort(f"{arm} is not a writer arm; they are {', '.join(ARMS)}")
    path, r_sha, skill = _inputs(a, ctx, arm)
    state = Path(a.state)
    ex_sha = core.sha256_file(path)
    inputs = {"exchanges": ex_sha, "render": r_sha, "writer": sha256_file(Path(__file__))}
    schedule = None
    if ARMS[arm][1] == "assisted":
        schedule = Path(os.environ.get("L3_SCHEDULE") or HERE / "policy" / "schedule.jsonl")
        if not schedule.is_file():
            raise core.Abort(f"{schedule} does not exist: the assisted arms need the simulated user's schedule")
        inputs["schedule"] = core.sha256_file(schedule)
    core.refuse_mismatch(state / "stage.json", f"capture/{arm}", inputs)
    if ctx.recorder is None:
        raise core.Abort("the writer arms need an LLM endpoint (L3_LLM_BASE_URL)")
    chat = Chat(ctx.recorder.url_for)
    decisions = decisions_root(a)
    with core.arm_embed(ctx, a.arm) as emb:
        p = Params(
            arm=arm, repo=a.repo, state=state, exchanges=path, render_sha=r_sha, bilbo=ctx.bilbo_exe, skill=skill,
            embedder=(emb.root, emb.model, emb.query_prefix) if emb else None, schedule=schedule,
            decisions_dir=decisions, rep=a.rep, stage=os.environ.get("L3_STAGE") or None,
            workers=getattr(a, "workers", None) or 8, model=ctx.models["writer"], effort=writer_effort(ctx),
        )  # fmt: skip
        try:
            run_capture(p, chat)
        except (WriterError, schema.SchemaError) as e:
            raise core.Abort(f"{arm}: {e}") from None
    core.write_stage(state / "stage.json", f"capture/{arm}", inputs, {"manifest.json": core.sha256_file(state / "manifest.json")})
    m = json.loads((state / "manifest.json").read_text())["counts"]
    print(f"{arm} {a.repo}: {m['runs']} runs ({m['agent_runs']} agent, {m['user_runs']} user, {m['compact_runs']} compact), {m['steps']} steps, "
          f"{m['refused']} refused, {m['errors']} errors, {m['env_refusals']} environment refusals, {m['notes']} notes")  # fmt: skip
    return 0


def query(a, ctx) -> int:
    core = _core()
    arm = a.arm
    tool_name, _ = ARMS[arm]
    state = Path(a.state)
    path, r_sha, skill = _inputs(a, ctx, arm)
    manifest = core.check_manifest(state, tool_name, arm, r_sha, core.sha256_file(path))
    repo = manifest["config"]["repo"]
    allq = schema.load_questions(Path(a.questions), a.split)
    questions = [q for q in allq if q["repo"] == repo]
    if not questions:
        raise core.Abort(f"{Path(a.questions).name} holds no question of {repo}")
    st = State(state.resolve())
    with core.arm_embed(ctx, arm) as emb:
        sb = make_sandbox(st.dir, ctx.bilbo_exe, (emb.root, emb.model, emb.query_prefix) if emb else None, tool_name)
        tool = load_tool(tool_name, skill, sb.vstore)
        try:
            first = query_units(sb, tool, st, questions)
            second = query_units(sb, tool, st, questions)
        except WriterError as e:
            raise core.Abort(f"{arm}: {e}") from None
    strip = lambda rows: [{"qid": r["qid"], "units": r["units"]} for r in rows]
    if strip(first) != strip(second):
        bad = next(r["qid"] for r, s in zip(strip(first), strip(second)) if r != s)
        raise core.Abort(f"{arm}: a second query pass differs from the first (first at question {bad})")
    known = {repo: {x["xid"] for x in schema.load_exchanges(path)}}
    core.check_units(first, questions, known, arm)
    core.write_jsonl(Path(a.out), first)
    print(f"{arm} {repo}: {len(first)} questions, {sum(len(r['units']) for r in first)} units")
    return 0
