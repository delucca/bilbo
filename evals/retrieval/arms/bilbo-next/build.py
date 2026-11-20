"""Build bilbo's stores from the sessions of /corpus with the bilbo binary of the image: `bilbo remember` for every
repository, then `bilbo reindex`, in one store of every repository (scope all) or one store a repository (scope repo). With
BILBO_EVAL_DERIVE set (a JSON object, below) each store also gets the notes `bilbo derive` writes for its sessions, between
the remembering and the reindex, through the provider the settings name behind cache.py.
Every `bilbo` run has its own HOME, store, state and cache folders under /work/store/stores/<name>, the same paths the
check and the search use again, since the catalog and the state folder record them. /work/store/entries.jsonl maps each
session file to its entry id; manifest.json is written last.

BILBO_EVAL_DERIVE, when set, is a JSON object: `provider` (anthropic or openai); `settings`, a flat object of
dotted derive.* keys written into the store's config.toml (the provider's real `derive.<provider>.api.endpoint`, an https
URL or an http URL at a loopback address, goes here, or the real `derive.anthropic.tool.program`, the image's tool by
default); `cache`, `{"write": false}` to replay or `{"write": true}` with the real provider behind, and
`timeout`, the seconds the cache waits for the provider, under the store's derive.timeout; `ceilings`, `{"calls", "tokens",
"refusals"}` the build stops at, held against /answers/<provider>-<way>/spent; `calls_per_run`. A folder STOP in /work/store
stops the loop."""

import fcntl
import hashlib
import ipaddress
import json
import os
import re
import shutil
import subprocess
import sys
import time
import uuid
from urllib.parse import urlsplit

import cache
from datetime import datetime, timezone
from pathlib import Path

CORPUS = Path("/corpus")
WORK = Path("/work")
STORE = WORK / "store"
STAGED = Path("/tmp/staged")
BILBO = "/usr/local/bin/bilbo"
LIMITED = bool(os.environ.get("BILBO_EVAL_SESSIONS"))
ANSWERS = Path("/answers")
LIFE = WORK / "life"
DERIVING = bool(os.environ.get("BILBO_EVAL_DERIVE", "").strip())
KEYS = {}
SESSION_ID = re.compile(r"^.+--([0-9a-f-]{36})\.jsonl$")
REMEMBER = re.compile(r"^new (\d+), appended (\d+), new generation (\d+), unchanged (\d+), refused (\d+)$")
SEARCHED = re.compile(r"^bilbo: searched ([\d,]+) entr(?:y|ies)$", re.M)
DERIVE_RUN = re.compile(r"^entries (\d+), calls (\d+), notes (\d+), dropped (\d+), refused (\d+), failed (\d+)\n"
                        r"tokens in (\d+), tokens out (\d+), seconds (\d+)$")
DERIVE_DUE = re.compile(r"^due: (\d+) entr(?:y|ies)$", re.M)
DERIVE_THIS = re.compile(r"^this run: (\d+) calls for (\d+) entries, (\d+) tokens of records to send \(derive\.calls (\d+)\)$", re.M)
DERIVE_TRIES = re.compile(r"^derive\.tries = (\d+) ", re.M)
DERIVE_KINDS = re.compile(r"^kinds: .+$", re.M)
ENTRY_ID = re.compile(r"^[0-9A-Z]{26}$")
STATES = ("due", "covered", "passed over", "left out: labels not current", "left out: no instruction",
          "left out: belongs to another entry", "left out: not named by derive.in", "left out: derive.not_in", "still growing")
DERIVE_DEFAULTS = {"calls_per_run": 20, "ceilings": {"calls": 1000, "tokens": 10000000, "refusals": 100},
                   "cache": {"write": False, "timeout": 240}}
KEY_VARIABLES = {"anthropic": "ANTHROPIC_API_KEY", "openai": "OPENAI_API_KEY"}
TOOLS = {"anthropic": "claude"}
LAUNCHER = Path("/tmp/claude")
RUN_FIELDS = ("entries", "calls", "notes", "dropped", "refused", "failed", "tokens_in", "tokens_out", "seconds")
DERIVE_TIMEOUT = re.compile(r'^derive\.timeout = "(\d+)(ms|s|m)"', re.M)
HIDDEN = re.compile(r"^(derive\.[a-z]+\.[a-z]+\.(?:headers|environment)) = ")
DRY_CALLS = 100000
REINDEX = re.compile(r"^entries (\d+), chunks (\d+), unread (\d+), failed (\d+)\nindex bytes (\d+), milliseconds (\d+)$")


