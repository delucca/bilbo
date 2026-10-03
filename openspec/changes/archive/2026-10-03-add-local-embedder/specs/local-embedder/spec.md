# Spec Delta

## Purpose

The embedder bilbo runs itself, for users who have none: a pinned model, downloaded and verified by `setup`, served by the user's `llama-server` as a login service on `127.0.0.1`, so meaning ranking works on a fresh install.

## ADDED Requirements

### Requirement: Pinned model
The local embedder SHALL use one model: `Qwen3-Embedding-0.6B-Q8_0.gguf` from the Hugging Face repository `Qwen/Qwen3-Embedding-0.6B-GGUF` at revision `370f27d7550e0def9b39c1f16d3fbaa13aa67728`, 639,150,592 bytes, SHA-256 `06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439`. It SHALL live at `<cache>/models/Qwen3-Embedding-0.6B-Q8_0.gguf`, where `<cache>` is the cache folder the `note-index` spec resolves.

#### Scenario: A fresh download
- **WHEN** no model file exists and setup installs the local embedder
- **THEN** the file at the model path has the pinned size and hash, and the model line says `installed: <model path>`

#### Scenario: A file that is already there
- **WHEN** the model path holds a file of the pinned size
- **THEN** no download request is sent and the model line says `kept: <model path>`

#### Scenario: A corrupted download
- **WHEN** the downloaded bytes do not hash to the pinned SHA-256
- **THEN** the model line says `failed` and names the expected and actual hashes, no file exists at the model path, the partial file is deleted, and the exit code is 1

### Requirement: Download is atomic and resumable
The download SHALL write to `<model path>.part` and rename it to the model path only after the size and hash match. A `.part` file left by an interrupted run SHALL be continued with an HTTP range request. When the server ignores the range, the download SHALL start over from the first byte.

#### Scenario: An interrupted download resumes
- **WHEN** a run stopped after writing 300,000,000 bytes to the `.part` file and setup runs again
- **THEN** the request asks for bytes from 300,000,000 on, and the finished file has the pinned hash

#### Scenario: A server that ignores the range
- **WHEN** a `.part` file exists and the server answers the range request with 200 and the whole file
- **THEN** the `.part` file is truncated and rewritten from the first byte, and the finished file has the pinned hash

#### Scenario: Nothing at the model path midway
- **WHEN** a download is in progress
- **THEN** no file exists at the model path itself

### Requirement: llama-server location
The local embedder SHALL run the `llama-server` given by `--llama-server`, else the one found on PATH, at the path as given or found, without resolving links. When neither exists, setup SHALL fail with a message saying `llama-server` was not found and naming `brew install llama.cpp`, the distribution's `llama.cpp` package and `--llama-server`. bilbo SHALL NOT download or build `llama-server`.

#### Scenario: Found on PATH
- **WHEN** `llama-server` is on PATH and no `--llama-server` is given
- **THEN** the service runs that executable at its path on PATH, without resolving links

#### Scenario: Not found
- **WHEN** a user runs `bilbo setup --yes --embedder-local` with no `llama-server` on PATH
- **THEN** bilbo prints a message naming `llama-server`, `brew install llama.cpp` and `--llama-server` to stderr, exits 1, and writes nothing

### Requirement: Server service
The local embedder SHALL run as a login service: the launchd agent `io.github.delucca.bilbo.embedder` on macOS, the systemd user service `bilbo-embedder.service` on Linux. It SHALL start at login, restart when it exits, serve the pinned model under the name `qwen3-embedding-0.6b` and append its output to `<state>/bilbo/embedder.log`.

#### Scenario: macOS
- **WHEN** a user on macOS runs `bilbo setup --yes --embedder-local`
- **THEN** `~/Library/LaunchAgents/io.github.delucca.bilbo.embedder.plist` runs `llama-server` with the model path, it is loaded and set to run at load and keep alive, and the server line says `installed: 127.0.0.1:8737`

#### Scenario: Linux
- **WHEN** a user on Linux with a systemd user session runs `bilbo setup --yes --embedder-local`
- **THEN** `~/.config/systemd/user/bilbo-embedder.service` runs `llama-server` with the model path and `Restart=on-failure`, it is enabled and started, and the server line says `installed: 127.0.0.1:8737`

