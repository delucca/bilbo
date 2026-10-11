#!/usr/bin/env bash
# Publish archives from dataset/ to the Hugging Face dataset repo in one commit, with dataset-card.md as its card, and
# write each one's line in dataset.lock: its sha256, the commit and, for a store built with an LLM, that LLM.
#   publish.sh <file>...
# For the repo's owner: needs HF_TOKEN with write access, which hf reads from the environment. Commit dataset.lock
# afterwards. Only a corpus, or a store built from every session whose manifest matches its name, is published.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
[[ ${BILBO_EVAL:-} == retrieval ]] || exec nix develop "$root" --command "$0" "$@"

die() { echo "publish: $*" >&2; exit 1; }
log() { echo "publish: $*" >&2; }
sha() { sha256sum "$1" | cut -d' ' -f1; }
# shellcheck source=dataset.sh
source "$root/scripts/dataset.sh"

(($#)) || die "usage: publish.sh <file>... (archives in dataset/)"
[[ -f $lock ]] || die "dataset.lock is missing"
dataset=$(realpath "$root/dataset")
declare -A sums models
names=()
for arg; do
  [[ -f $arg ]] || die "$arg is not a file"
  path=$(realpath "$arg")
  name=${path##*/}
  [[ ${path%/*} == "$dataset" ]] || die "$arg is not in dataset/"
  [[ -z ${sums[$name]:-} ]] || die "$name is given twice"
  sum=$(sha "$path")
  if [[ $name =~ ^corpus-([0-9a-f]{12})\.tar\.zst$ ]]; then
    [[ $sum == "${BASH_REMATCH[1]}"* ]] || die "$name has sha256 $sum, which its name does not start"
  elif [[ $name =~ ^store-([a-z0-9_-]+)-([0-9a-f]{12})\.tar\.zst$ ]]; then
    arm=${BASH_REMATCH[1]} key=${BASH_REMATCH[2]}
    manifest=$(zstd -dc "$path" | tar -xOf - store/manifest.json) || die "$name has no store/manifest.json"
    re='"arm": "([^"]*)"'
    [[ $manifest =~ $re && ${BASH_REMATCH[1]} == "$arm" ]] || die "$name's manifest is not for the $arm arm"
    [[ $manifest == *'"sessions all"'* ]] || die "$name is a trial: only a store built from every session is published"
    re='"key": "([^"]*)"'
    [[ $manifest =~ $re && ${BASH_REMATCH[1]} == "$key" ]] || die "$name's manifest has another key"
    # An arm that uses an LLM has it in its key, so the key check below refuses a store built with another LLM.
    re='"llm ([^"[:space:]]+)"'
    if [[ $manifest =~ $re ]]; then models[$name]=${BASH_REMATCH[1]}; fi
    current=$(BILBO_EVAL_LLM_MODEL=${models[$name]:-} "$root/scripts/build-store.sh" "$arm" --key) ||
      die "cannot compute the current key of the $arm store"
    [[ $current == "$key" ]] ||
      die "$name is stale: the current corpus, embedder and $arm files give store-$arm-$current.tar.zst"
  else
    die "$name is neither corpus-<sha12>.tar.zst nor store-<arm>-<key>.tar.zst"
  fi
  sums[$name]=$sum
  names+=("$name")
done

[[ -n ${HF_TOKEN:-} ]] || die "HF_TOKEN is not set"
# hf upload creates a missing repo: a wrong HF_REPO must stop here instead.
curl -fsS -o /dev/null "https://huggingface.co/api/datasets/$HF_REPO" ||
  die "huggingface.co/datasets/$HF_REPO is not there"

# Hard links in dataset/, so nothing is copied.
stage=$(mktemp -d "$root/dataset/.publish.XXXXXX")
trap 'rm -rf "$stage"' EXIT
for name in "${names[@]}"; do ln "$root/dataset/$name" "$stage/$name"; done
cp "$root/dataset-card.md" "$stage/README.md"

log "uploading ${names[*]} to huggingface.co/datasets/$HF_REPO"
# With --format quiet, hf prints only the commit's URL. Its update check would suggest a newer hf than the shell's.
url=$(HF_HUB_DISABLE_UPDATE_CHECK=1 hf upload "$HF_REPO" "$stage" . --repo-type dataset --format quiet \
  --commit-message "Publish ${names[*]}")
commit=${url##*/commit/}
[[ $commit =~ ^[0-9a-f]{40}$ ]] || die "hf upload printed '$url', not a commit URL"

declare -A written
line() { printf '%s %s %s%s\n' "$1" "${sums[$1]}" "$commit" "${models[$1]:+ ${models[$1]}}"; }
{
  while IFS= read -r l || [[ -n $l ]]; do
    read -r first _ <<<"$l"
    if [[ -n $first && -n ${sums[$first]:-} ]]; then line "$first"; written[$first]=1; else printf '%s\n' "$l"; fi
  done <"$lock"
  for name in "${names[@]}"; do [[ -n ${written[$name]:-} ]] || line "$name"; done
} >"$lock.part"
mv "$lock.part" "$lock"
log "dataset.lock records commit $commit"

# The Hub gives a large file's sha256 as its x-linked-etag.
for name in "${names[@]}"; do
  etag=$(curl -fsSI "https://huggingface.co/datasets/$HF_REPO/resolve/$commit/$name" | tr -d '\r' |
    grep -i '^x-linked-etag:' | cut -d'"' -f2) || true
  [[ $etag == "${sums[$name]}" ]] ||
    die "$name at $commit has sha256 ${etag:-unknown}, not ${sums[$name]};" \
      "dataset.lock now records it: do not commit it until this is resolved"
done
log "published and checked; commit dataset.lock"
