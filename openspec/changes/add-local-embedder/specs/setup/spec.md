# Spec Delta

## MODIFIED Requirements

### Requirement: Modes
`bilbo setup` SHALL run the interactive wizard when stdin and stderr are both terminals and no flag that answers a wizard question is given, and run non-interactively otherwise. The answer flags are `--embedder-url`, `--embedder-model`, `--embedder-token-env`, `--embedder-token-file`, `--embedder-query-prefix`, `--embedder-local`, `--embedder-port`, `--llama-server`, `--no-plugin`, `--no-timer` and `--index-every`. `--yes` SHALL force non-interactive mode. `--interactive` SHALL force the wizard and SHALL be a usage error when stdin or stderr is not a terminal. `--interactive` together with `--yes` or with an answer flag SHALL be a usage error.

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

### Requirement: Setup flags
`setup` SHALL accept `--yes`, `--interactive`, `--remove`, `--embedder-url <url>`, `--embedder-model <name>`, `--embedder-token-env <var>`, `--embedder-token-file <path>`, `--embedder-query-prefix <text>`, `--embedder-local`, `--embedder-port <port>`, `--llama-server <path>`, `--no-plugin`, `--claude <path>`, `--codex <path>`, `--plugin-source <source>`, `--no-timer` and `--index-every <minutes>`. Values SHALL obey the `config` spec's rules for the matching key. Any other argument SHALL be a usage error. No flag SHALL take an API key's value. `--embedder-local` together with `--embedder-url`, `--embedder-model`, `--embedder-token-env` or `--embedder-token-file` SHALL be a usage error, and so SHALL `--embedder-port` or `--llama-server` without `--embedder-local`. `--embedder-port` SHALL take 1024 to 65535.

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

### Requirement: Plan before writing
`setup` SHALL settle every answer, read the current state and run the embedder check before it creates, changes or deletes any file. The one exception is the local embedder: its download, its service and its embedder check SHALL run after the plan is settled (in the wizard, after the confirmation) and before any other step writes. When one of them fails, setup SHALL unload and delete the service it installed, keep the model file, write nothing else and exit 1. The wizard SHALL then show a summary of the actions and ask for one confirmation. Declining, pressing Ctrl-C or Esc at any prompt, or reaching end of input SHALL exit 1 and write nothing.

#### Scenario: The summary lists the actions
- **WHEN** a user answers every wizard prompt on a machine with no store, no config, and `claude` on PATH
- **THEN** the wizard shows a summary naming the store folder, the config file, the Claude Code plugin and the timer before asking to apply

#### Scenario: Declining writes nothing
- **WHEN** a user answers every prompt and then declines the summary
- **THEN** bilbo exits 1, and no store folder, config, key file or timer file exists and no agent command ran

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
After applying, `setup` SHALL print to stdout one line per step, in the order `store`, `config`, `key`, `model`, `server`, `embedder`, `claude`, `codex`, `timer`, each line being `<step> <status>` optionally followed by `: <detail>`. The status SHALL be one of `created`, `written`, `kept`, `ok`, `installed`, `updated`, `removed`, `skipped` or `failed`. The exit code SHALL be 0 when no step failed and 1 otherwise. A failed step SHALL NOT stop the steps after it.

#### Scenario: A fresh non-interactive run
- **WHEN** a user runs `bilbo setup --yes --no-plugin` on a machine with no store and no config
- **THEN** stdout holds the lines `store created: <root>/notes`, `config written: <config path>`, `key skipped: no embedder`, `model skipped: not local`, `server skipped: not local`, `embedder skipped: none configured`, `claude skipped: --no-plugin`, `codex skipped: --no-plugin` and `timer skipped: no embedder`, in that order, and the exit code is 0

#### Scenario: A fresh local run
- **WHEN** a user on macOS runs `bilbo setup --yes --no-plugin --embedder-local` on a machine with no store, no config and no model file
- **THEN** stdout holds `store created: <root>/notes`, `config written: <config path>`, `key skipped: local embedder`, `model installed: <model path>`, `server installed: 127.0.0.1:8737`, `embedder ok: 1024 dimensions`, `claude skipped: --no-plugin`, `codex skipped: --no-plugin` and `timer installed: every 15 min`, in that order, and the exit code is 0

#### Scenario: One failing step
- **WHEN** `claude plugin install` exits 1 during `bilbo setup --yes`
- **THEN** the `claude` line says `failed` with the first line of the tool's message, the `codex` and `timer` steps still run, and the exit code is 1

