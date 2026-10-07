# setup Specification

## Purpose
How `bilbo setup` turns a `bilbo` binary on PATH into a working install: the store folder, the config and embedder key, the local embedder when asked, the agent plugin at the binary's own version with its Codex hook trust, the index timer, the watch service, and the check of each syncing scope, which the wizard can also turn on. It covers `--remove`, and both the interactive wizard and the non-interactive mode that scripts and the home-manager module use.

## Requirements

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
- **THEN** bilbo prints a message saying `--no-watch` takes no value to stderr, exits 2, and writes nothing

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

### Requirement: Reruns change nothing
Running `setup` again with the same inputs on an installed machine SHALL report every step as `kept`, `ok` or `skipped`, SHALL NOT rewrite any file, and SHALL NOT run any agent command that changes state.

#### Scenario: A second run
- **WHEN** `bilbo setup --yes` succeeded and the user runs it again with the same flags
- **THEN** no line says `created`, `written`, `installed` or `updated`, the config, key and timer files keep their modification times, and no `marketplace add`, `marketplace remove`, `install` or `add` command ran

#### Scenario: A new version updates the plugin
- **WHEN** setup ran with `bilbo 0.2.0`, the user upgrades to `0.3.0` and runs `bilbo setup --yes`
- **THEN** the `claude` and `codex` lines say `updated` and name `v0.3.0`

### Requirement: Store step
The store step SHALL create `<root>/notes/`, where `<root>` follows the `note-store` spec's store root rules, and report `created`. When it exists, it SHALL report `kept`. The wizard SHALL show the root but SHALL NOT ask for it.

#### Scenario: An existing store is kept
- **WHEN** `<root>/notes/` holds notes and the user runs `bilbo setup --yes`
- **THEN** the store line says `kept` and no note changes

#### Scenario: The root cannot be created
- **WHEN** `BILBO_HOME` is `/proc/bilbo` on Linux and the user runs `bilbo setup --yes`
- **THEN** the store line says `failed`, names `/proc/bilbo`, and the exit code is 1

### Requirement: New config file
When no config file exists, the config step SHALL write one, at the path the `config` spec resolves, holding a comment header and one line for each embedder setting given, and report `written`. With no embedder given, it SHALL write the header and a commented example only, which every verb reads as all defaults. The file SHALL be written to a temporary name in the same folder and renamed into place.

#### Scenario: Embedder flags become settings
- **WHEN** a user runs `bilbo setup --yes --embedder-url http://127.0.0.1:8081 --embedder-model qwen3-embedding-0.6b` against a working embedder, with no config file
- **THEN** the config holds `embedder.url = http://127.0.0.1:8081` and `embedder.model = qwen3-embedding-0.6b`, and `bilbo recall` reads it with no error

#### Scenario: No embedder writes a valid empty config
- **WHEN** a user runs `bilbo setup --yes` with no embedder flags and no config file
- **THEN** the config holds only comments and blank lines, and `bilbo recall` runs keyword-only with nothing about the config on stderr

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
- **WHEN** the config path is a link to a file setting `embedder.url = http://embedder.example:8081`, and a user runs `bilbo setup --yes --embedder-local`
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

### Requirement: Query prefix default
`setup` SHALL default the query prefix to `Instruct: Given a question, retrieve notes that answer it\nQuery: ` when the model name holds `qwen3-embedding` in any case, and to empty otherwise, in both modes. `--embedder-query-prefix` SHALL replace that default. The wizard SHALL show the prefix for editing only when the user asks for advanced settings.

#### Scenario: A Qwen model gets the prefix
- **WHEN** the user picks the model `Qwen3-Embedding-0.6B` and accepts the defaults
- **THEN** the config sets `embedder.query_prefix` to that text, quoted, with `\n` for the newline

#### Scenario: Other models get none
- **WHEN** the user picks `text-embedding-3-small`
- **THEN** the config holds no `embedder.query_prefix` line

