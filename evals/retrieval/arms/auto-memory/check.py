"""Check an auto-memory store unpacked in /work/store. As packed: the manifest opens as the other arms' do, every
project's memory directory holds exactly the files and sha256 the manifest lists, MEMORY.md's size, links and the topic
files' frontmatter agree with the manifest's own numbers, the per-point records (init, closing prompt, recorded writes,
usage) agree with its counts, and no file carries a credential. It needs no LLM and no service.
build-store.sh --check runs it in the build service."""

import hashlib
import json
import re
import sys
from collections import Counter
from pathlib import Path

STORE = Path("/work/store")
VERSION = "2.1.296"
manifest = json.loads((STORE / "manifest.json").read_text())
failed = []


def check(name, ok, detail=""):
    print(f"{name}: {'ok' if ok else 'FAILED'}{' ' + str(detail) if detail else ''}")
    if not ok:
        failed.append(name)


def digests(root):
    return {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(root.rglob("*")) if p.is_file()} if root.is_dir() else {}


def stats(directory):
    index = directory / "MEMORY.md"
    text = index.read_text(errors="replace") if index.exists() else ""
    links = [m for m in re.findall(r"\]\(([^)\s]+\.md)\)", text) if "://" not in m]
    topics = [f for f in directory.rglob("*.md") if f.name != "MEMORY.md"] if directory.is_dir() else []
    framed = 0
    for f in topics:
        head = f.read_text(errors="replace")
        meta = head.split("---", 2)[1] if head.startswith("---") and head.count("---") >= 2 else ""
        framed += all(re.search(rf"^\s*{k}:", meta, re.M) for k in ("name", "description", "type"))
    files = [f for f in directory.rglob("*") if f.is_file()] if directory.is_dir() else []
    return {"memory_files": len(files), "bytes": sum(f.stat().st_size for f in files),
            "memory_md": {"lines": len(text.splitlines()), "chars": len(text), "over_limit": len(text.splitlines()) > 200 or len(text) > 25000,
                          "dangling_links": sum(not (directory / m).is_file() for m in links)},
            "topic_files": {"with_frontmatter": framed, "without": len(topics) - framed}}


head = list(manifest)[:5]
check("manifest opens with arm, key, inputs, device, versions", head == ["arm", "key", "inputs", "device", "versions"], head)
check("manifest names this arm and version", manifest["arm"] == "auto-memory" and manifest["versions"]["claude-code"] == VERSION,
      manifest["versions"]["claude-code"])
check("inputs name the sessions and the LLM", any(i.startswith("sessions ") for i in manifest["inputs"])
      and any(i.startswith("llm ") for i in manifest["inputs"]))

keys = sorted(k for found in manifest["projects"].values() for k in found)
on_disk = sorted(p.name for p in (STORE / "memory").iterdir()) if (STORE / "memory").is_dir() else []
check("memory directories are the manifest's projects", on_disk == keys and sorted(manifest["memory_files"]) == keys, on_disk)
for repo, found in manifest["projects"].items():
    for key, listed in found.items():
        directory = STORE / "memory" / key
        seen = digests(directory)
        check(f"{repo} {key}: files and sha256 match the manifest", seen == manifest["memory_files"].get(key), f"{len(seen)} files")
        counted = stats(directory)
        check(f"{repo} {key}: MEMORY.md, links and frontmatter match the manifest",
              all(listed[k] == v for k, v in counted.items()), f"MEMORY.md {counted['memory_md']}, topics {counted['topic_files']}")

lines = (STORE / "points.jsonl").read_text().splitlines()
records = [json.loads(line) for line in lines]
check("one record per replayed point", len(records) == manifest["points"]["replayed"] == len({r["id"] for r in records}), len(records))
check("kinds match the manifest", +Counter(r["kind"] for r in records)
      == +Counter({"end": manifest["points"]["at_session_end"], "compact": manifest["points"]["before_compaction"]}))
check("failed and cut-off points match the manifest", (sum(r["status"] == "failed" for r in records), sum(r["status"] == "cut_off" for r in records))
      == (manifest["points"]["failed"], manifest["points"]["cut_off"]))
check("replayed plus skipped is planned", manifest["points"]["replayed"] + sum(manifest["points"]["skipped"].values()) == manifest["points"]["planned"])
done = [r for r in records if r["status"] != "failed"]
check("every point's init: memory dir, model and version", all(
    ((r["init"].get("memory_paths") or {}).get("auto") or "").rstrip("/") == f"/work/claude/projects/{r['key']}/memory"
    and r["init"].get("model") == manifest["llm"]["model"] and r["init"].get("claude_code_version") == VERSION
    and not r["init"].get("analytics_disabled") for r in done))


check("every point: the closing prompt came first, no continue message, the transcript copy not rewritten",
      all(r["prompt_first"] and not r["continue_injected"] and not r["copy_rewritten"] for r in done))
