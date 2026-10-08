"""`generate notes`: one renderer call per note from its fact manifest, written as a bilbo note file."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path

from bilbo_evals import common, llm
from bilbo_evals.common import REPO_ROOT, Refused
from bilbo_evals.generate import finish, read_json, schema, sections, solve

STEP = "notes"
SKILL = REPO_ROOT / "plugins/bilbo/skills/note/SKILL.md"
LANGUAGES = {"en": "English", "pt": "Brazilian Portuguese"}
MIN_BODY = 40


@dataclass
class World:
    ds: Path
    projects: dict[str, dict]
    facts: dict[str, dict]
    notes: list[dict]
    style: str
    style_sha: str

    def note_by_id(self, nid: str) -> dict:
        return next(n for n in self.notes if n["id"] == nid)

    def component_name(self, project: str, slug: str) -> str:
        return next(c["name"] for c in self.projects[project]["components"] if c["slug"] == slug)


def load(ds: Path) -> World:
    w = Path(ds) / "world"
    world = read_json(w / "world.json", "run `generate world` first")
    facts = common.read_jsonl(w / "facts.jsonl") if (w / "facts.jsonl").is_file() else None
    if facts is None:
        raise Refused("facts.jsonl is missing: run `generate facts` first")
    notes = common.read_jsonl(w / "notes.jsonl")
    if not SKILL.is_file():
        raise Refused(f"the note skill is missing at {SKILL}")
    style = SKILL.read_text(encoding="utf-8")
    return World(
        Path(ds), {p["slug"]: p for p in world["projects"]}, {f["id"]: f for f in facts}, notes, style,
        common.sha256_bytes(style.encode("utf-8")),
    )


def save_notes(ds: Path, notes: list[dict]) -> None:
    common.write_jsonl(Path(ds) / "world/notes.jsonl", sorted(notes, key=lambda n: n["id"]))


def save_facts(ds: Path, w: World) -> None:
    common.write_jsonl(Path(ds) / "world/facts.jsonl", sorted(w.facts.values(), key=lambda f: f["id"]))


def words(topic: str) -> str:
    return topic.replace("-", " ")


def prompt_for(w: World, note: dict, feedback: list[str] = ()) -> str:
    parts = sections("note.md")
    project = w.projects[note["project"]]
    component = w.component_name(note["project"], note["component"])
    style = note["style"]
    extra = []
    if note["facts"]:
        live = [w.facts[i] for i in note["facts"] if w.facts[i]["status"] == "planted"]
        lines = []
        for f in live:
            lines.append(f"- {f['statement']}\n  Must appear exactly: " + ", ".join(f"`{v}`" for v in f["verbatim"]))
            if f["supersedes"]:
                old = w.note_by_id(w.facts[f["supersedes"]]["note_id"])
                extra.append(f"- This note replaces an earlier note about \"{words(old['topic'])}\": say so in one sentence "
                             "and give the new value.")
        task = parts["gold"].substitute(facts="\n".join(lines), component=component)
        if note.get("omit"):
            extra.append(f"- Leave out {note['omit']}: do not mention it at all, while still including every required string.")
    elif note["near_duplicate_of"]:
        orig = w.note_by_id(note["near_duplicate_of"])
        task = parts["duplicate"].substitute(subject=words(orig["topic"]), component=component, kind=note["kind"])
    else:
        task = parts["filler"].substitute(activity=note["activity"], component=component)
    return parts["common"].substitute(
        style_sha=w.style_sha, style_guide=w.style.strip(), project_name=project["name"], project_summary=project["summary"],
        technologies=", ".join(project["technologies"]), component=component, kind=note["kind"],
        language=LANGUAGES[note["lang"]], task=task, chars=style["chars"], headings=style["headings"],
        code_line=("- Include one short fenced code block that fits, using only values given here."
                   if style["code_block"] else "- Use no fenced code blocks."),
        wiki_line=f"- Mention the related note once as [[{style['wiki_link']}]], written exactly so." if style["wiki_link"] else "",
        extra="\n".join(extra),
        feedback=("\nYour previous attempt was rejected. Fix these and keep everything else:\n" + "\n".join(f"- {x}" for x in feedback)
                  if feedback else ""),
    )


def valid(item: str, out: dict) -> str | None:
    if not out["title"].strip():
        return "empty title"
    if len(out["body"].strip()) < MIN_BODY:
        return "body too short"
    return None


def latest(ds: Path, note_id: str) -> tuple[int, dict | None]:
    """The newest attempt that has an output file, and the newest valid output (None when there is none)."""
    newest, good = 0, None
    for a in range(1, 10):
        path = llm.output_path(ds, STEP, note_id, a)
        if path.is_file():
            newest = a
            out = json.loads(path.read_text(encoding="utf-8"))
            if valid(note_id, out) is None:
                good = out
    return newest, good


def clean_body(title: str, body: str) -> str:
    text = body.replace("\r\n", "\n").strip("\n")
    if text.startswith("---\n"):
        end = text.find("\n---", 4)
        text = text[end + 4:].lstrip("\n") if end != -1 else text
    lines, fenced, out = text.split("\n"), False, []
    for i, line in enumerate(lines):
        if line.lstrip().startswith("```"):
            fenced = not fenced
        elif not fenced and re.match(r"# \S", line):
            if i == 0 and line[2:].strip().lower() == title.strip().lower():
                continue
            line = "#" + line
        out.append(line)
    return "\n".join(out).strip("\n")


def note_text(note: dict, out: dict) -> str:
    title = " ".join(out["title"].replace("#", " ").split()) or words(note["topic"]).capitalize()
    head = [f"id: {note['id']}", f"created: {note['created']}"]
    if note.get("sources"):
        head.append("sources:")
        head += [f'  - "{s.replace(chr(92), chr(92) * 2).replace(chr(34), chr(92) + chr(34))}"' for s in note["sources"]]
    return "---\n" + "\n".join(head) + f"\n---\n\n# {title}\n\n{clean_body(title, out['body'])}\n"


def write_note(ds: Path, note: dict, out: dict) -> None:
    path = Path(ds) / "store/notes" / note["file"]
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(f".{path.name}.tmp")
    tmp.write_text(note_text(note, out), encoding="utf-8")
    tmp.replace(path)


def write_all(ds: Path, w: World) -> int:
    """Write every note from its newest output; keep render_attempts in step. Returns the count written."""
    count = 0
    for note in w.notes:
        newest, good = latest(ds, note["id"])
        if good is None:
            continue
        note["render_attempts"] = newest
        write_note(ds, note, good)
        count += 1
    save_notes(ds, w.notes)
    return count


def cmd(args) -> int:
    from bilbo_evals import dataset

    ds = Path(args.dataset)
    cfg = llm.load_config(ds)
    llm.require_cli("claude")
    w = load(ds)
    wants = {n["id"]: (prompt_for(w, n), schema("note.json")) for n in w.notes if n["status"] == "kept"}
    solved = solve(ds, cfg, STEP, "claude", wants, valid)
    written = write_all(ds, w)
    dataset.build_corpus(ds)
    common.out(f"notes: {written} of {len(wants)} written")
    finish(STEP, solved, len(wants), "notes")
    return 0