#### Scenario: A Qwen model from flags
- **WHEN** a user runs `bilbo setup --yes --embedder-url http://127.0.0.1:8081 --embedder-model qwen3-embedding-0.6b` against a working embedder, with no config file
- **THEN** the config sets `embedder.query_prefix` to that text, quoted, with `\n` for the newline

### Requirement: Embedder key
For a URL other than one on `localhost` or `127.0.0.1`, the wizard SHALL ask where the key comes from: an environment variable (default `OPENAI_API_KEY` for OpenAI), a file, pasting it now, or no key. A pasted key SHALL be read with hidden input and written to `<config folder>/token`, created with mode 0600 before any byte is written. The config SHALL then reference it through `embedder.token_file`.

#### Scenario: A pasted key
- **WHEN** the user picks OpenAI, chooses to paste the key, types it, and confirms
- **THEN** `<config folder>/token` holds the key, its mode is 0600, the config sets `embedder.token_file` to that path, the key line says `written`, and the key appears on neither stdout nor stderr

#### Scenario: A variable that is not set
- **WHEN** the user names the variable `OPENAI_API_KEY` and it is unset or empty
- **THEN** the wizard says so, naming the variable, and asks again before the embedder check

#### Scenario: An existing key file
- **WHEN** `<config folder>/token` exists and the user pastes a new key
- **THEN** the wizard asks before replacing it, and declining keeps the old file and the key line says `kept`

#### Scenario: A local embedder asks for no key
- **WHEN** the user picks Ollama
- **THEN** no key question is asked and the key line says `skipped: local embedder`

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

### Requirement: Plugin source
`setup` SHALL install the plugin from `<exe>/../share/bilbo` when that folder holds `.claude-plugin/marketplace.json`, where `<exe>` is the folder of the running binary with links resolved, and otherwise from the GitHub repository `delucca/bilbo` at the tag `v<version>`. `--plugin-source` SHALL override both with a folder path or `<owner>/<repo>#<ref>`.

#### Scenario: A Nix package uses its own folder
- **WHEN** the running binary is `/nix/store/abc-bilbo-0.2.0/bin/bilbo` and `/nix/store/abc-bilbo-0.2.0/share/bilbo/.claude-plugin/marketplace.json` exists
- **THEN** both tools add the marketplace from `/nix/store/abc-bilbo-0.2.0/share/bilbo`

#### Scenario: An installer binary uses the tag
- **WHEN** the running binary is `~/.local/bin/bilbo` version `0.2.0` with no `share/bilbo` beside it
- **THEN** Claude Code adds `delucca/bilbo#v0.2.0` and Codex adds `delucca/bilbo` with `--ref v0.2.0`

#### Scenario: A missing tag
- **WHEN** the tag `v0.2.0` does not exist on GitHub, as for a local build
- **THEN** the `claude` and `codex` lines say `failed` and name the source, and the hint names `--plugin-source`

### Requirement: Agent plugin step
For each of Claude Code and Codex, `setup` SHALL use the `--claude` or `--codex` path when given, else the tool on PATH, else report `skipped: not found`. When the tool's `bilbo` marketplace already has the planned source and `bilbo@bilbo` is installed and enabled, the step SHALL report `kept`. Otherwise it SHALL replace the marketplace, install `bilbo@bilbo`, and report `installed` or `updated` with the source.

#### Scenario: A fresh install in Claude Code
- **WHEN** `claude` is on PATH with no `bilbo` marketplace
- **THEN** setup runs `claude plugin marketplace add <source>` then `claude plugin install bilbo@bilbo`, and the claude line says `installed: <source>`

#### Scenario: Claude Code at an older source
- **WHEN** Claude Code's `bilbo` marketplace comes from `delucca/bilbo#v0.1.0` and the planned source is `delucca/bilbo#v0.2.0`
- **THEN** setup removes the marketplace, adds the new source, installs `bilbo@bilbo`, and the claude line says `updated: delucca/bilbo#v0.2.0`

