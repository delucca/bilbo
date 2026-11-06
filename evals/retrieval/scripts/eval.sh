#!/usr/bin/env bash
# Score an arm's store against the answerable questions: search each one in both forms, pack what came back into 4,000
# tokens, judge it with Claude Opus 5.5 through `claude -p`, and print the scores.
#   eval.sh <arm> [--limit N] [--questions id,id,...] [--jobs N] [--reworded]
# --questions or --limit runs a few questions; --jobs N sets the judge calls at once (default 4). Searches, contexts
# and verdicts are kept in $cache/eval/<arm>-<storekey>-<searchkey>, and the controls in
# $cache/eval/{gold,random}-<corpus>-<controlskey> (the cache is BILBO_EVALS_CACHE, as for build-store.sh), so a rerun
# repeats no search and no judge call already made. The search key hashes arms/<arm>/search.py, which the store key
# leaves out; the controls key hashes controls.py. --reworded searches with the prompt and query of
# questions/reworded.jsonl (reword.sh) instead, in a run dir keyed by that file too; the judge still reads the original
# question, so only what the search returns differs.
# Not built: the answer run, the 16,000-token run, the paired comparisons. The judge's gates are gates.sh.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
[[ ${BILBO_EVAL:-} == retrieval ]] || exec nix develop "$root" --command "$0" "$@"

die() { echo "eval: $*" >&2; exit 1; }
log() { echo "eval: $*" >&2; }
sha() { sha256sum "$1" | cut -d' ' -f1; }

arm=${1:-}
[[ -n $arm && -f $root/arms/$arm/search.py ]] ||
  die "usage: eval.sh <arm> [--limit N] [--questions id,id,...] [--jobs N] [--reworded] (the arm needs arms/<arm>/search.py)"
shift
limit='' ids='' jobs=4 reworded=''
while (($#)); do
  case $1 in
    --limit) [[ ${2:-} =~ ^[1-9][0-9]*$ ]] || die "--limit needs a number"; limit=$2; shift ;;
    --questions) [[ ${2:-} =~ ^[A-Za-z0-9_.-]+(,[A-Za-z0-9_.-]+)*$ ]] || die "--questions needs id,id"; ids=$2; shift ;;
    --jobs) [[ ${2:-} =~ ^[1-9][0-9]*$ ]] || die "--jobs needs a number"; jobs=$2; shift ;;
    --reworded) [[ -f $root/questions/reworded.jsonl ]] || die "no questions/reworded.jsonl; run reword.sh"; reworded=1 ;;
    *) die "unknown option $1" ;;
  esac
  shift
done

cache=${BILBO_EVALS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/bilbo-evals}/retrieval
export UV_PROJECT_ENVIRONMENT=$cache/eval/venv UV_PYTHON_DOWNLOADS=never TIKTOKEN_CACHE_DIR=$cache/tiktoken
export BILBO_EVAL_JUDGE_CWD=$cache/eval/judge-cwd
mkdir -p "$BILBO_EVAL_JUDGE_CWD" "$TIKTOKEN_CACHE_DIR"
py() { uv run --project "$root/scripts" --locked --quiet python "$@"; }

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
[[ -z $reworded ]] || run=$run-reworded-$(sha "$root/questions/reworded.jsonl" | cut -c1-12)
gold=$cache/eval/gold-${corpus_sha:0:12}-${controls_sha:0:12}
random=$cache/eval/random-${corpus_sha:0:12}-${controls_sha:0:12}
mkdir -p "$run" "$gold" "$random"

grep -F '"abstain": false' "$root/questions/questions.jsonl" > "$run/all.jsonl"
if [[ -n $ids ]]; then
  : > "$run/selected.jsonl"
  IFS=, read -ra want <<<"$ids"
  for id in "${want[@]}"; do
    grep -F "\"id\": \"$id\", \"prompt\"" "$run/all.jsonl" >> "$run/selected.jsonl" || die "$id is not an answerable question"
  done
else
  cp "$run/all.jsonl" "$run/selected.jsonl"
fi
rm "$run/all.jsonl"
if [[ -n $limit ]]; then
  head -n "$limit" "$run/selected.jsonl" > "$run/selected.part" && mv "$run/selected.part" "$run/selected.jsonl"
fi
log "$(wc -l < "$run/selected.jsonl" | tr -d ' ') questions"

touch "$run/search.jsonl"
: > "$run/todo.jsonl"
while read -r line; do
  [[ $line =~ \"id\":\ \"([^\"]+)\",\ \"prompt\" ]] || die "no id in a question"
  for f in query prompt; do
    if ! grep -qF "{\"id\": \"${BASH_REMATCH[1]}\", \"form\": \"$f\"," "$run/search.jsonl"; then
      if [[ -n $reworded ]]; then
        grep -F "\"id\": \"${BASH_REMATCH[1]}\", \"prompt\"" "$root/questions/reworded.jsonl" >> "$run/todo.jsonl" ||
          die "${BASH_REMATCH[1]} is not in questions/reworded.jsonl"
      else
        echo "$line" >> "$run/todo.jsonl"
      fi
      break
    fi
  done
done < "$run/selected.jsonl"
if [[ -s $run/todo.jsonl ]]; then
  "$root/scripts/build-store.sh" "$arm" --search "$run/todo.jsonl" "$run/search.jsonl"
else
  log "nothing to search"
fi
rm "$run/todo.jsonl"

py "$root/scripts/controls.py" "$corpus/corpus" "$run/selected.jsonl" "$gold/search.jsonl" "$random/search.jsonl"
for d in "$run" "$gold" "$random"; do
  py "$root/scripts/pack.py" "$d/search.jsonl" "$d/contexts-4000.jsonl" 4000
done

failed=''
for d in "$run" "$gold" "$random"; do
  py "$root/scripts/judge.py" "$run/selected.jsonl" "$d/contexts-4000.jsonl" "$d/judged.jsonl" --jobs "$jobs" || failed=1
done
[[ -z $failed ]] || die "some contexts are not judged; run again to judge the rest"

py "$root/scripts/score.py" "$run/selected.jsonl" "$root/questions/classes.json" "$run" "$gold" "$random"
