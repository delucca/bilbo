#!/usr/bin/env bash
# The judge's gates, to pass before any arm's number is read.
#   gates.sh [stress|audit] [--jobs N]
# stress judges the gold for every answerable question (the same gold control eval.sh judges, so they share verdicts)
# and the stress contexts stress.py builds from it; it needs the gold at 0.95 or more, a dropped fact and a swapped older
# value caught in 90% of cases, and another project's gold at 0.05 or less. audit has gpt-6.1-sol, through
# `codex exec`, re-judge 150 of the Opus verdicts in the arms' runs blind, and needs Cohen's kappa of 0.75 or more over
# their items. It reads each arm's newest run in the cache, so run eval.sh for the arms first. With no word it runs
# both. Reports go to $cache/eval/{stress,audit}-*/ as JSON; a rerun repeats no judge call.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
[[ ${BILBO_EVAL:-} == retrieval ]] || exec nix develop "$root" --command "$0" "$@"

die() { echo "gates: $*" >&2; exit 1; }
log() { echo "gates: $*" >&2; }
sha() { cat "$@" | sha256sum | cut -d' ' -f1; }

do_stress=1 do_audit=1 jobs=4
while (($#)); do
  case $1 in
    stress) do_audit='' ;;
    audit) do_stress='' ;;
    --jobs) [[ ${2:-} =~ ^[1-9][0-9]*$ ]] || die "--jobs needs a number"; jobs=$2; shift ;;
    *) die "usage: gates.sh [stress|audit] [--jobs N]" ;;
  esac
  shift
done

cache=${BILBO_EVALS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/bilbo-evals}/retrieval
export UV_PROJECT_ENVIRONMENT=$cache/eval/venv UV_PYTHON_DOWNLOADS=never TIKTOKEN_CACHE_DIR=$cache/tiktoken
export BILBO_EVAL_JUDGE_CWD=$cache/eval/judge-cwd BILBO_EVAL_CODEX_HOME=$cache/eval/codex-home
mkdir -p "$BILBO_EVAL_JUDGE_CWD" "$TIKTOKEN_CACHE_DIR" "$BILBO_EVAL_CODEX_HOME"
ln -sfn "${CODEX_HOME:-$HOME/.codex}/auth.json" "$BILBO_EVAL_CODEX_HOME/auth.json"
py() { uv run --project "$root/scripts" --locked --quiet python "$@"; }

selected=$cache/eval/answerable.jsonl
grep -F '"abstain": false' "$root/questions/questions.jsonl" > "$selected"

if [[ -n $do_stress ]]; then
  archive=$("$root/scripts/fetch-corpus.sh")
  corpus_sha=$(sha "$archive")
  corpus=$cache/corpus-${corpus_sha:0:12}
  if [[ ! -d $corpus ]]; then
    rm -rf "$corpus.part" && mkdir -p "$corpus.part"
    zstd -dc "$archive" | tar -xf - -C "$corpus.part"
    mv "$corpus.part" "$corpus"
  fi
  controls_sha=$(sha "$root/scripts/controls.py")
  gold=$cache/eval/gold-${corpus_sha:0:12}-${controls_sha:0:12}
  random=$cache/eval/random-${corpus_sha:0:12}-${controls_sha:0:12}
  stress=$cache/eval/stress-${corpus_sha:0:12}-$(sha "$root/scripts/controls.py" "$root/scripts/stress.py" | cut -c1-12)
  mkdir -p "$gold" "$random" "$stress"
  py "$root/scripts/controls.py" "$corpus/corpus" "$selected" "$gold/search.jsonl" "$random/search.jsonl"
  py "$root/scripts/stress.py" "$corpus/corpus" "$selected" "$stress/search.jsonl"
  failed=''
  for d in "$gold" "$stress"; do
    py "$root/scripts/pack.py" "$d/search.jsonl" "$d/contexts-4000.jsonl" 4000
    py "$root/scripts/judge.py" "$selected" "$d/contexts-4000.jsonl" "$d/judged.jsonl" --jobs "$jobs" || failed=1
  done
  [[ -z $failed ]] || die "some contexts are not judged; run again to judge the rest"
  py "$root/scripts/gates.py" stress "$selected" "$gold" "$stress" "$stress/stress.json"
fi

if [[ -n $do_audit ]]; then
  runs=()
  for f in "$root"/arms/*/search.py; do
    arm=$(basename "$(dirname "$f")")
    key=$("$root/scripts/build-store.sh" "$arm" --key)
    run=$(ls -td "$cache/eval/$arm-$key" "$cache/eval/$arm-$key"-* 2>/dev/null | while read -r d; do
      [[ -s $d/judged.jsonl ]] && echo "$d" && break; done || true)
    if [[ -n $run ]]; then runs+=("$arm=$run"); else log "no judged run of $arm; run eval.sh $arm"; fi
  done
  ((${#runs[@]})) || die "no judged arm runs"
  log "auditing ${#runs[@]} arms: ${runs[*]##*/}"
  py "$root/scripts/gates.py" sample "$selected" "$cache/eval/audit.part" "${runs[@]}"
  audit=$cache/eval/audit-gpt-6.1-sol-$(sha "$cache/eval/audit.part" | cut -c1-12)
  mkdir -p "$audit" && mv "$cache/eval/audit.part" "$audit/contexts-4000.jsonl"
  py "$root/scripts/judge.py" "$selected" "$audit/contexts-4000.jsonl" "$audit/judged.jsonl" --jobs "$jobs" \
    --model gpt-6.1-sol || die "some contexts are not judged; run again to judge the rest"
  py "$root/scripts/gates.py" audit "$selected" "$audit" "$audit/audit.json" "${runs[@]}"
fi
