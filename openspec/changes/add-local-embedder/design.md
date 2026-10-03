# Design

## Context

See proposal.md for why. Current state:

- `setup` builds a `Plan` (gather, plan, apply). The embedder check runs in the plan stage, so non-interactive mode writes nothing when it fails.
- `src/timer.rs` already writes one launchd plist or systemd unit pair as pure text, and loads and unloads it through the `Runner`. Both platforms and the "binary moved" rewrite are covered by tests with fake `launchctl` and `systemctl`.
- `src/embed.rs` is the `/v1/embeddings` client. `tests/common/mod.rs` has a fake embedder on a random `127.0.0.1` port.

Facts checked on 2026-10-03:

- **The model.** Hugging Face serves `Qwen3-Embedding-0.6B-Q8_0.gguf` at revision `370f27d7…` with `x-linked-size: 639150592` and `x-linked-etag` equal to its SHA-256, `06507c7b…e439`. That is the same file dnix pins for bagend (`sha256-BlB8e0Jo…`). Downloads redirect to a CDN.
- **llama-server.** nixpkgs 26.05 ships `llama-cpp` 9190 with `llama-server`. It has `--alias`, `--embedding`, `--pooling`, `--ctx-size`, `--batch-size`, `--ubatch-size`, `--parallel`, `--host`, `--port` and `--log-file`. Homebrew's `llama.cpp` formula installs the same binary.
- **Metal crash.** The add-embeddings report found that llama.cpp 9190 with Metal crashes (SIGTRAP) on its first batch under bagend's `-ub 512`, and that `-ub 4096` works. A cold Metal server took over 5 s on its first query.

## Goals / Non-Goals

**Goals:**
- **The same model and server settings on every machine,** so the add-embeddings measurements hold.
- **No half-installed state.** A failure leaves either nothing new, or the verified model file only.
- **Testable without downloading 639 MB or running a model.**

**Non-Goals:**
- **Health monitoring after setup.** If the server dies later, `recall` already falls back to keywords and says so on stderr.

## Decisions

### bilbo runs llama-server, not a model in-process

This was the maintainer's choice among three options:
- **Ollama,** pulling the model into an existing install. Rejected: bilbo would not control the model's quantization, pooling or batching, and Ollama unloads idle models after 5 minutes.
- **llama-server as a service (chosen).** bilbo owns the flags and the lifecycle. The process stays warm.
- **In-process inference** (candle or ort). Rejected: loading 639 MB per `recall` blows the digest hook's 1.5 s budget, and it would add a large native dependency tree.

### The user provides llama-server

bilbo looks it up on PATH or takes `--llama-server`. It never downloads it.
- Prebuilt binaries differ by OS, CPU and GPU backend (Metal, CUDA, Vulkan, CPU). The user's package manager already picks the right build, and keeps it patched.
- On macOS, a downloaded binary would be quarantined by Gatekeeper.
- Rejected alternative: download a pinned ggml-org release zip per target. That means four targets, each with backend variants, and a security update path that bilbo would own.

### The model is pinned, and lives in the cache folder

- **Pinned.** The URL names the revision, not `main`, and the size and SHA-256 are constants in `src/model.rs`. Changing the model means a bilbo release and a new measurement.
- **Location:** `<cache>/models/`, the folder `vectors.rs` already resolves.
  - Not the data folder, because the default store root is `$XDG_DATA_HOME/bilbo`. The model would sit beside `notes/` and travel with any later store sync.
  - The file can be downloaded again, which is what a cache holds.
  - Trade-off: a cache cleaner can delete it. See Risks.
- **Kept on size.** A file at the final path is trusted when it has the pinned size. Only a verified `.part` file is ever renamed there, so hashing 639 MB again on every home-manager activation would buy little.

### Download: ureq streaming, ring for SHA-256

`src/model.rs` (new; no existing module downloads files):
1. Open `<model>.part` for append, and hash its existing bytes.
2. Send `GET` with `Range: bytes=<len>-` when the length is not 0.
   - On 206, append.
   - On 200, truncate the file and start the hash over.
   - On any other status, fail and name it.
3. Stream the body into the file and the hash in 1 MB chunks, calling a progress callback `(done, total)`.
4. Check the size and the hash. On a match, `fsync` and rename into place. On a mismatch, delete the `.part` file.

Dependencies:
- **ureq** is already the HTTP client and follows redirects.
- **`ring` 0.17.14** is already in `Cargo.lock` through rustls, and is declared directly for `ring::digest::SHA256`. It adds no crate.
- Rejected: `sha2`, a new crate for something already compiled in. Also rejected: shelling out to `shasum` or `sha256sum`. They differ by platform, and neither is guaranteed on a home-manager activation PATH.

Progress:
- The wizard shows a progress bar through a new `Prompter::progress` method.
- Non-interactive mode prints one `bilbo: downloading … (639 MB) to <path>` line on stderr when a download starts. Activation logs then show why a switch is slow.

### Server flags

`<llama-server> --model <path> --alias qwen3-embedding-0.6b --embedding --pooling last --host 127.0.0.1 --port <port> --ctx-size 4096 --batch-size 4096 --ubatch-size 4096 --parallel 1`

