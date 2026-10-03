# Spec Delta

## Purpose

How `bilbo setup` turns a `bilbo` binary on PATH into a working install: the store folder, the config and embedder key, the agent plugin at the binary's own version, and the index timer. It covers both the interactive wizard and the non-interactive mode that scripts and the home-manager module use.

## ADDED Requirements

### Requirement: Modes
`bilbo setup` SHALL run the interactive wizard when stdin and stderr are both terminals and no flag that answers a wizard question is given, and run non-interactively otherwise. The answer flags are `--embedder-url`, `--embedder-model`, `--embedder-token-env`, `--embedder-token-file`, `--embedder-query-prefix`, `--no-plugin`, `--no-timer` and `--index-every`. `--yes` SHALL force non-interactive mode. `--interactive` SHALL force the wizard and SHALL be a usage error when stdin or stderr is not a terminal. `--interactive` together with `--yes` or with an answer flag SHALL be a usage error.

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

### Requirement: Setup flags
`setup` SHALL accept `--yes`, `--interactive`, `--remove`, `--embedder-url <url>`, `--embedder-model <name>`, `--embedder-token-env <var>`, `--embedder-token-file <path>`, `--embedder-query-prefix <text>`, `--no-plugin`, `--claude <path>`, `--codex <path>`, `--plugin-source <source>`, `--no-timer` and `--index-every <minutes>`. Values SHALL obey the `config` spec's rules for the matching key. Any other argument SHALL be a usage error. No flag SHALL take an API key's value.

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

### Requirement: Plan before writing
`setup` SHALL settle every answer, read the current state and run the embedder check before it creates, changes or deletes any file. The wizard SHALL then show a summary of the actions and ask for one confirmation. Declining, pressing Ctrl-C or Esc at any prompt, or reaching end of input SHALL exit 1 and write nothing.

#### Scenario: The summary lists the actions
- **WHEN** a user answers every wizard prompt on a machine with no store, no config, and `claude` on PATH
- **THEN** the wizard shows a summary naming the store folder, the config file, the Claude Code plugin and the timer before asking to apply

#### Scenario: Declining writes nothing
- **WHEN** a user answers every prompt and then declines the summary
- **THEN** bilbo exits 1, and no store folder, config, key file or timer file exists and no agent command ran

#### Scenario: Ctrl-C midway writes nothing
- **WHEN** a user presses Ctrl-C at the key prompt
- **THEN** bilbo exits 1 and writes nothing

### Requirement: Step report
After applying, `setup` SHALL print to stdout one line per step, in the order `store`, `config`, `key`, `embedder`, `claude`, `codex`, `timer`, each line being `<step> <status>` optionally followed by `: <detail>`. The status SHALL be one of `created`, `written`, `kept`, `ok`, `installed`, `updated`, `removed`, `skipped` or `failed`. The exit code SHALL be 0 when no step failed and 1 otherwise. A failed step SHALL NOT stop the steps after it.

#### Scenario: A fresh non-interactive run
- **WHEN** a user runs `bilbo setup --yes --no-plugin` on a machine with no store and no config
- **THEN** stdout holds the lines `store created: <root>/notes`, `config written: <config path>`, `key skipped: no embedder`, `embedder skipped: none configured`, `claude skipped: --no-plugin`, `codex skipped: --no-plugin` and `timer skipped: no embedder`, in that order, and the exit code is 0

#### Scenario: One failing step
- **WHEN** `claude plugin install` exits 1 during `bilbo setup --yes`
- **THEN** the `claude` line says `failed` with the first line of the tool's message, the `codex` and `timer` steps still run, and the exit code is 1

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
An existing config SHALL be kept. Non-interactive `setup` given embedder flags SHALL rewrite a config that sets no key at all (only comments and blank lines, as `bilbo setup --yes` writes with no embedder): the old file becomes `config.bak`, the config line says `updated`, and the embedder check runs as for a new config. Against a config that sets any key, embedder or not, it SHALL exit 1 before writing anything when the flags differ from the embedder settings in the file, and SHALL keep the file when they are equal. The wizard SHALL show the current embedder settings as defaults and SHALL rewrite the file only when the user changes one, renaming the old file to `config.bak` first and reporting `updated`.

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