#### Scenario: No service manager
- **WHEN** a user on Linux without a systemd user session runs `bilbo setup --yes --embedder-local`
- **THEN** bilbo prints a message saying the local embedder needs a systemd user session to stderr, exits 1, and writes nothing

### Requirement: Loopback only
The server SHALL listen only on `127.0.0.1`, on port 8737 or the port `--embedder-port` gives. No service file SHALL bind another address.

#### Scenario: Not reachable from the network
- **WHEN** the local embedder is installed
- **THEN** its service file passes `--host 127.0.0.1`, and nothing listens on the port on any other address

#### Scenario: A chosen port
- **WHEN** a user runs `bilbo setup --yes --embedder-local --embedder-port 9100`
- **THEN** the service listens on `127.0.0.1:9100` and the config sets `embedder.url = http://127.0.0.1:9100`

### Requirement: Port already in use
Before it loads the service, setup SHALL fail when something already answers on the port and no bilbo embedder service is installed, with a message naming the port and `--embedder-port`.

#### Scenario: Another program on the port
- **WHEN** another program listens on `127.0.0.1:8737` and a user runs `bilbo setup --yes --embedder-local`
- **THEN** bilbo prints a message naming `8737` and `--embedder-port` to stderr, exits 1, and installs no service

#### Scenario: bilbo's own server on the port
- **WHEN** the local embedder is installed and running and the user runs `bilbo setup --yes --embedder-local` again
- **THEN** no message is printed about the port and the server line says `kept`

### Requirement: Readiness
After loading the service, setup SHALL wait up to 120 seconds for the server's health endpoint to report ready before the embedder check. When it is not ready in time, the server step SHALL fail with a message naming `<state>/bilbo/embedder.log`.

#### Scenario: The model loads
- **WHEN** the server reports ready after 4 seconds
- **THEN** the embedder check runs and the embedder line says `ok: 1024 dimensions`

#### Scenario: The server never comes up
- **WHEN** the server's health endpoint does not report ready within 120 seconds
- **THEN** the server line says `failed` and names the log file, the service is unloaded and deleted, the model file is kept, and the exit code is 1

### Requirement: Service follows its inputs
When the installed service names another `llama-server` path, model path or port than the plan, setup SHALL rewrite and reload it and report `updated`. When they match, it SHALL leave the file and the running server alone and report `kept`.

#### Scenario: llama-server moved
- **WHEN** the service runs `/nix/store/old/bin/llama-server` and setup resolves `/nix/store/new/bin/llama-server`
- **THEN** the service file names the new path, the service is reloaded, and the server line says `updated`

#### Scenario: A rerun
- **WHEN** the local embedder is installed and the user runs the same `bilbo setup --yes --embedder-local` again
- **THEN** the model and server lines say `kept`, the service file keeps its modification time, and no `launchctl` or `systemctl` command that changes state runs

### Requirement: Unused service is removed
When the local embedder's service is installed and the config setup leaves has no embedder, or one whose model is not `qwen3-embedding-0.6b` or whose URL is not `http://127.0.0.1:<port>`, setup SHALL unload and delete the service, keep the model file, and report `server removed: not local`. A run that was not asked for the local embedder and leaves a local config SHALL leave the service alone and report `server skipped: not asked`. When `launchctl` or `systemctl` is missing, the server line SHALL say `failed` and the service files SHALL stay.

#### Scenario: The config moves to another embedder
- **WHEN** the local embedder is installed, the config path is a link to a file setting `embedder.url = http://bagend:8081`, and a user runs `bilbo setup --yes`
- **THEN** the service file is gone and unloaded, the server line says `removed: not local`, the model file still exists, and the exit code is 0

#### Scenario: Keyword search only in the wizard
- **WHEN** the local embedder is installed and the user picks no embedder in the wizard and confirms
- **THEN** the summary names the removal of the local embedder service, the service file is gone and unloaded, and the server line says `removed: not local`

#### Scenario: A local config without the local flag
- **WHEN** the local embedder is installed, the config sets `embedder.url = http://127.0.0.1:8737` and `embedder.model = qwen3-embedding-0.6b`, and a user runs `bilbo setup --yes`
- **THEN** the service file keeps its modification time, and the model and server lines say `skipped: not asked`
