"""Port of bilbo's passage splitting and embedder input (src/search/rank.rs, shared/markdown.rs)."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

PART_BYTES = 4000
INPUT_BYTES = 4000

KINDS = ("plan", "spec", "design", "decision", "gotcha", "research", "review", "report", "reference")
TOPIC = re.compile(r"[a-z0-9]+(-[a-z0-9]+)*")

# Rust's char::is_whitespace, which str::trim and split_whitespace use; Python's str.strip also takes \x1c-\x1f.
WS = "\t\n\x0b\x0c\r \x85\xa0                　"
_WS_RUN = re.compile(f"[{re.escape(WS)}]+")


@dataclass
class Passage:
    heading_path: list[str]
    text: str
    line: int


def _blank(line: str) -> bool:
    return not line.strip(WS)


def _floor(data: bytes, at: int) -> int:
    """Largest char boundary at or before `at` (str::floor_char_boundary)."""
    if at >= len(data):
        return len(data)
    while at > 0 and 0x80 <= data[at] < 0xC0:
        at -= 1
    return at


def split_lines(text: str) -> list[str]:
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    return [line[:-1] if line.endswith("\r") else line for line in lines]


def _fence_run(line: str) -> tuple[str, int, str] | None:
    stripped = line.lstrip(" ")
    if len(line) - len(stripped) > 3:
        return None
    if not stripped or stripped[0] not in "`~":
        return None
    ch = stripped[0]
    size = len(stripped) - len(stripped.lstrip(ch))
    rest = stripped[size:]
    if size >= 3 and not (ch == "`" and "`" in rest):
        return ch, size, rest
    return None


def _heading(line: str) -> tuple[int, str] | None:
    level = len(line) - len(line.lstrip("#"))
    if not 1 <= level <= 6:
        return None
    text = line[level:]
    if not text.startswith(" "):
        return None
    text = text[1:].rstrip(WS)
    if text.endswith("#"):
        stripped = text.rstrip("#")
        if not stripped or stripped[-1] in WS:
            text = stripped
    text = " ".join(part for part in _WS_RUN.split(text) if part)
    return (level, text) if text else None


def _outside_fences(lines: list[str]) -> list[int]:
    fence: tuple[str, int] | None = None
    out = []
    for i, line in enumerate(lines):
        run = _fence_run(line)
        if fence is not None:
            if run and run[0] == fence[0] and run[1] >= fence[1] and not run[2].strip(WS):
                fence = None
        elif run:
            fence = (run[0], run[1])
        else:
            out.append(i)
    return out


def _headings(lines: list[str]) -> list[tuple[int, int, str]]:
    found = []
    for i in _outside_fences(lines):
        h = _heading(lines[i])
        if h:
            found.append((i, h[0], h[1]))
    return found


def _trimmed(lines: list[str]) -> tuple[int, int] | None:
    live = [i for i, line in enumerate(lines) if not _blank(line)]
    return (live[0], live[-1] + 1) if live else None


def _parts(path: list[str], lines: list[str], first_line: int, start_line: int) -> list[Passage]:
    text = "\n".join(lines)
    if len(text.encode()) <= PART_BYTES:
        return [Passage(path, text, start_line)]

    done: list[tuple[int, str]] = []
    current: tuple[int, int] | None = None
    i = 0
    while i < len(lines):
        if _blank(lines[i]):
            i += 1
            continue
        start = i
        while i < len(lines) and not _blank(lines[i]):
            i += 1
        end = i - 1
        paragraph = "\n".join(lines[start : end + 1]).encode()
        if len(paragraph) > PART_BYTES:
            if current is not None:
                a, b = current
                done.append((first_line + a, "\n".join(lines[a : b + 1])))
                current = None
            pos = 0
            while True:
                lead = len(paragraph) - pos - len(paragraph[pos:].lstrip(b"\n"))
                skipped = paragraph[: pos + lead].count(b"\n")
                cut = _floor(paragraph[pos:], PART_BYTES) if len(paragraph) - pos > PART_BYTES else len(paragraph) - pos
                done.append((first_line + start + skipped, paragraph[pos : pos + cut].decode()))
                pos += cut
                if pos >= len(paragraph):
                    break
            continue
        if current is None:
            current = (start, end)
        elif len("\n".join(lines[current[0] : end + 1]).encode()) <= PART_BYTES:
            current = (current[0], end)
        else:
            a, b = current
            done.append((first_line + a, "\n".join(lines[a : b + 1])))
            current = (start, end)
    if current is not None:
        a, b = current
        done.append((first_line + a, "\n".join(lines[a : b + 1])))

    return [Passage(list(path), text, start_line if n == 0 else line) for n, (line, text) in enumerate(done)]


def split_body(lines: list[str], first_line: int, fallback_title: str) -> list[Passage]:
    """`lines` is a note's body, whose first line is physical line `first_line`."""
    headings = _headings(lines)
    title_at = next((h for h, (_, level, _) in enumerate(headings) if level == 1), None)
    title = headings[title_at][2] if title_at is not None else fallback_title
    stack: list[tuple[int, str]] = []
    paths: list[list[str]] = []
    for h, (_, level, text) in enumerate(headings):
        if h == title_at:
            stack.clear()
        else:
            while stack and stack[-1][0] >= level:
                stack.pop()
            stack.append((level, text))
        path = [title]
        if h != title_at:
            path.extend(t for _, t in stack)
        paths.append(path)

    found: list[Passage] = []
    preamble_end = headings[0][0] if headings else len(lines)
    span = _trimmed(lines[:preamble_end])
    if span:
        a, b = span
        found += _parts([title], lines[a:b], first_line + a, first_line + a)
    for h, (i, _, _) in enumerate(headings):
        end = headings[h + 1][0] if h + 1 < len(headings) else len(lines)
        body = lines[i + 1 : end]
        a, b = _trimmed(body) or (0, 0)
        found += _parts(paths[h], body[a:b], first_line + i + 1 + a, first_line + i)
    return found


def body_start(lines: list[str]) -> int:
    """Physical line the body starts on (note::read): after a closed frontmatter, else line 1."""
    if lines and lines[0] == "---":
        for n, line in enumerate(lines[1:], start=1):
            if line == "---":
                return n + 2
    return 1


def passages(note_file_text: str, stem: str = "") -> list[Passage]:
    """The passages of a whole note file, as search::documents::read_notes cuts them; `stem` is the file name without `.md`."""
    text = note_file_text.removeprefix("﻿")
    lines = split_lines(text)
    start = body_start(lines)
    return split_body(lines[start - 1 :], start, stem)


def embed_input(p: Passage) -> str | None:
    if not p.text:
        return None
    data = (" > ".join(p.heading_path) + "\n" + p.text).encode()
    return data[: _floor(data, INPUT_BYTES)].decode()


def _valid_name(name: str) -> bool:
    stem = name.removesuffix(".md")
    kind, _, topic = stem.partition("-")
    return name.endswith(".md") and kind in KINDS and bool(TOPIC.fullmatch(topic))


def inputs(store: Path) -> list[str]:
    """The embedder input of every passage with text, over `store/notes` in name order (repeats kept)."""
    out: list[str] = []
    notes = store / "notes"
    for path in sorted(notes.iterdir() if notes.is_dir() else [], key=lambda p: p.name):
        if path.name.startswith(".") or not path.is_file() or not _valid_name(path.name):
            continue
        text = path.read_bytes().decode("utf-8", errors="replace")
        for p in passages(text, path.name.removesuffix(".md")):
            if (item := embed_input(p)) is not None:
                out.append(item)
    return out
