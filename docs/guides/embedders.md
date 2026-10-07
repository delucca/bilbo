# Search by meaning with an embedder

This guide shows how to make `bilbo recall` find notes that share no word with
your query, by pointing bilbo at an embedder: a service that turns text into
vectors. It covers the choices, their setup, the key, the index timer, the local
embedder and the rule that keeps private notes off a remote embedder.

Without an embedder, bilbo is keyword-only and `recall` matches whole words,
ignoring case and accents. With one, `recall` fuses that order with a ranking by
meaning. If the embedder is down, `recall` falls back to keywords and says so on
stderr.

## Choose an embedder

| Choice | Setup | Notes |
| --- | --- | --- |
| None | Nothing to do. | Keyword search only. |
| Local, run by bilbo | `bilbo setup --embedder-local` | A pinned model served by your `llama-server`. See [Run the local embedder](#run-the-local-embedder). |
| Ollama | `--embedder-url http://localhost:11434 --embedder-model nomic-embed-text` | The wizard finds Ollama on `localhost:11434` and offers its first model whose name holds `embed`. |
| OpenAI | `--embedder-url https://api.openai.com --embedder-model text-embedding-3-small` | Needs a key. |
| Another OpenAI-compatible URL | `--embedder-url <url> --embedder-model <name>` | The URL serves `/v1/embeddings`. |

The URL is `http` or `https`. The model is required with a URL. The two options
go together.

## Set up an embedder

In a terminal, run the wizard and pick one:

```sh
bilbo setup
```

From a script, or from an agent, pass the answers as flags and ask nothing:

```sh
bilbo setup --yes \
  --embedder-url http://localhost:11434 \
  --embedder-model nomic-embed-text
```

Setup checks the embedder with one real request (15-second limit) and reports
`embedder ok: <dimensions> dimensions` before it writes anything. In a script, a
failed check exits 1 and writes nothing. [Set up](setup.md) lists every option.

To change an embedder later, rerun the wizard, or edit the config by hand: the
keys are in [Configuration](../reference/configuration.md#keys).

If your embedder wants a prefix before each query, pass
`--embedder-query-prefix <text>`, which sets `embedder.query_prefix`.

## Keep the key

An embedder at a URL that is not on your machine usually needs a bearer token.
No option takes the key itself. Pick one place for it:

- `--embedder-token-file <path>`, or `embedder.token_file`: a file, absolute or
  starting with `~/`.
- `--embedder-token-env <var>`, or `embedder.token_env`: the name of an
  environment variable.
- In the wizard, a pasted key with hidden input, saved to `<config
  folder>/token` with mode 0600.

Set at most one of the file and the variable. bilbo never prints the token.

The timer does not inherit your shell's environment, so it cannot read a key
from a variable. Keep the key in a file; with `--embedder-token-env` the timer
step fails and says so. Use `--no-timer` if you want the variable anyway, and
run `bilbo index` yourself.

## Keep the index current

`recall` ranks by meaning only the passages that `bilbo index` has embedded.
Setup installs a timer that runs `bilbo index` every 15 minutes (a launchd agent
on macOS, a systemd user timer on Linux) when an embedder is configured. Change
the interval with `--index-every <minutes>`, 1 to 1440, or skip the timer with
`--no-timer`.

You can also run it by hand:

```sh
bilbo index
```

It embeds the passages the vector cache lacks and drops the ones no note holds
any more. The vector cache lives in the cache folder
([Folders](../reference/configuration.md#folders)). Deleting it loses nothing
that `bilbo index` cannot rebuild.

Passages written since the last `bilbo index` rank by keywords only, and
`recall` says so on stderr.

## Run the local embedder

Meaning ranking needs an embedder. If you have none, `bilbo setup --yes
--embedder-local` (or the wizard's second choice) runs one for you:

```sh
bilbo setup --yes --embedder-local [--embedder-port <n>] [--llama-server <path>]
```

Setup downloads a pinned model, Qwen3-Embedding-0.6B (639 MB, checked against
its SHA-256), to `models/` under bilbo's cache folder. It then installs a login
service, a launchd agent on macOS or a systemd user service on Linux, that runs
`llama-server` on `127.0.0.1` and restarts it if it exits. It waits for the
server, checks it with one real request, and points the config at it. An
interrupted download continues on the next run.

bilbo does not install `llama-server`. Have it on your `PATH`, or pass
`--llama-server`: `brew install llama.cpp` on macOS, your distribution's
`llama.cpp` package, or Nix. Linux needs a systemd user session.

The port defaults to 8737, and `--embedder-port` takes 1024 to 65535.

The server stays loaded, about 1 GB of memory in use, so that `recall` never
waits for a model to load. The first `bilbo index` of a large store takes a
while. The server's log is `embedder.log` under bilbo's state folder.

Setup fails before it downloads anything when `llama-server` is missing or the
port is taken. If the config later moves to another embedder, a rerun removes
the service.

## The embedder rule

A note whose scope sets `embedder = local` must not reach a remote embedder. An
unassigned note takes `local` as soon as any declared scope sets it, and every
note takes `any` when none does. When `embedder.url` is not on `localhost`,
`127.0.0.1` or `::1`, `bilbo index` sends no passage of a `local` note, unless a
note whose rule is `any` holds an identical passage, and drops the vectors it
already cached for them. It says so on stderr, counting distinct inputs:

```console
$ bilbo index
embedded 1, kept 0, dropped 0
bilbo: withheld 2 passages from http://embedder.example:8081: their scope allows only a loopback embedder
```

That is the piped form. A terminal prints `◆  Embedded 1 passage · kept 0 ·
dropped 0` and, after it, the withheld line as a `▲` warning.

`recall` and the digest still reach withheld notes, by keywords, and `recall`
does not count them as not indexed. The digest admits a withheld passage on its
keyword gate, so a store withheld whole still reaches the digest without a
request to the embedder.

Declaring the first `embedder = local` scope withholds every note not yet in a
scope. Triage the store first, as in [Scopes](scopes.md#declare-a-scope), or run
[`--embedder-local`](#run-the-local-embedder), under which nothing is withheld.
The rule trusts the URL's host as written: an ssh tunnel on `localhost` counts
as local, and the text leaves the machine through it. A loopback embedder is
reached directly: bilbo ignores the proxy variables (`HTTP_PROXY`,
`HTTPS_PROXY`, `ALL_PROXY` and their lowercase forms) for it.

## See also

- [Configuration](../reference/configuration.md#keys): `embedder.*` and
  `digest.min_similarity`.
- [The digest](agents.md#the-digest): how the digest uses an embedder.
- [Security](../security.md)
- [Troubleshooting](../troubleshooting.md)
