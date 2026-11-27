"""Checks of the arm that need no store, no binary and no provider, as plain asserts: `uv run python selftest.py`."""

import http.client
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.dont_write_bytecode = True
import build
import cache
import check as arm_check
import search
import tool

HERE = Path(__file__).resolve().parent
FAKE_KEY = "sk-fake-key-0123456789"
PARENT = "01M6ZGE4KTBYYH07AS2KVMSCAF"
REFUSAL = '{"stop_reason":"refusal","content":[]}'
ANSWER = '{"stop_reason":"end_turn","content":[{"type":"text","text":"a note"}]}'
FAKE_TOKEN = "sk-ant-oat01-fake-token-0123456789"
TOLD = '{"is_error":false,"stop_reason":"end_turn","result":"a note"}'
FAKE_TOOL = """#!{python}
import json, os, sys, time
if sys.argv[1:] == ["--version"]:
    print("9.9.9 (Fake)")
    sys.exit(0)
with open({seen!r}, "a") as seen:
    seen.write(json.dumps({{"argv": sys.argv[1:], "env": dict(os.environ), "folder": os.getcwd(), "stdin": sys.stdin.read(), "pid": os.getpid()}}) + "\\n")
code, out, delay = json.loads(open({plan!r}).read())[sum(1 for _ in open({seen!r})) - 1]
time.sleep(delay)
sys.stdout.buffer.write(out.encode("latin-1"))
sys.exit(code)
"""
CLAUDE_FIXTURES = HERE.parents[3] / "libs" / "llm" / "rust" / "fixtures" / "anthropic" / "tool"


def check(name, ok, detail=""):
    if not ok:
        sys.exit(f"selftest: {name}: FAILED {detail}")
    print(f"selftest: {name}: OK")


def sources():
    table = [(f"{PARENT}#g1:r5 2026-11-07", (1, 5, 5)), (f"{PARENT}#g1:r5", (1, 5, 5)),
             (f"{PARENT}#g2:r3-19 2026-11-07", (2, 3, 19)), (f"{PARENT}#g1:r3-19", (1, 3, 19)),
             (f"{PARENT}#g1:r3-19 {PARENT}#g1:r30 2026-11-07", (1, 3, 19)),
             ("2T6W9YB0DE#g1:r3-19 2026-11-07", None), ("The user asked to thin the carrot rows.", None)]
    for line, want in table:
        got = search.source(line)
        check(f"source {line[:40]!r}", (got and (got["generation"], got["first"], got["last"])) == want and (not got or got["parent"] == PARENT), got)


def notes():
    out = f"=== 2T6W9YB0DE#r4\t2 lines\n[chunk] text\nmore\n=== 3V7X0ZC1EF#r1\t3 lines\tnote\n{PARENT}#g1:r3-19 2026-11-07\nThe user asked.\nmore\n"
    hits = search.parse("q", "query", out)
    check("a note hit has its kind", [h["kind"] for h in hits] == [None, "note"], hits)
    entries = {("all", PARENT): {"id": PARENT, "repo": "r", "session": "s.jsonl"}}
    row = search.note(entries, "q", "query", "all", 1, hits[1])
    check("a note maps to its session and records", (row["session"], row["records"], row["generation"], row["repo"]) == ("s.jsonl", [3, 19], 1, "r"), row)
    bad = dict(hits[1], text="The user asked.\nmore")
    try:
        search.note(entries, "q", "query", "all", 1, bad)
    except SystemExit as stop:
        check("a note with no source line stops the run naming its ref", "3V7X0ZC1EF#r1" in str(stop), stop)
    else:
        check("a note with no source line stops the run", False)


def derive_lines():
    out = "due: 2 entries\nkinds: session (built in, 9f86d081884c7d65)\nentry\t" + PARENT + "\tcovered\t" + PARENT[::-1] + "\t12\nentry\t" + PARENT + "\tdue\t-\t0\n"
    due, kinds, found = build.dry_run(out)
    check("the dry run's lines", (due, kinds, found[0], found[1][2]) == (2, "kinds: session (built in, 9f86d081884c7d65)", (PARENT, "covered", PARENT[::-1], 12), None), found)
    try:
        build.dry_run("due: 1 entry\nentry\tabc\tcovered\t-\t1\n")
    except SystemExit:
        check("a bad entry line stops the build", True)
    else:
        check("a bad entry line stops the build", False)
    run = build.DERIVE_RUN.match("entries 3, calls 4, notes 31, dropped 2, refused 0, failed 0\ntokens in 91806, tokens out 7340, seconds 48")
    check("derive's two lines", run and tuple(map(int, run.groups())) == (3, 4, 31, 2, 0, 0, 91806, 7340, 48))
    text = build.toml_tables({"derive.enabled": True, "derive.in": ["/c/a"], "llm.anthropic.model": "m", "llm.anthropic.tool.environment": {"A": "1"}})
    check("the derive settings as tables", text == '\n[derive]\nenabled = true\nin = ["/c/a"]\n\n[llm.anthropic]\nmodel = "m"\n\n[llm.anthropic.tool]\nenvironment = { "A" = "1" }\n', text)


ENTRY = "01M7R12WWJKJZK2510AVQDQ54R"
OTHER = "01M7R12WRR41RS1S0HWT5TGS42"
NOTE = "01M7R13JD8Z7MKV932D0E27WD8"
BODY = '{"model":"m","messages":[]}'


def dry(due, calls, states, version="2026-10-21"):
    lines = ["kinds: session (built in, a4ba66a07d51c0b1)", f"named: {len(states)} entries by derive.in, 0 of them left out by derive.not_in",
             f"due: {due} entries"]
    lines += [f"not due: {sum(s[1] == 'passed over' for s in states)} passed over"] if any(s[1] == "passed over" for s in states) else []
    lines += [f"this run: {calls} calls for {calls} entries, {calls * 100} tokens of records to send (derive.calls 20)",
              "provider: anthropic, via api, model claude-test-1, at http://127.0.0.1:62788/v1/messages", f"version: {version}",
              'key: the program ["/tmp/key.sh"], not run']
    lines += [f"entry\t{i}\t{state}\t{note or '-'}\t{n}" for i, state, note, n in states]
    return "\n".join(lines) + "\n"


def run(calls, notes, refused, failed, code=0, *effects):
    out = f"entries {calls}, calls {calls}, notes {notes}, dropped 0, refused {refused}, failed {failed}\ntokens in {calls * 50}, tokens out {notes * 10}, seconds 6\n"
    return (code, out, "bilbo: refused or failed\n" if refused or failed else "", *effects)


DUE = [(ENTRY, "due", None, 0)]
DONE = [(ENTRY, "covered", NOTE, 1)]
PASSED = [(ENTRY, "passed over", NOTE, 0)]
FINAL = (0, dry(0, 0, DONE), "")
DERIVE = {"provider": "anthropic", "ceilings": {"calls": 1000, "tokens": 10000000, "refusals": 100}, "calls_per_run": 20,
          "cache": {"write": True, "timeout": build.DERIVE_DEFAULTS["cache"]["timeout"]}}


def loop(tries, dries, runs, derive=DERIVE, writer=True, timeout="5m", spent=(), repos=("r",), stop=False, steps=1):
    dries, runs, asked = list(dries), list(runs), []
    saved = build.bilbo, build.STORE
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        life = build.Life(tmp / "answers", tmp / "life", writer)
        life.folder.mkdir()
        life.record.write_text("")
        (life.folder / "spent").write_text("".join(f"{line}\n" for line in spent))
        if stop:
            (tmp / "STOP").mkdir()

        def bilbo(name, *args):
            asked.append(args)
            if args[0] == "config":
                return 0, f'derive.tries = {tries} (default)\nllm.timeout = "{timeout}" (default)\n', ""
            queue = dries if "--dry-run" in args else runs
            if not queue:
                raise AssertionError(f"nothing scripted for {args}")
            code, out, err, *effects = queue.pop(0)
            for effect in effects:
                target = life.folder / "spent" if effect.startswith("ask ") else life.record
                with target.open("a") as handle:
                    handle.write(effect + "\n")
            return code, out, err

        build.bilbo, build.STORE = bilbo, tmp
        for store in range(steps):
            build.folders(f"s{store}").mkdir(parents=True)
        try:
            gave = [build.derive_step(f"s{store}", list(repos), {repo: [ENTRY] for repo in repos}, derive, {}, life) for store in range(steps)]
            return gave, None, life, asked
        except SystemExit as stopped:
            return None, str(stopped), life, asked
        except AssertionError as ran_out:
            return None, f"the stand-in ran out: {ran_out}", life, asked
        finally:
            life.config_text = (build.folders("s0") / "config.toml").read_text()
            build.bilbo, build.STORE = saved
            life.spent_text, life.sums = (life.folder / "spent").read_text(), build.spent_by(life.folder)


def expect_error(name, result, *words):
    error = result[1]
    check(name, error is not None and all(word in error for word in words), error)


def derive_loop():
    line = build.this_run(dry(2, 2, DUE))
    check("the `this run:` line read to its four numbers", line == (2, 2, 200, 20), line)
    check("`still growing` is a state of the verb's", "still growing" in build.STATES)
    refused = run(1, 0, 1, 0)
    result = loop(1, [(0, dry(2, 2, DUE), ""), (0, dry(2, 0, DUE), ""), (0, dry(0, 0, PASSED), ""), (0, dry(0, 0, PASSED), "")],
                  [run(2, 0, 2, 0), run(0, 0, 0, 0)])
    check("a run that writes the pass makes no call and the build goes on to the end",
          result[1] is None and result[0][0][0]["runs"] == 2, result[1])
    looped = [a for a in result[3] if "--dry-run" in a][:-1]
    check("the dry run of the loop is asked for every call left", looped and all("100000" in a for a in looped), result[3])
    check("each repository's record counts the entries that hold a note", result[0][0][1]["r"]["noted"] == 0 and result[0][0][1]["r"]["note_entries"] == 1, result[0][0][1])
    check("the run records hold the store, the repository, the exit and the nine numbers (L16)",
          result[2].runs[0] == {"store": "s0", "repo": "r", "exit": 0, "entries": 2, "calls": 2, "notes": 0, "dropped": 0, "refused": 2,
                                "failed": 0, "tokens_in": 100, "tokens_out": 0, "seconds": 6}, result[2].runs)
    check("the config the build writes names the way, api when the settings name none",
          '\n[llm.anthropic]\nvia = "api"\n' in result[2].config_text, result[2].config_text)
    result = loop(1, [FINAL], [], derive=DERIVE | {"settings": {"llm.anthropic.via": "tool"}})
    check("the config the build writes names the way tool when the settings name it",
          '\n[llm.anthropic]\nvia = "tool"\n' in result[2].config_text, result[2].config_text)
    result = loop(1, [FINAL], [], derive=DERIVE | {"provider": "openai"})
    check("the config the build writes names the way api for openai",
          '\n[llm.openai]\nvia = "api"\n' in result[2].config_text, result[2].config_text)
    result = loop(3, [(0, dry(1, 1, DUE), "")] * 3 + [(0, dry(0, 0, DONE), ""), FINAL], [refused, refused, run(1, 1, 0, 0)])
    check("two refusals of one window at derive.tries 3 with the dry run unchanged: the loop goes on", result[1] is None, result[1])
    result = loop(3, [(0, dry(1, 1, DUE), "")] * 4, [refused] * 3)
    expect_error("three still runs at derive.tries 3: the error with the last run's lines", result, "3 runs in a row changed nothing", "refused 1")
    result = loop(3, [(0, dry(1, 1, DUE), "")] * 5 + [(0, dry(0, 0, DONE), ""), FINAL], [run(1, 1, 0, 0)] * 5)
    check("notes written with the dry run unchanged is progress (finding 47)", result[1] is None, result[1])
    result = loop(3, [(0, dry(1, 1, DUE), ""), (0, dry(1, 1, DUE, "2026-10-22"), ""), (0, dry(0, 0, DONE), ""), FINAL],
                  [run(1, 0, 0, 1, 1), run(1, 1, 0, 0)])
    check("exit 1 with failed 1 and the two lines is a run", result[1] is None and result[0][0][0]["failed"] == 1, result[1])
    expect_error("exit 1 with calls 3 and no answer ends the build at once", loop(3, [(0, dry(1, 1, DUE), "")], [run(3, 0, 0, 0, 1)]),
                 "before any call was answered")
    expect_error("exit 2 ends the build", loop(3, [(0, dry(1, 1, DUE), "")], [(2, "", "bilbo: a ref it refuses\n")]), "exited 2")
    expect_error("exit 1 with no line ends the build", loop(3, [(0, dry(1, 1, DUE), "")], [(1, "", "bilbo: key\n")]), "exited 1")
    expect_error("exit 0 with no line ends the build", loop(3, [(0, dry(1, 1, DUE), "")], [(0, "nothing\n", "")]), "exited 0")


