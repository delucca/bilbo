#!/usr/bin/env bash
# Get an arm's store into dataset/store-<arm>-<key>.tar.zst: the archive already there, else the one in dataset.lock,
# else build it from the corpus, its services in Docker.
#   build-store.sh <arm> [--rebuild] [--sessions N|repo:N,...] [--check] [--key]
# Building needs the LLM variables the arm's compose.yaml reads, usually BILBO_EVAL_LLM_URL (as the containers see it),
# BILBO_EVAL_LLM_KEY and BILBO_EVAL_LLM_MODEL; BILBO_EVAL_LLM_HEADERS, a JSON object, adds headers to every LLM call
# for a gateway that needs them, and BILBO_EVAL_LLM_SMALL_MODEL is the model for the lighter calls of an arm whose
# compose.yaml reads it. Without BILBO_EVAL_LLM_MODEL, the store is the one in dataset.lock built from the current
# corpus, embedder and arm files (it stops when there are several). --key prints the store's key for the current inputs
# and stops.
# An arm whose compose.yaml reads none of the LLM variables needs none of them, and its key names no LLM.
# --rebuild builds the store again. --sessions N builds from the first N sessions only (repo:N,repo:N, for an arm built
# in lanes or hindsight, the first N sessions of each repository named): a trial, whose archive stays in the cache.
# --check then restores the store into fresh services and checks it with arms/<arm>/check.py.
# The embedder runs on an NVIDIA GPU when Docker reports one; BILBO_EVAL_GPU=1 or 0 forces it on or off.
# BILBO_EVAL_EMBED_THREADS=N caps a CPU embedder at N threads, so two CPU builds can share a host.
# BILBO_EVAL_PROJECT names the Compose project (default bilbo-retrieval-<arm>), so a second build of an arm with another
# key (a trial, another model) can run beside the first without touching its containers; one key has one work dir.
# An arm whose compose.yaml has `x-lanes:` is built in lanes: each repository's sessions in a build container
# of its own, all at once against the one embedder (`build.py --lanes` names them, BILBO_EVAL_LANE tells each its own),
# then the build service runs once more with `--merge` to combine the lanes' stores into one. A lane that fails leaves
# the others running; a rerun resumes each lane from its own checkpoint.
# An arm with a `pack` service (profile `pack`) runs it after the build with the other services stopped, to dump a
# database into $work/store; one with a `restore` service (profile `restore`) runs it before a check's services start,
# to load that database back.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
[[ ${BILBO_EVAL:-} == retrieval ]] || exec nix develop "$root" --command "$0" "$@"

EMBEDDER_URL=https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/370f27d7550e0def9b39c1f16d3fbaa13aa67728/Qwen3-Embedding-0.6B-Q8_0.gguf
EMBEDDER_SHA256=06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439

die() { echo "build-store: $*" >&2; exit 1; }
log() { echo "build-store: $*" >&2; }
sha() { sha256sum "$1" | cut -d' ' -f1; }
# shellcheck source=dataset.sh
source "$root/scripts/dataset.sh"

arm=${1:-}
[[ -n $arm && -f $root/arms/$arm/compose.yaml ]] ||
  die "usage: build-store.sh <arm> [--rebuild] [--sessions N|repo:N,...] [--check] [--key] (arms: $(cd "$root/arms" && echo *))"
shift
rebuild= sessions= check= print_key=
while (($#)); do
  case $1 in
    --rebuild) rebuild=1 ;;
    --key) print_key=1 ;;
    --sessions) [[ ${2:-} =~ ^([1-9][0-9]*|[A-Za-z0-9_.-]+:[1-9][0-9]*(,[A-Za-z0-9_.-]+:[1-9][0-9]*)*)$ ]] ||
                  die "--sessions needs a number or repo:N,repo:N"
                [[ $2 != *:* || $arm == hindsight ]] || grep -q '^x-lanes:' "$root/arms/$arm/compose.yaml" ||
                  die "only an arm built in lanes takes --sessions repo:N"
                sessions=$2; shift ;;
    --check) [[ -f $root/arms/$arm/check.py ]] || die "$arm has no check.py"; check=1 ;;
    *) die "unknown option $1" ;;
  esac
  shift
done
# The containers get the LLM only through compose.yaml.
uses_llm=
grep -q BILBO_EVAL_LLM_ "$root/arms/$arm/compose.yaml" && uses_llm=1

cache=${BILBO_EVALS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/bilbo-evals}/retrieval

archive=$("$root/scripts/fetch-corpus.sh")
corpus_sha=$(sha "$archive")

