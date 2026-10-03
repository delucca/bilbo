# Design

## Context

add-distribution ships the binary three ways (installer, Nix package, `cargo build`) and puts the plugin marketplace under the package's `share/bilbo/`. Verbs today never write outside the store and the cache, never prompt, and never run another program. `setup` does all three, so the decisions below are mostly about keeping those effects contained and testable.

Behavior of the agent CLIs, checked on 2026-10-02 against throwaway `CLAUDE_CONFIG_DIR` and `CODEX_HOME` folders (Claude Code 2.1.288, codex-cli 0.155.1), and checked again the same evening while planning:

- **Claude Code `claude plugin marketplace add <source> --json`.**
  - It accepts a folder path and `owner/repo#ref`.
  - It prints one JSON line on stdout with an `outcome` of `ok` or `failed`. A failure also prints `✘ <message>` on stderr and exits 1.
  - A missing ref exits 1 with `failureCode: "error_not_found"`. A folder without `.claude-plugin/marketplace.json` exits 1 with `manifest_missing`.
- **Claude Code refuses to re-add a marketplace from a different source** (`failureCode: "declared"`): from a folder to GitHub, and from one GitHub ref to another. Going from GitHub to a folder re-points it in place. bilbo always removes it first, which works in every direction. Removing a marketplace also uninstalls its plugins.
- **`claude plugin marketplace list --json`** gives `name` and `source`. That is `directory` with `path` for a folder, or `github` with `repo` and `ref`.
- **`claude plugin list --json`** gives `id`, `enabled`, and a `version` that is the commit SHA, or `unknown` for a folder outside git.
- **`claude plugin install`** re-enables a disabled plugin and succeeds with no change when it is already installed, so `enable` is never needed. `uninstall` of a missing plugin fails with `not_installed`, and `marketplace remove` of a missing marketplace with `not_configured`.
- **Codex `codex plugin marketplace add <source> [--ref <ref>] --json`** prints JSON with `alreadyAdded` on success. It now refuses a different source, including the same repository at another ref: `Error: marketplace 'bilbo' is already added from a different source; remove it before adding this source` on stderr, exit 1, no JSON. Every Codex failure is an `Error: ...` line on stderr and exit 1.
- **`codex plugin marketplace list --json`** gives `name`, `root` and `marketplaceSource` (`sourceType` `git` with `source` `https://github.com/<owner>/<repo>.git`, or `local` with the folder) but not the ref.
- **`codex plugin list --json`** gives `installed[]` with `pluginId`, `enabled` and the installed `version`, taken from `.codex-plugin/plugin.json`. That equals `Cargo.toml`'s version by the `agent-plugin` spec. `codex plugin add` re-enables a disabled plugin. `codex plugin remove` succeeds when nothing is installed; `marketplace remove` of a missing marketplace exits 1.
- **Plugin commands need no login** in either tool.
- The recorded outputs live in `tests/fixtures/agents/`.

dnix already runs launchd agents through home-manager (`nbrecall-index`, `StartInterval`, logs under `~/.local/log`). The legacy query prefix is `Instruct: Given a question, retrieve notes that answer it\nQuery: `.

## Goals / Non-Goals

**Goals:**
- **One implementation.** The wizard, the flags and the home-manager module all produce the same `Plan`, and one `apply` executes it.
- **Testable without a terminal and without the real agent tools.**
- **Nothing written until the plan is complete and confirmed.**

**Non-Goals:**
- **A general settings editor.** The wizard covers the embedder keys only. Other keys are edited in the file.
- **Repairing a broken agent install.** When `claude` itself fails, setup reports it and moves on.

## Decisions

### Plan, then apply

`setup` runs in three stages:

1. **Gather.** Read flags or ask questions, and read the current state: the store, the config, each tool's marketplace and plugin, and the timer file.
2. **Plan.** Build a `Plan`: a list of steps, each with the action it will take (`Create`, `Write`, `Keep`, `Replace`, `Skip(reason)`). The embedder check runs here, the last thing before the plan is complete.
3. **Apply.** Run the plan in step order and collect one report line per step.

The wizard only fills the inputs that stage 1 needs, and the summary screen renders the `Plan`. Non-interactive mode builds the same inputs from flags and defaults. This gives "Plan before writing", "Reruns change nothing" and the shared implementation in one structure.