def derive_stops():
    due = [(0, dry(1, 1, DUE), "")]
    expect_error("a STOP folder stops the loop", loop(3, due, [], stop=True), "stopped by")
    expect_error("the ceiling of tokens stops the loop", loop(3, due, [], spent=["run 6000000 5000000 0"]), "a ceiling of the build")
    expect_error("the ceiling of refusals stops the loop", loop(3, due, [], spent=["run 1 1 100"]), "a ceiling of the build")
    result = loop(3, due, [run(1, 0, 0, 0, 0, "missed x")], spent=["ask a", "ask b", "ask c"],
                  derive=DERIVE | {"ceilings": DERIVE["ceilings"] | {"calls": 3}})
    expect_error("the ceiling of calls: the run is a miss and the loop ends with the ceiling's text", result, "a ceiling of the build")
    check("with nothing left under the ceiling of calls, the run is asked for calls_per_run", ("derive", "--calls", "20", ENTRY) == result[3][-1], result[3])
    expect_error("a last state of due ends the build", loop(3, [(0, dry(0, 0, DUE), ""), (0, dry(0, 0, DUE), "")], []), "after the derive step")
    expect_error("a dry run that exits 1 ends the build", loop(3, [(1, "", "bilbo: x\n")], []), "exited 1")
    result = loop(3, [(0, dry(1, 1, DUE), ""), (0, dry(0, 0, DONE), ""), (0, dry(1, 1, DUE), ""), (0, dry(0, 0, DONE), ""), FINAL, FINAL],
                  [run(1, 1, 0, 0), run(1, 1, 0, 0)], repos=("a", "b"))
    check("two repositories are looped one after the other", result[1] is None and result[0][0][0]["runs"] == 2 and set(result[0][0][1]) == {"a", "b"}, result[1])
    expect_error("a key in a response stops the build with the text", loop(3, due, [run(1, 0, 0, 0, 0, "asked x key")]), "a response held the key")
    expect_error("a miss with no writer stops the build", loop(3, due, [run(1, 0, 0, 0, 0, "missed x")], writer=False), "no kept answer", "no writer")


def derive_allowance():
    ceiling = DERIVE | {"ceilings": DERIVE["ceilings"] | {"calls": 3}}
    result = loop(3, [(0, dry(2, 2, DUE), ""), (0, dry(0, 0, DONE), ""), FINAL, (0, dry(2, 2, DUE), ""), (0, dry(1, 1, DUE), "")],
                  [run(2, 2, 0, 0, 0, "ask p", "ask q", "asked p kept 0", "asked q kept 0"), run(1, 1, 0, 0, 0, "ask r", "asked r kept 0"),
                   run(1, 0, 0, 0, 0, "missed s")], derive=ceiling, steps=2)
    calls = [a[2] for a in result[3] if a[0] == "derive" and "--dry-run" not in a]
    check("every run of every store is asked for calls_per_run: the allowance is the cache's alone", calls == ["20", "20", "20"], calls)
    expect_error("the second store's run is a miss and the ceiling's text ends the build", result, "a ceiling of the build")
    dries = [(0, dry(2, 2, DUE), ""), (0, dry(0, 0, DONE), ""), FINAL]
    runs = [run(2, 2, 0, 0, 0, "asked p kept 0")]
    with_writer = loop(3, dries, runs, spent=["ask a", "ask b"], derive=ceiling)
    without = loop(3, dries, runs, writer=False)
    commands = lambda result: [a for a in result[3] if a[0] == "derive"]
    check("a life with a writer and one call left runs exactly as a replay runs (N8)",
          with_writer[1] is None and without[1] is None and commands(with_writer) == commands(without), (commands(with_writer), commands(without)))
    check("each of its runs is asked with calls_per_run", all(a[2] == "20" for a in commands(with_writer) if "--dry-run" not in a), commands(with_writer))
    result = loop(3, [(0, dry(1, 1, DUE), ""), (0, dry(0, 0, DONE), ""), FINAL], [run(1, 1, 0, 0, 0, "asked p kept 0")])
    check("a run in which the cache asked writes its `run` line to spent",
          result[1] is None and result[2].spent_text == "run 50 10 0\n", result[1])
    check("spent is summed from its lines", result[2].sums == {"asks": 0, "tokens_in": 50, "tokens_out": 10, "refused": 0}, result[2].sums)
    expect_error("cache.timeout not under llm.timeout stops the build at its start", loop(3, [(0, dry(1, 1, DUE), "")], [], timeout="4m"), "cache.timeout")


class Upstream(BaseHTTPRequestHandler):
    respond = None
    seen = []
    events = None

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length") or 0))
        Upstream.seen.append((self.path, dict(self.headers), body))
        if Upstream.events is not None:
            Upstream.events.append("upstream")
        status, text, headers, delay = Upstream.respond(body.decode())
        time.sleep(delay)
        self.send_response(status)
        for name, value in headers.items():
            self.send_header(name, value)
        self.send_header("content-length", str(len(text)))
        self.end_headers()
        self.wfile.write(text.encode())

    def log_message(self, *args):
        pass


def serving(server):
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}"


def post(url, body=BODY, key=cache.STAND_IN, header="x-api-key"):
    data = body if isinstance(body, bytes) else body.encode()
    connection = http.client.HTTPConnection(url.removeprefix("http://"), timeout=20)
    try:
        connection.request("POST", "/v1/messages", body=data, headers={header: key, "anthropic-version": "2026-10-21",
                                                                         "content-type": "application/json"})
        response = connection.getresponse()
        return response.status, response.read().decode(), {k.lower(): v for k, v in response.getheaders()}
    finally:
        connection.close()


class Rig:
    def __init__(self, tmp, script=(), write=True, ceiling=None, timeout=5, address=None, answers=None, respond=None, key=FAKE_KEY, clock=time.monotonic):
        Upstream.seen, self.script, self.servers = [], list(script), []
        Upstream.respond = respond or (lambda body: (lambda a: (a + ({}, 0)[len(a) - 2:])[:4])(self.script.pop(0)))
        if address is None:
            upstream = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
            address = serving(upstream)
            self.servers.append(upstream)
        self.address, self.folder, self.record = address, Path(answers or Path(tmp) / "answers"), Path(tmp) / f"life-{id(self)}"
        self.record.write_text("")
        self.cache = cache.Cache(self.folder, address, write, self.record, ceiling, timeout, key if write else None, clock)
        server = cache.serve(self.cache)
        self.servers.append(server)
        self.url = serving(server)

    def post(self, body=BODY, key=cache.STAND_IN, header="x-api-key"):
        return post(self.url, body, key, header)

    def record_lines(self):
        return [line.split() for line in self.record.read_text().splitlines()]

    def files(self):
        return sorted(p for p in self.folder.rglob("*") if p.is_file() and p.name != "spent")

    def stop(self):
        for server in self.servers:
            server.shutdown()


def api_cache():
    tmp = Path(tempfile.mkdtemp())
    named = cache.name_of("https://api.anthropic.com", "/v1/messages", [("content-type", "application/json"), ("x-api-key", "k"),
                                                                       ("anthropic-version", "2026-10-21"), ("host", "127.0.0.1:1")], b'{"model":"m"}')
    check("the name of an API request is the one kept answers were found by",
          named == "58c113d4f99425941643b9e51b565c83f4e0d6c24108219089a8c8bfaebd9d46", named)
    rows = [("an answer", 200, ANSWER, True), ("a refusal", 200, REFUSAL, True), ("a cut answer", 200, '{"stop_reason":"max_tokens","content":[]}', True),
            ("an unreadable 200", 200, "not json", True), ("a 200 over llm.response", 200, "x" * 3_000_000, True),
            ("a 400", 400, '{"error":"bad"}', True), ("a 422", 422, '{"error":"bad"}', True),
            ("a 408", 408, "{}", False), ("a 504", 504, "{}", False), ("a 429", 429, "{}", False), ("a 529", 529, "{}", False),
            ("a 503", 503, "{}", False), ("a 409", 409, "{}", False), ("a 500", 500, "{}", False), ("a 401", 401, "{}", False),
            ("a 402", 402, "{}", False), ("a 404", 404, "{}", False), ("a 403", 403, "{}", False), ("a 301", 301, "{}", False),
            ("a failed 200 of rate_limit_exceeded", 200, '{"status":"failed","error":{"code":"rate_limit_exceeded"}}', False),
            ("a failed 200 of server_error", 200, '{"status":"failed","error":{"code":"server_error"}}', False),
            ("a failed 200 of data_residency_mismatch", 200, '{"status":"failed","error":{"code":"data_residency_mismatch"}}', False),
            ("a failed 200 of another code", 200, '{"status":"failed","error":{"code":"other"}}', True)]
    for label, status, text, kept in rows:
        folder = tmp / label.replace(" ", "-")
        rig = Rig(folder.parent, [(status, text, {}, 0)], answers=folder)
        got = rig.post()
        check(f"{label}: the caller gets what came back", (got[0], got[1]) == (status, text), got[:1])
        check(f"{label}: {'kept' if kept else 'not kept'}", bool(rig.files()) == kept, rig.files())
        replay = Rig(folder.parent, write=False, address=rig.address, answers=folder)
        again = replay.post()
        check(f"{label}: a replay gives {'the kept answer' if kept else 'a miss'}", (again[0], again[1]) == ((status, text) if kept else (404, cache.MISS.decode())), again[:1])
        rig.stop(); replay.stop()
    folder = tmp / "three-400"
    rig = Rig(tmp, [(400, f'{{"n":{n}}}', {}, 0) for n in range(3)], answers=folder)
    sent = [rig.post()[1] for _ in range(3)]
    replay = Rig(tmp, write=False, address=rig.address, answers=folder)
    check("a rejected 400 asked three times is kept three times and replayed in order",
          sent == [replay.post()[1] for _ in range(3)] == ['{"n":0}', '{"n":1}', '{"n":2}'], sent)
    rig.stop(); replay.stop()

    rig = Rig(tmp, [(429, "{}", {}, 0), (200, ANSWER, {}, 0)], answers=tmp / "five")
    first, second = rig.post(), rig.post()
    replay = Rig(tmp, write=False, address=rig.address, answers=tmp / "five")
    check("a 429, then a 200 (L4): one file, and a replay gives it to the first ask", (first[0], second[0], [p.name for p in rig.files()], replay.post()[:2]) == (429, 200, ["0"], (200, ANSWER)))
    rig.stop(); replay.stop()

    rig = Rig(tmp, [(429, "{}", {"retry-after": "7", "x-request-id": "req-1"}, 0)], answers=tmp / "retry")
    got = rig.post()
    check("a 429 with retry-after 7 gives the caller the header and the request id (L11)", (got[2].get("retry-after"), got[2].get("x-request-id")) == ("7", "req-1"), got[2])
    rig.stop()
    rig = Rig(tmp, [(400, "{}", {"retry-after": "7", "x-request-id": "req-2"}, 0)], answers=tmp / "retry-kept")
    rig.post()
    check("no file holds a header", all(b"retry-after" not in p.read_bytes().lower() and b"req-2" not in p.read_bytes() for p in rig.files()), rig.files())
    rig.stop()


