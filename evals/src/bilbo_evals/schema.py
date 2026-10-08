"""Field lists for dataset rows and run records, and a small checker for generation outputs."""

from __future__ import annotations

# A type is a name, a tuple of names (any of them), or ("enum", values).
Spec = dict[str, object]

STRATA = [
    "known-item", "paraphrase", "pt-en", "alias", "supersession",
    "multi-hop", "kind-filter", "no-answer", "library",
]
PROMPT_LABELS = ["positive", "noise", "off-topic", "near-miss"]
NOTE_KINDS = ["plan", "spec", "design", "decision", "gotcha", "research", "review", "report", "reference"]
SPLITS = ["dev", "test"]

KINDS: dict[str, Spec] = {
    "corpus": {
        "_id": "str", "title": "str", "text": "str", "metadata": "dict", "canary": "str",
    },
    "query": {
        "id": "str", "text": "str", "stratum": ("enum", STRATA), "split": ("enum", SPLITS),
        "lang": ("enum", ["en", "pt"]), "project": ("str", "null"), "family": "str",
        "gold": "list", "evidence_sets": "list", "decoys": "list",
        "kind": ("str", "null"), "fact_ids": "list", "zero_overlap": ("bool", "null"),
        "gold_heading": ("str", "null"), "gen": "dict", "canary": "str",
    },
    "prompt": {
        "id": "str", "prompt": "str", "split": ("enum", SPLITS),
        "label": ("enum", PROMPT_LABELS), "gold": "list", "project": ("str", "null"), "canary": "str",
    },
    "per_item": {
        "item": "str", "stratum": "str", "split": ("enum", SPLITS), "trial": "int",
        "ranking": ("list", "null"), "metrics": "dict", "latency_ms": ("float", "null"),
        "exit": ("int", "null"), "warnings": "list", "fallback": "bool", "error": ("str", "null"),
        "tokens_in": ("int", "null"), "tokens_out": ("int", "null"), "cost_usd": ("float", "null"),
    },
    "run": {
        "schema_version": "int", "run_id": "str", "layer": ("enum", ["L1"]), "arm": "str",
        "split": ("enum", SPLITS), "draft": "bool", "dataset": "dict", "bilbo": "dict",
        "embedder": ("dict", "null"), "bilbo_config": "dict", "parity": ("str", "null"),
        "index": ("dict", "null"), "versions": "dict", "seeds": "list", "host": "dict",
        "root": "str", "folders": "dict", "started": "str", "ended": "str", "files": "dict",
    },
}

_PY = {
    "str": (str,), "int": (int,), "float": (int, float), "bool": (bool,),
    "list": (list,), "dict": (dict,), "null": (type(None),),
    "number": (int, float), "integer": (int,), "string": (str,), "boolean": (bool,),
    "array": (list,), "object": (dict,),
}


def _is(value, name: str) -> bool:
    if name in ("int", "integer", "float", "number") and isinstance(value, bool):
        return False
    return isinstance(value, _PY[name])


def _matches(value, spec) -> str | None:
    """None when value fits spec, else a short description of what was expected."""
    if isinstance(spec, str):
        return None if _is(value, spec) else spec
    if spec[0] == "enum":
        return None if value in spec[1] else f"one of {spec[1]}"
    return None if any(_is(value, n) for n in spec) else " or ".join(spec)


def check(kind: str, row: dict) -> list[str]:
    """Problems of a row against the field list of `kind`: missing keys, wrong types, enum values."""
    if kind not in KINDS:
        raise ValueError(f"unknown row kind {kind!r}")
    problems = []
    for key, spec in KINDS[kind].items():
        if key not in row:
            problems.append(f"missing key {key!r}")
            continue
        want = _matches(row[key], spec)
        if want:
            problems.append(f"{key!r}: expected {want}, got {row[key]!r}"[:200])
    return problems


def check_output(schema: dict, value, path: str = "$") -> list[str]:
    """Problems of a generation output against the JSON schema subset type, required, properties, items, enum."""
    problems = []
    t = schema.get("type")
    if t is not None:
        names = t if isinstance(t, list) else [t]
        if not any(_is(value, n) for n in names):
            return [f"{path}: expected {' or '.join(names)}, got {type(value).__name__}"]
    if "enum" in schema and value not in schema["enum"]:
        problems.append(f"{path}: {value!r} is not one of {schema['enum']}")
    if isinstance(value, dict):
        for key in schema.get("required", []):
            if key not in value:
                problems.append(f"{path}: missing key {key!r}")
        for key, sub in schema.get("properties", {}).items():
            if key in value:
                problems += check_output(sub, value[key], f"{path}.{key}")
    if isinstance(value, list) and "items" in schema:
        for i, item in enumerate(value):
            problems += check_output(schema["items"], item, f"{path}[{i}]")
    return problems