def log(msg):
    print(f"bilbo-next: {msg}", file=sys.stderr, flush=True)


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def sessions():
    """Every session as (repo, path), in the order they started across all repos, as the bm25 arm orders them. A
    transcript with no timestamp (Cursor's) comes last, by path."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        start = next((t for t in map(stamp, (json.loads(line) for line in path.open() if line.strip())) if t), None)
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    limit = os.environ.get("BILBO_EVAL_SESSIONS")
    return [(repo, path) for _, repo, path in found + other][: int(limit) if limit else None]


def folders(name):
    return STORE / "stores" / name


def environment(name):
    if name is None:
        return {"PATH": "/usr/bin:/bin", "NO_COLOR": "1"}
    d = folders(name)
    return {"HOME": f"{d}/home", "BILBO_HOME": f"{d}/root", "XDG_STATE_HOME": f"{d}/state",
            "XDG_CACHE_HOME": f"{d}/cache", "BILBO_CONFIG": f"{d}/config.toml", "PATH": "/usr/bin:/bin",
            "NO_COLOR": "1"}


def bilbo(name, *args):
    env = environment(name)
    if DERIVING and name is not None:
        env["PATH"] = f"{Path(sys.executable).parent}:{env['PATH']}"
        env |= KEYS
    done = subprocess.run([BILBO, *args], env=env, capture_output=True, text=True, check=False)
    return done.returncode, done.stdout, done.stderr


def sure(what, code, out, err):
    if code:
        sys.exit(f"bilbo-next: {what} exited {code}, not 0\nstdout: {out}\nstderr: {err}")


def derive_env():
    text = os.environ.get("BILBO_EVAL_DERIVE", "")
    if not text.strip():
        return None
    given = json.loads(text)
    unknown = sorted(set(given) - {"provider", "settings", "cache", "ceilings", "calls_per_run"})
    if unknown or not given.get("provider"):
        sys.exit(f"bilbo-next: BILBO_EVAL_DERIVE: {'unknown key ' + unknown[0] if unknown else 'no provider'}")
    if given["provider"] not in tuple(KEY_VARIABLES):
        sys.exit(f"bilbo-next: BILBO_EVAL_DERIVE: the provider {given['provider']!r} is none of anthropic and openai")
    way = way_of(given)
    if way not in ("api", "tool"):
        sys.exit(f"bilbo-next: BILBO_EVAL_DERIVE: the way {way!r} is none of api and tool")
    if way == "tool" and given["provider"] not in TOOLS:
        sys.exit(f"bilbo-next: BILBO_EVAL_DERIVE: the provider {given['provider']!r} has no way 'tool'")
    endpoint = given.get("settings", {}).get(f"derive.{given['provider']}.api.endpoint", "")
    if "@" in endpoint:
        sys.exit("bilbo-next: BILBO_EVAL_DERIVE: the endpoint must hold no user name or password")
    if endpoint and not loopback_or_https(urlsplit(endpoint)):
        sys.exit("bilbo-next: BILBO_EVAL_DERIVE: the endpoint must be an https URL, or an http URL at a loopback address")
    return DERIVE_DEFAULTS | given | {"ceilings": DERIVE_DEFAULTS["ceilings"] | given.get("ceilings", {}),
                                      "cache": DERIVE_DEFAULTS["cache"] | given.get("cache", {})}


def way_of(derive):
    return derive.get("settings", {}).get(f"derive.{derive['provider']}.via", "api")


def folder_of(derive):
    return f"{derive['provider']}-{way_of(derive)}"


def toml_value(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return repr(value)
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return "[" + ", ".join(toml_value(v) for v in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{json.dumps(k)} = {toml_value(v)}" for k, v in sorted(value.items())) + " }"
    sys.exit(f"bilbo-next: a derive setting of the type {type(value).__name__}")


def toml_tables(settings):
    tables = {}
    for key, value in settings.items():
        table, _, leaf = key.rpartition(".")
        tables.setdefault(table, {})[leaf] = value
    return "".join(f"\n[{table}]\n" + "".join(f"{leaf} = {toml_value(v)}\n" for leaf, v in sorted(keys.items()))
                   for table, keys in sorted(tables.items()))


def give_key(derive, settings):
    if way_of(derive) != "tool":
        KEYS[key_variable(derive, settings)] = cache.STAND_IN


def loopback_or_https(where):
    try:
        where.port
    except ValueError:
        return False
    if not where.hostname or where.scheme not in ("https", "http"):
        return False
    if where.scheme == "https":
        return True
    try:
        return ipaddress.ip_address(where.hostname).is_loopback
    except ValueError:
        return False


def tool_behind(provider, settings):
    program = settings.get(f"derive.{provider}.tool.program", TOOLS[provider])
    real = shutil.which(program)
    if not real:
        sys.exit(f"bilbo-next: BILBO_EVAL_DERIVE: no program {program!r} to stand behind the cache")
    done = subprocess.run([real, "--version"], env=environment(None) | {"HOME": "/tmp", "DISABLE_AUTOUPDATER": "1"},
                          capture_output=True, text=True, check=False)
    if done.returncode or not done.stdout.strip():
        sys.exit(f"bilbo-next: {real} --version exited {done.returncode}\nstdout: {done.stdout}\nstderr: {done.stderr}")
    return real, f"{TOOLS[provider]} {done.stdout.strip()}"


def launcher(port, here):
    """By no shell, which would add variables of its own to what tool.py sends; -I, since it stands in a folder others write."""
    LAUNCHER.write_text(f"#!{sys.executable} -I\nimport runpy\nimport sys\n\nsys.argv[:1] = [{str(here / 'tool.py')!r}, "
                        f"'http://127.0.0.1:{port}/']\nrunpy.run_path(sys.argv[0], run_name='__main__')\n")
    LAUNCHER.chmod(0o755)


def start_cache(derive):
    settings, option, here = dict(derive["settings"]), derive["cache"], Path(__file__).parent
    command = [sys.executable, str(here / "cache.py"), "--dir", str(ANSWERS / folder_of(derive)), "--record", str(LIFE),
               "--timeout", str(option["timeout"])]
    if way_of(derive) == "tool":
        key = f"derive.{derive['provider']}.tool.program"
        program, upstream = tool_behind(derive["provider"], settings)
        command += ["--upstream", upstream, "--tool", program]
    else:
        key = f"derive.{derive['provider']}.api.endpoint"
        if key not in settings:
            sys.exit(f"bilbo-next: BILBO_EVAL_DERIVE: settings hold no {key}")
        real = urlsplit(settings[key])
        upstream = f"{real.scheme}://{real.netloc}"
        command += ["--upstream", upstream]
    if option["write"]:
        command += ["--write", "--ceiling", str(derive["ceilings"]["calls"])]
    environment = dict(os.environ)
    if option["write"]:
        if not os.environ.get("BILBO_EVAL_DERIVE_KEY"):
            sys.exit("bilbo-next: BILBO_EVAL_DERIVE: a build with a writer behind the cache needs BILBO_EVAL_DERIVE_KEY")
        environment["BILBO_EVAL_CACHE_KEY"] = os.environ["BILBO_EVAL_DERIVE_KEY"]
    else:
        environment.pop("BILBO_EVAL_CACHE_KEY", None)
    environment.pop("BILBO_EVAL_DERIVE_KEY", None)
    server = subprocess.Popen(command, stdout=subprocess.PIPE, text=True, env=environment)
    line = server.stdout.readline()
    if not line.startswith("port "):
        server.kill()
        sys.exit(f"bilbo-next: cache.py printed {line!r}, not its port")
    port = line.split()[1]
    if way_of(derive) == "tool":
        launcher(port, here)
        settings[key] = str(LAUNCHER)
    else:
        settings[key] = f"http://127.0.0.1:{port}{real.path}" + (f"?{real.query}" if real.query else "")
    return settings, server, upstream


def dry_run(out):
    due = DERIVE_DUE.search(out)
    if not due:
        sys.exit(f"bilbo-next: the dry run printed no `due:` line: {out!r}")
    entries = []
    for line in out.split("\n"):
        if line.startswith("entry\t"):
            fields = line.split("\t")
            if len(fields) != 5 or not ENTRY_ID.match(fields[1]) or fields[2] not in STATES or not fields[4].isdigit() or not (
                    fields[3] == "-" or ENTRY_ID.match(fields[3])):
                sys.exit(f"bilbo-next: an entry line of the dry run is not five fields: {line!r}")
            entries.append((fields[1], fields[2], None if fields[3] == "-" else fields[3], int(fields[4])))
    kinds = DERIVE_KINDS.search(out)
    return int(due.group(1)), kinds.group(0) if kinds else None, entries


def this_run(out):
    found = DERIVE_THIS.search(out)
    if not found:
        sys.exit(f"bilbo-next: the dry run printed no `this run:` line: {out!r}")
    return tuple(map(int, found.groups()))


class Life:
    def __init__(self, folder, record, writer):
        self.folder, self.record, self.writer = Path(folder), Path(record), writer
        self.seen, self.runs = 0, []

    def lines(self):
        found = self.record.read_text().splitlines() if self.record.exists() else []
        fresh, self.seen = found[self.seen:], len(found)
        return [line.split() for line in fresh]


def spent_by(folder):
    sums = {"asks": 0, "tokens_in": 0, "tokens_out": 0, "refused": 0}
    spent = Path(folder) / "spent"
    for line in spent.read_text().splitlines() if spent.exists() else []:
        words = line.split()
        if words[:1] == ["ask"]:
            sums["asks"] += 1
        elif words[:1] == ["run"]:
            sums["tokens_in"] += int(words[1])
            sums["tokens_out"] += int(words[2])
            sums["refused"] += int(words[3])
    return sums


def spend(folder, line):
    Path(folder).mkdir(parents=True, exist_ok=True)
    with (Path(folder) / "spent").open("a") as handle:
        handle.write(line + "\n")
        handle.flush()
        os.fsync(handle.fileno())


def seconds_of(config):
    found = DERIVE_TIMEOUT.search(config)
    if not found:
        sys.exit(f"bilbo-next: bilbo config printed no derive.timeout: {config!r}")
    return int(found.group(1)) * {"ms": 0.001, "s": 1, "m": 60}[found.group(2)]


def derive_step(name, repos, ids, derive, settings, life):
    d = folders(name)
    ceilings, per_run = derive["ceilings"], derive["calls_per_run"]
    labels = [f"{STAGED if LIMITED else CORPUS}/{repo}" for repo in repos]
    config = {"derive.enabled": True, "derive.provider": derive["provider"], "derive.in": labels,
              f"derive.{derive['provider']}.via": way_of(derive), **settings}
    (d / "config.toml").write_text("follow = []\n" + toml_tables(config))
    totals = {"runs": 0, "calls": 0, "notes": 0, "dropped": 0, "refused": 0, "failed": 0, "tokens_in": 0, "tokens_out": 0}
    code, out, err = bilbo(name, "config")
    sure("config", code, out, err)
    tries = DERIVE_TRIES.search(out)
    if not tries:
        sys.exit(f"bilbo-next: bilbo config printed no derive.tries: {out!r}")
    tries = int(tries.group(1))
    allowed = seconds_of(out)
    if derive["cache"]["timeout"] >= allowed:
        sys.exit(f"bilbo-next: BILBO_EVAL_DERIVE: cache.timeout {derive['cache']['timeout']} is not under derive.timeout, {allowed:g} seconds")
    for repo in repos:
        before, wrote, still, last = None, 0, 0, ""
        while True:
            code, out, err = bilbo(name, "derive", "--dry-run", "--calls", str(DRY_CALLS), *ids[repo])
            sure(f"derive --dry-run in {repo}", code, out, err)
            due, _, _ = dry_run(out)
            this_run(out)
            dry = out
            if before is not None:
                still = 0 if wrote or dry != before else still + 1
                if still >= tries:
                    sys.exit(f"bilbo-next: {still} runs in a row changed nothing in {repo} with {due} entries due; last run:\n{last}")
            if due == 0:
                break
            if (STORE / "STOP").exists():
                sys.exit(f"bilbo-next: stopped by {STORE / 'STOP'} with {due} entries of {repo} due")
            spent = spent_by(life.folder)
            if life.writer and (spent["tokens_in"] + spent["tokens_out"] >= ceilings["tokens"] or spent["refused"] >= ceilings["refusals"]):
                sys.exit(f"bilbo-next: a ceiling of the build is reached with {due} entries of {repo} due: {spent}, ceilings {ceilings}")
            code, out, err = bilbo(name, "derive", "--calls", str(per_run), *ids[repo])
            fresh = life.lines()
            if any(line[0] == "asked" and line[2] == "key" for line in fresh):
                sys.exit("bilbo-next: a response held the key, so nothing of it was kept and the build stops: the key is in what was "
                         "sent, or the provider sends it back; build again with another key")
            if any(line[0] == "missed" for line in fresh):
                if life.writer:
                    sys.exit(f"bilbo-next: a ceiling of the build is reached with {due} entries of {repo} due: {spent_by(life.folder)}, ceilings {ceilings}")
                sys.exit(f"bilbo-next: no kept answer for a request of {repo}, and no writer is behind the cache")
            m = DERIVE_RUN.match(out.strip())
            if code not in (0, 1) or not m:
                sys.exit(f"bilbo-next: derive exited {code} in {repo} with {due} entries due, with its two lines or without\nstdout: {out}\nstderr: {err}")
            numbers = tuple(map(int, m.groups()))
            entries, calls, notes, dropped, refused, failed, tin, tout, _ = numbers
            life.runs.append({"store": name, "repo": repo, "exit": code, **dict(zip(RUN_FIELDS, numbers))})
            if any(line[0] == "asked" for line in fresh):
                spend(life.folder, f"run {tin} {tout} {refused}")
            for key, got in (("runs", 1), ("calls", calls), ("notes", notes), ("dropped", dropped), ("refused", refused),
                             ("failed", failed), ("tokens_in", tin), ("tokens_out", tout)):
                totals[key] += got
            log(f"{repo}: derive run {totals['runs']}: {calls} calls, {notes} notes, {refused} refused; {due} due before it")
            last = f"{out.strip()}\nstderr: {err}"
            if code == 1 and calls and not (notes or refused or failed):
                sys.exit(f"bilbo-next: the provider ended a run in {repo} before any call was answered with {due} entries due\nstdout: {out}\nstderr: {err}")
            before, wrote = dry, notes
    info, kinds = {}, None
    for repo in repos:
        code, out, err = bilbo(name, "derive", "--dry-run", *ids[repo])
        sure(f"derive --dry-run in {repo}", code, out, err)
        _, kinds, found = dry_run(out)
        wrong = [e for e in found if e[1] not in ("covered", "passed over")]
        if wrong:
            sys.exit(f"bilbo-next: the entry {wrong[0][0]} of {repo} is {wrong[0][1]!r} after the derive step")
        states = {}
        for _, state, _, _ in found:
            states[state] = states.get(state, 0) + 1
        info[repo] = {"sessions": len(ids[repo]), "note_entries": sum(e[2] is not None for e in found),
                      "noted": sum(e[3] > 0 for e in found), "notes": sum(e[3] for e in found), "states": states}
    return totals, info, kinds


def prepare(name):
    d = folders(name)
    for sub in ("root", "state", "cache", "home"):
        (d / sub).mkdir(parents=True, exist_ok=True)
    (d / "config.toml").write_text("follow = []\n")


def remember(name, repo, paths, limited):
    target = f"/corpus/{repo}"
    if limited:
        staged = STAGED / repo
        shutil.rmtree(staged, ignore_errors=True)
        staged.mkdir(parents=True)
        for p in paths:
            shutil.copy2(p, staged / p.name)
        target = str(staged)
    code, out, err = bilbo(name, "remember", target, "--kind", "session")
    sure(f"remember {repo}", code, out, err)
    first = out.splitlines()[0] if out else ""
    m = REMEMBER.match(first)
    if not m:
        sys.exit(f"bilbo-next: remember {repo}: unexpected output {out!r}")
    new, appended, generation, unchanged, refused = map(int, m.groups())
    if refused or appended or generation or new + unchanged != len(paths):
        sys.exit(f"bilbo-next: remember {repo}: {len(paths)} files but {first!r}; stderr {err!r}")
    return new


def entry_of(name, repo, path):
    session = SESSION_ID.match(path.name)
    if not session:
        sys.exit(f"bilbo-next: {repo}/{path.name} has no session id in its name")
    code, out, err = bilbo(name, "read", session.group(1), "--budget", "256")
    if code == 2:
        sys.exit(f"bilbo-next: {repo}/{path.name}: the session id names several entries: {err.strip()}")
    sure(f"read {session.group(1)}", code, out, err)
    head = out.split("\n", 1)[0]
    if not re.fullmatch(r"bilbo:[0-9A-Z]{26}", head):
        sys.exit(f"bilbo-next: {repo}/{path.name}: read printed {head!r}, not an entry id")
    return head.removeprefix("bilbo:")


def key_variable(derive, settings):
    return settings.get(f"derive.{derive['provider']}.api.key_env", KEY_VARIABLES[derive["provider"]])


def config_line(line):
    hidden = HIDDEN.match(line)
    return f"{hidden.group(1)} = [left out]" if hidden else line


def probe_counts(indexed, noted, sessions):
    return noted < indexed <= sessions + noted


def begin(life):
    shutil.rmtree(STORE / "stores", ignore_errors=True)
    life.record.write_text("")
    life.seen, life.runs = 0, []


def one_life(groups, by_repo, derive, life):
    begin(life)
    server, settings = None, None
    entries, stores, derived = [], {}, {"repos": {}, "totals": {}, "config": [], "kinds": []}
    if derive:
        settings, server, derived["upstream"] = start_cache(derive)
    try:
        for name, repos in groups.items():
            prepare(name)
            for repo in repos:
                new = remember(name, repo, by_repo[repo], LIMITED)
                log(f"{repo}: {len(by_repo[repo])} sessions, {new} new")
            mine = []
            for repo in repos:
                for path in by_repo[repo]:
                    mine.append({"store": name, "id": entry_of(name, repo, path), "repo": repo, "session": path.name})
            entries += mine
            note_entries, noted = 0, 0
            if derive:
                give_key(derive, settings)
                ids = {repo: [e["id"] for e in mine if e["repo"] == repo] for repo in repos}
                totals, info, kinds = derive_step(name, repos, ids, derive, settings, life)
                derived["repos"] |= info
                derived["totals"] = {k: derived["totals"].get(k, 0) + v for k, v in totals.items()}
                derived["kinds"].append(kinds)
                note_entries = sum(i["note_entries"] for i in info.values())
                noted = sum(i["noted"] for i in info.values())
                code, out, err = bilbo(name, "config")
                sure("config", code, out, err)
                derived["config"] += [config_line(line)
                                      for line in out.splitlines() if line.startswith(("derive.", "recall.notes_budget"))]
            code, out, err = bilbo(name, "reindex")
            sure("reindex", code, out, err)
            m = REINDEX.match(out.strip())
            if not m:
                sys.exit(f"bilbo-next: reindex {name}: unexpected output {out!r}")
            count, chunks, unread, failed, size, _ = map(int, m.groups())
            sessions = sum(len(by_repo[r]) for r in repos)
            want = sessions + note_entries
            if unread or failed or count != want:
                sys.exit(f"bilbo-next: reindex {name}: {sessions} sessions and {note_entries} note entries, {want} entries, but {out.strip()!r}; stderr {err!r}")
            code, out, err = bilbo(name, "recall", "--budget", "4000", "--", "user", "assistant")
            sure(f"recall in {name}", code, out, err)
            (folders(name) / "state" / "bilbo" / "queries.log").unlink(missing_ok=True)
            found = SEARCHED.search(err)
            indexed = int(found.group(1).replace(",", "")) if found else 0
            if not probe_counts(indexed, noted, sessions):
                sys.exit(f"bilbo-next: recall in {name} searched {indexed} entries; {sessions} sessions and {noted} note entries that hold a note; stderr {err!r}")
            stores[name] = {"repos": repos, "entries": sessions, "note_entries": note_entries, "noted": noted, "indexed": indexed,
                            "chunks": chunks, "index_bytes": size}
            log(f"{name}: {sessions} sessions and {note_entries} note entries, {chunks} chunks, index {size} bytes")
    finally:
        if server:
            server.terminate()
    return entries, stores, derived


def summary(record):
    asked, kept, came = 0, 0, {}
    for line in Path(record).read_text().splitlines():
        words = line.split()
        if words[:1] == ["asked"]:
            asked += 1
            kept += words[2] == "kept"
            if words[2] != "kept":
                what = "no answer" if words[2] == "none" else words[3] if words[2] == "status" else "a body with the key"
                came[what] = came.get(what, 0) + 1
    return asked, kept, came


def lives(derive, life, make):
    found = []
    while True:
        made = make()
        if not derive:
            return made, found
        asked, kept, came = summary(life.record)
        found.append({"runs": len(life.runs), "asked": asked, "kept": kept})
        if not asked:
            return made, found
        if not kept:
            what = ", ".join(f"{count} {kind}" for kind, count in sorted(came.items()))
            sys.exit(f"bilbo-next: the provider was asked {asked} times in a life and nothing it said could be kept: {what}; a replay would miss them")


def hold(folder, provider):
    Path(folder).mkdir(parents=True, exist_ok=True)
    cache.sync_folder(Path(folder).parent)
    handle = (Path(folder) / "lock").open("w")
    try:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        sys.exit(f"bilbo-next: another build is using the answers of {provider}: {Path(folder) / 'lock'} is held; it must end first")
    return handle


def drop_failures(folder):
    for answer in Path(folder).glob("*/[0-9]*"):
        if answer.is_file() and answer.read_bytes().split(b"\n", 1)[0] != b"200":
            answer.unlink()


def archive(derive, life):
    kept = STORE / "answers" / folder_of(derive)
    for line in life.record.read_text().splitlines():
        words = line.split()
        if words[:1] == ["given"]:
            target = kept / words[1] / words[2]
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(life.folder / words[1] / words[2], target)


def main():
    plan = sessions()
    scope = os.environ["BILBO_EVAL_SCOPE"]
    if scope not in ("all", "repo"):
        sys.exit(f"bilbo-next: BILBO_EVAL_SCOPE is {scope!r}, not all or repo")
    by_repo = {}
    for repo, path in plan:
        by_repo.setdefault(repo, []).append(path)
    for paths in by_repo.values():
        paths.sort()
    groups = {"all": sorted(by_repo)} if scope == "all" else {repo: [repo] for repo in sorted(by_repo)}
    log(f"{len(plan)} sessions of {len(by_repo)} repositories in {len(groups)} store(s)")

    STORE.mkdir(exist_ok=True)
    started = time.monotonic()
    derive = derive_env()
    life = Life(ANSWERS / folder_of(derive) if derive else ANSWERS, LIFE, bool(derive and derive["cache"]["write"]))
    lock = hold(life.folder, folder_of(derive)) if derive else None
    if life.writer:
        drop_failures(life.folder)
    (entries, stores, derived), found = lives(derive, life, lambda: one_life(groups, by_repo, derive, life))
    if derive:
        archive(derive, life)
        lock.close()
        derived["runs"], derived["lives"], derived["spent"] = life.runs, found, spent_by(life.folder)
    seen = {(e["store"], e["id"]) for e in entries}
    if len(seen) != len(entries):
        sys.exit("bilbo-next: two sessions share an entry id in one store")

    tmp = STORE / "entries.jsonl.part"
    tmp.write_text("".join(json.dumps(e) + "\n" for e in entries))
    tmp.rename(STORE / "entries.jsonl")
    inputs = (WORK / "key.txt").read_text().splitlines()
    code, out, err = bilbo(None, "--version")
    sure("--version", code, out, err)
    version = out.strip()
    manifest = {
        "arm": "bilbo-next",
        "key": inputs[0],
        "inputs": inputs[1:],
        "scope": scope,
        "remembered": str(STAGED if LIMITED else CORPUS),
        "bilbo": version,
        "binary_sha256": hashlib.sha256(Path(BILBO).read_bytes()).hexdigest(),
        "sessions": len(plan),
        "entries": len(entries),
        "stores": stores,
        **({"derive": derived} if derive else {}),
        "ingest_seconds": round(time.monotonic() - started),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }
    (STORE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    log(f"done: {STORE}")


if __name__ == "__main__":
    main()
