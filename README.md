# bilbo

Durable memory for coding agents.

![A librarian hands a saved note to a coding robot asking, "Didn't we fix this?"](docs/assets/bilbo-readme.png)

**Your agent has been here before. bilbo kept the notes.**

You fix a bug, settle a design decision, and finally figure out that strange
test failure. Then a new session begins, ready to rediscover all three.

bilbo gives your agent a notebook: plain Markdown notes it writes in one session
and finds again in the next, plus a library of sources it can cite. Its plugin
for Claude Code and Codex helps your agent save decisions and gotchas, and
brings relevant notes into later conversations. You rarely need to type a
`bilbo` command yourself.

<details>
<summary>Contents</summary>

- [See it work](#see-it-work)
- [Install](#install)
- [Quick start](#quick-start)
- [What you get](#what-you-get)
- [What you do and what your agent does](#what-you-do-and-what-your-agent-does)
- [Common commands](#common-commands)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [License](#license)

</details>

## See it work

A new Claude Code session with the bilbo plugin. The prompt hook hands the
agent a saved note, and the agent answers from it. The notes come from bilbo's
synthetic eval set.

https://github.com/user-attachments/assets/b8a991bb-c882-419a-aba6-2213f182dec0

In your own work, notes like that one come from earlier sessions: you tell the
agent to note a fix, and it saves one. You can also ask "What did we decide
about the database layer?" and have it search the notes.
[Getting started](docs/getting-started.md#5-see-your-agent-use-it) shows the
text the hook hands the agent.

## Install

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/delucca/bilbo/releases/latest/download/bilbo-installer.sh | sh
```

Check it with `bilbo --version`, which prints `bilbo <version>`.

On Nix, try it with `nix run github:delucca/bilbo -- --version`. Release
binaries cover macOS (arm64 and Intel) and Linux (x86_64 and arm64, glibc).
[docs/install.md](docs/install.md) covers `PATH`, upgrades, NixOS, home-manager
and removal.

## Quick start

1. **Run setup.**

   ```sh
   bilbo setup
   ```

   It asks which embedder to use for search by meaning (none is fine to start),
   shows its plan and asks once. Then it creates the store and the config,
   installs the plugin in Claude Code and Codex (each one found on your `PATH`)
   and installs a login service that records note history. It prints one line
   per step; [docs/getting-started.md](docs/getting-started.md) shows them, and
   [docs/guides/setup.md](docs/guides/setup.md) lists every option.

2. **Use it through your agent.** Open Claude Code or Codex and try `Note this
   as a gotcha: <something you learned today>`, then in a new session `What do
   we know about <that topic>?`. The agent needs no command from you. See
   [docs/guides/agents.md](docs/guides/agents.md).

3. **Try the commands yourself.** Create a note and search it:

   ```console
   $ bilbo new gotcha sqlite-busy-timeout --title "SQLite needs a busy timeout"
   /Users/me/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md
   ```

   Open the file it prints and add these lines under the title, after a blank
   line:

   ```markdown
   Two writers on the same database file fail with `SQLITE_BUSY` unless each
   connection sets `PRAGMA busy_timeout`.

   ## Fix

   Set `busy_timeout = 5000` right after opening the connection.
   ```

   Then check the store and search it:

   ```console
   $ bilbo check
   $ bilbo recall busy timeout
   /Users/me/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:11	gotcha	2026-10-06T14:39-03:00
   SQLite needs a busy timeout > Fix
   Set `busy_timeout = 5000` right after opening the connection.
   ```

   `check` prints nothing when the store is clean. `recall` prints the path and
   line of the best passage, the kind and `created`, the heading path and the
   start of the text. These are the bytes a pipe or an agent gets; a terminal
   shows the same facts laid out for a person.

More detail: [docs/getting-started.md](docs/getting-started.md).

## What you get

- **Plain files.** One note per topic in `<root>/notes/`, named
  `<kind>-<topic>.md`, with a small YAML frontmatter. Read, edit, grep or
  version them like any other file. See
  [files](docs/reference/files.md#note-format).
- **Keyword search out of the box, meaning search with an embedder.** Point
  bilbo at Ollama, OpenAI or any OpenAI-compatible URL, or let it run a local
  model. See [embedders](docs/guides/embedders.md).
- **History for every note.** A background watcher records each version, so a
  careless rewrite is one `bilbo restore` away from undone. See
  [history](docs/guides/history.md).
- **Agent plugin.** Four skills for Claude Code and Codex (`note`, `recall`,
  `reference`, `ingest`), a prompt hook that hands the agent the notes that bear
  on each prompt, and a reminder to write down what a session settled after a
  compaction. See [agents](docs/guides/agents.md).
- **A library of cited sources.** Pages, files and PDF text kept verbatim, read
  in full and cited by id, with every citation checked by `bilbo cite`. See
  [library](docs/guides/library.md).
- **Scopes, and optional encrypted sync.** Group notes by part of your work and
  keep a scope local to one machine. Sync between your devices, devices and the
  relay are all optional: bilbo works fully without them. See
  [scopes](docs/guides/scopes.md) and [sync](docs/guides/sync.md).

## What you do and what your agent does

You install bilbo, run `bilbo setup` once, and reach for `bilbo restore` when a
note went wrong. The rest happens in the conversation:

| You say | Your agent |
| --- | --- |
| "Note this" or "save this as a decision" | Writes or updates the note with the `note` skill, then runs `bilbo check` |
| "Recall X" or "what do we know about X" | Searches with the `recall` skill (`bilbo recall`) |
| A question the library answers | Answers with the `reference` skill, citing sources |
| "Ingest this page" | Adds it with the `ingest` skill |
| Anything at all | Gets the digest of the notes that bear on the prompt, before it starts |

## Common commands

```sh
bilbo new gotcha <topic> --title "<text>"   # start a note, prints its path
bilbo check                                 # lint the store, changes nothing
bilbo recall <words>                        # search notes, best first
bilbo recall <words> --library              # search the library's sources
bilbo index                                 # embed new passages for search by meaning
bilbo history <topic>                       # list a note's versions (the topic, not the path)
bilbo restore <topic> <version>             # write a past version back
bilbo setup --remove                        # undo setup, keep your notes
```

`bilbo --help` lists the verbs, and `bilbo <verb> --help` prints a verb's
options, output and exit codes. The rest is in
[docs/reference/commands.md](docs/reference/commands.md).

## Documentation

[docs/README.md](docs/README.md) is the index. The main pages:

- Start: [getting started](docs/getting-started.md), [install](docs/install.md),
  [troubleshooting](docs/troubleshooting.md)
- Guides: [setup](docs/guides/setup.md), [agents](docs/guides/agents.md),
  [embedders](docs/guides/embedders.md), [scopes](docs/guides/scopes.md),
  [history](docs/guides/history.md), [library](docs/guides/library.md)
- Sync, all optional: [devices](docs/guides/devices.md),
  [sync](docs/guides/sync.md), [relay](docs/guides/relay.md)
- Reference: [commands](docs/reference/commands.md),
  [configuration](docs/reference/configuration.md),
  [files](docs/reference/files.md)
- Understand: [concepts](docs/concepts.md), [security](docs/security.md)

## Contributing

Pull requests are welcome. Behavior changes start as an
[OpenSpec](https://github.com/Fission-AI/OpenSpec) change under
`openspec/changes/`; [`openspec/specs/`](openspec/specs/) holds the current
contract. [CONTRIBUTING.md](CONTRIBUTING.md) covers the workflow, and
[AGENTS.md](AGENTS.md) holds the rules for agents working on the repo. To run
the tests, with Nix: `nix develop -c cargo test --locked`.

## License

[Apache-2.0](LICENSE)
