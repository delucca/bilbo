# Spec Delta

## ADDED Requirements

### Requirement: Watch service
Unless `--no-watch` is given or the wizard's answer declines it, `setup` SHALL install a login service that runs `<absolute path of this bilbo, links resolved> watch`, starts at login, restarts when it exits, and appends its output to `<state>/bilbo/watch.log`. On macOS this SHALL be the launchd agent `io.github.delucca.bilbo.watch`, run at load and kept alive. On Linux it SHALL be the systemd user service `bilbo-watch.service` with `Restart=on-failure`, enabled and started. The service SHALL carry as its environment each of the six locations the `Index timer` requirement lists that `setup` itself saw set to an absolute path, and no other variable. The index timer's key rule SHALL NOT apply: a key in an environment variable SHALL NOT fail the watch step. The watch line SHALL say `installed: watching <root>/notes`, `kept` when the service is already as wanted, and `updated` when its file changes, as when the binary moved. When no watcher is wanted and one exists, `setup` SHALL unload and delete it and report `removed: --no-watch`. The wizard SHALL ask whether to record note history in the background, defaulting to yes.

#### Scenario: macOS
- **WHEN** a user on macOS runs `bilbo setup --yes`
- **THEN** `~/Library/LaunchAgents/io.github.delucca.bilbo.watch.plist` runs `bilbo watch`, it is loaded, set to run at load and keep alive, and the watch line says `installed: watching <root>/notes`

#### Scenario: Linux
- **WHEN** a user on Linux with a systemd user session runs `bilbo setup --yes`
- **THEN** `~/.config/systemd/user/bilbo-watch.service` runs `bilbo watch` with `Restart=on-failure`, it is enabled and started, and the watch line says `installed: watching <root>/notes`

#### Scenario: No embedder needed
- **WHEN** a user runs `bilbo setup --yes` with no embedder
- **THEN** the timer line says `skipped: no embedder` and the watch line says `installed`

#### Scenario: No systemd user session
- **WHEN** a user on Linux runs `bilbo setup --yes` and `systemctl --user` cannot reach a user manager
- **THEN** the watch line says `skipped: no systemd user session` and the exit code is 0

#### Scenario: Turning the watcher off
- **WHEN** the watcher is installed and the user runs `bilbo setup --yes --no-watch`
- **THEN** the service is unloaded, its file is deleted, and the watch line says `removed: --no-watch`

#### Scenario: A key variable does not stop the watcher
- **WHEN** the config sets `embedder.token_env = OPENAI_API_KEY` and a user runs `bilbo setup --yes`
- **THEN** the timer line says `failed` and names `OPENAI_API_KEY`, the watch line says `installed: watching <root>/notes`, the watcher's service file names no key variable, and the exit code is 1

#### Scenario: The watcher sees setup's locations
- **WHEN** a user runs `bilbo setup --yes` with `BILBO_HOME=/data/bilbo` set
- **THEN** the service file sets `BILBO_HOME` to `/data/bilbo`

## MODIFIED Requirements

### Requirement: Modes
`bilbo setup` SHALL run the interactive wizard when stdin and stderr are both terminals and no flag that answers a wizard question is given, and run non-interactively otherwise. The answer flags are `--embedder-url`, `--embedder-model`, `--embedder-token-env`, `--embedder-token-file`, `--embedder-query-prefix`, `--embedder-local`, `--embedder-port`, `--llama-server`, `--no-plugin`, `--no-timer`, `--index-every` and `--no-watch`. `--yes` SHALL force non-interactive mode. `--interactive` SHALL force the wizard and SHALL be a usage error when stdin or stderr is not a terminal. `--interactive` together with `--yes` or with an answer flag SHALL be a usage error.

#### Scenario: A terminal gets the wizard
- **WHEN** a user runs `bilbo setup` in a terminal
- **THEN** the wizard starts on stderr

#### Scenario: A pipe runs non-interactively
- **WHEN** an agent runs `bilbo setup < /dev/null`
- **THEN** bilbo asks nothing, applies the defaults, and prints the step report

