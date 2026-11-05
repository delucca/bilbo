"""What the replay changes in a transcript before L3's renderer reads it, shared by build.py and check.py."""

import json
import re
from pathlib import Path

NAME = re.compile(r"^(\d{4}-\d\d-\d\dT)(\d\d)(\d\d)(\d\d)Z--(.+)\.jsonl$")
# str.splitlines(), which render.py, schema.py and writer.jsonl split on, also ends a line at these; JSON allows them raw
# only inside a string, where the \u escape parses to the same string.
SEPARATORS = {"\x85": "\\u0085", " ": "\\u2028", " ": "\\u2029"}


def escape_separators(text):
    for c, esc in SEPARATORS.items():
        text = text.replace(c, esc)
    return text


def transcript_text(path):
    """The transcript as the renderer should read it, and what changed: the separators escaped, and a session continued
    from another machine (claude --teleport) started at its own first record, since the records before it ran there
    under another session id."""
    path = Path(path)
    text = path.read_text(encoding="utf-8")
    out = escape_separators(text)
    changes = {"separators": sum(text.count(c) for c in SEPARATORS)} if out != text else {}
    sid = NAME.match(path.name)[5]
    lines = out.split("\n")
    ids = [(i, session_of(line)) for i, line in enumerate(lines) if line.strip()]
    first = next((i for i, s in ids if s == sid), None)
    if first is not None and any(s and s != sid for i, s in ids if i < first):
        changes["teleported_records"] = sum(1 for i, _ in ids if i < first)
        out = "\n".join(lines[first:])
    return out, changes


def session_of(line):
    try:
        entry = json.loads(line)
    except ValueError:
        return None  # the renderer counts it as a bad line
    return entry.get("sessionId") if isinstance(entry, dict) else None


def escape_file(path):
    """Rewrite a JSON-lines file with the separators escaped (the rows parse the same); True when it changed."""
    path = Path(path)
    if not path.exists():
        return False
    text = path.read_text(encoding="utf-8")
    new = escape_separators(text)
    if new == text:
        return False
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(new, encoding="utf-8")
    tmp.replace(path)
    return True
