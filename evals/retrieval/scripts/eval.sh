#!/usr/bin/env bash
# Score an arm's store against the answerable questions: search each one in both forms, pack what came back into 4,000
# tokens, judge it with Claude Opus 5.5 through `claude -p`, and print the scores.
#   eval.sh <arm> [--limit N] [--questions id,id,...] [--jobs N] [--variation] [--rerun LABEL]
# --questions or --limit runs a few questions; --jobs N sets the judge calls at once (default 4). Searches, contexts
# and verdicts are kept in $cache/eval/<arm>-<storekey>-<searchkey>, and the controls in
# $cache/eval/{gold,random}-<corpus>-<controlskey> (the cache is BILBO_EVALS_CACHE, as for build-store.sh), so a rerun
# repeats no search and no judge call already made. The search key hashes arms/<arm>/search.py, which the store key
# leaves out; the controls key hashes controls.py. --variation searches with each question's query variation (vary.sh)
# instead, in a run dir keyed by the variation's text too; the judge still reads the original question, so only what
# the search returns differs. --rerun LABEL searches and judges again in a run dir of its own, <run>-rerun-LABEL, and
# leaves the cached run as it is. Every row handed to a search is also appended to <run>/searched.jsonl, the exact
# text each search read. A run of either kind starts its judge cache from the cached run of the same store and search,
# so a context the search returned again is not judged again.
# Not built: the answer run, the 16,000-token run, the paired comparisons. The judge's gates are gates.sh.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
[[ ${BILBO_EVAL:-} == retrieval ]] || exec nix develop "$root" --command "$0" "$@"

die() { echo "eval: $*" >&2; exit 1; }
log() { echo "eval: $*" >&2; }
sha() { sha256sum "$1" | cut -d' ' -f1; }

arm=${1:-}
[[ -n $arm && -f $root/arms/$arm/search.py ]] ||
  die "usage: eval.sh <arm> [--limit N] [--questions id,id,...] [--jobs N] [--variation] [--rerun LABEL] (the arm needs arms/<arm>/search.py)"
shift
limit='' ids='' jobs=4 wording=original rerun=''
while (($#)); do
  case $1 in
    --limit) [[ ${2:-} =~ ^[1-9][0-9]*$ ]] || die "--limit needs a number"; limit=$2; shift ;;
    --questions) [[ ${2:-} =~ ^[A-Za-z0-9_.-]+(,[A-Za-z0-9_.-]+)*$ ]] || die "--questions needs id,id"; ids=$2; shift ;;
    --jobs) [[ ${2:-} =~ ^[1-9][0-9]*$ ]] || die "--jobs needs a number"; jobs=$2; shift ;;
    --variation) wording=variation ;;
    --rerun) [[ ${2:-} =~ ^[a-z0-9]+$ ]] || die "--rerun needs a label of lower-case letters and digits"; rerun=$2; shift ;;
    *) die "unknown option $1" ;;
  esac
  shift
done

cache=${BILBO_EVALS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/bilbo-evals}/retrieval
export UV_PROJECT_ENVIRONMENT=$cache/eval/venv UV_PYTHON_DOWNLOADS=never TIKTOKEN_CACHE_DIR=$cache/tiktoken
export BILBO_EVAL_JUDGE_CWD=$cache/eval/judge-cwd
mkdir -p "$BILBO_EVAL_JUDGE_CWD" "$TIKTOKEN_CACHE_DIR"
py() { uv run --project "$root/scripts" --locked --quiet python "$@"; }
questions() { py "$root/scripts/questions.py" "$1" "$root/questions/questions.jsonl" "${@:2}"; }

archive=$("$root/scripts/fetch-corpus.sh")
corpus_sha=$(sha "$archive")
corpus=$cache/corpus-${corpus_sha:0:12}
if [[ ! -d $corpus ]]; then
  rm -rf "$corpus.part" && mkdir -p "$corpus.part"
  zstd -dc "$archive" | tar -xf - -C "$corpus.part"
  mv "$corpus.part" "$corpus"
fi

key=$("$root/scripts/build-store.sh" "$arm" --key)
search_sha=$(sha "$root/arms/$arm/search.py")
controls_sha=$(sha "$root/scripts/controls.py")
run=$cache/eval/$arm-$key-${search_sha:0:12}
if [[ $wording == variation ]]; then
  variation_key=$(questions key)
  run=$run-variation-$variation_key
fi
[[ -z $rerun ]] || run=$run-rerun-$rerun
gold=$cache/eval/gold-${corpus_sha:0:12}-${controls_sha:0:12}
random=$cache/eval/random-${corpus_sha:0:12}-${controls_sha:0:12}
mkdir -p "$run" "$gold" "$random"

picked=()
[[ -z $ids ]] || picked+=(--ids "$ids")
[[ -z $limit ]] || picked+=(--limit "$limit")
questions flat "${picked[@]}" > "$run/selected.jsonl"
log "$(wc -l < "$run/selected.jsonl" | tr -d ' ') questions"

"$root/scripts/build-store.sh" "$arm" --salvage
touch "$run/search.jsonl"
: > "$run/todo.jsonl"
unsearched=()
while read -r line; do
  [[ $line =~ \"id\":\ \"([^\"]+)\",\ \"prompt\" ]] || die "no id in a question"
  for f in query prompt; do
    if ! grep -qF "{\"id\": \"${BASH_REMATCH[1]}\", \"form\": \"$f\"," "$run/search.jsonl"; then
      unsearched+=("${BASH_REMATCH[1]}")
      break
    fi
  done
done < "$run/selected.jsonl"
((${#unsearched[@]} == 0)) || questions flat --wording "$wording" --ids "$(IFS=,; echo "${unsearched[*]}")" > "$run/todo.jsonl"
if [[ -s $run/todo.jsonl ]]; then
  cat "$run/todo.jsonl" >> "$run/searched.jsonl"
  "$root/scripts/build-store.sh" "$arm" --search "$run/todo.jsonl" "$run/search.jsonl"
else
  log "nothing to search"
fi
rm "$run/todo.jsonl"

py "$root/scripts/controls.py" "$corpus/corpus" "$run/selected.jsonl" "$gold/search.jsonl" "$random/search.jsonl"
for d in "$run" "$gold" "$random"; do
  py "$root/scripts/pack.py" "$d/search.jsonl" "$d/contexts-4000.jsonl" 4000
done

base=$cache/eval/$arm-$key-${search_sha:0:12}
if [[ $run != "$base" && -s $base/judged.jsonl && ! -e $run/judged.jsonl ]]; then
  cp "$base/judged.jsonl" "$run/judged.jsonl"
  log "judge cache seeded from ${base##*/}"
fi

failed=''
for d in "$run" "$gold" "$random"; do
  py "$root/scripts/judge.py" "$run/selected.jsonl" "$d/contexts-4000.jsonl" "$d/judged.jsonl" --jobs "$jobs" || failed=1
done
[[ -z $failed ]] || die "some contexts are not judged; run again to judge the rest"

py "$root/scripts/score.py" "$run/selected.jsonl" "$root/questions/classes.json" "$run" "$gold" "$random"
