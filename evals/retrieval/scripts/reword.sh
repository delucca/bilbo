#!/usr/bin/env bash
# Reword the answerable questions into questions/reworded.jsonl for the wording test (reword.py says how).
#   reword.sh [--jobs N]
# Every model call goes to $cache/eval/reword-log.jsonl. A rerun rewords only the rows reworded.jsonl lacks.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
[[ ${BILBO_EVAL:-} == retrieval ]] || exec nix develop "$root" --command "$0" "$@"

die() { echo "reword: $*" >&2; exit 1; }

jobs=4
while (($#)); do
  case $1 in
    --jobs) [[ ${2:-} =~ ^[1-9][0-9]*$ ]] || die "--jobs needs a number"; jobs=$2; shift ;;
    *) die "usage: reword.sh [--jobs N]" ;;
  esac
  shift
done

cache=${BILBO_EVALS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/bilbo-evals}/retrieval
export UV_PROJECT_ENVIRONMENT=$cache/eval/venv UV_PYTHON_DOWNLOADS=never
export BILBO_EVAL_JUDGE_CWD=$cache/eval/judge-cwd BILBO_EVAL_CODEX_HOME=$cache/eval/codex-home
mkdir -p "$BILBO_EVAL_JUDGE_CWD" "$BILBO_EVAL_CODEX_HOME"
ln -sfn "${CODEX_HOME:-$HOME/.codex}/auth.json" "$BILBO_EVAL_CODEX_HOME/auth.json"

uv run --project "$root/scripts" --locked --quiet python "$root/scripts/reword.py" "$root/questions/questions.jsonl" \
  "$root/questions/README.md" "$root/questions/reworded.jsonl" "$cache/eval/reword-log.jsonl" --jobs "$jobs"
