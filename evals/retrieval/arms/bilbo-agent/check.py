"""Check a bilbo store unpacked in /work/store, with no LLM: bilbo's own check on the one store, its notes, the writer's
journals and the manifest against each other, each project's sessions rendered again from the corpus, the vectors, and the
searches that must run on them (four generic ones, and the title of each project's first note). Then the smallest
project's notes are indexed again, and their vectors must point the way the stored ones do. build-store.sh --check runs
it in the build service."""

import hashlib
import platform
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.request
from array import array
from datetime import datetime
from math import isfinite, sqrt
from pathlib import Path, PurePosixPath

sys.path.insert(0, "/app/l3")

import render  # noqa: E402
import writer  # noqa: E402

STORE = Path("/work/store")
CORPUS = Path("/corpus")
BILBO = "/usr/local/bin/bilbo"
SKILL = Path("/app/skill/SKILL.md")
EMBEDDER = os.environ["EMBEDDER_URL"].rstrip("/")
MODEL, DIMS = "qwen3-embedding-0.6b", 1024
# The inner sha256 of each Linux release binary, as the Dockerfile pins them.
RELEASE_SHA256 = {
    "x86_64": "52e4eb13b5213f994e56400af4a8e0c7ff6b12b3d9787df0d2c5507e3228c31e",
    "aarch64": "00c0677058d461081e3b822194b4f24c1c35f7de118b7f0c2d56655af9436fb0",
}
SENTINEL = "\u0000"
# A passage embedded on a CPU and on a GPU differs by floating-point noise, down to a cosine of 0.9976 measured for dense;
# two different passages of a store rarely come above 0.99.
MIN_COSINE = 0.99
QUERIES = ("what was decided about the build", "which bug was fixed", "how are tests run", "what did the user prefer")
KINDS = "plan|spec|design|decision|gotcha|research|review|report|reference"
NOTE_NAME = re.compile(rf"^({KINDS})-[a-z0-9]+(-[a-z0-9]+)*\.md$")
ULID = re.compile(r"^[0-9A-HJKMNP-TV-Z]{26}$")
CREATED = re.compile(r"^\d{4}-\d\d-\d\dT\d\d:\d\d\+00:00$")
SESSION_FILE = re.compile(r"^(\d{4}-\d\d-\d\dT)(\d\d)(\d\d)(\d\d)Z--(.+)\.jsonl$")
FNV_OFFSET, FNV_PRIME = 0xCBF29CE484222325, 0x100000001B3

manifest = json.loads((STORE / "manifest.json").read_text())
problems = []


def fail(msg):
    problems.append(msg)
    print("FAIL", msg)


def expect(ok, msg):
    if not ok:
        fail(msg)


def fnv_name(root):
    h = FNV_OFFSET
    for b in str(PurePosixPath(root)).encode():
        h = ((h ^ b) * FNV_PRIME) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}.vectors"


def read_vectors(path):
    """(model name, dims, {key: floats}) of a bilbo vector file (vectors.rs:124-172)."""
    data = Path(path).read_bytes()
    magic = b"BILBOVEC1\n"
    if not data.startswith(magic):
        raise ValueError("not a bilbo vector file")
    n = len(magic)
    length = int.from_bytes(data[n : n + 4], "little")
    name = data[n + 4 : n + 4 + length].decode()
    n += 4 + length
    dims, count = int.from_bytes(data[n : n + 4], "little"), int.from_bytes(data[n + 4 : n + 8], "little")
    n += 8
    record = 8 + 4 * dims
    if len(data) != n + count * record:
        raise ValueError("cut short")
    out = {}
    for i in range(count):
        at = n + i * record
        floats = array("f")
        floats.frombytes(data[at + 8 : at + record])
        if sys.byteorder == "big":
            floats.byteswap()
        out[int.from_bytes(data[at : at + 8], "little")] = floats
    return name, dims, out


def env_for(home, cache, scratch):
    scratch = Path(scratch)
    for d in ("home", "config", "data", "state"):
        (scratch / d).mkdir(parents=True, exist_ok=True)
    return {
        "HOME": str(scratch / "home"), "BILBO_HOME": str(home), "BILBO_CONFIG": str(STORE / "config"), "XDG_CACHE_HOME": str(cache),
        "XDG_CONFIG_HOME": str(scratch / "config"), "XDG_DATA_HOME": str(scratch / "data"), "XDG_STATE_HOME": str(scratch / "state"),
        "PATH": "/usr/bin:/bin", "NO_COLOR": "1", "LANG": "en_US.UTF-8", "LC_ALL": "en_US.UTF-8",
    }  # fmt: skip


