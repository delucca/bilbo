# Spec Delta

## MODIFIED Requirements

### Requirement: Step report
After applying, `setup` SHALL print to stdout one line per step, in the order `store`, `config`, `key`, `model`, `server`, `embedder`, `claude`, `codex`, `hook`, `timer`, each line being `<step> <status>` optionally followed by `: <detail>`. The status SHALL be one of `created`, `written`, `kept`, `ok`, `installed`, `updated`, `removed`, `skipped` or `failed`. The exit code SHALL be 0 when no step failed and 1 otherwise. A failed step SHALL NOT stop the steps after it.

#### Scenario: A fresh non-interactive run
- **WHEN** a user runs `bilbo setup --yes --no-plugin` on a machine with no store and no config
- **THEN** stdout holds the lines `store created: <root>/notes`, `config written: <config path>`, `key skipped: no embedder`, `model skipped: not local`, `server skipped: not local`, `embedder skipped: none configured`, `claude skipped: --no-plugin`, `codex skipped: --no-plugin`, `hook skipped: no codex plugin` and `timer skipped: no embedder`, in that order, and the exit code is 0

#### Scenario: A fresh local run
- **WHEN** a user on macOS runs `bilbo setup --yes --no-plugin --embedder-local` on a machine with no store, no config and no model file
- **THEN** stdout holds `store created: <root>/notes`, `config written: <config path>`, `key skipped: local embedder`, `model installed: <model path>`, `server installed: 127.0.0.1:8737`, `embedder ok: 1024 dimensions`, `claude skipped: --no-plugin`, `codex skipped: --no-plugin`, `hook skipped: no codex plugin` and `timer installed: every 15 min`, in that order, and the exit code is 0

#### Scenario: One failing step
- **WHEN** `claude plugin install` exits 1 during `bilbo setup --yes`
- **THEN** the `claude` line says `failed` with the first line of the tool's message, the `codex`, `hook` and `timer` steps still run, and the exit code is 1

### Requirement: Remove
`bilbo setup --remove` SHALL unload and delete the timer and the local embedder's service, and in each tool found, uninstall `bilbo@bilbo` and remove the `bilbo` marketplace, and remove from Codex's config the trust of every bilbo hook, printing one line per step in the order `store`, `config`, `key`, `model`, `server`, `claude`, `codex`, `hook`, `timer`, each `removed`, `skipped` or `failed`. It SHALL keep the store, the config, the key file and the model file, and their lines SHALL say `skipped: kept <path>`. `--remove` SHALL accept only `--yes`, `--interactive`, `--claude` and `--codex` beside it. In a terminal it SHALL ask for confirmation first. With nothing installed it SHALL exit 0.

#### Scenario: Removing an install
- **WHEN** setup installed the timer and both plugins and the user runs `bilbo setup --remove --yes`
- **THEN** the timer file is gone and unloaded, neither tool lists a `bilbo` marketplace, Codex's config trusts no bilbo hook, the store, config and key file still exist, and stdout names their paths

#### Scenario: Removing the local embedder
- **WHEN** setup installed the local embedder and the user runs `bilbo setup --remove --yes`
- **THEN** the service file is gone and unloaded, the server line says `removed`, the model file still exists, and the model line says `skipped: kept <model path>`

#### Scenario: Nothing to remove
- **WHEN** nothing was installed and the user runs `bilbo setup --remove --yes`
- **THEN** every line says `skipped` and the exit code is 0

#### Scenario: Remove with setup flags
- **WHEN** a user runs `bilbo setup --remove --embedder-url http://x:1`
- **THEN** bilbo prints a message naming both flags to stderr, exits 2, and changes nothing

### Requirement: Existing config file
An existing config SHALL be kept. Non-interactive `setup` given embedder flags SHALL rewrite a config that sets no key at all (only comments and blank lines, as `bilbo setup --yes` writes with no embedder): the old file becomes `config.bak`, the config line says `updated`, and the embedder check runs as for a new config. Against a config that sets any key, embedder or not, it SHALL exit 1 before writing anything when the flags differ from the embedder settings in the file, and SHALL keep the file when they are equal. The wizard SHALL show the current embedder settings as defaults and SHALL rewrite the file only when the user changes one, renaming the old file to `config.bak` first and reporting `updated`. A rewrite SHALL keep every digest setting the old file held.

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

#### Scenario: The wizard keeps everything
- **WHEN** a config exists and the user accepts every default in the wizard
- **THEN** the config line says `kept` and no `config.bak` is written