def api_cache_waits():
    tmp = Path(tempfile.mkdtemp())

    def respond(body):
        return (200, ANSWER, {}, 4) if body.startswith("slow") else (200, ANSWER, {}, 0)

    rig = Rig(tmp, respond=respond, timeout=2, answers=tmp / "timeout")
    started = time.monotonic()
    got = rig.post("slow")
    check("no answer in cache.timeout: the caller gets 408 within the time", got[0] == 408 and time.monotonic() - started < 3.5, got[:1])
    check("nothing is kept of it, and the record says none", not rig.files() and any(line[2] == "none" for line in rig.record_lines() if line[0] == "asked"), rig.record_lines())
    holder = threading.Thread(target=lambda: rig.post("slow2"))
    holder.start()
    time.sleep(0.2)
    started = time.monotonic()
    other = rig.post("fast")
    check("a second request is not held up by the first", other[0] == 200 and time.monotonic() - started < 0.7, other[:1])
    holder.join()
    rig.stop()
    rig = Rig(tmp, [(200, ANSWER, {}, 0)] * 2, answers=tmp / "timeout-twice")
    for server in rig.servers[:1]:
        server.shutdown()
        server.server_close()
    got = rig.post()
    check("no connection: the caller gets 502 and nothing is kept", got[0] == 502 and not rig.files(), got[:1])
    rig.stop()


class Trickle(BaseHTTPRequestHandler):
    closed = []

    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length") or 0))
        self.send_response(200)
        self.send_header("content-length", "100")
        self.end_headers()
        try:
            for _ in range(100):
                self.wfile.write(b"x")
                self.wfile.flush()
                time.sleep(0.2)
        except OSError:
            Trickle.closed.append(True)

    def log_message(self, *args):
        pass


class Moved(BaseHTTPRequestHandler):
    where = ""
    seen = []

    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length") or 0))
        if Moved.where:
            self.send_response(302)
            self.send_header("location", Moved.where + "/v1/messages")
            self.send_header("content-length", "0")
        else:
            Moved.seen.append(self.headers.get("x-api-key"))
            self.send_response(200)
            self.send_header("content-length", "2")
        self.end_headers()
        if not Moved.where:
            self.wfile.write(b"{}")

    def log_message(self, *args):
        pass


def api_deadline_and_redirect():
    tmp = Path(tempfile.mkdtemp())
    upstream = ThreadingHTTPServer(("127.0.0.1", 0), Trickle)
    address = serving(upstream)
    rig = Rig(tmp, address=address, timeout=1, answers=tmp / "answers-trickle")
    started = time.monotonic()
    try:
        got = rig.post()
    except OSError as late:
        got = (None, str(late))
    took = time.monotonic() - started
    check("a body that trickles: the caller has its 408 within two seconds of a timeout of one (N7)", got[0] == 408 and took < 2, (got[:1], took))
    check("nothing is kept of it, and the record says none", not rig.files() and any(line[2] == "none" for line in rig.record_lines() if line[0] == "asked"), rig.record_lines())
    rig.stop(); upstream.shutdown()
    second = ThreadingHTTPServer(("127.0.0.1", 0), Moved)
    other = serving(second)
    first = ThreadingHTTPServer(("127.0.0.1", 0), Moved)
    here = serving(first)
    Moved.where, Moved.seen = other, []
    rig = Rig(tmp, address=here, answers=tmp / "answers-moved", respond=lambda body: (200, "{}", {}, 0))
    got = rig.post()
    check("a 302 to another host is given back as it came and followed by nobody (N1)", got[0] == 302 and Moved.seen == [] and not rig.files(), (got[:1], Moved.seen))
    for server in (rig, ):
        server.stop()
    first.shutdown(); second.shutdown()


def endpoint_rule():
    def refused(endpoint):
        saved = os.environ.get("BILBO_EVAL_DERIVE")
        os.environ["BILBO_EVAL_DERIVE"] = json.dumps({"provider": "anthropic", "settings": {"llm.anthropic.api.endpoint": endpoint}})
        try:
            build.derive_env()
        except SystemExit as stopped:
            return str(stopped)
        finally:
            if saved is None:
                del os.environ["BILBO_EVAL_DERIVE"]
            else:
                os.environ["BILBO_EVAL_DERIVE"] = saved
        return None

    for endpoint in ("https://example.com:not-a-port/v1", "https:///v1", "http://localhost/v1", "http://example.com/v1", "http://localhost:1/v1", "ftp://127.0.0.1/v1"):
        error = refused(endpoint)
        check(f"the endpoint {endpoint} is refused with the rule's text and no value (N1, P6)",
              error and "must be an https URL, or an http URL at a loopback address" in error and endpoint not in error, error)
    for endpoint in ("https://example.com/v1", "http://127.9.9.9/v1", "http://127.0.0.1:1/v1", "http://[::1]:1/v1", "https://api.example.com/v1"):
        check(f"the endpoint {endpoint} is taken", refused(endpoint) is None, refused(endpoint))