def bilbo(*args, home, cache=STORE / "cache", scratch="/tmp/check-home"):
    return subprocess.run([BILBO, *args], env=env_for(home, cache, scratch), cwd=scratch, capture_output=True, text=True, timeout=7200)


def sentinel():
    request = urllib.request.Request(EMBEDDER + "/v1/embeddings", json.dumps({"input": [SENTINEL]}).encode(), {"Content-Type": "application/json"})
    urllib.request.urlopen(request, timeout=600).read()


def minute(ts):
    return writer.minute(ts)


def render_again(repo, sids, tag="out"):
    """The repo's sessions rendered from the corpus as the build rendered them; returns the exchanges file."""
    tmp = Path("/tmp/check-render") / tag / repo
    raw = tmp / "data" / "raw" / "transcripts"
    raw.mkdir(parents=True, exist_ok=True)
    found = []
    for f in sorted((CORPUS / repo).glob("*.jsonl")):
        m = SESSION_FILE.match(f.name)
        if m and m[5] in sids:
            (raw / f"{m[5]}.jsonl").unlink(missing_ok=True)
            (raw / f"{m[5]}.jsonl").symlink_to(f)
            found.append({"session_id": m[5], "created_at": f"{m[1]}{m[2]}:{m[3]}:{m[4]}Z"})
    rid = repo.replace("__", "/", 1)
    (tmp / "data" / "selection.json").write_text(json.dumps({"dev": "", "test": [], "repos": {rid: {"key": repo, "sessions": found}}}))
    render.main(["--data", str(tmp / "data"), "--selection", str(tmp / "data" / "selection.json"), "--repo", rid, "--out", str(tmp / "out")])
    return tmp / "out" / f"{repo}.jsonl"


def topic_of(name):
    return re.sub(rf"^({KINDS})-", "", name).removesuffix(".md")


def note_problems(name, text, first, last):
    """What is wrong with one note file, given the minutes of its project's first and last exchange."""
    fm, _ = writer.split_frontmatter(text)
    keys = set(re.findall(r"^([A-Za-z_][\w-]*):", text.split("\n---\n", 1)[0], re.M))  # top-level lines: sources is a list
    out = []
    if not (keys <= {"id", "created", "sources"} and {"id", "created"} <= keys):
        out.append(f"{name} frontmatter keys {sorted(keys)}")
    if not ULID.match(fm.get("id", "")):
        out.append(f"{name} id {fm.get('id')!r}")
    created = fm.get("created", "")
    if not (CREATED.match(created) and first <= created <= last):
        out.append(f"{name} created {created!r} outside {first} to {last}")
    return out


def check_repo(repo, info):
    """One project: its notes in the store, its journals and its exchanges rendered again. Returns the exchanges."""
    state = STORE / "writer" / repo
    print(f"== {repo}")
    rendered = render_again(repo, set(info["sessions"]))
    exch = writer.jsonl(rendered)
    expect(len(exch) == info["exchanges"], f"{repo}: {len(exch)} exchanges rendered again, the manifest says {info['exchanges']}")
    expect(hashlib.sha256(rendered.read_bytes()).hexdigest() == info["exchanges_sha256"], f"{repo}: the exchanges rendered again differ from the build's")
    first, last = (minute(min(r["ts"] for r in exch)), minute(max(r["ts"] for r in exch))) if exch else ("", "")
    expect(info["notes_count"] == len(info["notes"]), f"{repo}: notes_count")
    expect(info["llm_calls"] == info["writer"]["decisions"] + info["writer"]["run_llm_calls"], f"{repo}: llm_calls against the writer's counts")
    for name in info["notes"]:
        n = STORE / "notes" / name
        if not n.exists():
            fail(f"{repo}: {name} is not in the store")
            continue
        for problem in note_problems(name, n.read_text(), first, last):
            fail(f"{repo}: {problem}")
    edits = {e["path"] for e in writer.jsonl(state / "edits.jsonl")}
    orphans = [n for n in info["notes"] if f"notes/{n}" not in edits]
    expect(not orphans, f"{repo}: notes no edit row names: {orphans}")
    done, decided = writer.jsonl(state / "done.jsonl"), writer.jsonl(state / "decisions.jsonl")
    xids = {r["xid"] for r in exch}
    expect({r["xid"] for r in done} == xids and len(done) == len(xids), f"{repo}: the done journal's exchanges are not the rendered ones")
    expect({r["xid"] for r in decided} == xids and len(decided) == len(xids), f"{repo}: the decisions' exchanges are not the rendered ones")
    tool = writer.load_tool("bilbo", SKILL, writer.VIRTUAL_STORE["bilbo"])
    try:
        counts = writer.run_counts(writer.State(state), {r["xid"]: r for r in decided}, tool) | {"notes": info["notes_count"]}
        expect(counts == info["writer"], f"{repo}: the journals count {counts}, the manifest says {info['writer']}")
    except writer.WriterError as e:
        fail(f"{repo}: {e}")
    print(f"   {info['notes_count']} notes, {info['exchanges']} exchanges")


