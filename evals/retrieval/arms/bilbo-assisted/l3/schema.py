"""Validators for every contract file of the L3 sessions eval (PLAN.md section 3.3).

Each `check_*` takes one parsed row (or document) and raises `SchemaError` naming the field; each `load_*` reads a file,
validates every row and the cross-row rules (unique ids, date order, references) and returns the rows. Every stream
calls these instead of re-implementing a contract. `python schema.py KIND FILE` validates a file from the shell.
"""

from __future__ import annotations

import json
import re
import sys
from datetime import datetime
from pathlib import Path


class SchemaError(ValueError):
    """A row or file breaks its contract."""


KINDS = ("decision", "value", "command", "location", "convention", "gotcha")
CHECKS = ("support", "current", "unique", "closed_book")
SETTLED = ("none", "decision", "fix", "finding", "plan", "value", "convention")
TRIGGERS = ("agent", "user", "compact")
ENDED = ("report", "max_steps", "error")
EDIT_OPS = ("new", "edit", "rename")
DECISIONS = ("skill", "none")
STAGES = ("smoke", "qval", "decide", "run", "capture", "query", "qa", "judge", "stress")
SPLITS = ("dev", "test")
XID = re.compile(r"^[^#\s]+#[1-9]\d*$")
SHA = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
ISO_UTC = re.compile(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?(Z|\+00:00)$")
KEY = re.compile(r"^[^\s/]+__[^\s/]+$")
LABEL = re.compile(r"^l3s/(" + "|".join(STAGES) + r")/[^/\s]+/[^/\s]+/r[1-9]\d*(/s[0-9]+)?$")


def _fail(where: str, msg: str):
    raise SchemaError(f"{where}: {msg}")


def _need(row, where: str, fields: dict) -> None:
    """`fields` maps a name to a type, a tuple of types or a callable returning an error text or None."""
    if not isinstance(row, dict):
        _fail(where, f"expected an object, got {type(row).__name__}")
    missing = [k for k in fields if k not in row]
    if missing:
        _fail(where, f"missing {', '.join(missing)}")
    extra = sorted(set(row) - set(fields))
    if extra:
        _fail(where, f"unknown field {', '.join(extra)}")
    for k, t in fields.items():
        v = row[k]
        if callable(t) and not isinstance(t, (type, tuple)):
            if err := t(v):
                _fail(where, f"{k}: {err}")
        else:
            types = t if isinstance(t, tuple) else (t,)
            if bool not in types and isinstance(v, bool) and int in types:
                _fail(where, f"{k}: expected {_names(types)}, got bool")
            if not isinstance(v, types):
                _fail(where, f"{k}: expected {_names(types)}, got {type(v).__name__}")


def _names(types) -> str:
    return "/".join("null" if t is type(None) else t.__name__ for t in types)


def _str_list(v):
    return None if isinstance(v, list) and all(isinstance(x, str) for x in v) else "expected a list of strings"


def _nonempty(v):
    return None if isinstance(v, str) and v.strip() else "expected a non-empty string"


def _enum(*allowed):
    return lambda v: None if v in allowed else f"expected one of {', '.join(allowed)}, got {v!r}"


def _opt_enum(*allowed):
    return lambda v: None if v is None or v in allowed else f"expected null or one of {', '.join(allowed)}, got {v!r}"


def _iso(v):
    if not isinstance(v, str) or not ISO_UTC.match(v):
        return "expected an ISO UTC timestamp ending in Z"
    try:
        datetime.fromisoformat(v.replace("Z", "+00:00"))
    except ValueError:
        return "not a valid timestamp"
    return None


def _xid(v):
    return None if isinstance(v, str) and XID.match(v) else "expected '<session_id>#<n>'"


def _xid_list(v):
    return None if isinstance(v, list) and all(isinstance(x, str) and XID.match(x) for x in v) else "expected a list of xids"


def _nonneg_int(v):
    return None if isinstance(v, int) and not isinstance(v, bool) and v >= 0 else "expected an integer >= 0"


def _pos_int(v):
    return None if isinstance(v, int) and not isinstance(v, bool) and v >= 1 else "expected an integer >= 1"


def _bool(v):
    return None if isinstance(v, bool) else "expected a boolean"


def _sha256(v):
    return None if isinstance(v, str) and SHA256.match(v) else "expected a sha256 hex digest"


# --- exchanges ---------------------------------------------------------------------------------------------------------


def check_exchange(row: dict, where: str = "exchange") -> None:
    _need(row, where, {
        "repo": _nonempty, "session_id": _nonempty, "s": _pos_int, "n": _pos_int, "xid": _xid, "ts": _iso, "ts_end": _iso,
        "user": str, "assistant": str, "tokens": _nonneg_int, "compact_before": _bool, "cut_chars": _nonneg_int,
    })  # fmt: skip
    if row["xid"] != f"{row['session_id']}#{row['n']}":
        _fail(where, f"xid {row['xid']!r} is not session_id#n")
    if row["ts_end"] < row["ts"]:
        _fail(where, "ts_end is before ts")


def load_exchanges(path: Path) -> list[dict]:
    """In date order; unique xids; one repo; session indexes 1.. contiguous and exchange indexes 1.. contiguous per session."""
    rows = _rows(path)
    seen, last, sessions = set(), "", {}
    for i, r in enumerate(rows, 1):
        w = f"{path.name}:{i}"
        check_exchange(r, w)
        if r["xid"] in seen:
            _fail(w, f"duplicate xid {r['xid']}")
        seen.add(r["xid"])
        if r["ts"] < last:
            _fail(w, "rows are not in date order")
        last = r["ts"]
        if r["repo"] != rows[0]["repo"]:
            _fail(w, "rows of more than one repo")
        s = sessions.setdefault(r["session_id"], {"s": r["s"], "n": 0})
        if s["s"] != r["s"] or r["n"] != s["n"] + 1:
            _fail(w, f"session {r['session_id']}: s or n out of sequence")
        s["n"] = r["n"]
    idx = sorted(v["s"] for v in sessions.values())
    if idx != list(range(1, len(idx) + 1)):
        _fail(path.name, "session indexes are not 1..N")
    return rows


# --- selection ---------------------------------------------------------------------------------------------------------


def check_selection(doc: dict, where: str = "selection") -> None:
    _need(doc, where, {
        "dataset": _nonempty, "revision": _nonempty, "rules": dict, "seed": (int, str), "eligible": _str_list, "dev": str,
        "test": _str_list, "skipped": list, "repos": dict,
    })  # fmt: skip
    for j, s in enumerate(doc["skipped"]):
        _need(s, f"{where}.skipped[{j}]", {"repo_id": _nonempty, "reason": _nonempty})
    chosen = [doc["dev"], *doc["test"]]
    if len(set(chosen)) != len(chosen):
        _fail(where, "a repo is in dev and test, or twice in test")
    for r in chosen:
        if r not in doc["repos"]:
            _fail(where, f"{r} is chosen but has no entry in repos")
        if r not in doc["eligible"]:
            _fail(where, f"{r} is chosen but not eligible")
    for rid, info in doc["repos"].items():
        w = f"{where}.repos[{rid}]"
        _need(info, w, {"key": lambda v: None if isinstance(v, str) and KEY.match(v) else "expected owner__repo",
                        "licence": _nonempty, "sessions": list})  # fmt: skip
        if not info["sessions"]:
            _fail(w, "no sessions")
        for k, s in enumerate(info["sessions"]):
            _need(s, f"{w}.sessions[{k}]", {"session_id": _nonempty, "created_at": _iso, "transcript_sha256": _sha256})


def load_selection(path: Path) -> dict:
    doc = _json(path)
    check_selection(doc, path.name)
    return doc


# --- questions ---------------------------------------------------------------------------------------------------------


def check_question(row: dict, where: str = "question") -> None:
    _need(row, where, {
        "qid": _nonempty, "repo": _nonempty, "split": _enum(*SPLITS), "text": _nonempty, "answer": str, "key_values": _str_list,
        "older_values": _str_list, "kind": _enum(*KINDS), "abstain": _bool, "evidence": _xid_list,
        "rediscovery": lambda v: None if v is None or (isinstance(v, dict) and set(v) == {"first", "later"} and _xid(v["first"]) is None and _xid(v["later"]) is None) else "expected null or {first, later} xids",
        "commit": lambda v: None if v is None or (isinstance(v, str) and SHA.match(v)) else "expected null or a 40-hex sha",
        "checks": dict, "audit": _opt_enum("pass", "fail"),
    })  # fmt: skip
    c = row["checks"]
    if set(c) != set(CHECKS) or any(c[k] not in ("pass", "fail") for k in CHECKS):
        _fail(where, f"checks: expected {', '.join(CHECKS)}, each pass or fail")
    if row["abstain"]:
        if row["key_values"] or row["older_values"] or row["evidence"] or row["rediscovery"] or row["commit"]:
            _fail(where, "a no-answer question has no key values, older values, evidence, rediscovery or commit")
    else:
        if not row["key_values"] or any(not k.strip() for k in row["key_values"]):
            _fail(where, "an answerable question needs at least one non-empty key value")
        if not row["evidence"]:
            _fail(where, "an answerable question needs evidence")
        if not row["answer"].strip():
            _fail(where, "an answerable question needs an answer")
        if set(row["key_values"]) & set(row["older_values"]):
            _fail(where, "a value is both key and older")
        if row["rediscovery"] and not {row["rediscovery"]["first"], row["rediscovery"]["later"]} <= set(row["evidence"]):
            _fail(where, "rediscovery xids are not in evidence")
    if "[REDACTED" in row["answer"] or any("[REDACTED" in k for k in row["key_values"]):
        _fail(where, "an answer or key value holds a redaction placeholder")


def load_questions(path: Path, split: str | None = None, exchanges: dict[str, set[str]] | None = None) -> list[dict]:
    """`exchanges` maps a repo to its xids; when given, every evidence xid must exist in its repo."""
    rows = _rows(path)
    seen = set()
    for i, r in enumerate(rows, 1):
        w = f"{path.name}:{i}"
        check_question(r, w)
        if r["qid"] in seen:
            _fail(w, f"duplicate qid {r['qid']}")
        seen.add(r["qid"])
        if split and r["split"] != split:
            _fail(w, f"split {r['split']} in a {split} file")
        if exchanges is not None:
            known = exchanges.get(r["repo"])
            if known is None:
                _fail(w, f"unknown repo {r['repo']}")
            ids = list(r["evidence"]) + [x for x in (r["rediscovery"] or {}).values()]
            if bad := [x for x in ids if x not in known]:
                _fail(w, f"evidence {bad[0]} is not an exchange of {r['repo']}")
    return rows


def check_units_table(doc: dict, where: str = "units") -> None:
    if not isinstance(doc, dict) or not doc:
        _fail(where, "expected a non-empty object")


# --- policy ------------------------------------------------------------------------------------------------------------


def check_label(row: dict, where: str = "label") -> None:
    _need(row, where, {
        "xid": _xid, "settled": _enum(*SETTLED), "investigation": _bool, "closed": _bool, "commit": _bool, "correction": _bool,
        "subject": lambda v: None if v is None or (isinstance(v, str) and 0 < len(v.split()) <= 8) else "expected null or at most 8 words",
    })  # fmt: skip


def load_labels(path: Path) -> list[dict]:
    rows = _rows(path)
    seen = set()
    for i, r in enumerate(rows, 1):
        check_label(r, f"{path.name}:{i}")
        if r["xid"] in seen:
            _fail(f"{path.name}:{i}", f"duplicate xid {r['xid']}")
        seen.add(r["xid"])
    return rows


def check_schedule_row(row: dict, where: str = "schedule") -> None:
    _need(row, where, {"xid": _xid, "request": _nonempty})


def load_schedule(path: Path) -> list[dict]:
    rows = _rows(path)
    seen = set()
    for i, r in enumerate(rows, 1):
        check_schedule_row(r, f"{path.name}:{i}")
        if r["xid"] in seen:
            _fail(f"{path.name}:{i}", f"duplicate xid {r['xid']}")
        seen.add(r["xid"])
    return rows


# --- adapters and writer state -----------------------------------------------------------------------------------------


def check_manifest(doc: dict, where: str = "manifest") -> None:
    _need(doc, where, {
        "tool": _nonempty, "version": _nonempty, "config_name": _nonempty, "config": dict, "render_sha": _nonempty,
        "exchanges_sha256": _sha256, "counts": dict, "failures": (int, list), "skipped": (int, list), "wall_s": (int, float),
        "done": _bool,
    })  # fmt: skip


def check_query_row(row: dict, where: str = "query row") -> None:
    _need(row, where, {"qid": _nonempty, "ms": (int, float), "units": list})
    for j, u in enumerate(row["units"]):
        _need(u, f"{where}.units[{j}]", {
            "text": str, "date": lambda v: None if v is None or (isinstance(v, str) and re.match(r"^\d{4}-\d\d-\d\d", v)) else "expected null or a YYYY-MM-DD date",
            "ref": (str, type(None)),
        })  # fmt: skip


def validate_query_rows(rows: list[dict], questions: list[dict] | None = None, name: str = "query rows") -> list[dict]:
    """Every question answered exactly once when `questions` is given."""
    for i, r in enumerate(rows, 1):
        check_query_row(r, f"{name}:{i}")
    ids = [r["qid"] for r in rows]
    if len(set(ids)) != len(ids):
        _fail(name, "a qid appears twice")
    if questions is not None and set(ids) != {q["qid"] for q in questions}:
        _fail(name, "its rows do not cover the questions exactly once")
    return rows


def load_query_rows(path: Path, questions: list[dict] | None = None) -> list[dict]:
    return validate_query_rows(_rows(path), questions, Path(path).name)


def check_done_row(row: dict, where: str = "done row") -> None:
    _need(row, where, {"xid": _xid})


def check_action(row: dict, where: str = "action") -> None:
    _need(row, where, {"xid": _xid, "trigger": _enum(*TRIGGERS), "request": (str, type(None)), "steps": list, "llm_calls": _nonneg_int,
                       "ended": _enum(*ENDED)})  # fmt: skip
    for j, s in enumerate(row["steps"]):
        _need(s, f"{where}.steps[{j}]", {"tool": _enum("Bash", "Read", "Edit"), "args": dict, "exit": (int, type(None)), "refused": _bool})


def check_edit(row: dict, where: str = "edit") -> None:
    _need(row, where, {"path": _nonempty, "xid": _xid, "ts": _iso, "op": _enum(*EDIT_OPS)})


def check_writer_decision(row: dict, where: str = "decision") -> None:
    _need(row, where, {"xid": _xid, "decision": _enum(*DECISIONS), "raw": str})


def check_label_name(name: str) -> None:
    """Endpoint label: l3s/<stage>/<arm>/<repo-key>/r<k>[/s<seed>]."""
    if not LABEL.match(name):
        _fail("label", f"{name!r} is not l3s/<stage>/<arm>/<repo-key>/r<k>[/s<seed>]")


# --- helpers -----------------------------------------------------------------------------------------------------------


def _json(path: Path):
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise SchemaError(f"{Path(path).name}: {e}") from None


def _rows(path: Path) -> list[dict]:
    out = []
    try:
        text = Path(path).read_text(encoding="utf-8")
    except OSError as e:
        raise SchemaError(f"{Path(path).name}: {e}") from None
    for i, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        try:
            out.append(json.loads(line))
        except ValueError as e:
            raise SchemaError(f"{Path(path).name}:{i}: not JSON ({e})") from None
    return out


LOADERS = {"exchanges": load_exchanges, "selection": load_selection, "questions": load_questions, "labels": load_labels,
           "schedule": load_schedule, "queries": load_query_rows}  # fmt: skip


def main(argv: list[str]) -> int:
    if len(argv) != 2 or argv[0] not in LOADERS:
        print(f"usage: schema.py {{{','.join(LOADERS)}}} FILE", file=sys.stderr)
        return 2
    try:
        LOADERS[argv[0]](Path(argv[1]))
    except SchemaError as e:
        print(f"schema: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