#### Scenario: Forcing the wizard without a terminal
- **WHEN** an agent runs `bilbo setup --interactive < /dev/null`
- **THEN** bilbo prints a message saying the wizard needs a terminal to stderr, exits 2, and writes nothing

#### Scenario: Conflicting mode flags
- **WHEN** a user runs `bilbo setup --yes --interactive`
- **THEN** bilbo prints a message naming both flags to stderr, exits 2, and writes nothing

#### Scenario: Answer flags skip the wizard
- **WHEN** a user runs `bilbo setup --no-plugin` in a terminal
- **THEN** bilbo asks nothing, applies the defaults with the plugin steps skipped, and prints the step report

#### Scenario: The local flag skips the wizard
- **WHEN** a user runs `bilbo setup --embedder-local` in a terminal
- **THEN** bilbo asks nothing, installs the local embedder, and prints the step report

#### Scenario: No watch skips the wizard
- **WHEN** a user runs `bilbo setup --no-watch` in a terminal
- **THEN** bilbo asks nothing and the watch line says `skipped: --no-watch`

### Requirement: Setup flags
`setup` SHALL accept `--yes`, `--interactive`, `--remove`, `--embedder-url <url>`, `--embedder-model <name>`, `--embedder-token-env <var>`, `--embedder-token-file <path>`, `--embedder-query-prefix <text>`, `--embedder-local`, `--embedder-port <port>`, `--llama-server <path>`, `--no-plugin`, `--claude <path>`, `--codex <path>`, `--plugin-source <source>`, `--no-timer`, `--index-every <minutes>` and `--no-watch`. Values SHALL obey the `config` spec's rules for the matching key. Any other argument SHALL be a usage error. No flag SHALL take an API key's value. `--embedder-local` together with `--embedder-url`, `--embedder-model`, `--embedder-token-env` or `--embedder-token-file` SHALL be a usage error, and so SHALL `--embedder-port` or `--llama-server` without `--embedder-local`. `--embedder-port` SHALL take 1024 to 65535.

#### Scenario: An unknown flag
- **WHEN** a user runs `bilbo setup --token sk-123`
- **THEN** bilbo prints a message naming `--token` as unknown to stderr, exits 2, writes nothing, and the message holds no part of `sk-123`

#### Scenario: A model without a URL
- **WHEN** a user runs `bilbo setup --yes --embedder-model m`
- **THEN** bilbo prints a message naming `--embedder-url` to stderr, exits 2, and writes nothing

#### Scenario: An interval out of range
- **WHEN** a user runs `bilbo setup --yes --index-every 0`
- **THEN** bilbo prints a message saying `--index-every` takes 1 to 1440 minutes to stderr, exits 2, and writes nothing

#### Scenario: A tool path that is not executable
- **WHEN** a user runs `bilbo setup --yes --claude /nope/claude` and `/nope/claude` does not exist
- **THEN** bilbo prints a message naming `/nope/claude` to stderr, exits 2, and writes nothing

#### Scenario: Local and a URL together
- **WHEN** a user runs `bilbo setup --yes --embedder-local --embedder-url http://x:1`
- **THEN** bilbo prints a message naming `--embedder-local` and `--embedder-url` to stderr, exits 2, and writes nothing

#### Scenario: A port without the local flag
- **WHEN** a user runs `bilbo setup --yes --embedder-port 9100`
- **THEN** bilbo prints a message naming `--embedder-port` and `--embedder-local` to stderr, exits 2, and writes nothing

#### Scenario: A port out of range
- **WHEN** a user runs `bilbo setup --yes --embedder-local --embedder-port 80`
- **THEN** bilbo prints a message saying `--embedder-port` takes 1024 to 65535 to stderr, exits 2, and writes nothing

#### Scenario: No watch with a value
- **WHEN** a user runs `bilbo setup --yes --no-watch=true`
- **THEN** bilbo prints a message naming `--no-watch=true` as unknown to stderr, exits 2, and writes nothing