#### Scenario: Codex already current
- **WHEN** Codex's `bilbo` marketplace comes from the planned source and `bilbo@bilbo` is installed, enabled, at the binary's version
- **THEN** the codex line says `kept` and no codex command that changes state runs

#### Scenario: A tool is absent
- **WHEN** `codex` is not on PATH and no `--codex` is given
- **THEN** the codex line says `skipped: not found`

#### Scenario: The wizard lets the user opt out
- **WHEN** both tools are found and the user unticks Codex in the wizard
- **THEN** the codex line says `skipped: not chosen` and no codex command runs

### Requirement: Index timer
When the resulting config has an `embedder.url`, `setup` SHALL install a job that runs `<absolute path of this bilbo, links resolved> index` every `--index-every` minutes (15 by default), the first time that many minutes after it is loaded, appending its output to `<state>/bilbo/index.log`. On macOS this SHALL be the launchd agent `io.github.delucca.bilbo.index`. On Linux it SHALL be the systemd user units `bilbo-index.service` and `bilbo-index.timer`.

A timer does not inherit the shell's environment, so the job SHALL carry as its environment each of `BILBO_HOME`, `BILBO_CONFIG`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` that `setup` itself saw set to an absolute path, and no other variable. When the key comes from an environment variable (`embedder.token_env`, from a flag, the wizard, or an existing or managed config), the timer step SHALL fail with a message naming the variable, `--embedder-token-file` and `--no-timer`, and SHALL leave the timer files as they were. No timer file SHALL hold a key. When no timer is wanted (`--no-timer`, the wizard's choice, or no embedder) and a bilbo timer exists, `setup` SHALL unload and delete it and report `removed`. When a bilbo timer exists but `launchctl` or `systemctl` is not on PATH, removing it SHALL fail with a message naming the tool and SHALL leave its files in place, so a later run with the tool can unload it.

#### Scenario: macOS
- **WHEN** a user on macOS with an embedder configured runs `bilbo setup --yes`
- **THEN** `~/Library/LaunchAgents/io.github.delucca.bilbo.index.plist` runs `bilbo index` every 900 seconds, `launchctl` has loaded it, and the timer line says `installed: every 15 min`

#### Scenario: Linux
- **WHEN** a user on Linux with a systemd user session and an embedder configured runs `bilbo setup --yes --index-every 30`
- **THEN** `~/.config/systemd/user/bilbo-index.timer` fires every 30 minutes, it is enabled and started, and the timer line says `installed: every 30 min`

#### Scenario: No embedder, no timer
- **WHEN** the config has no `embedder.url`
- **THEN** the timer line says `skipped: no embedder` and no timer file is written

#### Scenario: No systemd user session
- **WHEN** a user on Linux runs `bilbo setup --yes` and `systemctl --user` cannot reach a user manager
- **THEN** the timer line says `skipped: no systemd user session` and the exit code is 0

#### Scenario: The timer sees setup's locations
- **WHEN** a user runs `bilbo setup --yes` with `BILBO_HOME=/data/bilbo` and `XDG_CACHE_HOME=/data/cache` set, `XDG_STATE_HOME` unset, and an embedder configured
- **THEN** the timer file sets `BILBO_HOME` to `/data/bilbo` and `XDG_CACHE_HOME` to `/data/cache`, and sets no `XDG_STATE_HOME`

#### Scenario: A key in a variable
- **WHEN** the config sets `embedder.token_env = OPENAI_API_KEY` and a user runs `bilbo setup --yes`
- **THEN** the timer line says `failed` and names `OPENAI_API_KEY`, `--embedder-token-file` and `--no-timer`, no timer file is written, and the exit code is 1

#### Scenario: Turning the timer off
- **WHEN** a timer is installed and the user runs `bilbo setup --yes --no-timer`
- **THEN** the timer is unloaded, its files are deleted, and the timer line says `removed: --no-timer`

