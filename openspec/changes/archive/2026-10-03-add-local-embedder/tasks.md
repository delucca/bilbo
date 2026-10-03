# Tasks

Run every command inside `nix develop -c <cmd>` from the repo root, with `CARGO_TARGET_DIR` set to the checkout's `target`.

## 1. Foundations

- [x] 1.1 Declare `ring = "0.17.14"` in `Cargo.toml`, the version `Cargo.lock` already holds through rustls. Update the `Stack` line in `openspec/config.yaml` and the dependency list in `AGENTS.md`. The lock gains one line, `"ring",` in bilbo's own dependencies, and no package. Verify with `cargo build --locked && test "$(git diff --no-ext-diff Cargo.lock | grep -c '^+[^+]')" = 1 && git diff --no-ext-diff Cargo.lock | grep -qx '+ "ring",'`
- [x] 1.2 Add `embed::ready(url, deadline)`: poll `GET <url>/health` every 500 ms until it answers 200 or the deadline passes, returning the last status or error. Unit-test 200 at once, 503 then 200, and a deadline that passes, against the module's existing local `TcpListener` helpers. Verify with `cargo test --locked embed::`

## 2. The model (`local-embedder`: Pinned model, Download is atomic and resumable)

- [x] 2.1 Read the ureq 3 timeout docs through `use-context7` and settle design.md's open question on a stall limit; record the answer there. Add `src/model.rs`:
  - the pinned URL (revision `370f27d7…`), size and SHA-256 as constants;
  - the model path under the cache folder;
  - the kept-on-size check;
  - the streaming download to `<path>.part` with `Range` resume, 206 append, 200 restart, SHA-256 through `ring::digest`, a progress callback, then `fsync` and rename on a match and deletion of the `.part` file on a mismatch.

  The size and hash are parameters of the download function, so the unit tests serve a small payload from a local `TcpListener` and cover: a fresh download, a resume with 206, a range answered with 200, a hash mismatch (no file at the path, `.part` deleted), an interrupted body leaving a `.part` file, and a non-2xx status naming the URL. Verify with `cargo test --locked model::`

## 3. The service (`local-embedder`: Server service, Loopback only, Service follows its inputs)

- [x] 3.1 Extend `src/timer.rs` with a job kind: `Periodic { minutes }`, which is today's timer unchanged, and `Service`. For `Service`, write a launchd plist with `RunAtLoad` and `KeepAlive`, or a systemd `bilbo-embedder.service` with `Restart=on-failure`, `RestartSec=10`, `StartLimitIntervalSec=0` and `WantedBy=default.target`, loaded with `daemon-reload`, `enable` and `restart`. Make install, uninstall, `current` and the failure cleanup per job. Keep every existing timer test green, and unit-test the service text on both platforms: the arguments from design.md, `--host 127.0.0.1`, the log redirection to `<state>/bilbo/embedder.log`, and the load and unload command sequences with a scripted `Runner`. Verify with `cargo test --locked timer::`

## 4. Setup wiring (`setup`: Modes, Setup flags, Plan before writing, Step report, Managed config, Embedder check, Remove; `local-embedder`: llama-server location, Port already in use, Readiness)

- [x] 4.1 Parse `--embedder-local`, `--embedder-port` (1024 to 65535, default 8737) and `--llama-server`, with the usage errors from the `Setup flags` delta. Add them to the answer flags and to USAGE in `src/main.rs`. Cover the new `Modes` and `Setup flags` scenarios in `tests/setup.rs`. Verify with `cargo test --locked --test setup flags`
- [x] 4.2 Plan stage for the local choice: resolve `llama-server`, check the platform, probe the port (a 1 s connect, a conflict only when no bilbo service file exists), read the model and service states, and fix the URL and model. Allow `--embedder-local` against a managed config only when its URL and model are the local ones. Cover `llama-server location`, `Port already in use`, `No service manager` (Linux) and the two new `Managed config` scenarios. Verify with `cargo test --locked --test setup local_plan`
- [x] 4.3 Prepare stage: download, write and load the service, `embed::ready` for up to 120 s, then the embedder check. On failure, uninstall the service, keep the model, and exit 1 having written nothing else. Add the `model` and `server` report lines in their order. In `tests/setup.rs`:
  - pre-place a sparse model file of the pinned size;
  - point `--embedder-port` at the fake embedder, given a `/health` route and a 503-first mode;
  - put fake `llama-server`, `launchctl` and `systemctl` on PATH.

  Cover `Step report` (both fresh runs), the `Plan before writing` local scenarios, `Readiness`, `Service follows its inputs` (a moved `llama-server`, a rerun that keeps everything) and `A chosen port`. Verify with `cargo test --locked --test setup local`