### Requirement: Plan before writing
`setup` SHALL settle every answer, read the current state and run the embedder check before it creates, changes or deletes any file. The one exception is the local embedder: its download, its service and its embedder check SHALL run after the plan is settled (in the wizard, after the confirmation) and before any other step writes. When one of them fails, setup SHALL unload and delete the service it installed, keep the model file, write nothing else and exit 1. The wizard SHALL then show a summary of the actions and ask for one confirmation. Declining, pressing Ctrl-C or Esc at any prompt, or reaching end of input SHALL exit 1 and write nothing.

#### Scenario: The summary lists the actions
- **WHEN** a user answers every wizard prompt on a machine with no store, no config, and `claude` on PATH
- **THEN** the wizard shows a summary naming the store folder, the config file, the Claude Code plugin, the timer and the watcher before asking to apply

#### Scenario: Declining writes nothing
- **WHEN** a user answers every prompt and then declines the summary
- **THEN** bilbo exits 1, and no store folder, config, key file, timer file or watcher file exists and no agent command ran

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
After applying, `setup` SHALL print to stdout one line per step, in the order `store`, `config`, `key`, `model`, `server`, `embedder`, `claude`, `codex`, `hook`, `timer`, `watch`, each line being `<step> <status>` optionally followed by `: <detail>`. The status SHALL be one of `created`, `written`, `kept`, `ok`, `installed`, `updated`, `removed`, `skipped` or `failed`. The exit code SHALL be 0 when no step failed and 1 otherwise. A failed step SHALL NOT stop the steps after it.

#### Scenario: A fresh non-interactive run
- **WHEN** a user runs `bilbo setup --yes --no-plugin` on a machine with no store and no config
- **THEN** stdout holds the lines `store created: <root>/notes`, `config written: <config path>`, `key skipped: no embedder`, `model skipped: not local`, `server skipped: not local`, `embedder skipped: none configured`, `claude skipped: --no-plugin`, `codex skipped: --no-plugin`, `hook skipped: no codex plugin`, `timer skipped: no embedder` and `watch installed: watching <root>/notes`, in that order, and the exit code is 0

#### Scenario: A fresh local run
- **WHEN** a user on macOS runs `bilbo setup --yes --no-plugin --embedder-local` on a machine with no store, no config and no model file
- **THEN** stdout holds `store created: <root>/notes`, `config written: <config path>`, `key skipped: local embedder`, `model installed: <model path>`, `server installed: 127.0.0.1:8737`, `embedder ok: 1024 dimensions`, `claude skipped: --no-plugin`, `codex skipped: --no-plugin`, `hook skipped: no codex plugin`, `timer installed: every 15 min` and `watch installed: watching <root>/notes`, in that order, and the exit code is 0

#### Scenario: One failing step
- **WHEN** `claude plugin install` exits 1 during `bilbo setup --yes`
- **THEN** the `claude` line says `failed` with the first line of the tool's message, the `codex`, `hook`, `timer` and `watch` steps still run, and the exit code is 1

### Requirement: Existing config file
An existing config SHALL be kept. Non-interactive `setup` given embedder flags SHALL rewrite a config that sets no key at all (only comments and blank lines, as `bilbo setup --yes` writes with no embedder): the old file becomes `config.bak`, the config line says `updated`, and the embedder check runs as for a new config. Against a config that sets any key, embedder or not, it SHALL exit 1 before writing anything when the flags differ from the embedder settings in the file, and SHALL keep the file when they are equal. The wizard SHALL show the current embedder settings as defaults and SHALL rewrite the file only when the user changes one, renaming the old file to `config.bak` first and reporting `updated`. A rewrite SHALL keep every digest and history setting the old file held.

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

#### Scenario: The wizard keeps everything
- **WHEN** a config exists and the user accepts every default in the wizard
- **THEN** the config line says `kept` and no `config.bak` is written