#### Scenario: Removing without the service manager
- **WHEN** a timer is installed on macOS and a user runs `bilbo setup --yes --no-timer` with no `launchctl` on PATH
- **THEN** the timer line says `failed: launchctl not found on PATH`, the plist is still there, and the exit code is 1

#### Scenario: The binary moved
- **WHEN** a timer exists for `/nix/store/old/bin/bilbo` and setup runs from `/nix/store/new/bin/bilbo`
- **THEN** the timer file is rewritten to the new path, reloaded, and the timer line says `updated`

### Requirement: First index
After a successful apply that leaves an embedder configured, the wizard SHALL offer to run `bilbo index` now, naming the number of notes in the store. Non-interactive `setup` SHALL NOT run `index`.

#### Scenario: The user accepts
- **WHEN** the store holds 12 notes and the user accepts the offer
- **THEN** the wizard runs the index with a spinner, then shows its `embedded <n>, kept <n>, dropped <n>` line

#### Scenario: Non-interactive never indexes
- **WHEN** a user runs `bilbo setup --yes` with an embedder configured
- **THEN** no embed request is sent beyond the embedder check

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

### Requirement: Secrets stay out of setup output
`setup` SHALL NOT print an API key, in full or in part, on stdout or stderr, in the wizard, in the summary or in the report, and SHALL NOT pass it to `claude`, `codex`, `launchctl` or `systemctl`. The summary SHALL name a key by its source: a variable name, a file path, or "pasted".

#### Scenario: The summary names the source only
- **WHEN** the user pastes a key and reaches the summary
- **THEN** the summary says the key will be saved to `<config folder>/token` and holds no part of the key

### Requirement: Home-manager module
The flake SHALL export `homeManagerModules.default` with `programs.bilbo.enable`, `package`, `storeRoot` (a path exported as `BILBO_HOME`, or null for the default root), `settings` (embedder, digest, history, scope and sync keys to string values; a scope key is accepted only in the shape the `config` spec's Scope settings allow), `index.enable`, `index.every`, `watch.enable` (true by default), `claude` and `codex` (a path, or null for PATH), and `localEmbedder.enable`, `localEmbedder.port` and `localEmbedder.llamaServer` (nixpkgs' `llama-server` by default). When enabled, it SHALL install the package, write `settings` as the config file, and on activation run `bilbo setup --yes` with the matching flags, `--no-watch` among them when `watch.enable` is false. With `localEmbedder.enable`, the settings' URL and model SHALL default to the local embedder's, and activation SHALL pass `--embedder-local`, `--embedder-port` and `--llama-server`; an `embedder.url` other than the local one SHALL fail evaluation. Activation does not read session variables, so the module SHALL pass the locations explicitly: `BILBO_HOME` from `storeRoot` (unset when null), `BILBO_CONFIG` unset, and `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` from home-manager's `xdg` folders. It SHALL also put `launchctl` (macOS) or `systemctl` (Linux) on the PATH it gives `setup`. A key in `embedder.token_env` with `index.enable` SHALL fail evaluation. The module SHALL work without the flake's `home-manager` input, which only its flake check reads.

#### Scenario: Settings become the config
- **WHEN** a configuration sets `programs.bilbo.settings."embedder.url" = "http://embedder.example:8081"` and `"embedder.model" = "qwen3"`
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
- **WHEN** a configuration sets `programs.bilbo.localEmbedder.enable = true` and `programs.bilbo.settings."embedder.url" = "http://embedder.example:8081"`
- **THEN** evaluation fails with a message naming `localEmbedder.enable` and `embedder.url`

