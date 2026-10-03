# Proposal

## Why

Meaning ranking is what makes recall good. On the legacy blind set, success@5 goes from 24/39 with keywords only to 33/39 with Qwen3-Embedding-0.6B. Today a user only gets that if they already run an embedder somewhere and know its URL. `setup` offers to point at Ollama or another server, but it never provisions one, so a fresh install ends with keywords only.

`plan-bilbo` had settled that bilbo "runs the indexer and query server, not the model". This change reverses that for one case, by the maintainer's call: when the user asks for it, bilbo runs a local embedder itself. bilbo pins the model and its settings, so every install that takes this choice gets the same vectors the measurements were made with.

## What Changes

- **A local embedder that bilbo runs.** `setup` downloads Qwen3-Embedding-0.6B (Q8_0, 639 MB) from a pinned Hugging Face revision and checks its SHA-256. It then runs `llama-server` on `127.0.0.1` as a login service: a launchd agent on macOS, a systemd user service on Linux. The service starts at login and restarts if it exits, so recall never waits for a model to load. The config points at it like any other embedder.
- **llama-server comes from the user.** bilbo finds it on PATH or takes `--llama-server <path>`. It does not download or build it. The wizard and the error messages say how to install it (`brew install llama.cpp`, the distro package, or Nix).
- **The wizard gains a choice,** "Local embedder, run by bilbo", second after "No embedder". It names the download size, the memory it keeps in use, and that the first index of a large store takes a while. The download happens only after the user confirms the summary, with a progress bar.
- **New setup flags.** `--embedder-local` does the same non-interactively. `--embedder-port <n>` (default 8737) moves the port, and `--llama-server <path>` names the binary.
- **Two new report steps,** `model` and `server`, between `key` and `embedder`.
- **`setup --remove`** also unloads and deletes the server's service. It keeps the model file and prints its path, as it does for the store.
- **The home-manager module** gains `programs.bilbo.localEmbedder.enable` and `.llamaServer`. When enabled, the module writes the local URL and model into the config and passes `--embedder-local` with nixpkgs' `llama-server`.

## Capabilities

### New Capabilities

- `local-embedder`: the embedder bilbo runs itself. It covers the pinned model, its download and verification, the `llama-server` service and its settings, readiness, logs, and what happens when the binary or the model changes.

### Modified Capabilities

- `setup`:
  - **Modes** and **Setup flags** add `--embedder-local`, `--embedder-port` and `--llama-server`.
  - **Plan before writing**: the local embedder is downloaded, started and checked only after the plan is confirmed. If that fails, nothing else is written.
  - **Step report** adds `model` and `server`.
  - **Embedder choices** adds the local choice.
  - **Embedder check** runs against the started server for the local choice.
  - **Remove** unloads the server and keeps the model.
  - **Home-manager module** adds the `localEmbedder` options.

## Non-goals

- **Obtaining llama-server.** bilbo never downloads, builds or updates it. The user's package manager owns it.
- **Choosing the model.** One model is pinned, the one the measurements used. Offering others means measuring them first, so it waits for its own change.
- **GPU and performance tuning.** The flags are fixed: the ones bagend runs, with the micro-batch raised to 4096, because llama.cpp 9190 with Metal crashes under `-ub 512`.
- **Stopping the server when idle.** It stays loaded, and so keeps about 1 GB of memory in use, so that recall and the digest hook never hit a cold start.
- **Managing Ollama.** The existing Ollama choice is unchanged: it points at a running Ollama and installs nothing.
- **Deleting the model on `--remove`.** It is kept, like the store, and its path is printed.
- **Custom request headers** for gateways such as rohan's genai-api. That is a separate change.
- **Windows,** and Linux without a systemd user session. There the local choice is unavailable, with the reason.

## Impact

- **Changed modules.**
  - `src/setup.rs`: the flags, the two new steps, a prepare stage between the confirmation and the rest of apply, and `--remove`.
  - `src/wizard.rs`: the new choice, its summary lines and the download progress.
  - `src/timer.rs`: generalized from one periodic job to a periodic job and a long-running service, both with launchd and systemd text and load and unload.
  - `src/embed.rs`: a readiness probe for the server's `/health`.
  - `src/main.rs`: USAGE lists the new setup options.
- **New module.** `src/model.rs`: the pinned model's URL, size and hash, plus a resumable streaming download to a `.part` file. The file is verified, then renamed into place.
- **Dependencies.** `ring` 0.17.14, already in the lockfile through `rustls`, is declared directly for SHA-256. No new crate.
- **Tests.**
  - `tests/setup.rs` covers the new flags and steps. The fake embedder from `tests/common/mod.rs` stands in for the server through `--embedder-port`, and a pre-placed model file of the pinned size stands in for the download.
  - `src/model.rs` tests the download, the hash check and resuming against a local `TcpListener` with a small payload.
  - A recorded smoke run on macOS does the real download and the real `llama-server`.
- **Flake.** The home-manager module gains the `localEmbedder` options, and its flake check evaluates them.
- **Docs.** `AGENTS.md` lists `src/model.rs`, the direct `ring` dependency and the generalized `timer.rs`. `README.md` describes the local choice.
- **dnix.** Once released, rivendell and rohan can enable `programs.bilbo.localEmbedder` instead of pointing at bagend. That is a dnix commit after this change ships.