def check_store():
    """The one store: bilbo's check, the files against the manifest, topics and ids, the vectors, the searches."""
    repos = manifest["repos"]
    p = bilbo("check", home=STORE)
    dirty = [l for l in p.stdout.splitlines() if "(warning)" not in l]
    expect(p.returncode == 0 and not dirty, f"bilbo check exit {p.returncode}: {(dirty or [p.stderr])[:3]}")
    listed = [n for r in repos.values() for n in r["notes"]]
    files = sorted(f.name for f in (STORE / "notes").glob("*.md"))
    expect(sorted(listed) == files and len(set(listed)) == len(listed), f"the {len(files)} note files differ from the manifest's lists ({len(listed)})")
    topics = [topic_of(n) for n in files]
    expect(len(set(topics)) == len(topics), "two notes share a topic")
    for n in files:
        expect(bool(NOTE_NAME.match(n)), f"{n} is not <kind>-<topic>.md")
    ids = [writer.split_frontmatter((STORE / "notes" / n).read_text())[0].get("id") for n in files]
    expect(len(set(ids)) == len(ids), "a note id is not unique across the store")

    vectors = {}
    expect(manifest["notes"] == len(files), f"notes {manifest['notes']}, {len(files)} files")
    if not files:
        expect(manifest["vectors"] == 0 and manifest["index"]["vector_file"] is None, "vectors for an empty store")
        return None
    name = fnv_name(STORE)
    expect(manifest["index"]["vector_file"] == f"cache/bilbo/{name}", f"vector_file {manifest['index']['vector_file']} is not the hash of {STORE}")
    try:
        model, dims, vectors = read_vectors(STORE / manifest["index"]["vector_file"])
    except (OSError, ValueError) as e:
        fail(f"vector file: {e}")
        return None
    expect((model, dims, len(vectors)) == (MODEL, DIMS, manifest["vectors"]), f"vectors {model} {dims} dims x {len(vectors)}, expected {MODEL} {DIMS} x {manifest['vectors']}")
    expect(all(isfinite(x) for v in vectors.values() for x in v) and all(abs(sqrt(sum(x * x for x in v)) - 1) <= 1e-3 for v in vectors.values()), "a vector is not finite or not a unit vector")
    p = bilbo("index", home=STORE)
    expect(p.returncode == 0 and f"embedded 0, kept {manifest['vectors']}, dropped 0" in p.stdout, f"bilbo index: {p.stdout.strip()!r} {p.stderr.strip()[:200]!r}")

    queries = list(QUERIES)
    firsts = {}
    for repo, info in sorted(repos.items()):
        if info["notes"]:
            title = next((l[2:].strip() for l in (STORE / "notes" / info["notes"][0]).read_text().splitlines() if l.startswith("# ")), None)
            if title:
                firsts[(repo, title)] = info["notes"][0]
    for q in (*queries, *firsts):
        p = bilbo("recall", "--limit", "5", "--", q[1] if q in firsts else q, home=STORE)
        expect("not indexed" not in p.stderr and "embedder unavailable" not in p.stderr, f"recall {q!r} did not use the vectors: {p.stderr.strip()[:200]!r}")
        if q in firsts:
            expect(f"notes/{firsts[q]}:" in p.stdout, f"recall of the title of {firsts[q]} did not return it")
    return vectors


def first_stamp(path):
    """The first timestamp of a transcript, as build.py orders sessions by it; None for Cursor's."""
    for line in path.open():
        t = json.loads(line).get("timestamp")
        if t:
            return datetime.fromisoformat(t)
    return None