# The key covers everything that shapes the store. The device does not: it changes only floating-point noise. check.py
# only reads a store. The small model counts only for an arm that reads it, and only when it is not the LLM itself.
uses_small=
grep -q BILBO_EVAL_LLM_SMALL_MODEL "$root/arms/$arm/compose.yaml" && uses_small=1
inputs_for() {
  echo "corpus $corpus_sha"
  echo "embedder $EMBEDDER_SHA256"
  [[ -z $uses_llm ]] || echo "llm $1"
  [[ -z $uses_small || -z $2 || $2 == "$1" ]] || echo "llm-small $2"
  echo "sessions ${sessions:-all}"
  cd "$root/arms/$arm"
  find . -mindepth 1 \( -name '.*' -o -name __pycache__ \) -prune -o -type f ! -name compose.gpu.yaml ! -name check.py \
    -print | LC_ALL=C sort | while read -r f; do echo "file ${f#./} $(sha "$f")"; done
}
key_of() { printf '%s\n' "$1" | sha256sum | cut -c1-12; }

# Only a build needs the LLM, and only the variables the arm's compose.yaml reads.
need_llm() {
  for v in BILBO_EVAL_LLM_URL BILBO_EVAL_LLM_KEY BILBO_EVAL_LLM_MODEL; do
    grep -q "$v" "$root/arms/$arm/compose.yaml" || continue
    [[ -n ${!v:-} ]] || die "$v is not set, and building the $arm store needs it"
  done
}

llm=${BILBO_EVAL_LLM_MODEL:-} small=${BILBO_EVAL_LLM_SMALL_MODEL:-}
if [[ -n $uses_llm && -z $llm && -z $sessions ]]; then
  # A store's lock line names its LLM and small model; it is current when the key of the current inputs with them is
  # its name.
  current=()
  while read -r file _ _ m s; do
    [[ $file == "store-$arm-"*.tar.zst && -n $m ]] || continue
    if [[ $file == "store-$arm-$(key_of "$(inputs_for "$m" "$s")").tar.zst" ]]; then
      current+=("$m${s:+ (small $s)}") llm=$m small=$s
    fi
  done < <(lock_entries)
  if ((${#current[@]} > 1)); then
    printf -v built '%s, ' "${current[@]}"
    die "dataset.lock has $arm stores from the current inputs built by ${built%, }:" \
      "set BILBO_EVAL_LLM_MODEL (and BILBO_EVAL_LLM_SMALL_MODEL) to one"
  fi
  [[ -n $llm ]] || log "dataset.lock has no $arm store built from the current inputs"
fi
if [[ -n $uses_llm && -z $llm ]]; then
  [[ -z $print_key ]] || die "--key needs BILBO_EVAL_LLM_MODEL or a $arm store in dataset.lock"
  need_llm
fi

inputs=$(inputs_for "$llm" "$small")
key=$(key_of "$inputs")
[[ -z $print_key ]] || { echo "$key"; exit 0; }
name=store-$arm-$key.tar.zst
if [[ -n $sessions ]]; then out=$cache/trials/$name; else out=$root/dataset/$name; fi

build=
if [[ -n $rebuild ]]; then
  build=1
elif [[ -f $out ]]; then
  log "${out#"$root"/} is already built (--rebuild builds it again)"
  if [[ -z $sessions ]] && entry=$(locked "$name") && read -r want _ <<<"$entry" &&
     [[ $(sha "$out") != "$want" ]]; then
    log "warning: ${out#"$root"/} is not the archive dataset.lock pins (a local build?); using it"
  fi
elif [[ -n $sessions ]] || ! fetch_archive "$name"; then
  build=1
fi
[[ -z $build || -z $uses_llm ]] || need_llm
[[ -n $build$check ]] || exit 0

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

work=$cache/work/$arm-$key
check_dir=$cache/check/$arm-$key

files=(-f "$root/arms/$arm/compose.yaml")
gpu_file=$root/arms/$arm/compose.gpu.yaml
case ${BILBO_EVAL_GPU:-auto} in
  1) [[ -f $gpu_file ]] || die "$arm has no GPU variant"; files+=(-f "$gpu_file") ;;
  0) ;;
  auto) [[ -f $gpu_file ]] && docker info 2>/dev/null | grep -i nvidia >/dev/null && files+=(-f "$gpu_file") ;;
  *) die "BILBO_EVAL_GPU must be 0 or 1" ;;
