# shellcheck shell=bash
# Where the dataset is published, and how to fetch an archive that dataset.lock pins. Sourced by the scripts once they
# have set root and defined die, log and sha.
# A lock line is <file> <sha256> <commit> [<llm-model> [<small-model>]]: the Hugging Face commit that published the
# file, and for a store built with an LLM, that LLM and the small model when it differs from the LLM.

HF_REPO=deluccadev/bilbo-retrieval
lock=${root:?}/dataset.lock

# Print the lock's lines, without comments and blank lines.
lock_entries() {
  [[ -f $lock ]] || return 0
  local line
  while IFS= read -r line || [[ -n $line ]]; do
    line=${line%$'\r'}
    [[ -z $line || $line == '#'* ]] || printf '%s\n' "$line"
  done <"$lock"
}

# Print the rest of <file>'s lock line: <sha256> <commit> [<llm-model> [<small-model>]]. Return 1 when the lock does
# not list it.
locked() {
  local file rest
  while read -r file rest; do
    if [[ $file == "$1" ]]; then
      printf '%s\n' "$rest"
      return 0
    fi
  done < <(lock_entries)
  return 1
}

# Download <file> at its locked commit into dataset/ and check its sha256. Return 1 when the lock does not list it.
# Callers test the result, which turns errexit off in here, so every step checks itself.
fetch_archive() {
  local entry want commit out=$root/dataset/$1
  entry=$(locked "$1") || return 1
  read -r want commit _ <<<"$entry"
  [[ $want =~ ^[0-9a-f]{64}$ && $commit =~ ^[0-9a-f]{40}$ ]] || die "dataset.lock's line for $1 is malformed"
  log "downloading $1 from huggingface.co/datasets/$HF_REPO"
  mkdir -p "${out%/*}" || die "cannot create ${out%/*}"
  curl -fsSL --retry 3 -o "$out.part" "https://huggingface.co/datasets/$HF_REPO/resolve/$commit/$1" ||
    { rm -f "$out.part"; die "downloading $1 failed"; }
  local got
  got=$(sha "$out.part")
  [[ $got == "$want" ]] || { rm -f "$out.part"; die "$1 has sha256 $got, but dataset.lock says $want"; }
  mv "$out.part" "$out" || die "cannot move $1 into dataset/"
  log "$1 downloaded and checked"
}
