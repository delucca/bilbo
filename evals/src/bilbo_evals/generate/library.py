"""`generate library`: stage public-domain pages, let the renderer pick the kept lines, land them and copy the library in."""

from __future__ import annotations

import json
import random
import re
import shutil
import uuid
from datetime import datetime, timezone
from pathlib import Path

from bilbo_evals import common, llm, sandbox
from bilbo_evals.common import Refused
from bilbo_evals.generate import HERE, finish, schema, solve, template

STEP = "library"
CORPUS = "sqlite"
PAGES_FILE = HERE / "library_pages.txt"
# url -> local file: staged with `stage <file> --origin`, so tests never touch the network
STAGE_FROM_FILES: dict[str, str] = {}
LICENCES = {"public-domain"}
NAME = re.compile(r"[a-z][a-z0-9]*(-[a-z0-9]+){0,3}")
RESERVED = {"guide", "show", "stage", "land", "plan", "read"}
LEAD = ("The SQLite documentation, in the public domain: the SQL dialect, the file and locking model, "
        "write-ahead logging, pragmas, extensions and the C interface.")
HEAD_LINES, TAIL_LINES = 30, 20


def pages() -> list[tuple[str, str, str]]:
    """(url, licence, evidence) per usable line of the pages file; a line without a recorded licence is skipped."""
    out = []
    for raw in PAGES_FILE.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) < 3 or parts[1] not in LICENCES:
            common.err(f"skipped {parts[0]}: no recorded public-domain licence")
            continue
        out.append((parts[0], parts[1], parts[2]))
    return out


def slug(url: str) -> str:
    return re.sub(r"[^a-z0-9_]+", "-", url.split("://", 1)[-1].split("/", 1)[-1].removesuffix(".html").lower()).strip("-")


def parse_stage(stdout: str) -> dict:
    info: dict = {"headings": []}
    lines = stdout.splitlines()
    i = 0
    while i < len(lines) and lines[i].strip():
        key, _, value = lines[i].partition(":")
        info[key.strip()] = value.strip()
        i += 1
    for line in lines[i:]:
        num, _, text = line.partition("\t")
        if num.strip().isdigit():
            info["headings"].append((int(num), text.strip()))
    return info


def keep_problem(out: dict, lines: int) -> str | None:
    last = 0
    if not re.fullmatch(r"\d+-\d+(,\d+-\d+)*", out["keep"].replace(" ", "")):
        return f"keep {out['keep']!r} is not a list of a-b ranges"
    for r in out["keep"].replace(" ", "").split(","):
        a, b = (int(x) for x in r.split("-"))
        if not (last < a <= b <= lines):
            return f"keep range {r} is outside 1-{lines} or out of order"
        last = b
    name = out["name"].strip()
    if not NAME.fullmatch(name) or name in RESERVED:
        return f"name {name!r} is not a kebab-case source name"
    entry = out["guide_entry"].strip()
    if len(entry) < 20 or "TODO" in entry:
        return "guide entry is too short or holds a TODO"
    return None


def numbered(text: list[str], start: int) -> str:
    return "\n".join(f"{start + i}\t{line}" for i, line in enumerate(text))


def prompt_for(url: str, info: dict, capture: list[str]) -> str:
    n = len(capture)
    return template("library.md").substitute(
        url=url, lines=n, title=info.get("title", ""), default_keep=info.get("keep", f"1-{n}"),
        headings="\n".join(f"{a}\t{b}" for a, b in info["headings"][:80]) or "(none)",
        head=numbered(capture[:HEAD_LINES], 1), tail=numbered(capture[-TAIL_LINES:], max(1, n - TAIL_LINES + 1)),
    )


def land_title(capture: list[str], keep: str, name: str) -> str:
    """The first heading inside the kept ranges, else the source name humanized."""
    for r in keep.replace(" ", "").split(","):
        a, b = (int(x) for x in r.split("-"))
        for line in capture[a - 1: b]:
            m = re.match(r"#{1,6}\s+(.*?)\s*#*\s*$", line)
            if m and m.group(1):
                return m.group(1)
    return name.replace("-", " ").capitalize()


def redo(ds: Path) -> None:
    """Forget the landed library, keeping the generation record so cached renderer outputs are reused."""
    if (ds / "queries.jsonl").exists() or (ds / "generation/outputs/queries").exists():
        raise Refused("--redo is refused: queries exist and were worded from the library")
    (ds / "world/library.json").unlink(missing_ok=True)
    path = ds / "world/splits.json"
    if path.is_file():
        splits = json.loads(path.read_text(encoding="utf-8"))
        if splits.pop("library", None) is not None:
            common.write_json(path, splits)
    for rel in ("store/library", "store/.bilbo/captures"):
        shutil.rmtree(ds / rel, ignore_errors=True)


def stage(sb, exe: Path, url: str) -> dict | str:
    if url in STAGE_FROM_FILES:
        args = ["library", "stage", STAGE_FROM_FILES[url], "--origin", f"url: {url}",
                "--fetched", datetime.now(timezone.utc).date().isoformat()]
    else:
        args = ["library", "stage", url]
    p = sandbox.bilbo(sb, exe, args, timeout=120)
    if p.exit != 0:
        return f"exit {p.exit}: {p.stderr.strip().splitlines()[-1] if p.stderr.strip() else 'no message'}"
    info = parse_stage(p.stdout)
    capture = Path(info.get("capture", ""))
    if not capture.is_file():
        return "bilbo printed no capture file"
    text = capture.read_text(encoding="utf-8")
    info["capture_lines"] = text.splitlines()
    info["sha256"] = common.sha256_bytes(text.encode("utf-8"))
    return info