### Requirement: Codex hook trust
When the codex step leaves `bilbo@bilbo` installed (`installed`, `updated` or `kept`), `setup` SHALL ask Codex, through `codex app-server`, which hooks the bilbo plugin registers and whether Codex trusts them, and SHALL mark every untrusted or changed bilbo hook trusted through Codex's own config writer, never by editing Codex's files itself. The hook line SHALL say `installed: trusted in Codex` when it trusted a hook Codex had never trusted, `updated: trusted in Codex` when the hook had changed since it was trusted, `kept: trusted in Codex` when every bilbo hook was already trusted, `skipped: no codex plugin` when the codex step did not leave the plugin installed, `skipped: codex lists no bilbo hook` when Codex lists none, `failed: <message>` when Codex cannot be asked or cannot write its config, and `failed: codex reports trust status '<status>' for a bilbo hook` when Codex lists a bilbo hook with a status other than `untrusted`, `modified`, `trusted` or `managed`, in which case `setup` SHALL write no trust. The wizard's summary SHALL name the trust whenever it installs the Codex plugin. `--remove` SHALL delete, through the same writer, every trust entry whose hook key belongs to `bilbo@bilbo`, and its hook line SHALL say `removed`, `skipped: not trusted`, `skipped: not found` when there is no `codex`, or `failed: <message>`.

#### Scenario: A fresh install trusts the hook
- **WHEN** `codex` is on PATH with no `bilbo` marketplace and a user runs `bilbo setup --yes`
- **THEN** the codex line says `installed`, the hook line says `installed: trusted in Codex`, and Codex runs the digest hook and the compaction hook without asking for a review

#### Scenario: A rerun keeps the trust
- **WHEN** setup already trusted the hook and the user runs `bilbo setup --yes` again
- **THEN** the hook line says `kept: trusted in Codex` and Codex's config is not written

#### Scenario: A changed hook is trusted again
- **WHEN** a new bilbo release changes the hook's command, so Codex lists it as changed since it was trusted
- **THEN** setup writes the new trust and the hook line says `updated: trusted in Codex`

#### Scenario: One hook is untrusted
- **WHEN** Codex lists the digest hook as trusted and the compaction hook as untrusted
- **THEN** setup writes trust for the compaction hook only, leaves the digest hook's trust as it was, and the hook line says `installed: trusted in Codex`

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

### Requirement: Watch service
Unless `--no-watch` is given or the wizard's answer declines it, `setup` SHALL install a login service that runs `<absolute path of this bilbo, links resolved> watch`, starts at login, restarts when it exits, and appends its output to `<state>/bilbo/watch.log`. On macOS this SHALL be the launchd agent `io.github.delucca.bilbo.watch`, run at load and kept alive. On Linux it SHALL be the systemd user service `bilbo-watch.service` with `Restart=on-failure`, enabled and started. The service SHALL carry as its environment each of the six locations the `Index timer` requirement lists that `setup` itself saw set to an absolute path, and no other variable. The index timer's key rule SHALL NOT apply: a key in an environment variable SHALL NOT fail the watch step. The watch line SHALL say `installed: watching <root>/notes`, `kept` when the service is already as wanted, and `updated` when its file changes, as when the binary moved. When no watcher is wanted and one exists, `setup` SHALL unload and delete it and report `removed` with the reason: `--no-watch`, or `not chosen` when the wizard's answer declines it. The wizard SHALL ask whether to record note history in the background, defaulting to yes.

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

#### Scenario: Installing the watcher without the service manager
- **WHEN** a user on macOS runs `bilbo setup --yes` with no `launchctl` on PATH
- **THEN** the watch line says `failed: launchctl not found on PATH`, no plist is written, and the exit code is 1

### Requirement: Sync step
For each scope whose `sync` is a URL, the `sync` step SHALL check that the device has a key, that the watcher is wanted and that the transport answers: for a `file://` URL, that the folder exists and is writable; for an `https://` or loopback `http://` URL, that an unsigned `GET <url>/v1/` answers 200 with the relay identification of the `relay-api` spec's The API root. It SHALL then report `ok: <name> through <url> (<n> notes)`, with `note` when `<n>` is 1, scopes joined by `, `. A failed check SHALL report `failed: sync needs the watcher; drop --no-watch`, `failed: <url> is not reachable: <reason>`, or, for a relay URL that answers without that identification, `failed: <url> is not a bilbo relay`. With no device key it SHALL report `skipped: no device key; run bilbo device init in a terminal`, and with no syncing scope `skipped: no scope syncs`. It SHALL write nothing.

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
- **WHEN** the config holds `scope.personal.sync = https://relay.example`, the device has a key, 12 notes have `scope: personal`, and `GET https://relay.example/v1/` answers 200 with `{"relay":"bilbo","api":1}`
- **THEN** the sync line says `ok: personal through https://relay.example (12 notes)` and the exit code is 0

