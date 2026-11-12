#!/usr/bin/env bash
# Build bilbo for Linux from this checkout into arms/bilbo-next/bin/bilbo, which the arm's key hashes.
#   binary.sh
set -euo pipefail

arm=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$arm/../../../.." && pwd)
cache=${BILBO_EVALS_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/bilbo-evals}/retrieval/bilbo-next-build
# The toolchain of MODULE.bazel, on the Debian the arm's image runs.
image=rust:1.99.1-trixie@sha256:8a4e90c7946b74626488b1446f712c0fe0ed1a596cbd3d871684bc9c5433ae07

mkdir -p "$cache/target" "$cache/cargo" "$arm/bin"
start=$SECONDS
docker run --rm --user "$(id -u):$(id -g)" \
  -e HOME=/tmp -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target \
  -v "$repo":/src:ro -v "$cache":/cache -w /src "$image" \
  cargo build --release --locked -p bilbo-cli --bin bilbo
echo "binary: built in $((SECONDS - start)) s" >&2
install -m 755 "$cache/target/release/bilbo" "$arm/bin/bilbo"
shasum -a 256 "$arm/bin/bilbo" | cut -d' ' -f1
docker run --rm -v "$arm/bin/bilbo":/usr/local/bin/bilbo:ro "$image" bilbo --version
