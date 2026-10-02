# Design

## Context

add-distribution ships the binary three ways (installer, Nix package, `cargo build`) and puts the plugin marketplace under the package's `share/bilbo/`. Verbs today never write outside the store and the cache, never prompt, and never run another program. `setup` does all three, so the decisions below are mostly about keeping those effects contained and testable.

Behavior of the agent CLIs, checked on 2026-10-02 against throwaway `CLAUDE_CONFIG_DIR` and `CODEX_HOME` folders (Claude Code 2.1.288, codex-cli 0.155.1):

- **Claude Code `claude plugin marketplace add <source> --json`.**
  - It accepts a folder path and `owner/repo#ref`.
  - It prints one JSON line with an `outcome` of `ok` or `failed`.
  - A missing ref exits 1 with `failureCode: "error_not_found"`.
- **Claude Code refuses to re-add a marketplace from a different source** (`failureCode: "declared"`). It has to be removed first. Removing a marketplace also uninstalls its plugins.
- **`claude plugin marketplace list --json`** gives `name` and `source`. That is `path` for a folder, or `repo` and `ref` for GitHub.
- **`claude plugin list --json`** gives `id`, `enabled`, and a `version` that is the commit SHA, or `unknown` for a folder outside git.
- **Codex `codex plugin marketplace add <source> [--ref <ref>]`** replaces an existing marketplace of the same name in place.
- **`codex plugin marketplace list --json`** gives `name`, `root` and `marketplaceSource` but not the ref.
- **`codex plugin list --json`** gives the installed `version`, taken from `.codex-plugin/plugin.json`. That equals `Cargo.toml`'s version by the `agent-plugin` spec.
- **Plugin commands need no login** in either tool.

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

Ctrl-C and Esc reach cliclack as an `io::ErrorKind::Interrupted` error from `interact()`, and the wizard maps that to "abort, write nothing, exit 1". Apply first checks the current docs for whether a `ctrlc` handler is needed to restore the cursor.

cliclack draws on stderr, which keeps stdout for the report as the modified `Output streams` requirement says. Apply confirms this as its first check, before writing the adapter.

### serde_json becomes a direct dependency

Reading `claude … --json` and `codex … --json` needs a JSON parser. `serde_json` is already in the lockfile through `ureq`'s `json` feature and is a dev-dependency at 1.0.151. Promoting it adds no crate. add-note-digest plans the same promotion.

### Agents are subprocesses behind a small runner

`src/agents.rs` runs `claude` and `codex` through a `Runner` (program, args, env → status, stdout, stderr).
- It always passes `--json` where the tool supports it, and judges each command by the JSON `outcome` or the exit code.
- A tool's message is cut to its first line for the report.

The integration tests do not use the trait. They put fake `claude` and `codex` shell scripts first on PATH. The fakes log their arguments and replay recorded JSON taken from the real outputs above, so the tests also cover PATH lookup and argument building.

Planned commands per tool:

| State | Claude Code | Codex |
|---|---|---|
| no marketplace | `marketplace add <src>`, `install bilbo@bilbo` | `marketplace add <src> [--ref]`, `add bilbo@bilbo` |
| marketplace from another source | `marketplace remove bilbo`, then as above | as above (add replaces) |
| same source, plugin missing or disabled | `install bilbo@bilbo` (or `enable`) | `add bilbo@bilbo` |
| same source, plugin enabled | nothing (`kept`) | nothing, when the installed `version` equals the binary's |

"Same source" for Claude Code means `path` equals the folder, or `repo` and `ref` equal the planned ones. For Codex it means `marketplaceSource.source` equals the folder or `https://github.com/<owner>/<repo>.git`, plus the version check, since Codex does not report the ref.

### Plugin source from the binary's location

`std::env::current_exe()`, with links resolved, gives `<prefix>/bin/bilbo`. If `<prefix>/share/bilbo/.claude-plugin/marketplace.json` exists, the source is that folder. This holds for the Nix package and for any future package that copies the same layout. Otherwise the source is `delucca/bilbo` at `v<CARGO_PKG_VERSION>`. A local `cargo build` has no tag for an unreleased version, which is why the failure hint names `--plugin-source`, for example `--plugin-source ~/Developer/delucca/bilbo`.

### Timer files are written by bilbo, loaded by the OS tool