#### Scenario: The wizard keeps everything
- **WHEN** a config exists and the user accepts every default in the wizard
- **THEN** the config line says `kept` and no `config.bak` is written

### Requirement: Managed config
When the config path is a symbolic link, or its folder is not writable, `setup` SHALL treat the config as managed elsewhere: it SHALL report `kept: managed elsewhere (<target>)`, the wizard SHALL show its settings without offering to change them, and embedder flags SHALL be a usage error naming the path.

#### Scenario: A Nix-managed config
- **WHEN** the config path is a link into `/nix/store` and the user runs the wizard
- **THEN** the wizard shows the embedder settings as read-only, and the config line says `kept: managed elsewhere (/nix/store/…)`

#### Scenario: Flags against a managed config
- **WHEN** the config path is a link and a user runs `bilbo setup --yes --embedder-url http://x:1 --embedder-model m`
- **THEN** bilbo prints a message naming the config path to stderr, exits 2, and writes nothing

### Requirement: Embedder choices
The wizard SHALL offer, in this order: no embedder (keyword search only), Ollama on this machine, OpenAI, and another OpenAI-compatible URL. Choosing one SHALL set the URL and the default model as follows, then ask for the model.

- None: no URL.
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
Before writing anything, `setup` SHALL send one embed request to the chosen embedder with its model and key, with a 15-second limit, and report `embedder ok: <dimensions> dimensions`. In the wizard, a failure SHALL show the reason and offer to retry, change the settings, or continue keyword-only. In non-interactive mode, a failure SHALL exit 1 and write nothing.

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
`bilbo setup --remove` SHALL unload and delete the timer, and in each tool found, uninstall `bilbo@bilbo` and remove the `bilbo` marketplace, printing one line per step in the order `store`, `config`, `key`, `claude`, `codex`, `timer`, each `removed`, `skipped` or `failed`. It SHALL keep the store, the config and the key file, and their lines SHALL say `skipped: kept <path>`. `--remove` SHALL accept only `--yes`, `--interactive`, `--claude` and `--codex` beside it. In a terminal it SHALL ask for confirmation first. With nothing installed it SHALL exit 0.

#### Scenario: Removing an install
- **WHEN** setup installed the timer and both plugins and the user runs `bilbo setup --remove --yes`
- **THEN** the timer file is gone and unloaded, neither tool lists a `bilbo` marketplace, the store, config and key file still exist, and stdout names their paths

#### Scenario: Nothing to remove
- **WHEN** nothing was installed and the user runs `bilbo setup --remove --yes`
- **THEN** every line says `skipped` and the exit code is 0

#### Scenario: Remove with setup flags
- **WHEN** a user runs `bilbo setup --remove --embedder-url http://x:1`
- **THEN** bilbo prints a message naming both flags to stderr, exits 2, and changes nothing

### Requirement: Secrets stay out of setup output
`setup` SHALL NOT print an API key, in full or in part, on stdout or stderr, in the wizard, in the summary or in the report, and SHALL NOT pass it to `claude`, `codex`, `launchctl` or `systemctl`. The summary SHALL name a key by its source: a variable name, a file path, or "pasted".

#### Scenario: The summary names the source only
- **WHEN** the user pastes a key and reaches the summary
- **THEN** the summary says the key will be saved to `<config folder>/token` and holds no part of the key

### Requirement: Home-manager module
The flake SHALL export `homeManagerModules.default` with `programs.bilbo.enable`, `package`, `storeRoot` (a path exported as `BILBO_HOME`, or null for the default root), `settings` (embedder keys to values), `index.enable`, `index.every`, `claude` and `codex` (a path, or null for PATH). When enabled, it SHALL install the package, write `settings` as the config file, and on activation run `bilbo setup --yes` with the matching flags. Activation does not read session variables, so the module SHALL pass the locations explicitly: `BILBO_HOME` from `storeRoot` (unset when null), `BILBO_CONFIG` unset, and `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` from home-manager's `xdg` folders. It SHALL also put `launchctl` (macOS) or `systemctl` (Linux) on the PATH it gives `setup`. A key in `embedder.token_env` with `index.enable` SHALL fail evaluation. The module SHALL work without the flake's `home-manager` input, which only its flake check reads.

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
