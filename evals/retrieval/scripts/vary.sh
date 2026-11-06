#!/usr/bin/env bash
# Write each answerable question's query variation into questions/questions.jsonl (vary.py says how).
#   vary.sh [--jobs N] [--keep-list] [--redo id,id,...]
# keep.py first builds every question's keep-list from the corpus into $cache/eval/keep.jsonl and prints it;
# --keep-list stops there, for review. Every model call goes to $cache/eval/variation-log.jsonl. A rerun varies only
# the rows that have no variation. --redo drops those rows' variation and makes it again with new, logged calls.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
[[ ${BILBO_EVAL:-} == retrieval ]] || exec nix develop "$root" --command "$0" "$@"

die() { echo "vary: $*" >&2; exit 1; }

jobs=4 keep_only='' redo=''
while (($#)); do
  case $1 in
    --jobs) [[ ${2:-} =~ ^[1-9][0-9]*$ ]] || die "--jobs needs a number"; jobs=$2; shift ;;
    --keep-list) keep_only=1 ;;
    --redo) [[ ${2:-} =~ ^[A-Za-z0-9_.-]+(,[A-Za-z0-9_.-]+)*$ ]] || die "--redo needs id,id"; redo=$2; shift ;;
    *) die "usage: vary.sh [--jobs N] [--keep-list] [--redo id,id,...]" ;;
  esac
  shift
done

cache=${BILBO_EVALS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/bilbo-evals}/retrieval
export UV_PROJECT_ENVIRONMENT=$cache/eval/venv UV_PYTHON_DOWNLOADS=never
export BILBO_EVAL_JUDGE_CWD=$cache/eval/judge-cwd BILBO_EVAL_CODEX_HOME=$cache/eval/codex-home
mkdir -p "$BILBO_EVAL_JUDGE_CWD" "$BILBO_EVAL_CODEX_HOME"
ln -sfn "${CODEX_HOME:-$HOME/.codex}/auth.json" "$BILBO_EVAL_CODEX_HOME/auth.json"
py() { uv run --project "$root/scripts" --locked --quiet python "$@"; }

archive=$("$root/scripts/fetch-corpus.sh")
corpus_sha=$(sha256sum "$archive" | cut -d' ' -f1)
corpus=$cache/corpus-${corpus_sha:0:12}
if [[ ! -d $corpus ]]; then
  rm -rf "$corpus.part" && mkdir -p "$corpus.part"
  zstd -dc "$archive" | tar -xf - -C "$corpus.part"
  mv "$corpus.part" "$corpus"
fi

py "$root/scripts/keep.py" "$corpus/corpus" "$root/questions/questions.jsonl" "$cache/eval/keep.jsonl"
[[ -z $keep_only ]] || exit 0
py "$root/scripts/vary.py" "$root/questions/questions.jsonl" "$root/questions/README.md" "$cache/eval/keep.jsonl" \
  "$cache/eval/variation-log.jsonl" --jobs "$jobs" ${redo:+--redo "$redo"}