### Requirement: Home-manager module
The flake SHALL export `homeManagerModules.default` with `programs.bilbo.enable`, `package`, `storeRoot` (a path exported as `BILBO_HOME`, or null for the default root), `settings` (embedder and digest keys to values), `index.enable`, `index.every`, `claude` and `codex` (a path, or null for PATH), and `localEmbedder.enable`, `localEmbedder.port` and `localEmbedder.llamaServer` (nixpkgs' `llama-server` by default). When enabled, it SHALL install the package, write `settings` as the config file, and on activation run `bilbo setup --yes` with the matching flags. With `localEmbedder.enable`, the settings' URL and model SHALL default to the local embedder's, and activation SHALL pass `--embedder-local`, `--embedder-port` and `--llama-server`; an `embedder.url` other than the local one SHALL fail evaluation. Activation does not read session variables, so the module SHALL pass the locations explicitly: `BILBO_HOME` from `storeRoot` (unset when null), `BILBO_CONFIG` unset, and `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` from home-manager's `xdg` folders. It SHALL also put `launchctl` (macOS) or `systemctl` (Linux) on the PATH it gives `setup`. A key in `embedder.token_env` with `index.enable` SHALL fail evaluation. The module SHALL work without the flake's `home-manager` input, which only its flake check reads.

#### Scenario: Settings become the config
- **WHEN** a configuration sets `programs.bilbo.settings."embedder.url" = "http://bagend:8081"` and `"embedder.model" = "qwen3"`
- **THEN** after activation `~/.config/bilbo/config` is a link whose file holds those two lines, and `bilbo setup` reports it as managed elsewhere

#### Scenario: Digest settings from Nix
- **WHEN** a configuration sets `programs.bilbo.settings."digest.log" = "on"` and no other setting
- **THEN** the config file holds `digest.log = on` after the header, and evaluation succeeds

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
- **THEN** the session exports `BILBO_HOME=/Users/a/notes`, activation runs `bilbo setup` with that `BILBO_HOME` and the four `XDG_*_HOME` folders of the configuration, and the timer carries the same values

#### Scenario: Disabled does nothing
- **WHEN** `programs.bilbo.enable` is false
- **THEN** activation runs no `bilbo setup` and writes no config file

#### Scenario: The local embedder from Nix
- **WHEN** a configuration sets `programs.bilbo.localEmbedder.enable = true` and no embedder settings
- **THEN** the config holds `embedder.url = http://127.0.0.1:8737`, `embedder.model = qwen3-embedding-0.6b` and the Qwen `embedder.query_prefix` (`"Instruct: Given a question, retrieve notes that answer it\nQuery: "`), and activation runs `bilbo setup --yes --embedder-local --embedder-port 8737 --llama-server <nixpkgs llama-server>`

#### Scenario: The local embedder with another URL fails evaluation
- **WHEN** a configuration sets `programs.bilbo.localEmbedder.enable = true` and `programs.bilbo.settings."embedder.url" = "http://bagend:8081"`
- **THEN** evaluation fails with a message naming `localEmbedder.enable` and `embedder.url`

## ADDED Requirements

### Requirement: Codex hook trust
When the codex step leaves `bilbo@bilbo` installed (`installed`, `updated` or `kept`), `setup` SHALL ask Codex, through `codex app-server`, which hooks the bilbo plugin registers and whether Codex trusts them, and SHALL mark every untrusted or changed bilbo hook trusted through Codex's own config writer, never by editing Codex's files itself. The hook line SHALL say `installed: trusted in Codex` when it trusted a hook Codex had never trusted, `updated: trusted in Codex` when the hook had changed since it was trusted, `kept: trusted in Codex` when every bilbo hook was already trusted, `skipped: no codex plugin` when the codex step did not leave the plugin installed, `skipped: codex lists no bilbo hook` when Codex lists none, `failed: <message>` when Codex cannot be asked or cannot write its config, and `failed: codex reports trust status '<status>' for a bilbo hook` when Codex lists a bilbo hook with a status other than `untrusted`, `modified`, `trusted` or `managed`, in which case `setup` SHALL write no trust. The wizard's summary SHALL name the trust whenever it installs the Codex plugin. `--remove` SHALL delete, through the same writer, every trust entry whose hook key belongs to `bilbo@bilbo`, and its hook line SHALL say `removed`, `skipped: not trusted`, `skipped: not found` when there is no `codex`, or `failed: <message>`.

#### Scenario: A fresh install trusts the hook
- **WHEN** `codex` is on PATH with no `bilbo` marketplace and a user runs `bilbo setup --yes`
- **THEN** the codex line says `installed`, the hook line says `installed: trusted in Codex`, and Codex runs the hook on the next prompt without asking for a review

#### Scenario: A rerun keeps the trust
- **WHEN** setup already trusted the hook and the user runs `bilbo setup --yes` again
- **THEN** the hook line says `kept: trusted in Codex` and Codex's config is not written

#### Scenario: A changed hook is trusted again
- **WHEN** a new bilbo release changes the hook's command, so Codex lists it as changed since it was trusted
- **THEN** setup writes the new trust and the hook line says `updated: trusted in Codex`

#### Scenario: An unknown trust status is not trusted
- **WHEN** Codex lists a bilbo hook with a trust status `setup` does not know, such as `blocked`
- **THEN** setup writes no trust, the hook line says `failed: codex reports trust status 'blocked' for a bilbo hook`, and the exit code is 1

#### Scenario: No Codex plugin, no trust
- **WHEN** a user runs `bilbo setup --yes --no-plugin`, or unticks Codex in the wizard
- **THEN** the hook line says `skipped: no codex plugin` and `codex app-server` does not run

#### Scenario: Codex cannot write its config
- **WHEN** Codex's `config.toml` is a link into a read-only folder and setup installs the plugin
- **THEN** the hook line says `failed` with Codex's message, the steps after it still run, and the exit code is 1

#### Scenario: Removing the trust
- **WHEN** setup trusted the hook and the user runs `bilbo setup --remove --yes`
- **THEN** Codex's config holds no trust entry for a `bilbo@bilbo` hook and the hook line says `removed`