### Requirement: Managed config
When the config path is a symbolic link, or its folder is not writable, `setup` SHALL treat the config as managed elsewhere: it SHALL report `kept: managed elsewhere (<target>)`, the wizard SHALL show its settings without offering to change them, and embedder flags SHALL be a usage error naming the path. `--embedder-local` SHALL be allowed against a managed config whose `embedder.url` and `embedder.model` equal the local embedder's, and SHALL then install the model and the service only; against any other managed config it SHALL be a usage error naming the path.

#### Scenario: A Nix-managed config
- **WHEN** the config path is a link into `/nix/store` and the user runs the wizard
- **THEN** the wizard shows the embedder settings as read-only, and the config line says `kept: managed elsewhere (/nix/store/…)`

#### Scenario: Flags against a managed config
- **WHEN** the config path is a link and a user runs `bilbo setup --yes --embedder-url http://x:1 --embedder-model m`
- **THEN** bilbo prints a message naming the config path to stderr, exits 2, and writes nothing

#### Scenario: A managed config that points at the local embedder
- **WHEN** the config path is a link to a file setting `embedder.url = http://127.0.0.1:8737` and `embedder.model = qwen3-embedding-0.6b`, and a user runs `bilbo setup --yes --embedder-local`
- **THEN** the config line says `kept: managed elsewhere (<target>)`, the model and server lines say `installed`, and the exit code is 0

#### Scenario: A managed config that points elsewhere
- **WHEN** the config path is a link to a file setting `embedder.url = http://bagend:8081`, and a user runs `bilbo setup --yes --embedder-local`
- **THEN** bilbo prints a message naming the config path to stderr, exits 2, and writes nothing

### Requirement: Embedder choices
The wizard SHALL offer, in this order: no embedder (keyword search only), a local embedder run by bilbo, Ollama on this machine, OpenAI, and another OpenAI-compatible URL. Choosing one SHALL set the URL and the default model as follows, then ask for the model, except for the local embedder, whose model is fixed.

- None: no URL.
- Local: `http://127.0.0.1:8737` and the model `qwen3-embedding-0.6b`. Its choice SHALL name the download size and say whether `llama-server` was found.
- Ollama: `http://localhost:11434`, defaulting to the first of its models whose name holds `embed`.
- OpenAI: `https://api.openai.com`, with the model `text-embedding-3-small`.
- Another URL: the URL the user types, with no default model.

#### Scenario: Ollama is detected
- **WHEN** `http://localhost:11434/api/tags` answers within 1 second with models `llama3` and `nomic-embed-text`
- **THEN** the Ollama choice is marked as found, and the model prompt lists both, with `nomic-embed-text` selected

#### Scenario: Ollama is not running
- **WHEN** nothing listens on `localhost:11434`
- **THEN** the Ollama choice is marked as not found, and choosing it asks for the URL with `http://localhost:11434` as the default

#### Scenario: Keyword only
- **WHEN** the user picks no embedder
- **THEN** no key, model or interval is asked, and the summary says recall will use keywords only

#### Scenario: The local embedder with llama-server found
- **WHEN** `llama-server` is on PATH and the user picks the local embedder
- **THEN** no URL, model or key is asked, and the summary names the 639 MB download, the model path, the service, about 1 GB of memory kept in use, and that the first index of a large store takes a while

#### Scenario: The local embedder without llama-server
- **WHEN** no `llama-server` is on PATH and the user picks the local embedder
- **THEN** the wizard names `brew install llama.cpp` and the distribution's `llama.cpp` package and asks for the path to `llama-server`, and an empty answer returns to the list of embedders

#### Scenario: No service manager
- **WHEN** the wizard runs on Linux without a systemd user session
- **THEN** the local choice is marked as needing a systemd user session, and choosing it says so and returns to the list of embedders

#### Scenario: A rerun keeps the local choice
- **WHEN** the config sets `embedder.url = http://127.0.0.1:8737` and `embedder.model = qwen3-embedding-0.6b` and the user runs the wizard
- **THEN** the local choice is selected by default

### Requirement: Embedder check
Before writing anything, `setup` SHALL send one embed request to the chosen embedder with its model and key, with a 15-second limit, and report `embedder ok: <dimensions> dimensions`. For the local embedder, the request SHALL go to the started server once it reports ready, as the `Plan before writing` requirement orders. In the wizard, a failure SHALL show the reason and offer to retry, change the settings, or continue keyword-only; for the local embedder it SHALL also name the server's log file, and changing the settings SHALL NOT be offered. In non-interactive mode, a failure SHALL exit 1 and write nothing.