`Keep` is decided by comparing the planned state with the current state, not by remembering what an earlier run did. No state file is needed.

### cliclack for the wizard

The maintainer chose a rich wizard: arrow-key selects, multiselect for the agents, masked key input, a spinner for the embedder check and the first index. cliclack 0.5.6 provides exactly these widgets in the style chosen, on top of `console` and `indicatif`. It also brings `zeroize`, which clears the pasted key's buffer.

Alternatives:
- **Plain `std::io` prompts.** No dependencies, but no arrow keys, no masking without raw mode, and no spinner. Rejected by the maintainer.
- **inquire.** Comparable widgets on crossterm, a heavier dependency tree, and a different look.
- **dialoguer.** Same `console` base and widely used, but no intro/outro framing or log styling. We would rebuild what cliclack already has.

Containment:
- cliclack is used only in `src/wizard.rs`, behind a `Prompter` trait with `select`, `multiselect`, `input`, `password`, `confirm` and `spin`.
- Unit tests drive the wizard with a scripted `Prompter`, so every wizard scenario, including Ctrl-C at any prompt, runs without a terminal.
- The cliclack adapter is the only untested layer. Task 7.2's smoke run covers it.

Checked against the cliclack 0.5.6 docs (Context7 `/fadeevab/cliclack`) and source on 2026-10-02:
- Every prompt draws on `Term::stderr()`, and so do `intro`, `outro`, `note`, `log::*` and the spinner. This keeps stdout for the report, as the modified `Output streams` requirement says.
- A prompt reads raw keys, so Ctrl-C arrives as a key, not a signal. Ctrl-C and Esc make `interact()` return `io::ErrorKind::Interrupted`, and the wizard maps that to "abort, write nothing, exit 1". A prompt on a stream that is not a terminal returns `NotConnected`.
- No `ctrlc` handler is needed. A prompt hides the cursor only inside `interact()` and shows it again before returning, also on cancel. Spinners do not hide it. Ctrl-C outside a prompt, during a spinner, is an ordinary SIGINT that ends the process: before the confirmation nothing has been written; during apply it stops setup as it would any command, and a rerun finishes the job; the index run under the first-index spinner saves its cache atomically.
- cliclack 0.5.6 is the latest release and builds on Rust 1.95 in the dev shell.
- `console` switches the tty to raw mode only inside each key read (`unix_term.rs:346-354`) and restores cooked mode, with ECHO on, between reads, while cliclack redraws. A key pasted or typed during that gap is echoed in clear before cliclack draws it as `▪`. The smoke run caught this. The `password` adapter therefore clears ECHO on `/dev/tty` before drawing the prompt, and a Drop guard restores the saved termios on every exit path (Ok, Interrupted, error). The guard does nothing without a tty, stays in the adapter, and uses `libc`, already in the lockfile. One window stays open: ISIG is on between key reads, so a SIGINT that lands mid-redraw ends the process before the guard restores ECHO. The window is microseconds wide, and zsh resets the tty at its next prompt; `stty echo` repairs it elsewhere.

### serde_json and zeroize become direct dependencies

Reading `claude … --json` and `codex … --json` needs a JSON parser. `serde_json` is already in the lockfile through `ureq`'s `json` feature and is a dev-dependency at 1.0.151. Promoting it adds no crate. add-note-digest plans the same promotion.

`zeroize` 1.9.0 is already in the lockfile through `rustls`. cliclack's `password` returns a plain `String`, so the wizard wraps it in `zeroize::Zeroizing` itself, which needs the crate declared. It adds no crate either.

### Agents are subprocesses behind a small runner

`src/command.rs` holds a `Runner` (program, args → exit code, stdout, stderr), its real implementation on `std::process::Command` with stdin closed and the environment inherited, and the PATH lookup. `src/agents.rs` and `src/timer.rs` both use it, which is why it is its own module rather than part of either.
- `agents` always passes `--json`. It judges a Claude Code command by the JSON `outcome`, and a Codex command by its exit code.
- A tool's message is cut to its first line for the report: the JSON `message` for Claude Code, else the first stderr line without a leading `Error: ` or `✘ `.
- Unit tests in `agents` and `timer` drive a scripted `Runner`, so both platforms' timer commands are tested on any host.

