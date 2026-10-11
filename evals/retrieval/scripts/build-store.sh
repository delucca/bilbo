#!/usr/bin/env bash
# Build an arm's store from the corpus into dataset/store-<arm>-<key>.tar.zst, its services in Docker.
#   build-store.sh <arm> [--rebuild] [--sessions N]
# Needs BILBO_EVAL_LLM_URL (as the containers see it), BILBO_EVAL_LLM_KEY and BILBO_EVAL_LLM_MODEL; BILBO_EVAL_LLM_HEADERS,
# a JSON object, adds headers to every LLM call for a gateway that needs them.
# --sessions N builds from the first N sessions only: a trial, whose archive stays in the cache.
# The embedder runs on an NVIDIA GPU when Docker reports one; BILBO_EVAL_GPU=1 or 0 forces it on or off.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
[[ ${BILBO_EVAL:-} == retrieval ]] || exec nix develop "$root" --command "$0" "$@"

EMBEDDER_URL=https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/370f27d7550e0def9b39c1f16d3fbaa13aa67728/Qwen3-Embedding-0.6B-Q8_0.gguf
EMBEDDER_SHA256=06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439

die() { echo "build-store: $*" >&2; exit 1; }
log() { echo "build-store: $*" >&2; }
sha() { sha256sum "$1" | cut -d' ' -f1; }

arm=${1:-}
[[ -n $arm && -f $root/arms/$arm/compose.yaml ]] ||
  die "usage: build-store.sh <arm> [--rebuild] [--sessions N] (arms: $(cd "$root/arms" && echo *))"
shift
rebuild= sessions=
while (($#)); do
  case $1 in
    --rebuild) rebuild=1 ;;
    --sessions) [[ ${2:-} =~ ^[1-9][0-9]*$ ]] || die "--sessions needs a number"; sessions=$2; shift ;;
    *) die "unknown option $1" ;;
  esac
  shift
done
for v in BILBO_EVAL_LLM_URL BILBO_EVAL_LLM_KEY BILBO_EVAL_LLM_MODEL; do
  [[ -n ${!v:-} ]] || die "$v is not set"
done

cache=${BILBO_EVALS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/bilbo-evals}/retrieval

"$root/scripts/fetch-corpus.sh"
archive=$(echo "$root"/dataset/corpus-*.tar.zst)
corpus_sha=$(sha "$archive")
corpus=$cache/corpus-${corpus_sha:0:12}
if [[ ! -d $corpus ]]; then
  rm -rf "$corpus.part" && mkdir -p "$corpus.part"
  zstd -dc "$archive" | tar -xf - -C "$corpus.part"
  mv "$corpus.part" "$corpus"
fi

model=$cache/models/${EMBEDDER_URL##*/}
if [[ ! -f $model || $(sha "$model") != "$EMBEDDER_SHA256" ]]; then
  log "downloading ${model##*/}"
  mkdir -p "${model%/*}"
  curl -fsSL --retry 3 -o "$model.part" "$EMBEDDER_URL"
  [[ $(sha "$model.part") == "$EMBEDDER_SHA256" ]] || die "${model##*/} does not match its pinned sha256"
  mv "$model.part" "$model"
fi

# The key covers everything that shapes the store. The device does not: it changes only floating-point noise.
inputs=$(
  echo "corpus $corpus_sha"
  echo "embedder $EMBEDDER_SHA256"
  echo "llm $BILBO_EVAL_LLM_MODEL"
  echo "sessions ${sessions:-all}"
  cd "$root/arms/$arm"
  find . -mindepth 1 \( -name '.*' -o -name __pycache__ \) -prune -o -type f ! -name compose.gpu.yaml -print | LC_ALL=C sort |
    while read -r f; do echo "file ${f#./} $(sha "$f")"; done
)
key=$(printf '%s\n' "$inputs" | sha256sum | cut -c1-12)
if [[ -n $sessions ]]; then out=$cache/trials/store-$arm-$key.tar.zst; else out=$root/dataset/store-$arm-$key.tar.zst; fi
if [[ -f $out && -z $rebuild ]]; then
  log "${out#"$root"/} is already built (--rebuild builds it again)"
  exit 0
fi

work=$cache/work/$arm-$key
[[ -z $rebuild ]] || rm -rf "$work"
mkdir -p "$work"
printf '%s\n%s\n' "$key" "$inputs" > "$work/key.txt"

files=(-f "$root/arms/$arm/compose.yaml")
gpu_file=$root/arms/$arm/compose.gpu.yaml
case ${BILBO_EVAL_GPU:-auto} in
  1) [[ -f $gpu_file ]] || die "$arm has no GPU variant"; files+=(-f "$gpu_file") ;;
  0) ;;
  auto) [[ -f $gpu_file ]] && docker info 2>/dev/null | grep -i nvidia >/dev/null && files+=(-f "$gpu_file") ;;
  *) die "BILBO_EVAL_GPU must be 0 or 1" ;;
esac
device=cpu; ((${#files[@]} == 2)) || device=gpu

export BILBO_UID=$(id -u) BILBO_GID=$(id -g) BILBO_EVAL_CORPUS=$corpus/corpus BILBO_EVAL_WORK=$work
export BILBO_EVAL_EMBEDDER=$model BILBO_EVAL_DEVICE=$device BILBO_EVAL_SESSIONS=$sessions
compose() { docker-compose -p "bilbo-retrieval-$arm" --profile build "${files[@]}" "$@"; }
trap 'compose down -v --remove-orphans >/dev/null 2>&1 || true' EXIT
trap 'exit 130' INT TERM

log "building the $arm store $key on the $device"
compose up -d --wait qdrant embedder
compose run --rm --build build
compose down -v --remove-orphans

mkdir -p "${out%/*}"
# Fixed order, times and owners, as for the corpus.
tar --sort=name --mtime='1970-01-01 00:00:00Z' --owner=0 --group=0 --numeric-owner --format=posix \
  --pax-option='exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime' \
  -C "$work" -cf - store | zstd -q -19 -T0 -o "$out.part"
mv "$out.part" "$out"
rm -rf "$work"
log "${out#"$root"/} built ($(du -h "$out" | cut -f1))"
