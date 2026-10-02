# Proposal

## Why

add-distribution puts a `bilbo` binary on PATH, but a binary alone does nothing for an agent. To be useful, a machine also needs:
- a store folder;
- a config that points at an embedder, if the user has one;
- the bilbo plugin installed in Claude Code and Codex, at the same version as the binary;
- something that runs `bilbo index` in the background.

Today each of these is a manual step that only the maintainer knows. The Claude Code plugin also follows `main` rather than the installed binary, so a skill can ask for a flag the binary doesn't have yet.

One command should do all of it. It should be the same command whether a person runs it at a terminal or home-manager runs it on activation, so there is one implementation to test.

## What Changes

- **`bilbo setup`**, a new verb. It plans every step first, asks for one confirmation, then applies the steps. It prints one report line per step. The steps:
  - **store**: create `<root>/notes/`.
  - **config**: write the config file, from flags or from answers in the wizard. It never overwrites an existing file without asking, and it never touches a file managed elsewhere, such as a Nix store link.
  - **embedder check**: before anything is written, one real embed request confirms the URL, model and key work.
  - **key**: an API key can be referenced by environment variable or file, or pasted with hidden input. A pasted key is saved to `<config dir>/token` with mode 0600. No flag ever takes a key value.
  - **claude** and **codex**: for each tool found, add the `bilbo` marketplace at the binary's own version and install `bilbo@bilbo`. The marketplace is the package's `share/bilbo/` folder when the binary sits in a package that has one, otherwise `delucca/bilbo` at the tag `v<version>`.
  - **timer**: when an embedder is configured, a launchd agent on macOS or a systemd user timer on Linux runs `bilbo index` every 15 minutes, or at the interval chosen.
- **An interactive wizard.** It runs when stdin and stderr are terminals. It uses arrow-key selects, hidden key input and a spinner for the embedder check. It offers these embedders:
  - none (keyword search only);
  - Ollama, detected on `localhost:11434` with its models listed;
  - OpenAI;
  - any other OpenAI-compatible URL.

  On a rerun it shows current values as defaults. After applying, it offers to run the first `bilbo index`.
- **A non-interactive mode** for agents, scripts and Nix. It is used when there is no terminal, or with `--yes`. Every question has a flag, and a missing answer takes its default. A rerun with the same inputs changes nothing and reports every step as `kept`.
- **`bilbo setup --remove`** unloads the timer and removes the plugin and marketplace from each tool. It keeps the store, the config and the key file, and prints their paths.
- **A home-manager module**, `homeManagerModules.default` in the flake. It writes the config from `programs.bilbo.settings`, and on activation runs `bilbo setup --yes` for everything else. It is a thin wrapper, so Nix and the wizard share one implementation.

## Capabilities

### New Capabilities

- `setup`, which covers:
  - `bilbo setup`: its modes, planning and confirmation, each step, the plugin source, the timer, the report, reruns and `--remove`;
  - the home-manager module that drives it.

### Modified Capabilities

- `cli`:
  - Verb dispatch adds `setup`.
  - Output streams lets the wizard draw its prompts on stderr without the `bilbo: ` prefix.
- `config`: Config location names `setup` as a verb that reads settings, and says that `setup` creates the file a missing `BILBO_CONFIG` names instead of failing.

## Non-goals

- **Hooks.** The digest hook arrives with add-note-digest. Shipping it inside the plugin, where Claude Code and Codex load it with no settings edits, is decided there. Until then `setup` never edits agent settings files.
- **Moving the store.** The wizard shows the store root but does not ask for it. The root comes from `BILBO_HOME`, which `setup` cannot set for the user's shell.
- **Windows**, and Linux systems without a systemd user session. On those, the timer step is reported as `skipped`, with the reason.
- **Removing the store, the config or the key.** `--remove` leaves the user's data alone and says where it is.
- **Running `setup --remove` automatically** when the home-manager module is disabled.
- **The dnix side.** The flake input and `programs.bilbo` on rivendell go in a dnix commit once this change merges.

## Impact

- **New modules.**
  - `src/setup.rs`: the verb, the plan and apply logic, and the report.
  - `src/wizard.rs`: the interactive questions, behind a small prompt trait so the logic is unit-tested without a terminal.
  - `src/agents.rs`: the `claude` and `codex` subprocess calls and the parsing of their JSON output.
  - `src/timer.rs`: the launchd plist and systemd unit text, and the load and unload commands.
- **Changed modules.**
  - `src/main.rs`: the `setup` dispatch arm and its USAGE line.
  - `store::Env`: gains `xdg_state_home` for the timer log. add-note-digest adds the same field; whichever change lands second reuses it.
  - `src/embed.rs`: reused for the embedder check.
- **New dependencies.**
  - `cliclack` 0.5.6 for the wizard, which brings in `console`, `indicatif`, `textwrap`, `strsim`, `zeroize` and `once_cell`.
  - `serde_json`, moved from dev-dependency to direct dependency at the same version, to read `claude … --json` and `codex … --json`.
  - Both are justified in design.md.
- **Tests.**
  - `tests/setup.rs` (new) runs the binary non-interactively. Fake `claude`, `codex`, `launchctl` and `systemctl` scripts on a temporary PATH record their arguments, and the fake embedder from add-embeddings answers the embedder check.
  - The wizard is unit-tested with scripted answers, plus a recorded smoke run in a real terminal.
- **Flake.** `flake.nix` gains `homeManagerModules.default`. A flake check evaluates it with sample settings.
- **Docs.** `AGENTS.md` lists the new modules, dependencies and test file. `README.md` replaces its manual plugin steps with `bilbo setup`.
- **Order.** This change applies after add-distribution is archived: it uses `--version`, the package's `share/bilbo/` and the flake. Its `cli` delta modifies Verb dispatch, which add-note-digest also modifies, so whichever archives second rebases its delta onto the other.
