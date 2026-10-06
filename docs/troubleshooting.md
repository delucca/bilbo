# Troubleshooting

This page helps you when bilbo prints a message you do not expect. Find the
message or the symptom, read the cause, apply the fix.

## Setup

### A setup line says `failed`

Setup prints one line per step and a failed step does not stop the ones after
it, so read every line. The exit code is 1 when any step failed. Rerun `bilbo
setup` after the fix: with the same inputs, steps that worked are `kept`.

### `timer failed`, naming a variable, `--embedder-token-file` and `--no-timer`

Cause: the key comes from an environment variable (`embedder.token_env`). The
timer does not inherit your shell's environment, so it cannot read it. Setup
leaves the timer files as they were.

Fix: keep the key in a file, with `--embedder-token-file <path>` or by pasting
it in the wizard. Or skip the timer with `--no-timer` and run `bilbo index`
yourself. See [Keep the key](guides/embedders.md#keep-the-key).

### `watch failed: launchctl not found on PATH`

Cause: setup installs the watcher through `launchctl` (macOS) or `systemctl`
(Linux) and could not find the tool on `PATH`. The same message shape appears on
the `timer` line.

Fix: put the tool on `PATH` and rerun setup. See [Run the watch
service](guides/setup.md#watch-service).

### `claude` or `codex` says `skipped: not found`

Cause: setup found no `claude` or `codex` executable on `PATH`.

Fix: install the tool, or pass `--claude <path>` or `--codex <path>`. To skip
the plugin on purpose, use `--no-plugin`.

### `claude` or `codex` says `failed`, naming a source

Cause: setup could not install the plugin from its source.

Fix: the hint names `--plugin-source`; pass a folder or `owner/repo#ref` there.

### `hook failed: codex reports trust status '<status>' for a bilbo hook`

Cause: Codex lists a bilbo hook with a status other than `untrusted`,
`modified`, `trusted` or `managed`. Setup writes no trust in that case.

Fix: read Codex's message in the line and resolve the status in Codex, then
rerun setup.

### `hook updated: trusted in Codex` after an upgrade

Not a problem. A release that changes the hook changes Codex's hash, and the
next setup trusts it again.

### Setup fails before it downloads the local embedder

Cause: `llama-server` is missing, or something already answers on the port (8737
by default).

Fix: put `llama-server` on `PATH` or pass `--llama-server <path>`; where to get
it is under [Run the local
embedder](guides/embedders.md#run-the-local-embedder). For a taken port, pass
`--embedder-port <n>`, 1024 to 65535. See [Run the local
embedder](guides/embedders.md#run-the-local-embedder).

### The server step fails and names `embedder.log`

Cause: the local embedder's server did not report ready within 120 seconds.

Fix: read `embedder.log` under bilbo's state folder.

### A flag exits 2 and names `--embedder-port` and `--embedder-local`

Cause: `--embedder-port` and `--llama-server` apply only with
`--embedder-local`, and `--embedder-local` cannot go with `--embedder-url`,
`--embedder-model`, `--embedder-token-env` or `--embedder-token-file`.

Fix: drop the conflicting flag. See [Options](guides/setup.md#options).

### `config kept: managed elsewhere (<target>)`

Cause: the config is a symbolic link, or its folder is not writable, as with the
home-manager module. Setup treats it as managed elsewhere and leaves it alone;
embedder flags against it are a usage error.

Fix: change the setting where the config comes from. See
[Install](install.md#install-with-home-manager).

## Search

### `bilbo: no notes match`

Cause: nothing in the store matches. `recall` exits 1. How it matches without an
embedder: [Search by meaning](guides/embedders.md).

Fix: try the words the note would use. For a note that shares no word with your
query, set up an [embedder](guides/embedders.md).

### `bilbo: embedder unavailable (<reason>); keyword results only`

Cause: the embedder did not answer within 5 seconds, or answered with an error.
`recall` falls back to keywords and exits 0.

Fix: check that the embedder is running and that `embedder.url` is right. For
the local embedder, read `embedder.log` under bilbo's state folder.

### `bilbo: <n> passages not indexed; run bilbo index`

Cause: passages written since the last `bilbo index` have no vector yet, so they
rank by keywords only.

Fix: run `bilbo index`, or wait for the timer. Setup installs the timer only
when an embedder is configured (see [Keep the index
current](guides/embedders.md#keep-the-index-current)).

### `bilbo: no embedder configured; set embedder.url in <config>`

Cause: you ran `bilbo index` with no embedder.

Fix: set one up. See [Embedders](guides/embedders.md#set-up-an-embedder).

### `bilbo: withheld <n> passages from <url>: their scope allows only a loopback embedder`

Cause: the embedder rule. A note whose scope sets `embedder = local` must not
reach a remote embedder. `recall` and the digest still find those notes by
keywords.

Fix: use a loopback embedder (`--embedder-local`), or change the scope. See [The
embedder rule](guides/embedders.md#the-embedder-rule).

### `bilbo: no sources match` or `bilbo: no library at <root>`

Cause: `recall --library` found nothing, or the store has no corpus.

Fix: add a source. See [Library](guides/library.md).

## Notes

### `bilbo: no scope for <path>; scopes: <names>; set one with bilbo scope set <name> <path>`

Cause: you declared scopes, and `bilbo new` found none for the note: no
`--scope`, no `scope.<name>.paths` entry that covers the working directory, no
`scope.default`. The note is created without a scope, and the exit code is 0.

Fix: run the command the message prints. See
[Scopes](guides/scopes.md#declare-a-scope).

### `bilbo check` prints lines

Each line is `<path relative to the root>: <message>`, and `check` exits 1 when
any is a problem. A warning does not change the exit code. Fix the file the line
names. `check` changes nothing.

### `bilbo: no store at <root>`

Cause: neither `<root>/notes/` nor `<root>/library/` exists, so the root is
wrong or setup has not run.

Fix: run `bilbo setup`, or check `$BILBO_HOME`. See [Store](concepts.md#store).

## The digest

### The digest prints nothing

That is normal when no note is close to the prompt, and the digest then prints
nothing. See [What passes](guides/agents.md#what-passes). It is also silent when
`digest.enable = off`.

### One line starting with `bilbo: ` on stderr, and no digest

Cause: the digest hit an error that stops it: bad input, no store, an unreadable
config, an unwritable cache folder. It always exits 0, so it never blocks a
prompt. The line names the error.

Fix: fix what the line names, such as the config key or the cache folder.

### The digest lists notes and stderr names the embedder's URL and 500

Cause: the embedder refused, and the digest switched to its keyword gate.

Fix: check the embedder, as for `embedder unavailable` above.

To see what each run did, set `digest.log = on` and read `digest.jsonl` under
bilbo's state folder. See [The digest](guides/agents.md#the-digest).

## Sync

### `failed: sync needs the watcher; drop --no-watch`

Cause: a scope syncs and setup ran with `--no-watch`.

Fix: rerun setup without `--no-watch`.

### `failed: <url> is not reachable: <reason>`

Cause: the transport does not answer: the folder is missing or not writable, or
the relay URL is down.

Fix: fix what the reason names, then rerun setup. See [Sync](guides/sync.md).

### `failed: <url> is not a bilbo relay`

Cause: the URL answers, but not as a bilbo relay.

Fix: check the URL. See [Relay](guides/relay.md).

### `skipped: no device key; run bilbo device init in a terminal`

Cause: this device has no key yet.

Fix: run `bilbo device init` in a terminal, or `bilbo device recover` if you
already have a recovery phrase. See [Devices](guides/devices.md).

### `skipped: no scope syncs`

Not a problem: no declared scope has a `sync` URL. See
[Scopes](guides/scopes.md).

### `bilbo sync` exits 1

It means something needs you. Read its lines. For a conflict, see
[Conflicts](guides/sync.md#conflicts).