#### Scenario: One note
- **WHEN** the config holds `scope.work.sync = file:///srv/work`, the device has a key, 1 note has `scope: work` and `/srv/work` exists
- **THEN** the sync line says `ok: work through file:///srv/work (1 note)`

#### Scenario: A missing folder
- **WHEN** the config holds `scope.personal.sync = file:///Volumes/usb/bilbo` and that volume is not mounted
- **THEN** the sync line says `failed`, names the URL, no folder is created, and the exit code is 1

#### Scenario: A URL that is not a relay
- **WHEN** the config holds `scope.personal.sync = https://example.org`, whose `GET /v1/` answers 404
- **THEN** the sync line says `failed: https://example.org is not a bilbo relay` and the exit code is 1

#### Scenario: A relay that is down
- **WHEN** the config holds `scope.personal.sync = https://relay.example` and nothing answers there
- **THEN** the sync line says `failed: https://relay.example is not reachable: <reason>`, no request is signed, and the exit code is 1

### Requirement: Turning sync on in the wizard
After the watcher's question, the wizard SHALL ask whether to sync notes between devices, yes by default only when a scope already syncs. On yes, it SHALL ask for the scope (the declared ones and `personal`) and where to sync: a folder (absolute or starting with `~/`, its parent existing) or a relay URL (`https://`, or `http://` to a loopback host) whose unsigned `GET <url>/v1/` answers the relay identification. Then, when the device has no key, ask `Do you already have a recovery phrase from another device?` and run the ceremony `bilbo device recover` runs on yes, or `bilbo device init` on no, under the same terminal and agent rules, those of the `device-identity` spec's Terminal-only forms. Against a managed config it SHALL only show the syncing scopes.

#### Scenario: First device
- **WHEN** a user with no device key answers yes, picks `personal`, enters `~/Dropbox/bilbo`, writes down the new phrase, types its 3 words back and confirms
- **THEN** the config sets `scope.personal.sync = file:///Users/a/Dropbox/bilbo`, the folder exists, a device key exists, and the sync line says `ok: personal through file:///Users/a/Dropbox/bilbo (0 notes)`

#### Scenario: Second device
- **WHEN** a user on a second machine answers yes, picks `personal`, enters the same synced folder and types the first device's phrase
- **THEN** the config and the key are written, and once the watcher polls, the scope's manifest lists both devices

#### Scenario: First device on a relay
- **WHEN** a user with no device key answers yes, picks `personal`, enters `https://relay.example`, whose relay was started with the fingerprint the new phrase derives, writes down the phrase, types its 3 words back and confirms
- **THEN** the config sets `scope.personal.sync = https://relay.example`, a device key exists, the sync line says `ok: personal through https://relay.example (0 notes)`, and once the watcher runs the relay holds `personal`'s manifest 1

#### Scenario: Enrolled, and the folder holds a scope it cannot open
- **WHEN** a device that holds keys, is in no scope on the folder and has no manifest of `personal` turns sync on for `personal`, and the folder holds a scope of the same owner that this device cannot open
- **THEN** the wizard mints no scope id, says `the folder holds scopes of this owner that this device cannot open; run bilbo device recover on this device`, and the sync line says `skipped`

#### Scenario: A folder whose parent is missing
- **WHEN** the user enters `/nope/bilbo` and `/nope` does not exist
- **THEN** the wizard says the parent folder does not exist and asks again