esac
device=cpu; ((${#files[@]} == 2)) || device=gpu
# Like the device, the CPU embedder's thread count changes only floating-point noise: it stays out of the key, goes in
# through a Compose override of its own, and the manifest records it with the device.
[[ -z ${BILBO_EVAL_EMBED_THREADS:-} || $BILBO_EVAL_EMBED_THREADS =~ ^[1-9][0-9]*$ ]] || die "BILBO_EVAL_EMBED_THREADS must be a number"
if [[ -n ${BILBO_EVAL_EMBED_THREADS:-} && $device == cpu ]] && grep -q '^  embedder:' "$root/arms/$arm/compose.yaml"; then
  threads_file=$cache/compose.embed-threads-$BILBO_EVAL_EMBED_THREADS.yaml
  mkdir -p "$cache"
  printf 'services:\n  embedder:\n    environment:\n      LLAMA_ARG_THREADS: "%s"\n' "$BILBO_EVAL_EMBED_THREADS" > "$threads_file.$$"
  mv "$threads_file.$$" "$threads_file"  # another build may be reading it
  files+=(-f "$threads_file")
  device+=" (embedder threads $BILBO_EVAL_EMBED_THREADS)"
fi

export BILBO_UID=$(id -u) BILBO_GID=$(id -g) BILBO_EVAL_CORPUS=$corpus/corpus BILBO_EVAL_WORK=$work
export BILBO_EVAL_EMBEDDER=$model BILBO_EVAL_DEVICE=$device BILBO_EVAL_SESSIONS=$sessions
# Compose warns about each unset variable, and a check has no LLM.
export BILBO_EVAL_LLM_URL=${BILBO_EVAL_LLM_URL:-} BILBO_EVAL_LLM_KEY=${BILBO_EVAL_LLM_KEY:-}
export BILBO_EVAL_LLM_MODEL=${BILBO_EVAL_LLM_MODEL:-}
# A check-only run gets its own project: check.py empties the Qdrant it reaches, and the trap takes the project down.
project=${BILBO_EVAL_PROJECT:-bilbo-retrieval-$arm}
[[ -n $build ]] || project+=-check
services() { docker-compose -p "$project" ${1:+--profile "$1"} "${files[@]}" config --services; }
compose() { docker-compose -p "$project" --profile build "${files[@]}" "$@"; }
# Every service without a profile: the arm's database and the embedder. With none, `up` would start the build service
# itself, since compose() always selects the build profile.
start() { local names; names=$(services); [[ -z $names ]] || compose up -d --wait $names; }
trap 'compose down -v --remove-orphans >/dev/null 2>&1 || true; rm -rf "$check_dir"' EXIT
trap 'exit 130' INT TERM

if [[ -n $build ]]; then
  [[ -z $rebuild ]] || rm -rf "$work"
  mkdir -p "$work"
  printf '%s\n%s\n' "$key" "$inputs" > "$work/key.txt"

  log "building the $arm store $key on the $device"
  start
  if grep -q '^x-lanes:' "$root/arms/$arm/compose.yaml"; then
    compose build -q build
    named=$(compose run --rm --no-deps -T build --lanes) || die "build.py --lanes failed"
    mapfile -t lanes <<<"$named"
    [[ -n ${lanes[0]:-} ]] || die "$arm has no lanes to build"
    log "building ${#lanes[@]} lanes at once: ${lanes[*]}"
    pids=()
    for lane in "${lanes[@]}"; do
      # Each lane's lines carry its name; a lane that fails says so and leaves the others running.
      (
        compose run --rm --no-deps -T -e BILBO_EVAL_LANE="$lane" build 2>&1 | sed -u "s/^/[$lane] /" && exit 0
        log "the lane $lane failed; the other lanes go on"
        exit 1
      ) &
      pids+=($!)
    done
    failed=()
    for i in "${!pids[@]}"; do wait "${pids[$i]}" || failed+=("${lanes[$i]}"); done
    ((${#failed[@]} == 0)) || die "lanes failed: ${failed[*]}; run again to resume them from their checkpoints"
    compose run --rm --no-deps -T build --merge
  else
    compose run --rm --build build
  fi
  if services pack | grep -x pack >/dev/null; then
    compose stop
    compose --profile pack run --rm --no-deps pack
  fi
  compose down -v --remove-orphans

  mkdir -p "${out%/*}"
  # Fixed order, times and owners, as for the corpus.
  tar --sort=name --mtime='1970-01-01 00:00:00Z' --owner=0 --group=0 --numeric-owner --format=posix \
    --pax-option='exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime' \
    -C "$work" -cf - store | zstd -q -19 -T0 -o "$out.part"
  mv "$out.part" "$out"
  # With --check, the work dir goes only once the store passes: a lanes build that fails its check keeps its lanes, so
  # removing the archive and running again merges them again without replaying them.
  [[ -n $check ]] || rm -rf "$work"
  log "${out#"$root"/} built ($(du -h "$out" | cut -f1))"
fi

if [[ -n $check ]]; then
  rm -rf "$check_dir" && mkdir -p "$check_dir"
  zstd -dc "$out" | tar -xf - -C "$check_dir"
  cp "$root/arms/$arm/check.py" "$check_dir/check.py"
  export BILBO_EVAL_WORK=$check_dir

  log "checking ${out#"$root"/} on the $device"
  if services restore | grep -x restore >/dev/null; then compose --profile restore run --rm --no-deps restore; fi
  start
  compose run --rm --build --entrypoint python build /work/check.py ||
    die "${out#"$root"/} failed its check"
  compose down -v --remove-orphans
  [[ -z $build ]] || rm -rf "$work"
fi
