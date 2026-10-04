# Spec Delta

## ADDED Requirements

### Requirement: Sync step
For each scope whose `sync` is a URL, the `sync` step SHALL check that the device has a key, that the watcher is wanted and that the folder exists and is writable, and report `ok: <name> through <url> (<n> notes)`, scopes joined by `, `. A failed check SHALL report `failed: sync needs the watcher; drop --no-watch` or `failed: <url> is not reachable: <reason>`. A URL whose scheme this bilbo cannot sync through SHALL report `failed: <scheme> transports are not supported yet; use a file:// folder`. With no device key it SHALL report `skipped: no device key; run bilbo device init in a terminal`, and with no syncing scope `skipped: no scope syncs`. It SHALL write nothing.

#### Scenario: Nothing syncs
- **WHEN** a user runs `bilbo setup --yes` with a config that declares no syncing scope
- **THEN** the sync line says `skipped: no scope syncs`

#### Scenario: A scope syncs
- **WHEN** the config holds `scope.personal.sync = file:///srv/bilbo`, the device has a key, 12 notes have `scope: personal` and `/srv/bilbo` exists
- **THEN** the sync line says `ok: personal through file:///srv/bilbo (12 notes)` and the exit code is 0

#### Scenario: No device key
- **WHEN** the config syncs `personal` and the device has no key, as on a first home-manager activation, and a user runs `bilbo setup --yes`
- **THEN** the sync line says `skipped: no device key; run bilbo device init in a terminal`, and the step does not make the exit code 1

#### Scenario: Sync without the watcher
- **WHEN** the config syncs `personal` and a user runs `bilbo setup --yes --no-watch`
- **THEN** the sync line says `failed: sync needs the watcher; drop --no-watch` and the exit code is 1

#### Scenario: A scope on a relay
- **WHEN** the config holds `scope.personal.sync = https://relay.example` and this bilbo has no client for it
- **THEN** the sync line says `failed: https transports are not supported yet; use a file:// folder` and the exit code is 1

#### Scenario: A missing folder
- **WHEN** the config holds `scope.personal.sync = file:///Volumes/usb/bilbo` and that volume is not mounted
- **THEN** the sync line says `failed`, names the URL, no folder is created, and the exit code is 1

### Requirement: Turning sync on in the wizard
After the watcher's question, the wizard SHALL ask whether to sync notes between devices, yes by default only when a scope already syncs. On yes, it SHALL ask for the scope (the declared ones and `personal`) and the folder (absolute or starting with `~/`, its parent existing), then, when the device has no key, ask `Do you already have a recovery phrase from another device?` and run the ceremony `bilbo device recover` runs on yes, or `bilbo device init` on no, under the same terminal, `CLAUDECODE` and `CODEX_THREAD_ID` rules. Against a managed config it SHALL only show the syncing scopes.

#### Scenario: First device
- **WHEN** a user with no device key answers yes, picks `personal`, enters `~/Dropbox/bilbo`, writes down the new phrase, types its 3 words back and confirms
- **THEN** the config sets `scope.personal.sync = file:///Users/a/Dropbox/bilbo`, the folder exists, a device key exists, and the sync line says `ok: personal through file:///Users/a/Dropbox/bilbo (0 notes)`

#### Scenario: Second device
- **WHEN** a user on a second machine answers yes, picks `personal`, enters the same synced folder and types the first device's phrase
- **THEN** the config and the key are written, and once the watcher polls, the scope's manifest lists both devices

#### Scenario: Enrolled, and the folder holds a scope it cannot open
- **WHEN** a device that holds keys, is in no scope on the folder and has no manifest of `personal` turns sync on for `personal`, and the folder holds a scope of the same owner that this device cannot open
- **THEN** the wizard mints no scope id, says `the folder holds scopes of this owner that this device cannot open; run bilbo device recover on this device`, and the sync line says `skipped`

#### Scenario: A folder whose parent is missing
- **WHEN** the user enters `/nope/bilbo` and `/nope` does not exist
- **THEN** the wizard says the parent folder does not exist and asks again

#### Scenario: Under Claude Code
- **WHEN** `CLAUDECODE=1` or `CODEX_THREAD_ID` is set and the device has no key, and the user answers yes
- **THEN** the wizard says the phrase is shown only in a terminal outside an agent, turns sync off for this run, and the sync line says `skipped: no device key; run bilbo device init in a terminal`

#### Scenario: Declining sync
- **WHEN** the user answers no
- **THEN** the config gains no sync setting and the sync line says `skipped: no scope syncs`