#### Scenario: A working embedder
- **WHEN** the embedder answers with 1,024-dimensional vectors
- **THEN** the embedder line says `ok: 1024 dimensions`

#### Scenario: A wrong model, non-interactive
- **WHEN** a user runs `bilbo setup --yes --embedder-url http://127.0.0.1:8081 --embedder-model nope` and the embedder answers 404
- **THEN** bilbo prints a message naming the URL and the status 404 to stderr, exits 1, and writes nothing

#### Scenario: A rejected key in the wizard
- **WHEN** the embedder answers 401 to the pasted key
- **THEN** the wizard shows the URL and the status 401, holds no part of the key, and offers to retry, change the settings or continue keyword-only

#### Scenario: An existing config is not checked
- **WHEN** a config with an embedder exists and the user runs `bilbo setup --yes`
- **THEN** no embed request is sent and the embedder line says `skipped: config kept`

#### Scenario: The local server fails in the wizard
- **WHEN** the user confirmed the local embedder and the started server answers the embed request with 500
- **THEN** the wizard shows the status 500 and `<state>/bilbo/embedder.log`, and offers to retry or continue keyword-only; continuing keyword-only removes the service, keeps the model file, and applies the other steps with no embedder

### Requirement: Remove
`bilbo setup --remove` SHALL unload and delete the timer and the local embedder's service, and in each tool found, uninstall `bilbo@bilbo` and remove the `bilbo` marketplace, printing one line per step in the order `store`, `config`, `key`, `model`, `server`, `claude`, `codex`, `timer`, each `removed`, `skipped` or `failed`. It SHALL keep the store, the config, the key file and the model file, and their lines SHALL say `skipped: kept <path>`. `--remove` SHALL accept only `--yes`, `--interactive`, `--claude` and `--codex` beside it. In a terminal it SHALL ask for confirmation first. With nothing installed it SHALL exit 0.

#### Scenario: Removing an install
- **WHEN** setup installed the timer and both plugins and the user runs `bilbo setup --remove --yes`
- **THEN** the timer file is gone and unloaded, neither tool lists a `bilbo` marketplace, the store, config and key file still exist, and stdout names their paths

#### Scenario: Removing the local embedder
- **WHEN** setup installed the local embedder and the user runs `bilbo setup --remove --yes`
- **THEN** the service file is gone and unloaded, the server line says `removed`, the model file still exists, and the model line says `skipped: kept <model path>`

#### Scenario: Nothing to remove
- **WHEN** nothing was installed and the user runs `bilbo setup --remove --yes`
- **THEN** every line says `skipped` and the exit code is 0

#### Scenario: Remove with setup flags
- **WHEN** a user runs `bilbo setup --remove --embedder-url http://x:1`
- **THEN** bilbo prints a message naming both flags to stderr, exits 2, and changes nothing

### Requirement: Home-manager module
The flake SHALL export `homeManagerModules.default` with `programs.bilbo.enable`, `package`, `storeRoot` (a path exported as `BILBO_HOME`, or null for the default root), `settings` (embedder keys to values), `index.enable`, `index.every`, `claude` and `codex` (a path, or null for PATH), and `localEmbedder.enable`, `localEmbedder.port` and `localEmbedder.llamaServer` (nixpkgs' `llama-server` by default). When enabled, it SHALL install the package, write `settings` as the config file, and on activation run `bilbo setup --yes` with the matching flags. With `localEmbedder.enable`, the settings' URL and model SHALL default to the local embedder's, and activation SHALL pass `--embedder-local`, `--embedder-port` and `--llama-server`; an `embedder.url` other than the local one SHALL fail evaluation. Activation does not read session variables, so the module SHALL pass the locations explicitly: `BILBO_HOME` from `storeRoot` (unset when null), `BILBO_CONFIG` unset, and `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` from home-manager's `xdg` folders. It SHALL also put `launchctl` (macOS) or `systemctl` (Linux) on the PATH it gives `setup`. A key in `embedder.token_env` with `index.enable` SHALL fail evaluation. The module SHALL work without the flake's `home-manager` input, which only its flake check reads.

#### Scenario: Settings become the config
- **WHEN** a configuration sets `programs.bilbo.settings."embedder.url" = "http://bagend:8081"` and `"embedder.model" = "qwen3"`
- **THEN** after activation `~/.config/bilbo/config` is a link whose file holds those two lines, and `bilbo setup` reports it as managed elsewhere

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