### Requirement: Remove
`bilbo setup --remove` SHALL unload and delete the timer, the watcher and the local embedder's service, and in each tool found, uninstall `bilbo@bilbo` and remove the `bilbo` marketplace, and remove from Codex's config the trust of every bilbo hook, printing one line per step in the order `store`, `config`, `key`, `model`, `server`, `claude`, `codex`, `hook`, `timer`, `watch`, each `removed`, `skipped` or `failed`. It SHALL keep the store, its history, the config, the key file and the model file, and their lines SHALL say `skipped: kept <path>`. `--remove` SHALL accept only `--yes`, `--interactive`, `--claude` and `--codex` beside it. In a terminal it SHALL ask for confirmation first. With nothing installed it SHALL exit 0.

#### Scenario: Removing an install
- **WHEN** setup installed the timer, the watcher and both plugins and the user runs `bilbo setup --remove --yes`
- **THEN** the timer and watcher files are gone and unloaded, neither tool lists a `bilbo` marketplace, Codex's config trusts no bilbo hook, the store, `<root>/.bilbo/`, config and key file still exist, and stdout names their paths

#### Scenario: Removing the local embedder
- **WHEN** setup installed the local embedder and the user runs `bilbo setup --remove --yes`
- **THEN** the service file is gone and unloaded, the server line says `removed`, the model file still exists, and the model line says `skipped: kept <model path>`

#### Scenario: Nothing to remove
- **WHEN** nothing was installed and the user runs `bilbo setup --remove --yes`
- **THEN** every line says `skipped` and the exit code is 0

#### Scenario: Remove with setup flags
- **WHEN** a user runs `bilbo setup --remove --embedder-url http://x:1`
- **THEN** bilbo prints a message naming both flags to stderr, exits 2, and changes nothing

#### Scenario: Removing the watcher without the service manager
- **WHEN** the watcher is installed on macOS and a user runs `bilbo setup --remove --yes` with no `launchctl` on PATH
- **THEN** the watch line says `failed: launchctl not found on PATH`, the plist is still there, and the exit code is 1

### Requirement: Home-manager module
The flake SHALL export `homeManagerModules.default` with `programs.bilbo.enable`, `package`, `storeRoot` (a path exported as `BILBO_HOME`, or null for the default root), `settings` (embedder, digest and history keys to string values), `index.enable`, `index.every`, `watch.enable` (true by default), `claude` and `codex` (a path, or null for PATH), and `localEmbedder.enable`, `localEmbedder.port` and `localEmbedder.llamaServer` (nixpkgs' `llama-server` by default). When enabled, it SHALL install the package, write `settings` as the config file, and on activation run `bilbo setup --yes` with the matching flags, `--no-watch` among them when `watch.enable` is false. With `localEmbedder.enable`, the settings' URL and model SHALL default to the local embedder's, and activation SHALL pass `--embedder-local`, `--embedder-port` and `--llama-server`; an `embedder.url` other than the local one SHALL fail evaluation. Activation does not read session variables, so the module SHALL pass the locations explicitly: `BILBO_HOME` from `storeRoot` (unset when null), `BILBO_CONFIG` unset, and `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` from home-manager's `xdg` folders. It SHALL also put `launchctl` (macOS) or `systemctl` (Linux) on the PATH it gives `setup`. A key in `embedder.token_env` with `index.enable` SHALL fail evaluation. The module SHALL work without the flake's `home-manager` input, which only its flake check reads.

#### Scenario: Settings become the config
- **WHEN** a configuration sets `programs.bilbo.settings."embedder.url" = "http://bagend:8081"` and `"embedder.model" = "qwen3"`
- **THEN** after activation `~/.config/bilbo/config` is a link whose file holds those two lines, and `bilbo setup` reports it as managed elsewhere

#### Scenario: Digest settings from Nix
- **WHEN** a configuration sets `programs.bilbo.settings."digest.log" = "on"` and no other setting
- **THEN** the config file holds `digest.log = on` after the header, and evaluation succeeds

#### Scenario: History setting from Nix
- **WHEN** a configuration sets `programs.bilbo.settings."history.keep_days" = "30"`
- **THEN** the config file holds `history.keep_days = 30`, and evaluation succeeds

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
