# Spec Delta

## MODIFIED Requirements

### Requirement: Existing config file
An existing config SHALL be kept. Non-interactive `setup` given embedder flags SHALL rewrite a config that sets no key at all (only comments and blank lines, as `bilbo setup --yes` writes with no embedder): the old file becomes `config.bak`, the config line says `updated`, and the embedder check runs as for a new config. Against a config that sets any key, embedder or not, it SHALL exit 1 before writing anything when the flags differ from the embedder settings in the file, and SHALL keep the file when they are equal. The wizard SHALL show the current embedder settings as defaults and SHALL rewrite the file only when the user changes one, renaming the old file to `config.bak` first and reporting `updated`. A rewrite SHALL keep every digest, history and scope setting the old file held.

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

#### Scenario: The wizard keeps everything
- **WHEN** a config exists and the user accepts every default in the wizard
- **THEN** the config line says `kept` and no `config.bak` is written

### Requirement: Home-manager module
The flake SHALL export `homeManagerModules.default` with `programs.bilbo.enable`, `package`, `storeRoot` (a path exported as `BILBO_HOME`, or null for the default root), `settings` (embedder, digest, history and scope keys to string values; a scope key is accepted only in the shape the `config` spec's Scope settings allow), `index.enable`, `index.every`, `watch.enable` (true by default), `claude` and `codex` (a path, or null for PATH), and `localEmbedder.enable`, `localEmbedder.port` and `localEmbedder.llamaServer` (nixpkgs' `llama-server` by default). When enabled, it SHALL install the package, write `settings` as the config file, and on activation run `bilbo setup --yes` with the matching flags, `--no-watch` among them when `watch.enable` is false. With `localEmbedder.enable`, the settings' URL and model SHALL default to the local embedder's, and activation SHALL pass `--embedder-local`, `--embedder-port` and `--llama-server`; an `embedder.url` other than the local one SHALL fail evaluation. Activation does not read session variables, so the module SHALL pass the locations explicitly: `BILBO_HOME` from `storeRoot` (unset when null), `BILBO_CONFIG` unset, and `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` from home-manager's `xdg` folders. It SHALL also put `launchctl` (macOS) or `systemctl` (Linux) on the PATH it gives `setup`. A key in `embedder.token_env` with `index.enable` SHALL fail evaluation. The module SHALL work without the flake's `home-manager` input, which only its flake check reads.

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
