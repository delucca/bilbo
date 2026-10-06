# bilbo documentation

These pages help you install bilbo, use it with your agent and look up the
details. New here? Start with [Getting started](getting-started.md).

## Start

- [Getting started](getting-started.md): install, set up, write a first note,
  recall it and see your agent use it.
- [Install](install.md): the installer, Nix, the home-manager module, upgrading
  and removing bilbo.
- [Troubleshooting](troubleshooting.md): find a message or symptom, then the
  cause and the fix.

## Guides

- [Set up bilbo](guides/setup.md): what `bilbo setup` does, the wizard, scripted
  runs, the watch service.
- [What your agent does with bilbo](guides/agents.md): the four skills, the
  compaction reminder and the digest.
- [Search by meaning with an embedder](guides/embedders.md): embedders, keys,
  the index timer, the local embedder and the embedder rule.
- [Scopes](guides/scopes.md): split your notes into named parts, and keep a part
  off remote embedders or off sync.
- [Note history](guides/history.md): see, diff and restore past versions of a
  note.
- [The library](guides/library.md): add sources and cite them.
- [Devices and the owner](guides/devices.md): keys, the recovery phrase, pairing
  and revoking.
- [Sync notes between devices](guides/sync.md): keep a scope in step across your
  devices.
- [Run a relay](guides/relay.md): run a server that syncs your devices.

## Reference

- [Commands](reference/commands.md): every verb, with its flags and exit codes.
- [Configuration](reference/configuration.md): config keys, paths and
  environment variables.
- [Files and formats](reference/files.md): the note format, kinds, the store
  layout and citation syntax.

## Understand

- [Concepts](concepts.md): the moving parts and how they fit together.
- [Security](security.md): what bilbo protects, what it does not and what a lost
  key means.

## Contributing

- [CONTRIBUTING.md](../CONTRIBUTING.md): how to build, test and propose a
  change.
- [AGENTS.md](../AGENTS.md): the rules an agent working on this repository
  follows.
- [Manual tests](manual-tests.md): the procedures for the terminal code that
  unit tests cannot reach.