The integration tests do not use the trait. They put fake `claude`, `codex`, `launchctl` and `systemctl` shell scripts on a PATH that holds nothing else. The fakes keep their state in files, log their arguments and print JSON in the shapes recorded above, so the tests also cover PATH lookup and argument building. They use only shell builtins, because their PATH has no `cat` or `rm`.

Planned commands per tool:

| State | Claude Code | Codex |
|---|---|---|
| no marketplace | `marketplace add <src>`, `install bilbo@bilbo` | `marketplace add <src> [--ref]`, `add bilbo@bilbo` |
| marketplace from another source | `marketplace remove bilbo`, then as above | `marketplace remove bilbo`, then as above |
| same GitHub repository, installed `version` not the binary's | (cannot happen: the ref is part of the source) | `marketplace remove bilbo`, then as above, since the ref is unknown |
| same source, plugin missing or disabled | `install bilbo@bilbo` | `add bilbo@bilbo` |
| same source, plugin enabled | nothing (`kept`) | nothing, when the installed `version` equals the binary's |

"Same source" for Claude Code means `path` equals the folder, or `repo` and `ref` equal the planned ones. For Codex it means `marketplaceSource.source` equals the folder or `https://github.com/<owner>/<repo>.git`, plus the version check, since Codex does not report the ref. The step reports `installed` when the tool had no `bilbo` marketplace and `updated` otherwise.

`--remove` runs `plugin uninstall bilbo@bilbo` (Codex: `plugin remove`) when the plugin is listed, then `plugin marketplace remove bilbo` when the marketplace is listed.

### Plugin source from the binary's location

`std::env::current_exe()`, with links resolved, gives `<prefix>/bin/bilbo`. If `<prefix>/share/bilbo/.claude-plugin/marketplace.json` exists, the source is that folder. This holds for the Nix package and for any future package that copies the same layout. Otherwise the source is `delucca/bilbo` at `v<CARGO_PKG_VERSION>`. A local `cargo build` has no tag for an unreleased version, which is why the failure hint names `--plugin-source`, for example `--plugin-source ~/Developer/delucca/bilbo`.

### Timer files are written by bilbo, loaded by the OS tool