check("one credential across the build", len(manifest["auth"]["credentials_seen"]) <= 1, manifest["auth"]["credentials_seen"])
rec = manifest["recorded"]
check("recorded writes: applied and skipped match the records", (rec["applied"]["write"], rec["applied"]["edit"], rec["applied"]["bash"], rec["applied"]["recorded_failed"])
      == tuple(sum(r["recorded"][k] for r in records) for k in ("write", "edit", "bash", "recorded_failed"))
      and rec["skipped"] == dict(sum((Counter(r["recorded"]["skipped"]) for r in records), Counter())))
APPLY_TIME = ("edit_file_unknown", "edit_target_missing", "edit_ambiguous", "bash_unknown_file", "bash_replace_target_missing",
              "write_bad_path", "edit_bad_path", "bash_bad_path", "after_last_point")
planned = {"write": sum(v["writes"] for v in rec["in_plan"].values()), "edit": sum(v["edits"] for v in rec["in_plan"].values()),
           "bash": sum(v["bash"] for v in rec["in_plan"].values())}
met = {k: rec["applied"][k] + sum(n for r, n in rec["skipped"].items() if r in APPLY_TIME and r.split("_")[0] == k) for k in planned}
check("recorded writes: applied plus apply-time skips is the plan's count, per kind",
      met == planned if not rec["skipped"].get("after_last_point") else sum(met.values()) + rec["skipped"]["after_last_point"] == sum(planned.values()),
      {"planned": planned, "met": met})
check("every recorded file a point wrote is in the store or deleted later",
      all(f"{r['key']}/{f}" in {f"{k}/{x}" for k, x in {(key, f) for key, files in manifest["memory_files"].items() for f in files}}
          | set(manifest["deleted_later"]) for r in records for f in r["recorded_files"]))
check("compacted_on_resume sums to the manifest's", sum(r["compacted_on_resume"] for r in records) == manifest["context"]["compacted_on_resume"])


def total(name):
    return sum(r.get(name) or 0 for r in records)


e, m = manifest["extraction"], manifest["main_model"]
check("extraction totals match the records", (e["started"], e["finished"], e["files_written"], e["nothing_to_save"], e["errors"], e["cut_off"], e["denied_tool_calls"],
                                             e["memory_saved_stream"], e["memory_saved_transcript"])
      == tuple(total(k) for k in ("started", "finished", "files_written", "nothing", "errors", "cut_off", "denied", "memory_saved_stream", "memory_saved_transcript"))
      and e["memories_saved_debug"] == sum(len(r["saved_paths"]) for r in records)
      and e["skipped"] == {k: total("skipped_" + k) for k in ("direct_write", "no_prose", "rule")})
check("main model totals match the records", (m["memory_writes"], m["memory_edits"], m["turns"], m["num_turns"], m["permission_denials"], m["thinking_strips"], m["usage_limit_notes"])
      == tuple(total(k) for k in ("main_writes", "main_edits", "turns", "num_turns", "permission_denials", "strips", "usage_limit_notes"))
      and m["closing_turn_preempted"] == sum(bool(r["closing_turn_preempted"]) for r in records))
check("the extraction ran or reported on at least one point when the flags say it runs",
      not manifest["flags"]["extraction_under_p"] or not records or e["started"] + sum(e["skipped"].values()) + e["errors"] > 0)
tokens = Counter()
for r in records:
    for k, v in (r.get("usage") or {}).items():
        tokens[k] += v if isinstance(v, int) else 0
llm = manifest["llm"]
check("LLM usage is the records' plus the warm-ups'", all(llm[k] == tokens[k] + (llm["warmup"].get(k) or 0) for k in
      ("input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"))
      and llm["calls"] == len(records) + llm["warmup"].get("calls", 0)
      and llm["extraction"] == {"cache_read_input_tokens": sum(r["extraction_usage"]["cache_read"] for r in records),
                                "cache_creation_input_tokens": sum(r["extraction_usage"]["cache_create"] for r in records),
                                "input_tokens": sum(r["extraction_usage"]["input"] for r in records)})
listed = {(key, f) for key, files in manifest["memory_files"].items() for f in files}
check("every file a point changed is in the store or listed as deleted later",
      all(f"{r['key']}/{f}" in {f"{k}/{x}" for k, x in listed} | set(manifest["deleted_later"]) for r in records for f in r["files"]))
check("a file listed as deleted later is one a later record removed",
      set(manifest["deleted_later"]) <= {f"{r['key']}/{x}" for r in records for x in r["removed"]})
check("no file in the store carries a credential", not [f.name for f in STORE.rglob("*") if f.is_file()
                                                       and re.search(rb"sk-ant-[A-Za-z0-9_-]{20,}|oat01-[A-Za-z0-9_-]{20,}", f.read_bytes())])

print("FAILED" if failed else "OK")
sys.exit(1 if failed else 0)