- These are bagend's flags, with the micro-batch raised to 4096.
- With pooling, a whole input must fit in one micro-batch. bilbo's passages run up to 4,000 bytes, roughly 1,000 to 1,500 tokens. `-ub 512` would reject them, and it also hits the Metal crash.
- `--alias` makes the server answer to the model name the config sets, so `embed.rs` needs no special case.
- No API key: the server is reachable from loopback only (the `Loopback only` requirement).
- Logs go through the service's stdout and stderr redirection to `<state>/bilbo/embedder.log`, as the timer does, not through `--log-file`. That keeps one logging path.

### timer.rs grows a second job kind

The rule is to extend an existing module before adding one, so `src/timer.rs` is extended:
- `Job` gains a kind: `Periodic { minutes }` (today's timer) or `Service`, plus its label and arguments.
- **launchd:** `Service` writes `RunAtLoad` and `KeepAlive` instead of `StartInterval`.
- **systemd:** `Service` writes `bilbo-embedder.service` with `Restart=on-failure` and `WantedBy=default.target`, and has no `.timer`. Loading it is `daemon-reload` plus `enable --now`.
- `install`, `uninstall`, `current` and the cleanup on failure become per-job, keyed by label and unit name.
- The file keeps its name. Its doc line becomes "launchd and systemd jobs: the index timer and the embedder service".

### setup gains a prepare stage

**Plan stage** (writes nothing). For the local choice it:
- resolves `llama-server` and checks the platform;
- probes the port with a 1 s TCP connect, which is a conflict only when no bilbo service file exists;
- reads the model state (`kept` or `installed`) and the service state (`kept`, `updated` or `installed`);
- fixes the URL as `http://127.0.0.1:<port>` and the model as `qwen3-embedding-0.6b`.

So a missing `llama-server`, a busy port or an unsupported platform fails before any download.

**Prepare stage.** It runs after the confirmation in the wizard, and directly after the plan in non-interactive mode, and only when the model or the service is not `kept`:
1. Download the model.
2. Write and load the service.
3. Poll `GET /health` every 500 ms for up to 120 s, through a new `embed::ready`. llama-server answers 503 while it loads the model.
4. Run the usual embedder check.

On failure, it uninstalls the service and stops. Non-interactive mode exits 1. The wizard offers to retry or to continue keyword-only. When everything is already `kept`, as on a home-manager rerun, nothing is probed: the existing "config kept" rule applies.

**Apply stage.** Unchanged, plus the `model` and `server` report lines, which come from the prepare stage's outcome.

### The home-manager module

- `localEmbedder.enable` sets `settings."embedder.url"` and `."embedder.model"` with `lib.mkDefault`.
- An assertion fails when the URL differs from the local one.
- Activation adds `--embedder-local --embedder-port <port> --llama-server <path>`. `llamaServer` defaults to `lib.getExe' pkgs.llama-cpp "llama-server"`.
- The flake check evaluates one more sample with the option on.
- `setup` accepts `--embedder-local` against a managed config whose URL and model are the local ones (the modified `Managed config` requirement).

### Tests stand in for the model and the server

- **Model.** `tests/setup.rs` pre-places a sparse file of the pinned size (`File::set_len`), so the model step reports `kept` with no network. The download itself is unit-tested in `src/model.rs` against a local `TcpListener` serving a small payload. The size and hash are parameters there, so the tests cover 200, 206, a range ignored, a hash mismatch and an interrupted body.
- **Server.** The fake embedder from `tests/common/mod.rs` listens on a random port, passed with `--embedder-port`. It gains a `/health` route, and a mode that answers 503 a few times first. Fake `launchctl` and `systemctl` record the service load. A fake `llama-server` is any executable script on the temporary PATH; it is never run, because the fake service manager does not start it.
- **The real thing.** A recorded smoke run on rivendell (`smoke.md`): a real download into a temporary cache, Homebrew's or nixpkgs' `llama-server`, the real launchd agent, `bilbo index` and `bilbo recall` on a few notes, then `setup --remove`.

## Risks / Trade-offs

- [A llama.cpp update changes or drops a flag] → The flags used have been stable for a long time. A failed start shows up in the readiness step with the log path. The smoke run covers both the Homebrew and nixpkgs builds.
- [About 1 GB of memory stays in use] → The wizard says so before the user confirms. `setup --remove` frees it.
- [A cache cleaner deletes the model] → The service fails to start and launchd or systemd keeps retrying (launchd throttles to one start every 10 s). `recall` falls back to keywords with its warning. Running setup again downloads the model again. The service log names the missing file.
- [Hugging Face drops the revision, or rate-limits] → The download fails with the status and the URL. Nothing half-written stays at the model path.
- [A stalled download hangs] → ureq has no per-read stall timeout (see Open Questions). Ctrl-C and a rerun resume from the `.part` file.
- [A 639 MB download during `just switch`] → It happens once. Later activations see the model `kept`.
- [Any local process can use the server] → It listens on loopback only and serves embeddings only. A machine's own users are trusted the same way bilbo trusts its own store.

## Migration Plan

- Existing installs are untouched: no config changes unless the user picks the local choice.
- Rollback: `bilbo setup --remove` unloads the service, or in Nix, set `localEmbedder.enable = false`. Then delete `<cache>/models/` by hand.
- dnix adopts it in a later commit by enabling `programs.bilbo.localEmbedder` on rivendell and rohan.

## Open Questions

- **Download timeouts.** Which ureq 3 timeouts give a stall limit for a long body without capping the whole download? `timeout_recv_body` is a total. Confirm with the ureq docs during task 2.1. It does not change the specs.