- **macOS.** Write `~/Library/LaunchAgents/io.github.delucca.bilbo.index.plist` with these keys, as the legacy agent has: `Label`, `ProgramArguments` `[<exe>, "index"]`, `StartInterval` in seconds, `RunAtLoad` false, `StandardOutPath` and `StandardErrorPath` pointing at `<state>/bilbo/index.log`, and `EnvironmentVariables` when there are locations to carry. Then reload: `launchctl bootout gui/<uid>/<label>`, ignoring exit 3 (`No such process`) and 113 (not found), followed by `launchctl bootstrap gui/<uid> <plist>`. A second `bootstrap` without the `bootout` fails with exit 5. `RunAtLoad` is false so that setup itself never starts a long first index. The wizard offers that index separately. The uid comes from `/usr/bin/id -u`, as home-manager's own `$UID`; `launchctl manageruid` would name root's manager under `sudo -u`.
- **Linux.** Write `bilbo-index.service` (`Type=oneshot`, `Environment=` lines, `ExecStart=<exe> index`, output appended to the log) and `bilbo-index.timer` (`OnActiveSec=<n>min` and `OnUnitActiveSec=<n>min`, `WantedBy=timers.target`) under `$XDG_CONFIG_HOME/systemd/user`, else `~/.config/systemd/user`. `OnActiveSec` makes the first run come `<n>` minutes after the timer starts, as launchd's `RunAtLoad` false does; the earlier `OnBootSec=5min` would fire at once on a machine up for more than five minutes, and `Persistent=` only applies to calendar timers. Then run `systemctl --user daemon-reload`, `systemctl --user enable bilbo-index.timer` and `systemctl --user restart bilbo-index.timer`, so a changed interval takes effect. If `systemctl` is not on PATH, or `systemctl --user is-system-running` prints none of `running`, `degraded`, `starting`, `initializing` or `maintenance`, the step is skipped rather than failed, because containers and WSL often have no user manager.
- **Environment.** A launchd agent or a systemd user unit starts with the service manager's environment, not the shell's. Without help, `index` would miss a `BILBO_HOME` or `BILBO_CONFIG` the user exported and read another store or config, and with a non-default `XDG_CACHE_HOME` it would fill a cache `recall` never reads. So the job carries each of `BILBO_HOME`, `BILBO_CONFIG`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` that `setup` saw set to an absolute path, in that order, and nothing else. A value that is not UTF-8, or that holds a newline, fails the step.
- **A key in a variable.** The same applies to `embedder.token_env`: the job would not see it. Copying the key into the unit would put a secret in a world-readable file, so the step fails instead, with `failed: the index timer cannot read the key variable <VAR>; keep the key in a file (--embedder-token-file) or pass --no-timer`. Existing timer files are left as they are.
- **Removal.** When no timer is wanted (`--no-timer`, the wizard's choice, or no embedder) and the timer files exist, setup unloads (`bootout`, or `systemctl --user disable --now bilbo-index.timer`) and deletes them, reporting `removed: <reason>`. A timer that runs `index` with no embedder would fail every 15 minutes, and home-manager's `index.enable = false` must not leave the old one running.
- **Lookup.** `launchctl` and `systemctl` are found on PATH, never at a fixed path, so the fakes on the tests' PATH stand in for them and a test can never load a real job. home-manager's activation PATH holds neither, so the module adds them (see below).
- **Log folder.** setup creates `<state>/bilbo/` before loading. launchd on macOS 26 creates a missing log folder itself, with mode 0744, but systemd fails the unit with status 209.
- The file content is a pure function of the exe path (links resolved), the interval, the log path and the carried locations, all in `src/timer.rs`. "Kept" therefore means the generated text equals the file on disk. A run from a shell with different locations than a home-manager activation rewrites the timer, which is correct: the timer must match the last setup.
- `<state>` is `$XDG_STATE_HOME` when that is absolute, otherwise `~/.local/state`, the same rule as the other folders. `store::Env` gains `xdg_state_home`.

### The key file

- **Path.** `<config folder>/token` is opened with `OpenOptions::new().write(true).create_new(true).mode(0o600)` under a temporary name and renamed over the target. The mode is therefore set before the first byte, and a crash never leaves a half-written key.
- **Config reference.** The config gets `embedder.token_file = <absolute path>`.
- **Memory.** The key `String` is wrapped in `zeroize::Zeroizing` from the moment the `Prompter` returns it.
- **Never passed on.** It reaches no other program, no timer file and no output. The embedder check uses `embed.rs`, whose messages already never hold the token (`config` spec). A pasted key is not on disk while the plan is built, so `embed::Client` gains a constructor that takes the key in memory.

### Config writing

The writer emits a fixed header (`# bilbo config, written by bilbo setup <version> on <date>`), then the settings in the order of `config::KEYS`. A value is quoted when it is empty, starts or ends with a space or a tab, starts with `"`, or holds a newline or a carriage return; inside quotes `\`, newline and `"` are escaped as `\\`, `\n` and `\"`. An unquoted value escapes only `\`, because `\n` is an escape in any value. A unit test round-trips values through `config::parse`. An empty query prefix writes no line, and `embedder.min_similarity` is written only when it differs from 0.5. A rewrite renames the old file to `<name>.bak` (`config.bak` for the default name), replacing any older one.

A config is treated as managed when `symlink_metadata` says it is a link, or when its folder exists and has no write bit set. Home-manager links the file into the store, which is the case this exists for. The folder test reads the mode instead of probing with a write, so that planning writes nothing; a folder that is writable by mode but not by this user fails at apply as `config failed: <error>`.

Non-interactive embedder flags against an existing config compare the planned URL, model, key source and query prefix with the file's. Equal settings keep the file, so a script can rerun the same command; different ones exit 1 before writing. A config that sets no key (only comments and blank lines, what `setup --yes` writes without an embedder) holds nothing to compare or lose: the flags rewrite it, the old file becomes `config.bak`, the line says `updated` and the embedder check runs as for a new config.

### The home-manager module is a wrapper

```nix
programs.bilbo = {
  enable = true;
  storeRoot = null;          # a path → BILBO_HOME in the session and for setup
  settings = { "embedder.url" = "http://bagend:8081"; "embedder.model" = "qwen3-embedding-0.6b"; };
  index.every = 15;          # index.enable = false → --no-timer
  claude = "/path/to/claude"; # null → PATH at activation
  codex = null;
};
```