def reindex(repo, info, vectors):
    """The smallest project's notes indexed again, after a sentinel, in a folder of their own: each of its passages must
    point the way the store's vector for the same key does."""
    home = Path("/tmp/reindex")
    cache = Path("/tmp/reindex-cache")
    shutil.rmtree(home, ignore_errors=True)
    (home / "notes").mkdir(parents=True)
    cache.mkdir(parents=True, exist_ok=True)
    for n in info["notes"]:
        shutil.copyfile(STORE / "notes" / n, home / "notes" / n)
    sentinel()
    p = bilbo("index", home=home, cache=cache, scratch="/tmp/reindex-home")
    if p.returncode != 0 or not re.search(r"^embedded \d+, kept 0, dropped 0$", p.stdout, re.M):
        fail(f"{repo}: index again: exit {p.returncode} {p.stdout.strip()!r} {p.stderr.strip()[:200]!r}")
        return
    _, _, again = read_vectors(cache / "bilbo" / fnv_name(home))
    expect(set(again) <= set(vectors), f"{repo}: the vectors indexed again have keys the store lacks")
    cosines, diffs = [], []
    for k in set(again) & set(vectors):
        a, b = vectors[k], again[k]
        cosines.append(sum(x * y for x, y in zip(a, b)) / (sqrt(sum(x * x for x in a)) * sqrt(sum(y * y for y in b))))
        diffs.append(max(abs(x - y) for x, y in zip(a, b)))
    if not cosines:
        return fail(f"{repo}: no vector indexed again shares a key with the store")
    print(f"{repo} indexed again: {sum(d == 0 for d in diffs)} of {len(diffs)} identical, min cosine {min(cosines):.6f}, max abs diff {max(diffs):.2e}")
    expect(min(cosines) >= MIN_COSINE, f"{repo}: a vector indexed again points elsewhere (cosine {min(cosines):.4f})")


def main():
    expect(list(manifest)[:5] == ["arm", "key", "inputs", "device", "versions"], f"the manifest opens with {list(manifest)[:5]}")
    expect(manifest["arm"] in ("bilbo-agent", "bilbo-assisted"), f"arm {manifest['arm']!r}")
    v = subprocess.run([BILBO, "--version"], capture_output=True, text=True).stdout.strip()
    expect(v == "bilbo 0.19.0", f"bilbo --version prints {v!r}")
    pin = RELEASE_SHA256.get(manifest["versions"]["arch"])
    expect(manifest["versions"]["bilbo_sha256"] == pin, f"the binary that built the store is not the release pinned for {manifest['versions']['arch']}")
    if manifest["versions"]["arch"] == platform.machine():
        expect(hashlib.sha256(Path(BILBO).read_bytes()).hexdigest() == pin, "the image's binary is not the pinned release")
    limit = next(i.split()[1] for i in manifest["inputs"] if i.startswith("sessions "))
    found = []
    for f in sorted(CORPUS.glob("*/*.jsonl")):
        start = first_stamp(f)
        if start:
            found.append((start, f.parent.name, f.name))
    found.sort()
    expected = found if limit == "all" else found[: int(limit)]
    written = {(r, s) for r, i in manifest["repos"].items() for s in i["sessions"]}
    want = {(r, SESSION_FILE.match(name)[5]) for _, r, name in expected}
    expect(written <= want, f"sessions written that the first {limit} do not include: {sorted(written - want)[:3]}")
    for repo in sorted({r for r, _ in want}):
        missing = {s for r, s in want - written if r == repo}
        if missing and writer.jsonl(render_again(repo, missing, "missing")):
            fail(f"{repo}: sessions {sorted(missing)[:3]} have exchanges but are not in the store")
    kept = sum(len(r["sessions"]) for r in manifest["repos"].values())
    expect(manifest["sessions"] == kept, f"sessions {manifest['sessions']}, the repos list {kept}")
    expect(manifest["llm"]["calls"] == sum(r["llm_calls"] for r in manifest["repos"].values()), "llm.calls")
    on_disk = {p.name for p in (STORE / "writer").iterdir() if p.is_dir()}
    expect(on_disk == set(manifest["repos"]), f"writer folders {sorted(on_disk)} against the manifest's repos")

    for repo in sorted(manifest["repos"]):
        check_repo(repo, manifest["repos"][repo])
    vectors = check_store()
    withnotes = [(len(i["notes"]), r, i) for r, i in manifest["repos"].items() if i["notes"]]
    if vectors and withnotes:
        _, repo, info = min(withnotes)
        reindex(repo, info, vectors)
    else:
        print("no note: nothing to index again")
    print("FAILED" if problems else "OK")
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