- **macOS.** Write `~/Library/LaunchAgents/io.github.delucca.bilbo.index.plist` with these keys, as the legacy agent has: `Label`, `ProgramArguments` `[<exe>, "index"]`, `StartInterval` in seconds, `RunAtLoad` false, and `StandardOutPath` and `StandardErrorPath` pointing at `<state>/bilbo/index.log`. Then reload: `launchctl bootout gui/<uid>/<label>`, ignoring "not loaded", followed by `launchctl bootstrap gui/<uid> <plist>`. `RunAtLoad` is false so that setup itself never starts a long first index. The wizard offers that index separately.
- **Linux.** Write `bilbo-index.service` (`Type=oneshot`, `ExecStart=<exe> index`, output appended to the log) and `bilbo-index.timer` (`OnUnitActiveSec=<n>min`, `OnBootSec=5min`, `Persistent=true`). Then run `systemctl --user daemon-reload` and `systemctl --user enable --now bilbo-index.timer`. If `systemctl --user is-system-running` cannot reach a manager, the step is skipped rather than failed, because containers and WSL often have no user manager.
- The file content is a pure function of the exe path, the interval and the log path, all in `src/timer.rs`. "Kept" therefore means the generated text equals the file on disk.
- `<state>` is `$XDG_STATE_HOME` when that is absolute, otherwise `~/.local/state`, the same rule as the other folders. `store::Env` gains `xdg_state_home`.

### The key file

- **Path.** `<config folder>/token` is opened with `OpenOptions::new().write(true).create_new(true).mode(0o600)` under a temporary name and renamed over the target. The mode is therefore set before the first byte, and a crash never leaves a half-written key.
- **Config reference.** The config gets `embedder.token_file = <absolute path>`.
- **Memory.** The key `String` is wrapped in `zeroize::Zeroizing` from the moment the `Prompter` returns it.
- **Never passed on.** It reaches no other program and no output. The embedder check uses `embed.rs`, whose messages already never hold the token (`config` spec).

### Config writing

The writer emits a fixed header (`# bilbo config, written by bilbo setup <version> on <date>`), then the settings in key order. Values are written unquoted unless they have leading or trailing spaces or a newline. In that case they are quoted and escaped as the `config` spec's format allows, and a unit test round-trips them through `config::parse`. A rewrite renames the old file to `config.bak`, replacing any older `.bak`.

A config is treated as managed when `symlink_metadata` says it is a link, or a write probe in its folder fails. Home-manager links the file into the store, which is the case this exists for.

### The home-manager module is a wrapper

```nix
programs.bilbo = {
  enable = true;
  settings = { "embedder.url" = "http://bagend:8081"; "embedder.model" = "qwen3-embedding-0.6b"; };
  index.every = 15;          # index.enable = false → --no-timer
  claude = "/path/to/claude"; # null → PATH at activation
  codex = null;
};
```

- `settings` keys are checked against the known keys at evaluation time with an assertion, which gives the "unknown setting" scenario. Values are rendered with the same quoting rule as the Rust writer.
- `xdg.configFile."bilbo/config".text` holds the rendered settings.
- `home.activation.bilboSetup = lib.hm.dag.entryAfter [ "writeBoundary" "setupLaunchAgents" ] "run ${cfg.package}/bin/bilbo setup --yes …"`.
  - Activation's PATH is minimal, which is why `claude` and `codex` are options. dnix passes the paths of its own binaries.
  - A failing setup prints its report and does not abort activation (`|| true` with the report kept). A broken plugin install must not block a system switch.
- On a rollback, activation runs the old generation's `bilbo setup`, which re-points the timer and the marketplaces to that version.

## Risks / Trade-offs

- **The agent CLIs' JSON can change shape between releases.** → The parser reads only the fields listed above, an unknown shape fails that step with "could not read `claude plugin list --json`", and the fakes' recorded outputs are refreshed when a field moves.
- **Removing Claude Code's marketplace uninstalls the plugin and deletes its saved data.** → bilbo's plugin keeps no data or options today. If it ever does, the update path has to move to `marketplace update` instead.
- **Activation runs network commands** for a GitHub source. → On Nix the source is always the local `share/bilbo` folder, so activation never clones.
- **cliclack's dependency tree.** → It is used in one module. If it ever has to be dropped, only `wizard.rs`'s adapter changes.
- **The 15-second embedder check can be slow on a CPU embedder's first load.** → The wizard shows a spinner and offers a retry. The 15 s limit has not been measured against a cold model load. The smoke run in task 7.2 measures it against bagend, and the limit changes if that run misses it.

## Migration Plan

- On rivendell, after the dnix commit, the legacy `nbrecall-index` agent keeps running until the cutover. Both are cheap.
- The bilbo timer only indexes bilbo's store, which stays empty until the notes move, so the two never touch the same files.