### Requirement: Applying sync from the wizard
After the summary is confirmed, and only then, the wizard SHALL write `scope.<name>.sync = file://<folder>` to the config, create the folder's last component when missing, copy into the store only the folder's manifest of the phrase's owner whose sealed name is the picked scope, and write the keys and manifests as `bilbo device init` or `recover` does, so a second device joins that scope and no other. It SHALL mint a scope id only when the folder holds no scope with the picked name and, unless this device is in a scope on the folder, no scope of this owner that the keys in hand cannot open.

#### Scenario: The key waits for the confirmation
- **WHEN** a user picks the folder `~/Sync/bilbo`, confirms a new phrase and then presses Ctrl-C at the summary
- **THEN** no device key, no `~/Sync/bilbo` and no config change exist

#### Scenario: Only the picked scope
- **WHEN** the folder holds this owner's `personal` and `shared`, and the user picks `personal` and types the phrase
- **THEN** only `personal`'s manifest is in the store, `bilbo device` lists this device in `personal` only, and no version of `shared` lists it

## MODIFIED Requirements

### Requirement: Plan before writing
`setup` SHALL settle every answer, read the current state and run the embedder check before it creates, changes or deletes any file. The one exception is the local embedder: its download, its service and its embedder check SHALL run after the plan is settled (in the wizard, after the confirmation) and before any other step writes. When one of them fails, setup SHALL unload and delete the service it installed, keep the model file, write nothing else and exit 1. The wizard SHALL then show a summary of the actions and ask for one confirmation. Declining, pressing Ctrl-C or Esc at any prompt, or reaching end of input SHALL exit 1 and write nothing. A recovery phrase the wizard shows before the summary SHALL NOT produce a key file until the summary is confirmed.

#### Scenario: The summary lists the actions
- **WHEN** a user answers every wizard prompt on a machine with no store, no config, and `claude` on PATH
- **THEN** the wizard shows a summary naming the store folder, the config file, the Claude Code plugin, the timer, the watcher and, when the user turned it on, the scope that syncs and where it syncs, before asking to apply

#### Scenario: Declining writes nothing
- **WHEN** a user answers every prompt and then declines the summary
- **THEN** bilbo exits 1, and no store folder, config, key file, timer file or watcher file exists and no agent command ran

#### Scenario: Declining after a recovery phrase
- **WHEN** a user turns sync on in the wizard, is shown a new recovery phrase, types the 3 words back and then declines the summary
- **THEN** bilbo exits 1, no device key exists under `<state>/bilbo/keys/`, and the config and any folder the user entered are unchanged

#### Scenario: Ctrl-C midway writes nothing
- **WHEN** a user presses Ctrl-C at the key prompt
- **THEN** bilbo exits 1 and writes nothing

#### Scenario: Declining the local embedder downloads nothing
- **WHEN** a user picks the local embedder and declines the summary
- **THEN** bilbo exits 1, no download request was sent, and no model file, `.part` file or service file exists

#### Scenario: A failed local check writes nothing else
- **WHEN** a user runs `bilbo setup --yes --embedder-local` on a machine with no store and no config, and the server answers the embed request with 500
- **THEN** bilbo prints a message naming the URL and the status 500 to stderr, exits 1, the model file exists, and no service file, store folder or config exists

### Requirement: Step report
After applying, `setup` SHALL print to stdout one line per step, in the order `store`, `config`, `key`, `model`, `server`, `embedder`, `claude`, `codex`, `hook`, `timer`, `watch`, `sync`, each line being `<step> <status>` optionally followed by `: <detail>`. The status SHALL be one of `created`, `written`, `kept`, `ok`, `installed`, `updated`, `removed`, `skipped` or `failed`. The exit code SHALL be 0 when no step failed and 1 otherwise. A failed step SHALL NOT stop the steps after it.

#### Scenario: A fresh non-interactive run
- **WHEN** a user runs `bilbo setup --yes --no-plugin` on a machine with no store and no config
- **THEN** stdout holds the lines `store created: <root>/notes`, `config written: <config path>`, `key skipped: no embedder`, `model skipped: not local`, `server skipped: not local`, `embedder skipped: none configured`, `claude skipped: --no-plugin`, `codex skipped: --no-plugin`, `hook skipped: no codex plugin`, `timer skipped: no embedder`, `watch installed: watching <root>/notes` and `sync skipped: no scope syncs`, in that order, and the exit code is 0

#### Scenario: A fresh local run
- **WHEN** a user on macOS runs `bilbo setup --yes --no-plugin --embedder-local` on a machine with no store, no config and no model file
- **THEN** stdout holds `store created: <root>/notes`, `config written: <config path>`, `key skipped: local embedder`, `model installed: <model path>`, `server installed: 127.0.0.1:8737`, `embedder ok: 1024 dimensions`, `claude skipped: --no-plugin`, `codex skipped: --no-plugin`, `hook skipped: no codex plugin`, `timer installed: every 15 min`, `watch installed: watching <root>/notes` and `sync skipped: no scope syncs`, in that order, and the exit code is 0