def two_builds():
    tmp = Path(tempfile.mkdtemp())
    folder = tmp / "answers" / "anthropic-api"
    code = ("import sys; sys.dont_write_bytecode = True; sys.path.insert(0, sys.argv[1]); import build\n"
            "held = build.hold(sys.argv[2], 'anthropic-api'); print('held', flush=True); sys.stdin.readline()")
    first = subprocess.Popen([sys.executable, "-c", code, str(HERE), str(folder)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    check("the first build takes the lock", first.stdout.readline().strip() == "held")
    try:
        build.hold(folder, "anthropic-api")
    except SystemExit as stopped:
        text = str(stopped)
    else:
        text = None
    check("a second build, while the first holds it, stops with the lock's text (N4)",
          text == f"bilbo-next: another build is using the answers of anthropic-api: {folder / 'lock'} is held; it must end first", text)
    first.stdin.close()
    first.wait()
    third = build.hold(folder, "anthropic-api")
    check("after the first ends, a third takes the lock", third is not None)
    third.close()


def main_order():
    tmp = Path(tempfile.mkdtemp())
    names = ("CORPUS", "WORK", "STORE", "ANSWERS", "LIFE", "BILBO", "one_life", "hold", "drop_failures", "archive", "start_cache")
    saved = {name: getattr(build, name) for name in names}
    saved_env = {name: os.environ.get(name) for name in ("BILBO_EVAL_SCOPE", "BILBO_EVAL_DERIVE")}
    events = []
    build.CORPUS, build.WORK, build.STORE, build.ANSWERS = tmp / "corpus", tmp / "work", tmp / "work" / "store", tmp / "answers"
    build.LIFE, build.BILBO = build.WORK / "life", sys.executable
    build.CORPUS.mkdir(parents=True)
    build.STORE.mkdir(parents=True)
    (build.WORK / "key.txt").write_text("key\nsessions all\n")
    os.environ["BILBO_EVAL_SCOPE"] = "all"
    os.environ["BILBO_EVAL_DERIVE"] = json.dumps({"provider": "anthropic", "cache": {"write": True}})
    real_hold, real_drop, real_archive = saved["hold"], saved["drop_failures"], saved["archive"]

    class Held:
        def close(self):
            events.append("release")

    build.hold = lambda folder, provider: (events.append("lock"), Held())[1]
    build.drop_failures = lambda folder: events.append("drop the kept failures")
    build.archive = lambda derive, life: events.append("archive")
    build.one_life = lambda groups, by_repo, derive, life: (events.append("life"), life.record.write_text(""), ([], {}, {}))[2]
    try:
        build.main()
    finally:
        for name, value in saved.items():
            setattr(build, name, value)
        for name, value in saved_env.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value
    check("the lock is taken before the kept failures are removed and held until the archive is made",
          events == ["lock", "drop the kept failures", "life", "archive", "release"], events)


def bilbo_starts():
    tmp = Path(tempfile.mkdtemp())
    names = ("CORPUS", "WORK", "STORE", "ANSWERS", "LIFE", "BILBO", "one_life", "hold", "drop_failures", "archive", "DERIVING")
    saved = {name: getattr(build, name) for name in names}
    saved_env = {name: os.environ.get(name) for name in ("BILBO_EVAL_SCOPE", "BILBO_EVAL_DERIVE", "BILBO_EVAL_DERIVE_KEY")}
    runs, popens = [], []

    class Done:
        returncode, stdout, stderr = 0, "bilbo 1\n", ""

    class Started:
        stdout = type("Out", (), {"readline": staticmethod(lambda: "port 1\n")})()

    build.CORPUS, build.WORK, build.STORE, build.ANSWERS = tmp / "corpus", tmp / "work", tmp / "work" / "store", tmp / "answers"
    build.LIFE, build.BILBO, build.DERIVING = build.WORK / "life", sys.executable, True
    build.CORPUS.mkdir(parents=True)
    build.STORE.mkdir(parents=True)
    (build.WORK / "key.txt").write_text("key\nsessions all\n")
    os.environ["BILBO_EVAL_SCOPE"] = "all"
    os.environ["BILBO_EVAL_DERIVE_KEY"] = FAKE_KEY
    os.environ["BILBO_EVAL_DERIVE"] = json.dumps({"provider": "anthropic", "cache": {"write": True},
                                                  "settings": {"llm.anthropic.api.endpoint": "http://127.0.0.1:1/v1/messages"}})

    def one_life(groups, by_repo, derive, life):
        life.record.write_text("")
        settings, _, _ = build.start_cache(derive)
        build.give_key(derive, settings)
        build.bilbo("s0", "config")
        return [], {}, {}

    build.hold = lambda folder, provider: type("Held", (), {"close": staticmethod(lambda: None)})()
    build.drop_failures = lambda folder: None
    build.archive = lambda derive, life: None
    build.one_life = one_life
    saved_run, saved_popen = build.subprocess.run, build.subprocess.Popen
    build.subprocess.run = lambda command, **options: (runs.append((command, options.get("env"))), Done())[1]
    build.subprocess.Popen = lambda command, **options: (popens.append((command, options.get("env"))), Started())[1]
    try:
        build.main()
    finally:
        build.subprocess.run, build.subprocess.Popen = saved_run, saved_popen
        build.KEYS.clear()
        for name, value in saved.items():
            setattr(build, name, value)
        for name, value in saved_env.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value
    version = [env for command, env in runs if "--version" in command]
    check("main starts bilbo for --version and for its store, each through the wrapper with an environment of its own",
          len(runs) == 2 and all(env is not None and "BILBO_CONFIG" in env or env == {"PATH": "/usr/bin:/bin", "NO_COLOR": "1"} for _, env in runs), runs)
    check("the real key is in the environment of no start of bilbo, `--version` included, and the stand-in is in the store's only",
          all(FAKE_KEY not in json.dumps(env) for _, env in runs) and version[0] == {"PATH": "/usr/bin:/bin", "NO_COLOR": "1"}
          and [env for command, env in runs if "config" in command][0].get("ANTHROPIC_API_KEY") == cache.STAND_IN, runs)
    check("the cache's start alone has the real key", len(popens) == 1 and popens[0][1].get("BILBO_EVAL_CACHE_KEY") == FAKE_KEY, popens)


def tags_refusals():
    names = ("CORPUS", "WORK", "STORE", "ANSWERS", "LIFE", "BILBO", "LAUNCHER", "one_life", "hold", "drop_failures", "archive", "DERIVING")
    env_names = ("BILBO_EVAL_SCOPE", "BILBO_EVAL_DERIVE", "BILBO_EVAL_DERIVE_KEY")

    def attempt(tmp, settings):
        saved = {name: getattr(build, name) for name in names}
        saved_env = {name: os.environ.get(name) for name in env_names}
        runs, popens, events = [], [], []

        class Done:
            returncode, stdout, stderr = 0, "claude 1\n", ""

        class Started:
            stdout = type("Out", (), {"readline": staticmethod(lambda: "port 1\n")})()

        build.CORPUS, build.WORK, build.STORE, build.ANSWERS = tmp / "corpus", tmp / "work", tmp / "work" / "store", tmp / "answers"
        build.LIFE, build.BILBO, build.DERIVING, build.LAUNCHER = build.WORK / "life", sys.executable, True, tmp / "claude"
        build.CORPUS.mkdir(parents=True)
        build.STORE.mkdir(parents=True)
        (build.WORK / "key.txt").write_text("key\nsessions all\n")
        os.environ["BILBO_EVAL_SCOPE"] = "all"
        os.environ["BILBO_EVAL_DERIVE_KEY"] = FAKE_KEY
        os.environ["BILBO_EVAL_DERIVE"] = json.dumps({"provider": "anthropic", "cache": {"write": True},
                                                      "settings": {"llm.anthropic.via": "tool", "llm.anthropic.tool.program": "/bin/sh", **settings}})

        def one_life(groups, by_repo, derive, life):
            life.record.write_text("")
            build.start_cache(derive)
            return [], {}, {}

        build.hold = lambda folder, provider: type("Held", (), {"close": staticmethod(lambda: None)})()
        build.drop_failures = lambda folder: None
        build.archive = lambda derive, life: None
        build.one_life = one_life
        saved_run, saved_popen = build.subprocess.run, build.subprocess.Popen
        build.subprocess.run = lambda command, **options: (runs.append(command), events.append("run"), Done())[2]
        build.subprocess.Popen = lambda command, **options: (popens.append(command), events.append("popen"), Started())[2]
        stopped = None
        try:
            build.main()
        except SystemExit as error:
            stopped = str(error)
        finally:
            build.subprocess.run, build.subprocess.Popen = saved_run, saved_popen
            build.KEYS.clear()
            for name, value in saved.items():
                setattr(build, name, value)
            for name, value in saved_env.items():
                if value is None:
                    os.environ.pop(name, None)
                else:
                    os.environ[name] = value
        return stopped, runs, popens, events

    def drive(settings):
        with tempfile.TemporaryDirectory() as raw:
            return attempt(Path(raw), settings)

    stopped, runs, popens, events = drive({})
    probe = [at for at, command in enumerate(runs) if command[:2] == ["/bin/sh", "--version"]]
    check("with no tags' table the build asks the tool, not bilbo, for --version before it starts the cache, and starts it",
          stopped is None and len(probe) == 1 and len(popens) == 1 and events.index("run") <= events.index("popen"),
          (stopped, runs, popens, events))
    for spelling in ({"derive.tags.enabled": True}, {"derive.tags": {"enabled": True}}, {'derive."tags".enabled': True}, {"derive.tags": {}}):
        stopped, runs, popens, events = drive(spelling)
        check(f"a settings table that makes a tags table under derive stops the build before the cache is started: {list(spelling)[0]}",
              stopped == "bilbo-next: BILBO_EVAL_DERIVE: the arm does not tag" and not runs and not popens, (stopped, runs, popens))
    for spelling in ({"derive.summary.enabled": True}, {"derive.summary": {"enabled": True}}, {'derive."summary".enabled': True},
                     {"derive.summary": {}}, {"derive.summary.most": 100}):
        stopped, runs, popens, events = drive(spelling)
        check(f"a settings table that makes a summary table under derive stops the build before the cache is started: {list(spelling)[0]}",
              stopped == "bilbo-next: BILBO_EVAL_DERIVE: the arm does not summarise" and not runs and not popens, (stopped, runs, popens))


def api_secret():
    tmp = Path(tempfile.mkdtemp())
    found = lambda where, key=FAKE_KEY: [p for p in where.rglob("*") if p.is_file() and key.encode() in p.read_bytes()]
    rig = Rig(tmp, [(200, f'{{"echo":"{FAKE_KEY}"}}', {}, 0)], answers=tmp / "answers-key")
    got = rig.post()
    check("a body that holds the real key is neither kept nor given back: the caller gets the cache's 502 (P4)",
          got[0] == 502 and FAKE_KEY not in got[1] and not found(tmp), (got[:1], found(tmp)))
    check("the record says key", any(line[2] == "key" for line in rig.record_lines() if line[0] == "asked"), rig.record_lines())
    rig.stop()
    rig = Rig(tmp, [(200, ANSWER, {}, 0)], answers=tmp / "answers-clean")
    rig.post()
    check("the key the case sent is in no file of the cache (G20)", rig.files() and not found(rig.folder), found(rig.folder))
    saved = cache.Cache.keep
    cache.Cache.keep = lambda self, name, number, status, data, new: saved(self, name, number, status, data + FAKE_KEY.encode(), new)
    try:
        leaky = Rig(tmp, [(200, ANSWER, {}, 0)], answers=tmp / "answers-leaky")
        leaky.post()
    finally:
        cache.Cache.keep = saved
    check("with a cache made to write the request's key, the look for it finds it", found(leaky.folder), leaky.files())
    rig.stop(); leaky.stop()
    for status in (200, 401):
        rig = Rig(tmp, [(status, f'{{"echo":"{FAKE_KEY}"}}', {}, 0)], answers=tmp / f"answers-key-{status}")
        got = rig.post()
        check(f"a {status} whose body holds the key as plain bytes: the cache's 502 and own body, nothing written, the record says key (Q3)",
              got[0] == 502 and FAKE_KEY not in got[1] and not found(rig.folder) and any(line[2] == "key" for line in rig.record_lines() if line[0] == "asked"),
              (got[:1], rig.record_lines()))
        rig.stop()
    for header in ("request-id", "x-request-id", "retry-after"):
        rig = Rig(tmp, [(401, "{}", {header: FAKE_KEY}, 0)], answers=tmp / f"answers-header-{header}")
        got = rig.post()
        check(f"the key in {header} on a 401 with a clean body: the same end, and the caller sees no header (Q1)",
              got[0] == 502 and FAKE_KEY not in json.dumps(got[2]) and not found(rig.folder) and any(line[2] == "key" for line in rig.record_lines() if line[0] == "asked"),
              (got[:1], got[2], rig.record_lines()))
        rig.stop()
    check("holds is one test of plain bytes", cache.holds(FAKE_KEY, FAKE_KEY.encode() + b" tail") and not cache.holds(FAKE_KEY, b"nothing")
          and not cache.holds(FAKE_KEY, b'{"a":"\\u0073k-fake-key-0123456789"}') and not cache.holds("", b"anything"))


def api_key_is_the_caches():
    tmp = Path(tempfile.mkdtemp())
    rig = Rig(tmp, [(200, ANSWER, {}, 0), (200, REFUSAL, {}, 0)], answers=tmp / "answers-keyed")
    first = rig.post(key=cache.STAND_IN)
    second = rig.post(body='{"m":2}', key=f"Bearer {cache.STAND_IN}", header="authorization")
    seen = [{k.lower(): v for k, v in headers.items()} for _, headers, _ in Upstream.seen]
    check("the upstream sees the real key in x-api-key, and after Bearer, and the stand-in nowhere (P4)",
          (first[0], second[0]) == (200, 200) and seen[0].get("x-api-key") == FAKE_KEY and seen[1].get("authorization") == f"Bearer {FAKE_KEY}"
          and cache.STAND_IN not in json.dumps(seen), seen)
    rig.stop()
    rig.stop()
    rig = Rig(tmp, [(200, ANSWER, {}, 0)], answers=tmp / "answers-nokey")
    got = rig.post(key="some-other-key")
    check("a request that carries the stand-in in neither place is not sent: the cache's 502", got[0] == 502 and not Upstream.seen and not rig.files(), got[:1])
    rig.stop()
    saved_env = {name: os.environ.get(name) for name in ("BILBO_EVAL_DERIVE_KEY", "BILBO_EVAL_CACHE_KEY")}
    launched = []

    class Stopped(Exception):
        pass

    saved_popen = build.subprocess.Popen
    build.subprocess.Popen = lambda command, **options: (launched.append((command, options.get("env"))), (_ for _ in ()).throw(Stopped()))[1]
    derive = DERIVE | {"settings": {"llm.anthropic.api.endpoint": "http://127.0.0.1:1/v1/messages"}}
    try:
        os.environ["BILBO_EVAL_DERIVE_KEY"] = FAKE_KEY
        for write in (True, False):
            try:
                build.start_cache(derive | {"cache": derive["cache"] | {"write": write}})
            except Stopped:
                pass
        env_writer, env_replay = launched[0][1], launched[1][1]
        check("the real key is in the cache's environment alone, with a writer; in no environment of a replay (P4)",
              env_writer.get("BILBO_EVAL_CACHE_KEY") == FAKE_KEY and "BILBO_EVAL_DERIVE_KEY" not in env_writer
              and "BILBO_EVAL_CACHE_KEY" not in env_replay and "BILBO_EVAL_DERIVE_KEY" not in env_replay, (env_writer.get("BILBO_EVAL_CACHE_KEY"), env_replay))
        check("the cache of an API is started in the folder of its provider and its way",
              launched[0][0][launched[0][0].index("--dir") + 1] == str(build.ANSWERS / "anthropic-api"), launched[0][0])
        del os.environ["BILBO_EVAL_DERIVE_KEY"]
        try:
            build.start_cache(derive)
        except SystemExit as stopped:
            text = str(stopped)
        except Stopped:
            text = None
        check("a build with a writer and no real key stops at its start", text and "BILBO_EVAL_DERIVE_KEY" in text, text)
    finally:
        build.subprocess.Popen = saved_popen
        for name, value in saved_env.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value
    build.KEYS.clear()
    os.environ["BILBO_EVAL_DERIVE_KEY"] = FAKE_KEY
    build.give_key(derive, {})
    stand = dict(build.KEYS)
    seen_env = []
    saved_run = build.subprocess.run
    build.DERIVING = True
    build.subprocess.run = lambda command, **options: (seen_env.append(options["env"]), type("Done", (), {"returncode": 0, "stdout": "", "stderr": ""})())[1]
    try:
        build.bilbo("all", "config")
    finally:
        build.subprocess.run = saved_run
        build.DERIVING = False
        build.KEYS.clear()
        del os.environ["BILBO_EVAL_DERIVE_KEY"]
    check("the environment bilbo runs in holds the stand-in, the same with a writer and with none, and never the real key (P4)",
          stand == {"ANTHROPIC_API_KEY": cache.STAND_IN} and seen_env[0].get("ANTHROPIC_API_KEY") == cache.STAND_IN and FAKE_KEY not in json.dumps(seen_env[0]), (stand, seen_env))
    folder = tmp / "answers-stand"
    request = BODY
    probe = Rig(tmp, [(200, ANSWER, {}, 0)], answers=folder)
    probe.post(body=request)
    name = [p.name for p in folder.iterdir() if p.is_dir()][0]
    probe.stop()
    (folder / name / "0").write_bytes(("200\n" + '{"echo":"%s"}' % cache.STAND_IN).encode())
    kept = []
    for writer in (True, False):
        again = Rig(tmp, [], write=writer, address=probe.address, answers=folder)
        kept.append(again.post(body=request)[:2])
        again.stop()
    check("a kept answer that holds the stand-in's text is given back as the same bytes with a writer and with none (P4)",
          kept[0] == kept[1] == (200, '{"echo":"%s"}' % cache.STAND_IN), kept)


class ToolRig:
    def __init__(self, tmp, plan=(), write=True, ceiling=None, timeout=5, answers=None, key=FAKE_TOKEN, upstream="claude 9.9.9 (Fake)"):
        mine = Path(tempfile.mkdtemp(dir=tmp))
        self.seen, self.program, self.folder, self.record = mine / "seen", mine / "claude", Path(answers or mine / "answers"), mine / "life"
        (mine / "plan").write_text(json.dumps(list(plan)))
        self.seen.write_text("")
        self.record.write_text("")
        self.program.write_text(FAKE_TOOL.format(python=sys.executable, seen=str(self.seen), plan=str(mine / "plan")))
        self.program.chmod(0o755)
        self.cache = cache.Cache(self.folder, upstream, write, self.record, ceiling, timeout, key if write else None, tool=str(self.program))
        self.server = cache.serve(self.cache)
        self.url = serving(self.server) + "/"

    def call(self, *arguments, env=None, stdin="the material", folder=None, command=None):
        return subprocess.run(command or [sys.executable, str(HERE / "tool.py"), self.url, *(arguments or ("-p",))], input=stdin.encode(),
                              env={"HOME": "/nowhere", "PATH": "/usr/bin:/bin"} if env is None else env, cwd=folder, capture_output=True, timeout=30)

    def started(self):
        return [json.loads(line) for line in self.seen.read_text().splitlines()]

    def record_lines(self):
        return [line.split() for line in self.record.read_text().splitlines()]

    def files(self):
        return sorted(p for p in self.folder.rglob("*") if p.is_file() and p.name != "spent")

    def stop(self):
        self.server.shutdown()


def ended(done):
    return done.returncode, done.stdout.decode("latin-1")


def running(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


def tool_cache():
    tmp = Path(tempfile.mkdtemp())
    api = '{"is_error":true,"stop_reason":"stop_sequence","api_error_status":%d}'
    rows = [("an answer", 0, TOLD, 200), ("a refusal, which the tool reports as an error", 1, '{"is_error":true,"stop_reason":"refusal"}', 200),
            ("a cut answer", 0, '{"is_error":false,"stop_reason":"max_tokens"}', 200),
            ("an unknown stop reason", 0, '{"is_error":false,"stop_reason":"pause_turn"}', 200),
            ("a request the API rejects", 1, api % 400, 400), ("a request the API finds too large", 1, api % 413, 413),
            ("a login the API refuses", 1, api % 401, None), ("a limit", 1, api % 429, None), ("an overloaded API", 1, api % 529, None),
            ("an error with no status", 1, '{"is_error":true,"subtype":"error_during_execution","result":"a limit"}', None),
            ("an error with a status that is none", 1, api.replace("%d", "200") , None),
            ("an error that exits 0", 0, '{"is_error":true,"stop_reason":"end_turn"}', None),
            ("an answer that exits 1", 1, TOLD, None), ("a reply with no stop reason", 0, '{"is_error":false}', None),
            ("a reply that is no JSON", 0, "not json", None), ("a reply that is a list", 0, "[]", None), ("no reply", 2, "", None),
            ("an answer that is not UTF-8", 0, TOLD.replace("a note", "caf\xe9"), None)]
    for label, code, out, kept in rows:
        rig = ToolRig(tmp, [(code, out, 0)])
        got = ended(rig.call())
        check(f"the tool, {label}: the caller ends as the tool ended, byte for byte", got == (code, out), got)
        first = [p.read_bytes().split(b"\n", 1)[0] for p in rig.files()]
        check(f"the tool, {label}: {'kept as a %d' % kept if kept else 'not kept'}", first == ([str(kept).encode()] if kept else [])
              and cache.told(code, out.encode("latin-1")) is (kept == 200), first)
        replay = ToolRig(tmp, write=False, answers=rig.folder)
        again = ended(replay.call())
        check(f"the tool, {label}: a replay {'ends the same' if kept else 'misses, and the caller ends with no reply'} and starts no tool",
              again == ((code, out) if kept else (tool.NO_ANSWER, "")) and not replay.started(), again)
        if kept == 400:
            build.drop_failures(rig.folder)
            check("a build with a writer removes the tool's kept rejection, as it removes an API's", not rig.files(), rig.files())
        rig.stop(); replay.stop()
    told_at = {code: sorted(p.name for p in CLAUDE_FIXTURES.glob("*.json") if cache.told(code, p.read_bytes())) for code in (0, 1)}
    every = sorted(p.name for p in CLAUDE_FIXTURES.glob("*.json"))
    check("bilbo's own fixtures of the tool's replies: at exit 0 all are kept but the error and the one with no stop reason; at exit 1 the refusals alone",
          every and told_at[0] == [n for n in every if n not in ("error.json", "no-stop-reason.json")]
          and told_at[1] == [n for n in every if n.startswith("refusal")], told_at)

    rig = ToolRig(tmp, [(0, TOLD, 0)])
    folder = Path(tempfile.mkdtemp(dir=tmp)).resolve()
    os.environ["BILBO_EVAL_CACHE_KEY"] = "the cache's own"
    try:
        rig.call("-p", "--model", "m", env={"HOME": "/nowhere", "PATH": "/usr/bin:/bin", "A_B": "1"}, folder=folder)
    finally:
        del os.environ["BILBO_EVAL_CACHE_KEY"]
    got = rig.started()[0]
    home = got["env"]["HOME"]
    check("the tool is started with the caller's arguments, standard input and folder",
          (got["argv"], got["stdin"], got["folder"]) == (["-p", "--model", "m"], "the material", str(folder)), got)
    check("the tool gets the caller's variables, the token, the quiet ones and a home of the cache's own",
          got["env"]["A_B"] == "1" and got["env"]["PATH"] == "/usr/bin:/bin" and got["env"][cache.TOKEN_VARIABLE] == FAKE_TOKEN
          and cache.QUIET.items() <= got["env"].items() and home != "/nowhere" and Path(home).is_dir(), got["env"])
    check("the tool gets no other variable, the cache's own environment least of all (Python and macOS add two to the fake's)",
          set(got["env"]) - {"LC_CTYPE", "__CF_USER_TEXT_ENCODING"} == {"A_B", "PATH", "HOME", cache.TOKEN_VARIABLE, *cache.QUIET}, sorted(got["env"]))
    check("the token is in no file of the answers and in no line of the record",
          all(FAKE_TOKEN.encode() not in p.read_bytes() for p in rig.files()) and FAKE_TOKEN not in rig.record.read_text() and rig.files())
    name = rig.record_lines()[0][1]
    check("the record of an ask that is kept is the API cache's", rig.record_lines() == [["asked", name, "kept", "0"], ["given", name, "0"]], rig.record_lines())
    kept = (rig.folder / name / "0").read_bytes()
    check("a kept answer is a 200 and the tool's exit code and output", kept == b"200\n" + json.dumps({"exit": 0, "stdout": TOLD}).encode(), kept)

    def replayed(upstream="claude 9.9.9 (Fake)", **call):
        replay = ToolRig(tmp, write=False, answers=rig.folder, upstream=upstream)
        try:
            return ended(replay.call("-p", "--model", "m", **{"env": {"HOME": "/nowhere", "PATH": "/usr/bin:/bin", "A_B": "1"}, "folder": folder} | call))
        finally:
            replay.stop()

    other = Path(tempfile.mkdtemp(dir=tmp))
    check("a request's name holds no HOME, USER, PATH or TMPDIR and no folder: another machine finds the answer",
          replayed(env={"HOME": "/elsewhere", "USER": "u", "PATH": "/bin", "TMPDIR": "/t", "A_B": "1"}, folder=other) == (0, TOLD))
    for label, call in (("another variable", {"env": {"HOME": "/nowhere", "PATH": "/usr/bin:/bin", "A_B": "2"}}),
                        ("another standard input", {"stdin": "other material"}), ("another version of the tool", {"upstream": "claude 9.9.10 (Fake)"})):
        check(f"a request's name holds its variables, its standard input and the tool's version: {label} misses", replayed(**call) == (tool.NO_ANSWER, ""))
    replay = ToolRig(tmp, write=False, answers=rig.folder)
    check("a request's name holds its arguments: another model misses",
          ended(replay.call("-p", "--model", "n", env={"HOME": "/nowhere", "PATH": "/usr/bin:/bin", "A_B": "1"}, folder=folder)) == (tool.NO_ANSWER, ""))
    check("a miss is recorded as the API cache's", [line[0] for line in replay.record_lines()] == ["missed"], replay.record_lines())
    rig.stop(); replay.stop()

    rig = ToolRig(tmp, [(0, TOLD.replace("a note", FAKE_TOKEN), 0)])
    got = ended(rig.call())
    check("a reply that holds the token is given to no one and kept nowhere, and the record says so",
          got == (tool.NO_ANSWER, "") and not rig.files() and [line[0::2] for line in rig.record_lines()] == [["asked", "key"]], (got, rig.record_lines()))
    rig.stop()
    check("the build's log gets nothing of a failure that holds the token, and the tool's own words of another",
          FAKE_TOKEN not in cache.said(f'{{"result":"{FAKE_TOKEN}"}}'.encode(), FAKE_TOKEN)
          and FAKE_TOKEN not in cache.said(f'{{"result":"{FAKE_TOKEN}"}}'.replace("-", "\\u002d").encode(), FAKE_TOKEN)
          and cache.said(b'{"subtype":"error_during_execution","result":"a limit"}', FAKE_TOKEN) == "error_during_execution: a limit"
          and FAKE_TOKEN not in cache.said(b"[]", FAKE_TOKEN) and cache.said(b'{"result":"\\udc80"}', FAKE_TOKEN) == "?")

    rig = ToolRig(tmp, [(0, TOLD, 0)], key=None)
    got = ended(rig.call())
    check("a writer with no token starts no tool", got == (tool.NO_ANSWER, "") and not rig.started() and [line[0] for line in rig.record_lines()] == ["unkeyed"], rig.record_lines())
    rig.stop()
    for label, call in (("its standard input", {"stdin": f"material {FAKE_TOKEN}"}), ("a variable", {"env": {"HOME": "/nowhere", "PATH": "/usr/bin:/bin", "A_B": FAKE_TOKEN}}),
                        ("its home", {"env": {"HOME": f"/{FAKE_TOKEN}", "PATH": "/usr/bin:/bin"}})):
        rig = ToolRig(tmp, [(0, TOLD, 0)])
        got = ended(rig.call(**call))
        check(f"a request that holds the token in {label} starts no tool and spends no ask",
              got == (tool.NO_ANSWER, "") and not rig.started() and [line[0] for line in rig.record_lines()] == ["unkeyed"] and not (rig.folder / "spent").exists(), rig.record_lines())
        rig.stop()

    rig = ToolRig(tmp, [(0, TOLD, 0), (0, TOLD, 0)], ceiling=1)
    got = [ended(rig.call()), ended(rig.call(stdin="other material"))]
    check("the ceiling of calls holds for the tool: the second ask starts nothing", got == [(0, TOLD), (tool.NO_ANSWER, "")] and len(rig.started()) == 1
          and (rig.folder / "spent").read_text().count("ask ") == 1, got)
    rig.stop()

    rig = ToolRig(tmp, [(0, TOLD, 4)], timeout=1)
    began = time.monotonic()
    got = ended(rig.call())
    check("a tool that is not done by the deadline: the caller ends with no reply then, and nothing is kept",
          got == (tool.NO_ANSWER, "") and time.monotonic() - began < 3 and not rig.files() and [line[0::2] for line in rig.record_lines()] == [["asked", "none"]], (got, rig.record_lines()))
    pid = rig.started()[0]["pid"]
    until = time.monotonic() + 2
    while running(pid) and time.monotonic() < until:
        time.sleep(0.05)
    check("the tool that passed the deadline is stopped", not running(pid))
    rig.stop()

    left = tmp / "left"
    leaver = "import os, sys, time\nif os.fork() == 0:\n    open(sys.argv[1], 'w').write(str(os.getpid()))\n    time.sleep(30)\n    os._exit(0)\n"
    rig = ToolRig(tmp, [(0, TOLD, 0)], timeout=20)
    rig.program.write_text(f"#!{sys.executable}\nimport os, sys\n{leaver.replace('sys.argv[1]', repr(str(left)))}"
                           f"while not os.path.exists({str(left)!r}):\n    pass\nsys.stdout.write({TOLD!r})\n")
    began = time.monotonic()
    got = ended(rig.call())
    check("a tool that ends and leaves a process behind: its reply is taken at once, and the process is stopped",
          got == (0, TOLD) and time.monotonic() - began < 10 and not running(int(left.read_text())), got)
    rig.stop()

    block = tmp / "environ"
    block.write_bytes(b"A=1\0B=x=y\0LANG=en_US.UTF-8\0")
    check("the variables a start was given are read from the kernel's block, and from Python's own where there is none",
          tool.started_with(block) == {"A": "1", "B": "x=y", "LANG": "en_US.UTF-8"} and tool.started_with(tmp / "none") == dict(os.environ))

    body = tool.request(["-p", "--model", "m"], {"DISABLE_PROMPT_CACHING": "1", "LANG": "en_US.UTF-8", "HOME": "/h", "PATH": "/bin"}, "records", "/w")[0]
    check("the name of a tool's request is the one kept answers were found by",
          cache.name_of("claude 2.1.327 (Claude Code)", "/", (), body) == "4eaecb10319d96a1cedffaf3d4445c4e142c4e6e71fb4546b73c97eeee7f34ce",
          cache.name_of("claude 2.1.327 (Claude Code)", "/", (), body))

    body, run = tool.request(["-p"], {"HOME": "/h", "LANG": "C", "PATH": "/p"}, "in", "/f")
    check("what the caller sends: the request in the body, and what a machine alone decides in a header",
          (json.loads(body), json.loads(run)) == ({"argv": ["-p"], "env": {"LANG": "C"}, "stdin": "in"}, {"folder": "/f", "env": {"HOME": "/h", "PATH": "/p"}}), (body, run))


def tool_starts():
    tmp = Path(tempfile.mkdtemp())
    rig = ToolRig(tmp, [(0, TOLD, 0)])
    saved = {name: getattr(build, name) for name in ("LAUNCHER", "ANSWERS", "LIFE", "DERIVING")}
    saved_env = os.environ.get("BILBO_EVAL_DERIVE_KEY")
    saved_popen, saved_run = build.subprocess.Popen, build.subprocess.run
    popens, runs = [], []
    port = rig.url.rsplit(":", 1)[1].rstrip("/")
    build.LAUNCHER, build.ANSWERS, build.LIFE, build.DERIVING = tmp / "launcher", tmp / "answers", tmp / "life", True
    started = type("Started", (), {"stdout": type("Out", (), {"readline": staticmethod(lambda: f"port {port}\n")})()})

    def popen(command, **options):
        if str(HERE / "cache.py") not in command:
            return saved_popen(command, **options)
        popens.append((command, options.get("env")))
        return started()

    build.subprocess.Popen = popen
    derive = {"provider": "anthropic", "settings": {"llm.anthropic.via": "tool", "llm.anthropic.model": "m", "llm.anthropic.tool.program": str(rig.program)},
              "cache": {"write": True, "timeout": 240}, "ceilings": {"calls": 7}}
    os.environ["BILBO_EVAL_DERIVE_KEY"] = FAKE_TOKEN
    try:
        settings, _, upstream = build.start_cache(derive)
        build.give_key(derive, settings)
        build.subprocess.run = lambda command, **options: (runs.append(options["env"]), type("Done", (), {"returncode": 0, "stdout": "", "stderr": ""})())[1]
        build.bilbo("all", "config")
        build.subprocess.run = saved_run
        command, env = popens[0]
        check("the cache of the tool is started with the real program, its name and version as the upstream, the ceiling of calls, and the folder of its provider and its way",
              command[command.index("--tool") + 1] == str(rig.program) and command[command.index("--upstream") + 1] == upstream == "claude 9.9.9 (Fake)"
              and command[command.index("--ceiling") + 1] == "7" and "--write" in command
              and command[command.index("--dir") + 1] == str(build.ANSWERS / "anthropic-tool"), command)
        check("the token is in the cache's environment alone: bilbo's holds no key and no stand-in",
              env.get("BILBO_EVAL_CACHE_KEY") == FAKE_TOKEN and "BILBO_EVAL_DERIVE_KEY" not in env and not build.KEYS
              and FAKE_TOKEN not in json.dumps(runs[0]) and cache.STAND_IN not in json.dumps(runs[0]), runs)
        check("the store's program is the file the build wrote, and the settings' other keys are as given",
              settings == {"llm.anthropic.via": "tool", "llm.anthropic.model": "m", "llm.anthropic.tool.program": str(build.LAUNCHER)}, settings)
        got = ended(rig.call(command=[str(build.LAUNCHER), "-p"]))
        check("bilbo's start of that file reaches the tool behind the cache and ends as it ended",
              got == (0, TOLD) and rig.started()[0]["argv"] == ["-p"] and os.access(build.LAUNCHER, os.X_OK), got)
        broken = tmp / "broken"
        broken.write_text("#!/bin/sh\nexit 3\n")
        broken.chmod(0o755)
        try:
            build.start_cache(derive | {"settings": {"llm.anthropic.via": "tool", "llm.anthropic.tool.program": str(broken)}})
        except SystemExit as stopped:
            check("a program that cannot say its version stops the build at its start", "--version exited 3" in str(stopped), stopped)
        else:
            check("a program that cannot say its version stops the build at its start", False)
        try:
            build.start_cache(derive | {"settings": {"llm.anthropic.via": "tool", "llm.anthropic.tool.program": str(tmp / "none")}})
        except SystemExit as stopped:
            check("a program that is not there stops the build at its start", "to stand behind the cache" in str(stopped), stopped)
        else:
            check("a program that is not there stops the build at its start", False)
    finally:
        build.subprocess.Popen, build.subprocess.run = saved_popen, saved_run
        build.KEYS.clear()
        for name, value in saved.items():
            setattr(build, name, value)
        if saved_env is None:
            os.environ.pop("BILBO_EVAL_DERIVE_KEY", None)
        else:
            os.environ["BILBO_EVAL_DERIVE_KEY"] = saved_env
        rig.stop()


class Raw(threading.Thread):
    def __init__(self, reply, silent=False, trickle=False, drip=0):
        super().__init__(daemon=True)
        self.reply, self.silent, self.trickle, self.drip, self.served = reply, silent, trickle, drip, []
        self.server = socket_server()
        self.address = f"http://127.0.0.1:{self.server.getsockname()[1]}"
        self.start()

    def run(self):
        import socket as _socket
        while True:
            try:
                line, _ = self.server.accept()
            except OSError:
                return
            threading.Thread(target=self.serve, args=(line,), daemon=True).start()

    def serve(self, line):
        try:
            data = b""
            while b"\r\n\r\n" not in data:
                data += line.recv(4096)
            self.served.append(data)
            if self.drip:
                for at in range(len(self.reply)):
                    line.sendall(self.reply[at:at + 1])
                    time.sleep(self.drip)
            elif self.silent:
                time.sleep(30)
            elif self.trickle:
                line.sendall(b"HTTP/1.1 200 OK\r\n")
                for _ in range(100):
                    line.sendall(b"x-a: b\r\n")
                    time.sleep(0.2)
            else:
                line.sendall(self.reply)
        except OSError:
            pass
        finally:
            line.close()


def socket_server():
    import socket
    server = socket.socket()
    server.bind(("127.0.0.1", 0))
    server.listen(8)
    return server


def api_short_and_deadline():
    tmp = Path(tempfile.mkdtemp())
    replies = [("a 200 with a length of 100 and no body", b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n"),
               ("a chunk cut in two", b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nab"),
               ("a 200 with no length and no chunks", b"HTTP/1.0 200 OK\r\n\r\n" + ANSWER.encode())]
    for label, reply in replies:
        raw = Raw(reply)
        rig = Rig(tmp, address=raw.address, answers=tmp / f"short-{label[:8]}")
        got = rig.post()
        check(f"{label}: the caller gets 502, nothing is kept, and the record says none (P3)",
              got[0] == 502 and not rig.files() and any(line[2] == "none" for line in rig.record_lines() if line[0] == "asked"), (got[:1], rig.record_lines()))
        rig.stop()
    whole = Raw(b"HTTP/1.1 200 OK\r\nContent-Length: " + str(len(ANSWER)).encode() + b"\r\n\r\n" + ANSWER.encode())
    rig = Rig(tmp, address=whole.address, answers=tmp / "whole")
    check("a whole body of the same kind is kept", rig.post()[0] == 200 and rig.files(), rig.files())
    rig.stop()
    for label, raw in (("an upstream that takes the connection and says nothing", Raw(b"", silent=True)),
                       ("an upstream that sends headers a line at a time", Raw(b"", trickle=True))):
        rig = Rig(tmp, address=raw.address, timeout=1, answers=tmp / f"dead-{label[12:18]}")
        started = time.monotonic()
        try:
            got = rig.post()
        except OSError as late:
            got = (None, str(late))
        took = time.monotonic() - started
        check(f"{label}: the caller has its 408 within a second of the timeout (P5)", got[0] == 408 and took < 2, (got[:1], took))
        check(f"{label}: nothing is kept", not rig.files(), rig.files())
        rig.stop()


def api_late_and_busy():
    tmp = Path(tempfile.mkdtemp())
    reply = b"HTTP/1.1 200 OK\r\nContent-Length: " + str(len(ANSWER)).encode() + b"\r\n\r\n" + ANSWER.encode()
    slow = Raw(reply, drip=0.02)
    rig = Rig(tmp, address=slow.address, timeout=1, answers=tmp / "late")
    started = time.monotonic()
    got = rig.post()
    check("an answer that comes whole after the deadline: the caller has the 408 at the deadline", got[0] == 408 and time.monotonic() - started < 2, got[:1])
    time.sleep(3)
    check("it is dropped: no file, no given, nothing kept, the folder as it was",
          not rig.files() and not any(line[0] == "given" or line[2:3] == ["kept"] for line in rig.record_lines()), (rig.files(), rig.record_lines()))
    rig.stop()
    slow = Raw(reply, drip=0.02)
    rig = Rig(tmp, address=slow.address, timeout=1, answers=tmp / "busy")
    first = rig.post()
    started = time.monotonic()
    second = rig.post()
    took = time.monotonic() - started
    spent = (rig.folder / "spent").read_text().splitlines()
    check("a second ask while the first is abandoned and running: the 408 at once, `busy`, no line added to spent, one request at the upstream (P11)",
          (first[0], second[0]) == (408, 408) and took < 0.5 and any(line[0] == "busy" for line in rig.record_lines()) and len(spent) == 1
          and len(slow.served) == 1, (first[:1], second[:1], took, spent, len(slow.served)))
    time.sleep(3)
    third = rig.post()
    check("when the abandoned ask has ended, the next ask of the request reaches the upstream", len(slow.served) == 2, (third[:1], len(slow.served)))
    rig.stop()


def api_framing_and_order():
    tmp = Path(tempfile.mkdtemp())
    rig = Rig(tmp, [(200, ANSWER, {}, 0)], answers=tmp / "basic")
    got = rig.post(key=f"Basic {cache.STAND_IN}", header="authorization")
    check("authorization Basic and the stand-in: the cache's 502, the upstream sees nothing, spent is as it was (Q5)",
          got[0] == 502 and not Upstream.seen and not (rig.folder / "spent").exists(), (got[:1], len(Upstream.seen)))
    rig.stop()
    for length in ("-1", "nonsense"):
        raw = Raw(b"HTTP/1.1 200 OK\r\nContent-Length: " + length.encode() + b"\r\n\r\n" + ANSWER.encode())
        rig = Rig(tmp, address=raw.address, answers=tmp / f"length{length}")
        got = rig.post()
        check(f"content-length: {length} on a 200: the cache's 502 and nothing kept (Q8)", got[0] == 502 and not rig.files(), (got[:1], rig.files()))
        rig.stop()
    deep = "[" * 10000 + "]" * 10000
    rig = Rig(tmp, respond=lambda body: (200, deep, {}, 0), answers=tmp / "deep")
    try:
        got = rig.post()
    except OSError as reset:
        got = (None, str(reset))
    replay = Rig(tmp, write=False, address=rig.address, answers=tmp / "deep")
    try:
        again = replay.post()
    except OSError as reset:
        again = (None, str(reset))
    check("ten thousand arrays inside each other, whole by their length: asked, kept, given back, and a replay gives the same bytes (Q4)",
          got[:2] == (200, deep) and rig.files() and again[:2] == (200, deep) and any(line[0] == "given" for line in replay.record_lines()),
          (got[0], len(rig.files()), again[0]))
    rig.stop(); replay.stop()

    now, arrived, gate = [0.0], threading.Event(), threading.Event()

    def respond(body):
        arrived.set()
        gate.wait(10)
        return 200, ANSWER, {}, 0

    rig = Rig(tmp, respond=respond, timeout=3, clock=lambda: now[0], answers=tmp / "order")
    answers = []
    poster = threading.Thread(target=lambda: answers.append(rig.post()))
    poster.start()
    arrived.wait(10)
    now[0] = 100.0
    gate.set()
    poster.join()
    check("a result stamped after the deadline is not given on, whatever the handler woke to: 408, no file, no given (Q6)",
          answers and answers[0][0] == 408 and not rig.files() and not any(line[0] == "given" for line in rig.record_lines()), (answers, rig.record_lines()))
    rig.stop()

    real = cache.Cache.fetch
    calls = []

    def dying(self, *arguments):
        calls.append(1)
        if len(calls) == 1:
            raise KeyError("an error the cache does not name")
        return real(self, *arguments)

    cache.Cache.fetch = dying
    try:
        rig = Rig(tmp, [(200, ANSWER, {}, 0)], answers=tmp / "dies")
        first = rig.post()
        second = rig.post()
    finally:
        cache.Cache.fetch = real
    check("an ask that raises an error the cache does not name: the caller gets 502, and the next ask of the request is sent (Q7)",
          first[0] == 502 and second[0] == 200 and len(Upstream.seen) == 1, (first[:1], second[:1], len(Upstream.seen)))
    rig.stop()

    release, entered = threading.Event(), threading.Event()
    calls.clear()

    def late_death(self, *arguments):
        calls.append(1)
        if len(calls) == 1:
            entered.set()
            release.wait(10)
            raise KeyError("an error the cache does not name")
        return real(self, *arguments)

    cache.Cache.fetch = late_death
    try:
        rig = Rig(tmp, [(200, ANSWER, {}, 0)], timeout=1, answers=tmp / "dies-late")
        first = rig.post()
        busy = rig.post()
        release.set()
        waited = time.monotonic()
        while rig.cache.busy and time.monotonic() - waited < 5:
            time.sleep(0.01)
        third = rig.post()
    finally:
        cache.Cache.fetch = real
        release.set()
    check("an abandoned ask that dies clears its mark: the next ask of the request is sent (Q7)",
          (first[0], busy[0], third[0]) == (408, 408, 200) and not rig.cache.busy, (first[:1], busy[:1], third[:1], rig.cache.busy))
    rig.stop()


def hold_syncs():
    tmp = Path(tempfile.mkdtemp())
    events = []
    saved = cache.sync_folder
    cache.sync_folder = lambda folder: events.append(f"folder {Path(folder).name}")
    try:
        first = build.hold(tmp / "answers" / "anthropic-api", "anthropic-api")
        made = list(events)
        events.clear()
        first.close()
        second = build.hold(tmp / "answers" / "anthropic-api", "anthropic-api")
        found = list(events)
        second.close()
    finally:
        cache.sync_folder = saved
    check("the build's own start: the provider's folder is made, and /answers is synced first; found, it is synced again (P2)",
          made == ["folder answers"] and found == ["folder answers"], (made, found))


def api_sync():
    tmp = Path(tempfile.mkdtemp())
    events = []
    saved = cache.sync_file, cache.sync_folder, cache.replace
    cache.sync_file = lambda handle: events.append(f"file {Path(handle.name).name}")
    cache.sync_folder = lambda folder: events.append(f"folder {Path(folder).name}")
    cache.replace = lambda source, target: (events.append(f"rename {Path(target).name}"), os.replace(source, target))

    class Tap(list):
        def append(self, item):
            events.append(item)

    try:
        rig = Rig(tmp, [(200, ANSWER, {}, 0), (200, ANSWER, {}, 0)], answers=tmp / "answers-sync")
        Upstream.events = Tap()
        rig.post()
        first = list(events)
        events.clear()
        rig.post(body='{"model":"m","messages":[1]}')
        second = list(events)
    finally:
        cache.sync_file, cache.sync_folder, cache.replace = saved
        Upstream.events = None
    names = {p.name for p in rig.folder.iterdir() if p.is_dir()}
    normal = lambda events: ["folder <request>" if e.removeprefix("folder ") in names else e for e in events]
    check("a first ask of a new provider: its folder into /answers, spent made to last, the ask line, then the provider, then the answer, its folder and the provider's (N5)",
          normal(first) == [f"folder {tmp.name}", "file spent", "folder answers-sync", "upstream", "file 0.part", "rename 0", "folder <request>",
                            "folder answers-sync"], normal(first))
    check("a second request: its ask line is synced, then the provider, then the answer's paths",
          normal(second) == ["file spent", "upstream", "file 0.part", "rename 0", "folder <request>", "folder answers-sync"], normal(second))
    rig.stop()


def api_lives():
    tmp = Path(tempfile.mkdtemp())
    folder = tmp / "answers-lives"
    refusal = (200, REFUSAL, {}, 0)
    rig = Rig(tmp, [refusal, refusal], answers=folder)
    rig.post(); rig.post()
    second = Rig(tmp, [(200, ANSWER, {}, 0)], address=rig.address, answers=folder, respond=lambda body: (200, ANSWER, {}, 0))
    got = [second.post()[1] for _ in range(3)]
    check("a start again (finding 45): a new life gives the two kept refusals to the first two asks and sends the third on",
          got == [REFUSAL, REFUSAL, ANSWER] and len(Upstream.seen) == 1, got)
    rig.stop(); second.stop()

    folder = tmp / "answers-ceiling"
    (folder).mkdir()
    (folder / "spent").write_text("ask a\nask b\nask c\nrun 10 5 1\n")
    rig = Rig(tmp, [(200, ANSWER, {}, 0)], ceiling=3, answers=folder)
    got = rig.post()
    check("a start again after a ceiling (finding 35): the three `ask` lines in spent leave nothing to send on", got[0] == 404 and not Upstream.seen, got[:1])
    spent = build.spent_by(folder)
    check("the manifest's spent is the folder's", spent == {"asks": 3, "tokens_in": 10, "tokens_out": 5, "refused": 1}, spent)
    rig.stop()

    folder = tmp / "answers-rebuild"
    rig = Rig(tmp, [(200, ANSWER, {}, 0), (200, REFUSAL, {}, 0)], answers=folder)
    rig.post(); rig.post()
    replay = Rig(tmp, write=False, address=rig.address, answers=folder)
    got = [replay.post() for _ in range(3)]
    check("a rebuild after a stopped paid build (L6): a life with no writer gets every kept answer, then a miss",
          [g[1] for g in got] == [ANSWER, REFUSAL, cache.MISS.decode()] and got[2][0] == 404, [g[:1] for g in got])
    rig.stop(); replay.stop()

    folder = tmp / "answers-failures"
    rig = Rig(tmp, [(200, ANSWER, {}, 0), (400, "{}", {}, 0), (200, REFUSAL, {}, 0)], answers=folder)
    for _ in range(3):
        rig.post()
    replay = Rig(tmp, write=False, address=rig.address, answers=folder)
    check("a replay gives all three: the 200, the 400 and the 200", [replay.post()[0] for _ in range(3)] == [200, 400, 200])
    build.drop_failures(folder)
    again = Rig(tmp, write=False, address=rig.address, answers=folder)
    got = [again.post()[1] for _ in range(2)]
    check("a build with a writer removes the kept failure and gives the two 200s in order", got == [ANSWER, REFUSAL], got)
    rig.stop(); replay.stop(); again.stop()


def lives_begin():
    tmp = Path(tempfile.mkdtemp())
    saved = build.STORE
    build.STORE = tmp
    try:
        (tmp / "stores" / "a").mkdir(parents=True)
        (tmp / "stores" / "a" / "state").write_text("the life before")
        life = build.Life(tmp / "answers", tmp / "life", True)
        life.record.write_text("asked x kept 0\n")
        life.runs, life.seen = [{"store": "a"}], 1
        build.begin(life)
        check("a life starts from nothing: its stores, its record and its runs are made anew",
              not (tmp / "stores").exists() and life.record.read_text() == "" and life.runs == [] and life.seen == 0)
    finally:
        build.STORE = saved
    check("the answers are in no folder a rebuild removes", not build.ANSWERS.is_relative_to(build.WORK) and build.LIFE.is_relative_to(build.WORK))


def lives_end():
    tmp = Path(tempfile.mkdtemp())
    derive = DERIVE

    def attempt(script):
        life = build.Life(tmp / "answers", tmp / "life-end", True)
        script = list(script)

        def make():
            life.record.write_text("".join(f"{line}\n" for line in script.pop(0)))
            return "made"

        try:
            return build.lives(derive, life, make), None, script
        except SystemExit as stopped:
            return None, str(stopped), script

    got, error, left = attempt([["asked a kept 0", "given a 0"], ["given a 0"]])
    check("a life that asked is followed by another; a life that asked nothing is the last",
          error is None and got[1] == [{"runs": 0, "asked": 1, "kept": 1}, {"runs": 0, "asked": 0, "kept": 0}] and not left, (got, error))
    got, error, left = attempt([["asked a status 429", "asked b none", "asked c status 429"], ["given a 0"]])
    check("a life that asked and kept nothing ends the build with what came back",
          error is not None and "3 times" in error and "2 429" in error and "1 no answer" in error and left, error)
    got, error, left = attempt([["given a 0"]])
    check("a build with nothing asked ends on its first life", error is None and len(got[1]) == 1)


def settings_stops():
    def refused(text, endpoint=None):
        saved = os.environ.get("BILBO_EVAL_DERIVE")
        os.environ["BILBO_EVAL_DERIVE"] = json.dumps(text)
        try:
            build.derive_env()
        except SystemExit as stopped:
            return str(stopped)
        finally:
            if saved is None:
                del os.environ["BILBO_EVAL_DERIVE"]
            else:
                os.environ["BILBO_EVAL_DERIVE"] = saved
        return None

    error = refused({"provider": "claude"})
    check("the provider claude is refused by name", error and "'claude' is none of anthropic and openai" in error, error)
    errors = [refused({"provider": ["anthropic"]}), refused({"provider": {"anthropic": 1}})]
    check("a provider that is no text is refused by name", all(error and "is none of anthropic and openai" in error for error in errors), errors)
    check("anthropic by the way tool is taken", refused({"provider": "anthropic", "settings": {"llm.anthropic.via": "tool"}}) is None)
    error = refused({"provider": "openai", "settings": {"llm.openai.via": "tool"}})
    check("openai by the way tool is refused", error and "the provider 'openai' has no way 'tool'" in error, error)
    error = refused({"provider": "anthropic", "settings": {"llm.anthropic.via": "cli"}})
    check("a way that is none is refused by name", error and "the way 'cli' is none of api and tool" in error, error)
    check("the folder of a build is its provider and its way",
          (build.folder_of({"provider": "anthropic"}), build.folder_of({"provider": "anthropic", "settings": {"llm.anthropic.via": "tool"}}),
           build.folder_of({"provider": "openai", "settings": {}})) == ("anthropic-api", "anthropic-tool", "openai-api"))
    error = refused({"provider": "gemini"})
    check("another provider is refused by name", error and "'gemini' is none of anthropic and openai" in error, error)
    error = refused({"provider": "anthropic", "settings": {"llm.provider": "openai"}})
    check("settings that name another provider than the cache's are refused", error and "the settings name the provider 'openai', not 'anthropic'" in error, error)
    check("settings that name the same provider are taken", refused({"provider": "anthropic", "settings": {"llm.provider": "anthropic"}}) is None)
    error = refused({"provider": "anthropic", "settings": {'llm."provider"': "openai"}})
    check("a leaf of the provider's key spelt in quotes makes a config that does not parse", error and "the settings make a config that does not parse" in error, error)
    error = refused({"provider": "anthropic", "settings": {"llm": {"provider": "openai"}}})
    check("a table given as a value makes a config that does not parse", error and "the settings make a config that does not parse" in error, error)
    check("the config the build writes is the arm's keys, then the settings",
          build.bilbo_config({"provider": "anthropic", "settings": {}}, ["a"], {"derive.tries": 2})
          == 'follow = []\n\n[derive]\nenabled = true\nin = ["a"]\ntries = 2\n\n[derive.summary]\nenabled = false\n\n[llm]\nprovider = "anthropic"\n\n[llm.anthropic]\nvia = "api"\n')
    secret = "https://user:hunter2@api.example.com/v1"
    error = refused({"provider": "anthropic", "settings": {"llm.anthropic.api.endpoint": secret}})
    check("an endpoint with an @ is refused, and its value is in no text", error and "no user name or password" in error and "hunter2" not in error, error)
    for where in ("https://bob:2024?x@127.0.0.1/v1", "https://bob:2024#x@127.0.0.1/v1", "https://127.0.0.1/v1?a=b@c"):
        error = refused({"provider": "anthropic", "settings": {"llm.anthropic.api.endpoint": where}})
        check(f"an endpoint with an @ anywhere is refused: {where.split('@')[0][-12:]}@", error and "no user name or password" in error and "2024" not in error, error)
    for spelling in ({"derive.tags.enabled": True}, {"derive.tags": {"enabled": True}}, {'derive."tags".enabled': True}, {"derive.tags": {}}):
        error = refused({"provider": "anthropic", "settings": spelling})
        check(f"a settings table that makes a tags table under derive is refused by the default way: {list(spelling)[0]}",
              error == "bilbo-next: BILBO_EVAL_DERIVE: the arm does not tag", error)
    for spelling in ({"derive.summary.enabled": True}, {"derive.summary": {"enabled": True}}, {'derive."summary".enabled': True},
                     {"derive.summary": {}}, {"derive.summary.most": 100}):
        error = refused({"provider": "anthropic", "settings": spelling})
        check(f"a settings table that makes a summary table under derive is refused by the default way: {list(spelling)[0]}",
              error == "bilbo-next: BILBO_EVAL_DERIVE: the arm does not summarise", error)
    check("a settings table with no tags is taken by the default way", refused({"provider": "anthropic", "settings": {"derive.tries": 2}}) is None)
    check("an endpoint with no @ is taken", refused({"provider": "anthropic", "settings": {"llm.anthropic.api.endpoint": "https://api.example.com/v1"}}) is None)
    check("the key's variable is the setting's key_env, else the provider's own (L17)",
          (build.key_variable({"provider": "openai"}, {}), build.key_variable({"provider": "anthropic"}, {}),
           build.key_variable({"provider": "openai"}, {"llm.openai.api.key_env": "MY_KEY"})) == ("OPENAI_API_KEY", "ANTHROPIC_API_KEY", "MY_KEY"))
    manifest = {"stores": {"a": {"note_entries": 2, "noted": 1}, "b": {"note_entries": 2, "noted": 0}}}
    check("a store is asked for a note when it holds a note, not when it holds a note entry (L18)",
          (arm_check.has_notes(manifest, "a"), arm_check.has_notes(manifest, "b")) == (True, False))
    cases = [("every entry passed over: 2 sessions, 2 note entries, searched 2", (2, 0, 2), True), ("one note written: searched 3", (3, 1, 2), True),
             ("a long session added: 3 sessions, 2 notes, searched 5", (5, 2, 3), True),
             ("2 sessions, one note, searched 1", (1, 1, 2), False), ("more than sessions plus notes", (4, 1, 2), False),
             ("nothing searched", (0, 0, 2), False)]
    for label, args, wanted in cases:
        check(f"the probe's count (L19): {label}", build.probe_counts(*args) is wanted, args)
    for line in ('llm.anthropic.api.headers = { "x" = "hunter2" } (file)', 'llm.anthropic.tool.environment = { "T" = "hunter2" } (file)',
                 'llm.openai.api.headers = { "x" = "hunter2" } (file)', 'llm.openai.tool.environment = { "T" = "hunter2" } (file)'):
        shown = build.config_line(line)
        check("the manifest keeps the name of a headers or environment setting and not its value (finding 38)",
              shown.startswith(line.split(" = ")[0]) and "hunter2" not in shown, shown)
    check("another setting is kept whole", build.config_line("derive.tries = 3 (default)") == "derive.tries = 3 (default)")
    check("recall.notes_budget is kept whole", build.config_line("recall.notes_budget = 4000 (default)") == "recall.notes_budget = 4000 (default)")
    listing = "".join(line + "\n" for line in [
        'store.wait = "5s" (default)', 'recall.budget = 9000 (default)', 'recall.notes_budget = 4000 (default)',
        'llm.provider = "anthropic" (file)', 'llm.anthropic.api.headers = { "x" = "hunter2" } (file)',
        'llm.anthropic.tool.environment = { "A" = "a\u2028llm.leak = hunter2" } (file)',
        'llm.openai.api.headers = { "x" = "hunter2" } (file)', 'llm.openai.tool.environment = { "T" = "hunter2" } (file)',
        'derive.tries = 3 (default)'])
    check("the manifest keeps the selected lines of a listing, cut at the line feed alone, with no value of a headers or environment setting",
          build.manifest_lines(listing) == ['recall.notes_budget = 4000 (default)', 'llm.provider = "anthropic" (file)',
                                            'llm.anthropic.api.headers = [left out]', 'llm.anthropic.tool.environment = [left out]',
                                            'llm.openai.api.headers = [left out]', 'llm.openai.tool.environment = [left out]',
                                            'derive.tries = 3 (default)'], build.manifest_lines(listing))


def api_key_in_a_kept_answer():
    tmp = Path(tempfile.mkdtemp())
    folder = tmp / "answers-kept-key"
    first = Rig(tmp, [(200, f'{{"echo":"{FAKE_KEY}"}}', {}, 0)], answers=folder, key="sk-another-key-9876543210")
    check("an answer that holds another key is kept by the build of that key", first.post()[0] == 200 and len(first.files()) == 1, first.files())
    spent = (folder / "spent").read_text()
    again = Rig(tmp, address=first.address, answers=folder)
    got = again.post()
    lines = again.record_lines()
    check("a kept body that holds the cache's real key is not given back: the cache's 502 and own body, no given line, an asked key line, nothing sent, nothing spent",
          (got[0], got[1]) == (502, cache.UNREACHED.decode()) and FAKE_KEY not in got[1] and not any(line[0] == "given" for line in lines)
          and [line[0] for line in lines] == ["asked"] and lines[0][2] == "key" and not Upstream.seen and (folder / "spent").read_text() == spent, (got[:1], lines))
    asked = [" ".join(line) for line in lines]
    result = loop(3, [(0, dry(1, 1, DUE), "")], [run(1, 0, 0, 0, 0, *asked)])
    expect_error("the loop stops on that record line", result, "a response held the key")
    first.stop(); again.stop()


def api_stand_in_elsewhere():
    tmp = Path(tempfile.mkdtemp())
    stand = cache.STAND_IN
    cases = [("x-api-key with the stand-in beside authorization Basic with it", "/v1/messages", {"x-api-key": stand, "authorization": f"Basic {stand}"}, BODY),
             ("the stand-in in another header", "/v1/messages", {"x-api-key": stand, "x-other": stand}, BODY),
             ("the stand-in in the body", "/v1/messages", {"x-api-key": stand}, BODY.replace("{", f'{{"s":"{stand}",', 1) if isinstance(BODY, str) else BODY + stand.encode()),
             ("the stand-in in the path", f"/v1/messages?k={stand}", {"x-api-key": stand}, BODY),
             ("the stand-in in both key headers", "/v1/messages", {"x-api-key": stand, "authorization": f"Bearer {stand}"}, BODY)]
    for label, path, headers, body in cases:
        rig = Rig(tmp, [(200, ANSWER, {}, 0)], answers=tmp / f"answers-{abs(hash(label))}")
        data = body if isinstance(body, bytes) else body.encode()
        connection = http.client.HTTPConnection(rig.url.removeprefix("http://"), timeout=20)
        try:
            connection.request("POST", path, body=data, headers=headers | {"content-type": "application/json"})
            response = connection.getresponse()
            got = response.status, response.read().decode()
        finally:
            connection.close()
        check(f"{label}: not sent, the cache's 502, nothing spent", got == (502, cache.UNREACHED.decode()) and not Upstream.seen and not (rig.folder / "spent").exists()
              and not rig.files(), (got, Upstream.seen))
        rig.stop()


def header_fields():
    check_exit = arm_check.lines()
    search_exit = search.lines()
    check_ok = check_exit == 0 and search_exit == 0
    if not check_ok:
        sys.exit("selftest: the header tables FAILED")
    print("selftest: both header tables: OK")


def main():
    header_fields()
    sources()
    notes()
    derive_lines()
    derive_loop()
    derive_stops()
    derive_allowance()
    api_cache()
    api_cache_waits()
    api_deadline_and_redirect()
    endpoint_rule()
    two_builds()
    main_order()
    bilbo_starts()
    tags_refusals()
    api_secret()
    api_key_is_the_caches()
    api_key_in_a_kept_answer()
    api_stand_in_elsewhere()
    api_short_and_deadline()
    api_late_and_busy()
    hold_syncs()
    api_framing_and_order()
    api_sync()
    api_lives()
    lives_begin()
    lives_end()
    settings_stops()
    tool_cache()
    tool_starts()
    print("selftest: ALL OK")


if __name__ == "__main__":
    main()