- `settings` is a submodule with one `nullOr str` option per known key, so an unknown key fails evaluation with the module system's own message, `The option programs.bilbo.settings."embeder.url" does not exist`. Values are rendered in `config::KEYS` order with the same quoting rule as the Rust writer.
- `xdg.configFile."bilbo/config".text` holds the rendered settings under a `# bilbo config, written by home-manager from programs.bilbo.settings` header.
- An assertion rejects `settings."embedder.token_env"` while `index.enable` is true, for the reason in the timer section.
- `home.activation.bilboSetup = lib.hm.dag.entryAfter [ "writeBoundary" "setupLaunchAgents" ] "run env … ${lib.getExe cfg.package} setup --yes …"`.
  - **Locations.** Activation does not source `hm-session-vars.sh`, and it inherits whatever environment started it: a user's shell for `home-manager switch`, a near-empty `sudo -u` environment under nix-darwin. So the command sets them itself, the same on every run: `env -u BILBO_HOME -u BILBO_CONFIG`, then `BILBO_HOME=<storeRoot>` when set, and `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` from `config.xdg.*Home`. Those equal bilbo's own defaults when the user changed nothing, equal the session's values when `xdg.enable` is on, and always name the folder the module wrote the config to. `setup` then finds that config, and the timer carries the same values. `storeRoot` also sets `home.sessionVariables.BILBO_HOME`, so the shell, `setup` and the timer agree on the store.
  - **PATH.** With `stateVersion` 22.11 or later, activation's PATH holds only bash, coreutils and a few other tools: no `claude`, `codex`, `launchctl` or `systemctl`. That is why `claude` and `codex` are options; dnix passes the paths of its own binaries. The command appends `/usr/bin:/bin` on macOS, and on Linux the folder of `config.systemd.user.systemctlPath` plus `XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"`, as home-manager's own `reloadSystemd` does.
  - A failing setup prints its report and does not abort activation: `|| warnEcho "bilbo setup reported a failed step; see its lines above"`. A broken plugin install must not block a system switch.
- On a rollback, activation runs the old generation's `bilbo setup`, which re-points the timer and the marketplaces to that version.
- **The flake input.** The flake gains `home-manager` (`github:nix-community/home-manager/release-26.05`, `inputs.nixpkgs.follows = "nixpkgs"`), read only by the `home-manager-module` check. The module itself uses only `lib.hm`, which home-manager supplies to every module. The check builds four configurations with `home-manager.lib.homeManagerConfiguration`: sample settings (asserting the rendered config text and the activation command), a misspelled key and a `token_env` key (both must fail evaluation, checked with `builtins.tryEval`), and a disabled module (no activation entry, no config file). It discards the activation text's string context, so evaluating it never builds the package. A consumer such as dnix adds `bilbo.inputs.home-manager.follows = "home-manager"` to keep one copy.

### The first index runs as a subprocess

Verbs never build on each other, so the wizard's first-index offer runs `<current_exe> index` as a child process with the same environment, under a spinner, and shows its stdout line, or its stderr lines without the `bilbo: ` prefix. This also gives the index its own 600-second client and the exact behavior a timer run has.

### Embedder checks

- Non-interactive mode checks the embedder only when it writes a new config. An existing or managed config is not checked (`skipped: config kept`), so a rerun sends no request.
- The wizard checks whatever embedder its answers leave, including unchanged settings, because a person is there to act on the failure menu.

## Risks / Trade-offs

- **The agent CLIs' JSON can change shape between releases.** → The parser reads only the fields listed above, an unknown shape fails that step with "could not read `claude plugin list --json`", and the fakes' recorded outputs are refreshed when a field moves.
- **Removing Claude Code's marketplace uninstalls the plugin and deletes its saved data.** → bilbo's plugin keeps no data or options today. If it ever does, the update path has to move to `marketplace update` instead.
- **Activation runs network commands** for a GitHub source. → On Nix the source is always the local `share/bilbo` folder, so activation never clones.
- **cliclack's dependency tree.** → It is used in one module. If it ever has to be dropped, only `wizard.rs`'s adapter changes.
- **The 15-second embedder check can be slow on a CPU embedder's first load.** → The wizard shows a spinner and offers a retry. The 15 s limit has not been measured against a cold model load. The smoke run in task 7.2 measures it against bagend, and the limit changes if that run misses it.

## Migration Plan

- On rivendell, after the dnix commit, the legacy `nbrecall-index` agent keeps running until the cutover. Both are cheap.
- The bilbo timer only indexes bilbo's store, which stays empty until the notes move, so the two never touch the same files.