#### Scenario: Under Claude Code
- **WHEN** `CLAUDE_CODE_CHILD_SESSION=1` or `CODEX_THREAD_ID` is set and the device has no key, and the user answers yes
- **THEN** the wizard says the phrase is shown only in a terminal outside an agent, turns sync off for this run, and the sync line says `skipped: no device key; run bilbo device init in a terminal`

#### Scenario: In an IDE terminal
- **WHEN** only `CLAUDECODE=1` is set, the device has no key, and the user answers yes in a terminal
- **THEN** the wizard runs the phrase ceremony as it does with no marker

#### Scenario: Declining sync
- **WHEN** the user answers no
- **THEN** the config gains no sync setting and the sync line says `skipped: no scope syncs`

#### Scenario: A URL that is not a relay
- **WHEN** the user enters `https://example.org`, whose `GET /v1/` answers 404
- **THEN** the wizard says `https://example.org is not a bilbo relay` and asks again

#### Scenario: Plain HTTP to another host
- **WHEN** the user enters `http://relay.example:8738`
- **THEN** the wizard says plain `http://` reaches only a loopback host, sends no request, and asks again

### Requirement: Applying sync from the wizard
After the summary is confirmed, and only then, the wizard SHALL write `scope.<name>.sync` with the folder's `file://` URL or the relay URL to the config, create a folder's last component when missing, copy into the store only the manifests of the phrase's owner whose sealed name is the picked scope, fetched from the folder or the relay as `bilbo device recover` fetches them, and write the keys and manifests as `bilbo device init` or `recover` does, so a second device joins that scope and no other. It SHALL mint a scope id only when the folder or relay holds no scope with the picked name and, unless this device is in a scope on that transport, no scope of this owner that the keys in hand cannot open.

#### Scenario: The key waits for the confirmation
- **WHEN** a user picks the folder `~/Sync/bilbo`, confirms a new phrase and then presses Ctrl-C at the summary
- **THEN** no device key, no `~/Sync/bilbo` and no config change exist

#### Scenario: Only the picked scope
- **WHEN** the folder holds this owner's `personal` and `shared`, and the user picks `personal` and types the phrase
- **THEN** only `personal`'s manifest is in the store, `bilbo device` lists this device in `personal` only, and no version of `shared` lists it

#### Scenario: Every device lost, through the wizard
- **WHEN** every device is lost, and on a new machine the user runs `bilbo setup`, turns sync on for `personal` with `https://relay.example`, types the phrase and confirms
- **THEN** `personal`'s manifests are fetched from the relay with the owner key, the device is added as the next version, the sync line says `ok` for `personal`, and no new scope id exists

### Requirement: Human view of the step report
When stdout gets the `cli` spec's human view, the step report SHALL print one line per step in the same order: a mark for its status, `■` for `failed`, `○` for `skipped`, `◇` for `kept` and `◆` for any other status, then the step, the status and the detail in aligned columns, the detail's paths under the home folder starting with `~/`. A blank line and a summary SHALL follow: `■  <n> of <m> steps failed` when a step failed, else `◆  Setup done: ` and the number of steps of each status, in the order the statuses first appear, joined by `, `. This holds after the wizard too. The exit code SHALL be as in Step report.

#### Scenario: A rerun on a terminal
- **WHEN** a user runs `bilbo setup --yes` in a terminal on an installed machine with no embedder and no syncing scope
- **THEN** each step line starts with `◇` or `○`, the store line shows its path from `~/`, and the last line starts with `◆  Setup done: ` and holds `kept` and `skipped`

#### Scenario: A failed step on a terminal
- **WHEN** a user runs `bilbo setup --yes` in a terminal with an embedder configured and `launchctl` is not on `PATH`
- **THEN** the timer and watch lines start with `■`, the last line is `■  2 of 12 steps failed`, and the exit code is 1

#### Scenario: The home-manager module reads the plain report
- **WHEN** the home-manager module runs `bilbo setup --yes` with stdout not a terminal
- **THEN** stdout is the `<step> <status>: <detail>` lines of Step report
