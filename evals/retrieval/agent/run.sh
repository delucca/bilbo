#!/usr/bin/env bash
# Check an arm's read rule against a real agent: the rule's run of the questions (eval.sh), then a Claude Code agent
# that answers each question's prompt on the arm's own memory surface under the same 4,000-token reading budget
# (drive.py), both judged the same way, then compare.py on what each read and how it scored.
#   agent/run.sh <arm> [--questions id,id,...] [--jobs N]
# The questions default to agent/questions.txt. The agent runs on CLAUDE_CODE_OAUTH_TOKEN, a `claude setup-token`
# token, with BILBO_EVAL_AGENT_MODEL (default claude-sonnet-5-5); the judge runs as eval.sh's does. The agent's rows,
# transcripts, verdicts and comparison are kept in $cache/eval/agent-<arm>-<storekey>-<agentkey>, the agent key hashing
# what runs in the container (drive.py, budget.py, skills/), the arm's search.py and the model, so a rerun repeats no
# session and no judge call already made.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
[[ ${BILBO_EVAL:-} == retrieval ]] || exec nix develop "$root" --command "$0" "$@"

# Claude Code 2.1.296, the release claude-mem's and auto-memory's images pin, checked against its signed manifest.
RELEASE=https://downloads.claude.ai/claude-code-releases/2.1.296
declare -A CLAUDE_SHA256=(
  [x86_64]=24972e3bc859fab2b46ed4c1e51f7d6130f06d3bd550811a114640de3370d0de
  [aarch64]=f1f6e96e0d8342b9dbf41d7e88255397a6a52ce3d8736ad6a4c6b59c9b62fefa
)
declare -A CLAUDE_PLATFORM=([x86_64]=linux-x64 [aarch64]=linux-arm64)

die() { echo "agent: $*" >&2; exit 1; }
log() { echo "agent: $*" >&2; }
sha() { sha256sum "$1" | cut -d' ' -f1; }

arm=${1:-}
[[ -n $arm ]] || die "usage: agent/run.sh <arm> [--questions id,id,...] [--jobs N]"
shift
ids=$(paste -sd, "$here/questions.txt") jobs=4
while (($#)); do
  case $1 in
    --questions) [[ ${2:-} =~ ^[A-Za-z0-9_.-]+(,[A-Za-z0-9_.-]+)*$ ]] || die "--questions needs id,id"; ids=$2; shift ;;
    --jobs) [[ ${2:-} =~ ^[1-9][0-9]*$ ]] || die "--jobs needs a number"; jobs=$2; shift ;;
    *) die "unknown option $1" ;;
  esac
  shift
done
[[ -n ${CLAUDE_CODE_OAUTH_TOKEN:-} ]] || die "set CLAUDE_CODE_OAUTH_TOKEN to a token from claude setup-token"
export BILBO_EVAL_AGENT_MODEL=${BILBO_EVAL_AGENT_MODEL:-claude-sonnet-5-5}

cache=${BILBO_EVALS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/bilbo-evals}/retrieval
export UV_PROJECT_ENVIRONMENT=$cache/eval/venv UV_PYTHON_DOWNLOADS=never TIKTOKEN_CACHE_DIR=$cache/tiktoken
export BILBO_EVAL_JUDGE_CWD=$cache/eval/judge-cwd
py() { uv run --project "$root/scripts" --locked --quiet python "$@"; }

arch=$(docker info --format '{{.Architecture}}')
[[ -n ${CLAUDE_SHA256[$arch]:-} ]] || die "no Claude Code build for a $arch engine"
claude=$cache/agent-claude/2.1.296/claude-$arch
if [[ ! -f $claude ]] || [[ $(sha "$claude") != "${CLAUDE_SHA256[$arch]}" ]]; then
  mkdir -p "${claude%/*}"
  curl -fsSL -o "$claude.part" "$RELEASE/${CLAUDE_PLATFORM[$arch]}/claude"
  [[ $(sha "$claude.part") == "${CLAUDE_SHA256[$arch]}" ]] || die "Claude Code's sha256 does not match"
  chmod +x "$claude.part" && mv "$claude.part" "$claude"
fi

log "the rule's run"
"$root/scripts/eval.sh" "$arm" --questions "$ids" --jobs "$jobs"

key=$("$root/scripts/build-store.sh" "$arm" --key)
search_sha=$(sha "$root/arms/$arm/search.py")
rule=$cache/eval/$arm-$key-${search_sha:0:12}
agent_key=$( (cd "$here" && find drive.py budget.py skills -type f | LC_ALL=C sort | xargs sha256sum
              echo "search $search_sha model $BILBO_EVAL_AGENT_MODEL") | sha256sum | cut -c1-12)
run=$cache/eval/agent-$arm-$key-$agent_key
mkdir -p "$run/transcripts"
cp "$rule/selected.jsonl" "$run/selected.jsonl"

touch "$run/search.jsonl"
: > "$run/todo.jsonl"
while read -r line; do
  [[ $line =~ \"id\":\ \"([^\"]+)\",\ \"prompt\" ]] || die "no id in a question"
  grep -qF "{\"id\": \"${BASH_REMATCH[1]}\", \"form\": \"query\"," "$run/search.jsonl" || echo "$line" >> "$run/todo.jsonl"
done < "$run/selected.jsonl"
if [[ -s $run/todo.jsonl ]]; then
  stage=$run/driver
  rm -rf "$stage" && mkdir -p "$stage"
  cp -R "$here/drive.py" "$here/budget.py" "$here/skills" "$stage/"
  cp "$claude" "$stage/claude"
  log "the agent's run: $(wc -l < "$run/todo.jsonl" | tr -d ' ') questions with $BILBO_EVAL_AGENT_MODEL"
  status=0
  "$root/scripts/build-store.sh" "$arm" --search "$run/todo.jsonl" "$run/search.jsonl" --driver "$stage" || status=$?
  # build-store.sh searches in $cache/check/<arm>-<key>, which the next search of the arm empties.
  [[ ! -d $cache/check/$arm-$key/agent ]] || cp "$cache/check/$arm-$key/agent/"*.jsonl "$run/transcripts/"
  rm -rf "$stage"
  ((status == 0)) || die "the agent's run failed; run again to answer the rest"
else
  log "nothing left for the agent"
fi
rm "$run/todo.jsonl"

py "$root/scripts/pack.py" "$run/search.jsonl" "$run/contexts-4000.jsonl" 4000
py "$root/scripts/judge.py" "$run/selected.jsonl" "$run/contexts-4000.jsonl" "$run/judged.jsonl" --jobs "$jobs" ||
  die "some contexts are not judged; run again to judge the rest"
py "$here/compare.py" "$arm" "$run/selected.jsonl" "$rule" "$run"