- [x] 4.4 `--remove` unloads and deletes the embedder service and reports the model as `skipped: kept <path>`, in the new line order. A run whose config ends with no local embedder unloads and deletes an installed service (`server removed: not local`); a run not asked for the local embedder that keeps a local config reports `skipped: not asked`. Cover both `Remove` scenarios that mention the order and the local embedder, and the `Unused service is removed` scenarios that run without the wizard. Verify with `cargo test --locked --test setup remove`

## 5. Wizard (`setup`: Embedder choices, Embedder check)

- [x] 5.1 Add the local choice to `src/wizard.rs`, second in the list:
  - its description: whether `llama-server` was found, and the download size;
  - the path prompt with install hints when it is missing, an empty answer going back to the list;
  - unavailable without a service manager;
  - preselected when the config already points at the local embedder;
  - summary lines for the download, the model path, the service, the memory and the first index;
  - after the confirmation, a `Prompter::progress` bar for the download;
  - the failure menu: retry or continue keyword-only, with the log path.

  Unit-test each new `Embedder choices` and `Embedder check` scenario and the wizard's `Unused service is removed` scenario with the scripted `Prompter`, including declining the summary sending no download request. Verify with `cargo test --locked wizard:: && cargo test --locked --bin bilbo setup::tests::driven`

## 6. Home-manager module and docs (`setup`: Home-manager module)

- [x] 6.1 Add `localEmbedder.enable`, `.port` and `.llamaServer` (default ``lib.getExe' pkgs.llama-cpp "llama-server"``) to `homeManagerModules.default`. It sets the URL and model with `mkDefault`, asserts against another `embedder.url`, and passes the three flags on activation. Extend the flake check with a sample that has the option on (the rendered config and the activation command) and one that must fail (`localEmbedder` with bagend's URL). Verify with `nix flake check -L`
- [x] 6.2 Update `AGENTS.md` (`src/model.rs`, `ring`, the two job kinds in `timer.rs`, the fake `llama-server` and the health route in the fakes) and `README.md` (the local choice, `--embedder-local`, `llama-server` from Homebrew, a distribution or Nix, the memory and disk it uses, `localEmbedder` in the home-manager snippet). Verify with `rg -qF src/model.rs AGENTS.md && rg -q 'embedder-local' README.md`

## 7. Integration

- [x] 7.1 Smoke-test on rivendell with a temporary `XDG_CACHE_HOME`, `XDG_STATE_HOME`, `BILBO_HOME` and `BILBO_CONFIG`, once with Homebrew's `llama-server` and once with nixpkgs':
  - `bilbo setup --yes --no-plugin --embedder-local` downloads the real model and loads the real launchd agent;
  - `launchctl print gui/$(id -u)/io.github.delucca.bilbo.embedder` shows it running;
  - `bilbo index` and `bilbo recall` work on a few notes;
  - a rerun shows `kept`;
  - the wizard path runs through `/usr/bin/expect`;
  - `bilbo setup --remove --yes` cleans up.

  Record the download time, the resident memory of `llama-server`, and the transcripts in `smoke.md`. Verify with `rg -q 'io.github.delucca.bilbo.embedder' openspec/changes/add-local-embedder/smoke.md`
- [x] 7.2 Run the full suite and the flake checks. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
