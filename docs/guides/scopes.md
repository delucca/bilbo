# Scopes

Keep work and personal notes apart: a scope names a part of your work, and each
scope can have its own embedder rule and transport. This page covers declaring
scopes, filing notes into them and checking the result.

A scope is a name for a part of your work, such as `work` or `personal`. A note
says which one it belongs to with a `scope: <name>` line in its frontmatter. The
name follows the topic's grammar: lowercase words joined by single hyphens. What
a scope means is set per device in the [config](../reference/configuration.md),
so the same note can sit in `work` on one machine and be unassigned on another.

## Declare a scope

A scope exists on a device when its config holds at least one `scope.<name>.*`
key. A note is unassigned when it has no `scope` line, or names a scope the
config does not declare. A scope has four settings:

```
scope.personal.sync = off
scope.work.embedder = local
scope.work.paths = ~/Developer/acme
scope.work.marks = acme, ~/Developer/acme
scope.default = personal
```

- `sync` is `off`, the default, or a URL; see [Sync URLs](sync.md#sync-urls). A
  URL only records where the scope will sync; `bilbo device init` makes its
  manifest.
- `embedder` is `any` (default) or `local`. A `local` scope's notes are never
  sent to an embedder outside this machine; see [The embedder
  rule](embedders.md#the-embedder-rule).
- `paths` lists folders, comma-separated. `bilbo new` gives a note the scope
  whose entry is the working directory or a folder above it, the longest entry
  winning, compared by whole folder names. `~/` and `/` are valid entries, so
  `scope.personal.paths = ~/` makes `personal` the scope of everything under
  home that no more specific entry claims. A worktree outside every listed
  folder is not covered: list the parent folder that also holds your worktrees,
  not each clone.
- `marks` lists what only this scope's notes should mention, for `check`; see
  [Set marks](#set-marks).
- `scope.default` names the scope for a working directory no `paths` entry
  holds.

Two scopes cannot share a path or a mark.

## How new picks a scope

`bilbo new` picks the scope from `--scope`, else `paths`, else `scope.default`.
`--scope` must name a declared scope, or it exits 2. When nothing matches and
any scope is declared, the note is created without a `scope` line and stderr
says so:

```console
$ bilbo new plan release
/Users/me/.local/share/bilbo/notes/plan-release.md
bilbo: no scope for /Users/me/.local/share/bilbo/notes/plan-release.md; scopes: personal, work; set one with bilbo scope set <name> /Users/me/.local/share/bilbo/notes/plan-release.md
```

## List your scopes

`bilbo scope` prints one line per declared scope, sorted by name, with
tab-separated fields: the name, the note count, `sync`, `embedder`, `paths` as
written (`-` when unset) and `default` for the default scope. The last line is
the unassigned notes:

```console
$ bilbo scope
personal	3 notes	sync off	embedder any	paths -	default
work	1 notes	sync off	embedder local	paths ~/Developer/acme
(unassigned)	2 notes	embedder local
```

With no scope declared it prints only the `(unassigned)` line and says on
stderr, `bilbo: no scopes declared; add scope.<name>.* keys to <config path>`.

## Give notes a scope

`bilbo scope set [--force] <name> <file>...` gives each note the scope. A note
with no `scope` line gains one as the last line of its frontmatter and prints
`notes/<file>: set <name>`. A note already in `<name>` prints `kept <name>`. A
note in another scope prints `kept <old>; --force replaces it`, and with
`--force` `replaced <old> with <name>`. A key line not written as
`scope: <value>`, such as `scope:work`, prints
`kept 'scope:work' as written; --force rewrites it`, and with `--force`
`rewrote 'scope:work' as scope: work`. No other byte of the file changes.

To triage the notes a store already holds, run two passes, the specific glob
first. Notes that already have a scope are kept:

```sh
bilbo scope set work ~/.local/share/bilbo/notes/*acme*.md
bilbo scope set personal ~/.local/share/bilbo/notes/*.md
bilbo check
```

### Refusals

A file that is not a note directly in `<root>/notes/`, whose text is not UTF-8
or whose frontmatter is broken, is refused on stderr and the run exits 1 after
handling every file; an undeclared name or missing files exit 2.

`scope set` works under the history lock and swaps the file in atomically, so a
write by an agent in the same instant is never lost: the run reports `changed
while bilbo scope set ran; run it again` and leaves the agent's text. A
filesystem that cannot swap files atomically is refused.

## Set marks

A mark is either one word (letters and digits only, so `acme.`, `Acme's` and
`ac-me` are config errors) or a path starting with `/` or `~/`. A word matches a
whole word of the note's file name topic, `sources` or body, fenced code
included, ignoring case and accents. A path matches where the next character
ends a path segment: not a letter, a digit, `-`, `_` or `.`. A path inside home
matches as `~/...` and as the absolute path, so `~/Developer/acme` finds
`/Users/me/Developer/acme/api/main.go` but not `~/Developer/acme-tools`.

List words first: the employer, its products and its repo names. Path marks
catch the absolute paths that end up in bodies and code blocks.

## Check the scopes of your notes

`bilbo check` reads the config and reports a note that has no `scope` line (once
any scope is declared) or names a scope the config does not declare, and lists
the declared scopes:

```console
$ bilbo check
notes/gotcha-acme-deploy.md: scope: 'acme' is not declared in /Users/me/.config/bilbo/config; scopes: personal, work
notes/gotcha-deploy.md: scope: 'personal' but line 9 holds 'acme', a mark of 'work' (warning)
notes/plan-release.md: scope: missing; scopes: personal, work
notes/plan-x.md: scope: missing; scopes: personal, work; holds marks of work
```

An unassigned note that holds a mark gets `; holds marks of <scopes>` on its
problem, to say where to put it. A note in a scope that holds another scope's
mark gets one `(warning)` line per other scope: the first place is `the file
name` or `line <n>`. A warning leaves the exit code alone, and a mark never
holds anything back. A mark of `~/` warns on every path under home.

## See also

- [Configuration](../reference/configuration.md#keys) for every `scope.*` key.
- [The embedder rule](embedders.md#the-embedder-rule) for what `embedder =
  local` withholds.
- [Sync](sync.md) for scopes that sync.