#### Scenario: One failing step
- **WHEN** `claude plugin install` exits 1 during `bilbo setup --yes`
- **THEN** the `claude` line says `failed` with the first line of the tool's message, the `codex`, `hook`, `timer`, `watch` and `sync` steps still run, and the exit code is 1

### Requirement: Existing config file
An existing config SHALL be kept. Non-interactive `setup` given embedder flags SHALL rewrite a config that sets no key at all (only comments and blank lines, as `bilbo setup --yes` writes with no embedder): the old file becomes `config.bak`, the config line says `updated`, and the embedder check runs as for a new config. Against a config that sets any key, embedder or not, it SHALL exit 1 before writing anything when the flags differ from the embedder settings in the file, and SHALL keep the file when they are equal. The wizard SHALL show the current embedder settings and syncing scopes as defaults and SHALL rewrite the file only when the user changes one, or turns sync on for a scope, renaming the old file to `config.bak` first and reporting `updated`. A rewrite SHALL keep every digest, history, scope and sync setting the old file held.

#### Scenario: Flags against an existing config
- **WHEN** a config exists and a user runs `bilbo setup --yes --embedder-url http://x:1 --embedder-model m`
- **THEN** bilbo prints a message naming the config path and saying it already sets other settings, to stderr, exits 1, and changes no file

#### Scenario: Adding an embedder to an empty config
- **WHEN** `bilbo setup --yes` wrote a config with only comments, and the user runs `bilbo setup --yes --embedder-url http://127.0.0.1:8081 --embedder-model m` against a working embedder
- **THEN** `config.bak` holds the old file, the config sets the embedder, the config line says `updated`, and the exit code is 0

#### Scenario: The same flags again
- **WHEN** `bilbo setup --yes --embedder-url http://127.0.0.1:8081 --embedder-model m` wrote the config and the user runs the same command again
- **THEN** the config line says `kept`, the embedder line says `skipped: config kept`, and the exit code is 0

#### Scenario: The wizard changes the model
- **WHEN** the config sets `embedder.model = a` and the user picks model `b` in the wizard and confirms
- **THEN** `config.bak` holds the old file, the config sets `embedder.model = b`, and the config line says `updated`

#### Scenario: The wizard keeps the digest settings
- **WHEN** the config sets `embedder.model = a`, `digest.log = on` and `digest.min_similarity = 0.6`, and the user picks model `b` in the wizard and confirms
- **THEN** the new config sets `embedder.model = b`, `digest.min_similarity = 0.6` and `digest.log = on`

#### Scenario: The wizard keeps the history setting
- **WHEN** the config sets `embedder.model = a` and `history.keep_days = 30`, and the user picks model `b` in the wizard and confirms
- **THEN** the new config sets `embedder.model = b` and `history.keep_days = 30`

#### Scenario: The wizard keeps the scope settings
- **WHEN** the config sets `embedder.model = a`, `scope.work.embedder = local`, `scope.work.paths = ~/Developer/acme` and `scope.default = work`, and the user picks model `b` in the wizard and confirms
- **THEN** the new config sets `embedder.model = b` and holds the three scope lines as they were

#### Scenario: The wizard turns sync on
- **WHEN** the config sets `embedder.model = a`, `scope.work.sync = off` and `sync.poll_seconds = 60`, and the user turns sync on for `personal` with the folder `/Users/a/Dropbox/bilbo` and confirms
- **THEN** `config.bak` holds the old file, the new config sets `embedder.model = a`, `scope.work.sync = off`, `sync.poll_seconds = 60` and `scope.personal.sync = file:///Users/a/Dropbox/bilbo`, and the config line says `updated`

#### Scenario: The wizard keeps everything
- **WHEN** a config exists and the user accepts every default in the wizard
- **THEN** the config line says `kept` and no `config.bak` is written