def edit_guide(guide: Path, entries: dict[str, str]) -> None:
    text = guide.read_text(encoding="utf-8")
    text = text.replace("TODO: describe this corpus.", LEAD)
    for name, entry in entries.items():
        text, n = re.subn(rf"(^## {re.escape(name)}\n+)TODO: describe this source\.", lambda m: m.group(1) + entry, text, flags=re.M)
        if n != 1:
            raise Refused(f"the guide has no TODO entry for {name}")
    guide.write_text(text if text.endswith("\n") else text + "\n", encoding="utf-8")


def copy_in(sb, ds: Path) -> None:
    store = ds / "store"
    store.mkdir(parents=True, exist_ok=True)
    for rel, ignore in (("library", shutil.ignore_patterns(".lock")), (".bilbo/captures", None)):
        src, dst = sb.store / rel, store / rel
        if dst.exists():
            shutil.rmtree(dst)
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(src, dst, ignore=ignore)


def write_splits(ds: Path, cfg: llm.GenConfig, refs: list[str]) -> None:
    w = cfg.world
    ratio = int(w["dev_projects"]) / int(w["projects"])
    order = sorted(refs)
    random.Random(f"{cfg.seed}:library").shuffle(order)
    cut = round(len(order) * ratio)
    path = ds / "world/splits.json"
    splits = json.loads(path.read_text(encoding="utf-8")) if path.is_file() else {"seed": cfg.seed}
    splits["library"] = {"dev": sorted(order[:cut]), "test": sorted(order[cut:])}
    common.write_json(path, splits)


def cmd(args) -> int:
    from bilbo_evals import dataset

    ds = Path(args.dataset)
    cfg = llm.load_config(ds)
    llm.require_cli("claude")
    if not args.bilbo:
        raise Refused("generate library needs --bilbo <binary>")
    exe = Path(args.bilbo)
    if getattr(args, "redo", False):
        redo(ds)
    target = int(cfg.library["pages"])
    candidates = pages()
    random.Random(f"{cfg.seed}:library-pages").shuffle(candidates)
    sb = sandbox.create(f"library-{uuid.uuid4().hex[:10]}")
    sandbox.write_config(sb, None)
    landed: list[dict] = []
    names: set[str] = set()
    stop = None
    try:
        at = 0
        while len(landed) < target and at < len(candidates) and stop is None:
            batch = candidates[at: at + target - len(landed)]
            at += len(batch)
            staged = {}
            for url, licence, evidence in batch:
                got = stage(sb, exe, url)
                if isinstance(got, str):
                    common.err(f"skipped {url}: {got}")
                else:
                    staged[slug(url)] = (url, licence, evidence, got)
            wants = {
                item: (prompt_for(url, info, info["capture_lines"]), schema("library.json"))
                for item, (url, _l, _e, info) in staged.items()
            }
            solved = solve(
                ds, cfg, STEP, "claude", wants,
                lambda item, out: keep_problem(out, len(staged[item][3]["capture_lines"])),
            )
            stop = solved.stop
            for item, (url, licence, evidence, info) in staged.items():
                out = solved.good.get(item)
                if out is None:
                    if stop is None:
                        common.err(f"skipped {url}: the renderer gave no usable keep range")
                    continue
                name, n = out["name"].strip(), 1
                while name in names:
                    n += 1
                    name = f"{out['name'].strip()}-{n}"
                keep_ranges = out["keep"].replace(" ", "")
                title = land_title(info["capture_lines"], keep_ranges, name)
                p = sandbox.bilbo(sb, exe, ["library", "land", info["stage"], f"{CORPUS}/{name}", "--keep", keep_ranges, "--title", title])
                if p.exit != 0:
                    tail = " | ".join(p.stderr.strip().splitlines()[-2:])
                    common.err(f"skipped {url}: land exited {p.exit}: {tail}")
                    continue
                names.add(name)
                landed.append({
                    "ref": f"{CORPUS}/{name}", "url": url, "licence": licence, "licence_evidence": evidence,
                    "keep": out["keep"].replace(" ", ""), "stage_sha256": info["sha256"], "guide_entry": out["guide_entry"].strip(),
                })
        if landed:
            edit_guide(sb.store / "library" / CORPUS / "guide.md", {r["ref"].split("/", 1)[1]: r["guide_entry"] for r in landed})
            checked = sandbox.bilbo(sb, exe, ["check"])
            if checked.exit != 0 or checked.stdout.strip():
                raise Refused("bilbo check rejects the landed library: " + "; ".join((checked.stdout + checked.stderr).strip().splitlines()[:3]))
            copy_in(sb, ds)
    finally:
        sandbox.destroy(sb)
    if landed:
        landed.sort(key=lambda r: r["ref"])
        common.write_json(ds / "world/library.json", landed)
        write_splits(ds, cfg, [r["ref"] for r in landed])
        dataset.build_corpus(ds)
    common.out(f"library: {len(landed)} of {target} sources landed")
    if stop is not None:
        common.err(f"{target - len(landed)} sources left")
        raise stop
    if len(landed) < target:
        raise Refused(f"only {len(landed)} of {target} sources landed; add pages to library_pages.txt")
    return 0