### Requirement: Home-manager module
The flake SHALL export `homeManagerModules.default` with `programs.bilbo.enable`, `package`, `storeRoot` (a path exported as `BILBO_HOME`, or null for the default root), `settings` (embedder, digest, history, scope and sync keys to string values; a scope key is accepted only in the shape the `config` spec's Scope settings allow), `index.enable`, `index.every`, `watch.enable` (true by default), `claude` and `codex` (a path, or null for PATH), and `localEmbedder.enable`, `localEmbedder.port` and `localEmbedder.llamaServer` (nixpkgs' `llama-server` by default). When enabled, it SHALL install the package, write `settings` as the config file, and on activation run `bilbo setup --yes` with the matching flags, `--no-watch` among them when `watch.enable` is false. With `localEmbedder.enable`, the settings' URL and model SHALL default to the local embedder's, and activation SHALL pass `--embedder-local`, `--embedder-port` and `--llama-server`; an `embedder.url` other than the local one SHALL fail evaluation. Activation does not read session variables, so the module SHALL pass the locations explicitly: `BILBO_HOME` from `storeRoot` (unset when null), `BILBO_CONFIG` unset, and `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` from home-manager's `xdg` folders. It SHALL also put `launchctl` (macOS) or `systemctl` (Linux) on the PATH it gives `setup`. A key in `embedder.token_env` with `index.enable` SHALL fail evaluation. The module SHALL work without the flake's `home-manager` input, which only its flake check reads.

#### Scenario: Settings become the config
- **WHEN** a configuration sets `programs.bilbo.settings."embedder.url" = "http://bagend:8081"` and `"embedder.model" = "qwen3"`
- **THEN** after activation `~/.config/bilbo/config` is a link whose file holds those two lines, and `bilbo setup` reports it as managed elsewhere

#### Scenario: Digest settings from Nix
- **WHEN** a configuration sets `programs.bilbo.settings."digest.log" = "on"` and no other setting
- **THEN** the config file holds `digest.log = on` after the header, and evaluation succeeds

#### Scenario: History setting from Nix
- **WHEN** a configuration sets `programs.bilbo.settings."history.keep_days" = "30"`
- **THEN** the config file holds `history.keep_days = 30`, and evaluation succeeds

#### Scenario: Scope settings from Nix
- **WHEN** a configuration sets `programs.bilbo.settings."scope.work.embedder" = "local"` and `"scope.work.paths" = "~/Developer/acme"`
- **THEN** the config file holds `scope.work.embedder = local` and `scope.work.paths = ~/Developer/acme`, and evaluation succeeds

#### Scenario: A bad scope key fails evaluation
- **WHEN** a configuration sets `programs.bilbo.settings."scope.work.colour" = "red"`
- **THEN** evaluation fails with a message naming `scope.work.colour`

#### Scenario: Sync settings from Nix
- **WHEN** a configuration sets `programs.bilbo.settings."scope.personal.sync" = "file:///Users/a/Sync/bilbo"` and `"sync.poll_seconds" = "60"`
- **THEN** the config file holds both lines, and evaluation succeeds

#### Scenario: Watcher off from Nix
- **WHEN** a configuration sets `programs.bilbo.watch.enable = false`
- **THEN** activation runs `bilbo setup --yes` with `--no-watch`

#### Scenario: Activation installs the plugin from the package
- **WHEN** activation runs with `programs.bilbo.claude = "/opt/claude/bin/claude"`
- **THEN** setup runs that `claude` with the package's `share/bilbo` as the marketplace source

#### Scenario: An unknown setting fails evaluation
- **WHEN** a configuration sets `programs.bilbo.settings."embeder.url"`
- **THEN** evaluation fails with a message naming `embeder.url`

#### Scenario: A key variable fails evaluation
- **WHEN** a configuration sets `programs.bilbo.settings."embedder.token_env"` and leaves `index.enable` true
- **THEN** evaluation fails with a message naming `embedder.token_file`

#### Scenario: Activation passes the locations
- **WHEN** a configuration sets `programs.bilbo.storeRoot = "/Users/a/notes"`
- **THEN** the session exports `BILBO_HOME=/Users/a/notes`, activation runs `bilbo setup` with that `BILBO_HOME` and the four `XDG_*_HOME` folders of the configuration, and the timer and the watcher carry the same values

#### Scenario: Disabled does nothing
- **WHEN** `programs.bilbo.enable` is false
- **THEN** activation runs no `bilbo setup` and writes no config file

#### Scenario: The local embedder from Nix
- **WHEN** a configuration sets `programs.bilbo.localEmbedder.enable = true` and no embedder settings
- **THEN** the config holds `embedder.url = http://127.0.0.1:8737`, `embedder.model = qwen3-embedding-0.6b` and the Qwen `embedder.query_prefix` (`"Instruct: Given a question, retrieve notes that answer it\nQuery: "`), and activation runs `bilbo setup --yes --embedder-local --embedder-port 8737 --llama-server <nixpkgs llama-server>`

#### Scenario: The local embedder with another URL fails evaluation
- **WHEN** a configuration sets `programs.bilbo.localEmbedder.enable = true` and `programs.bilbo.settings."embedder.url" = "http://bagend:8081"`
- **THEN** evaluation fails with a message naming `localEmbedder.enable` and `embedder.url`
