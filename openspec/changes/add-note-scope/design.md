# Design

## Context

- Today's frontmatter allows only `id`, `created` and `sources`. `note::read` reports any other key as `frontmatter: unknown key` (`src/note.rs`, `Keys::key`), and the `note-store` spec's Frontmatter shape says so.
- The config is a fixed list of nine keys, `config::KEYS` in `src/config.rs`. An unknown key is an error that lists them. `parse` keeps the digest lines as written so `setup` can rewrite the file (`Settings.digest_lines`). Change 1 adds `history.keep_days` the same way.
- `new` and `check` never read the config today (`src/new.rs`, `src/check.rs`; the `config` spec's Config location).
- `config::is_local` already tells a loopback embedder URL (`localhost`, `127.0.0.1`, `::1`) from any other.
- `bilbo index` embeds every passage of every note `recall` searches (`src/index.rs`). `recall` counts passages with no vector as "not indexed" (`src/recall.rs`, `meaning`). The digest ignores that count (`src/digest.rs`, `vectors::lookup(...).0`), and its gate lets no unembedded passage pass on meaning (`note-digest` spec, The gate).
- `rank::words` is recall's word rule: runs of letters and digits, case and Latin accents folded.
- Change 1 (`add-note-history`) gives `src/swap.rs` (`exchange`), `bilbo watch`, which records every settled edit, and `src/versions.rs` with `history/lock` and the sweep of `notes/.bilbo-restore-<id>` leftovers. A leftover present while a process holds the lock is recorded as an `edited` version when history lacks its bytes, then deleted. `bilbo restore` and every locked watcher pass sweep (`note-restore` spec).
- `index`, `recall` and the digest all read notes through `store::read_notes`, whose `Stored` carries path, kind, created and passages (`src/store.rs`), but not frontmatter keys.
- The user's decisions are binding: the notebook's `design-bilbo-remote-sync.md`, "Decisions taken (2026-10-03)", and the three scope designs in `work/sync-design-panel/scope-assignment/`. Where the designs differ, those decisions win.

## Goals / Non-Goals

**Goals:**
- A forgotten scope is safe and visible: it never widens where a note goes, and `check` names it.
- A store with no `scope.*` key behaves exactly as today, except for the config errors `new` and `check` now report.
- Everything sync will need from a note's scope is settled here, so `add-sync` only routes by it.

**Non-Goals:**
- Guessing a scope from a note's content. Marks only warn.
- Reading the hook's `cwd`. Codex sends none. The digest's only change is the keyword gate for withheld passages.

## Decisions

### The marker is one named key, absent means unassigned

`scope: <name>`, with the topic's grammar. All three designs and the user picked it over a boolean `share:` key, a list of scopes and visibility levels. A boolean cannot carry two policies at once (sync and embedder), and it hides from a human which unshared notes are work. A list puts one note in two logs. Levels do not fit, because work and personal are not ordered.

### Any `scope.<name>.*` key declares a scope; `sync` defaults to `off`

Design B declared a scope only through `scope.<name>.sync`, design C through any key. I took C's rule. `scope.work.embedder = local` alone then declares `work`, which is what a user means by writing it. A default of `off` cannot widen anything. `default` is reserved as a name, so `scope.default` never reads as a scope's group of keys.

Alternatives: requiring `sync` on every scope (one more line, nothing gained while `off` is the only value).

### Unassigned takes the strictest embedder rule

This was decided by the user. In code it is a function of the parsed settings and the note's `scope` value: the scope's rule when declared; else `local` when any declared scope says `local`; else `any`. `config.rs` owns it, beside the settings it reads.

### Resolution in `bilbo new`

The order is the user's: `--scope`, then the longest `paths` match on `bilbo new`'s own working directory, then `scope.default`, then unassigned. Details I settled:
- **Comparing paths.** Paths are compared by whole folder names, never by string prefix, so `~/Developer/acme` does not hold `~/Developer/acme-tools`. Both sides have links resolved when they exist. macOS reports `/private/tmp` for `/tmp`, and a session started through a link would otherwise miss.
- **Ties.** "Longest" means the most folder names. Two scopes naming one folder, after `~/` expansion and link resolution at load, is a config error. A link created after load can still make two entries resolve alike; such a tie matches nothing, and the note stays unassigned, the safe side.
- **`~/` and `/`.** Both are allowed as `paths` entries and mean the home folder and the root. `~/` is how a user says "everything under home is personal unless a longer path says otherwise".
- **The working directory.** It is `std::env::current_dir`. When that fails, as for a deleted folder, it matches nothing.
- **The stderr line.** It is printed only when a scope is declared and none applied. It names the note's path and the exact `bilbo scope set` command, so the agent can act on it without reading the spec.
- **Refusing.** `new` never refuses for want of a scope, as the user decided. An unknown `--scope` still exits 2, like an unknown kind, so the skill's fix-and-retry row handles it.
- **Where the key goes.** `scope: <name>` comes after `created`. In a fresh note that is also the last frontmatter line, the place `scope set` uses.

Alternatives: string prefixes (wrong for sibling folders); the hook's `cwd` (absent in Codex, and the user's sessions start in `~/Developer`).

### `check` reads the config; marks warn without failing

- **Problems.** Missing and undeclared scopes are problems, so they exit 1, like any rule break. The note skill's existing loop then sees them on the note it wrote.
- **Warnings.** A mark is a warning that leaves the exit code alone. The user decided that a mark never holds sync back. A mark can also be a false positive, such as a personal note about riding in an Uber, and there is no per-note override. Failing `check` on it would keep the store red forever, and push agents to reword true notes to quiet it.
- **Format.** A warning keeps the `<path>: <message>` line, ending in `(warning)`, so the lines still sort and grep together.
- **Unassigned notes.** Their marks are folded into the one `scope: missing` line as `holds marks of <names>`, rather than one warning per scope. That keeps first-sync triage to one line per note, and the suffix is what `bilbo scope set` triage greps for.

Alternatives:
- Marks as problems (design A's hold). Rejected by the user's "only a warning".
- No marks for unassigned notes. That would lose the best triage hint.

### Where marks are searched

- **Where.** In the topic of the file name, `sources` items and the whole body, fenced code included. Design A excluded fenced code. I did not, because a missed mark costs more than a noisy one, and a mark only warns. The topic is searched because a note like `gotcha-acme-deploy.md` may never repeat `acme` in its text.
- **Words carry the load.** The note skill writes `code:` sources as repo-relative paths (`code: src/new.rs`), so a path mark rarely matches a source. Path marks catch absolute paths in bodies and code blocks. The README tells users to list words first: the employer, its products, its repo names.
- **Words.** A word mark uses `rank::words`, so it matches as recall matches.
- **Paths.** A path mark matches when the next character ends a path segment. Inside the home folder it matches with `~/` or with the absolute home path, because agents write both.
- **The `scope:` line.** It is not searched. It always names the note's own scope, and that never warns.

### The embedder rule lives in `index`; recall and the digest fall back to keywords

- **What `index` does.** It filters the needed inputs by each note's rule before it diffs against the cache. A withheld input is neither sent nor kept, so moving a note into a local scope drops its old vectors. The passage itself is local, but a vector kept from a remote embedder would keep ranking the note as if nothing had changed.
- **Shared text.** An input that a note with the rule `any` also holds is sent anyway: the same text leaves through the other note, so withholding it protects nothing.
- **Reporting.** The stdout line keeps its shape, because tests and the timer log read it, and one stderr line counts the withheld passages.
- **Counting.** The withheld count is distinct embedder inputs, the unit `recall`'s "not indexed" uses. An input shared with an `any` note counts as sent.
- **Recall.** `recall` computes the same set and leaves it out of "not indexed". Otherwise every `recall` would say `run bilbo index` forever, and the recall skill passes that warning on to the user. Withheld passages rank by keywords in the fusion, as unindexed ones already do.
- **The digest.** Its gate admits a note only on meaning while the embedder answers, so withheld notes would vanish from it, and the moment the first `embedder = local` scope is declared, that is every untriaged note. The gate now admits a withheld passage on the keyword gate (3 distinct query words of 4 or more letters) while embedded passages keep the meaning gate. That is the split `recall` already makes. It shows every scope, reads no `cwd`, adds no line, and keeps withheld text off the remote embedder. The panel's "no digest change" was about the digest as a channel for assigning scopes, a different question, and the user's decisions say nothing against this.
- **Sharing the set.** `store::Stored` gains the note's raw `scope` value, and `config` gives the rule, so the three verbs compute one set without depending on each other.

Alternatives:
- Keep the old vectors. Recall would go on using the remote embedder's view of a note that now asks for a local one.
- Add `, withheld <n>` to the stdout line. That breaks the pinned line.
- Leave the digest meaning-only. The whole untriaged store would leave the digest until triage.

### `bilbo scope` and `scope set`

- **The verb.** `src/scope.rs` is a verb, so it builds on `config` and `note` and on no other verb.
- **Listing.** It uses fixed tab-separated columns per row kind, plus an optional trailing `default` on a scope row, so an agent can cut it. The unassigned row is `(unassigned)`, which no scope name can be. A missing store counts as empty, so the skill can learn the names before the first note exists. With no scope declared, it still exits 0 and prints one stderr hint. "No scopes" is a normal state, not a refusal.
- **`set` changes one line.** It inserts `scope: <name>` before the closing `---`, or replaces the value in place with `--force`, and changes no other byte.
- **`set` reuses restore's machinery.** One file is changed at a time, under `history/lock`, through change 1's hidden name `notes/.bilbo-restore-<id>`. A second name for the same job would need its own sweep. The lock matters: change 1's sweep treats any `.bilbo-restore-<id>` present while a process holds the lock as a leftover, so `set` must hold it while its file exists. The sequence per run, then per file:
  1. Take `history/lock` and sweep leftovers through `versions.rs`, as restore's step 1 does.
  2. Read the note's bytes `R` and render `W`, with one line changed.
  3. Write `W` to `notes/.bilbo-restore-<id>` and `exchange` it with the note's file. Call what came out `O`.
  4. When `O` equals `R`, delete it: those are the bytes bilbo read, and the note now holds them with one line changed. Report the file as `set`, `kept` or `replaced`.
  5. When `O` differs, another writer changed the file after step 2. `exchange` again, so the note holds `O`, the other writer's bytes, and call what came out `O2`. Delete `O2` only when it equals `W`, which bilbo wrote. Otherwise a second write landed on bilbo's version between the two exchanges. Leave it at the hidden name and name it on stderr. The next locked pass of `bilbo watch`, the next restore or the next `scope set` sweeps it into history as an `edited` version. Report the file as changed and failed.
- **What history sees.** In this change `set` writes the note and `bilbo watch` records the result as an edit, like any other write. How a write by a bilbo verb is recorded once sync exists is `add-sync`'s rule, not this change's.
- **The guarantee.** `set` never unlinks bytes it neither read nor wrote. The bytes of a racing write end up in the note, or parked at the hidden name until a sweep records them. The earlier draft deleted what came out of the second exchange and relied on `bilbo watch` to have seen it, which a 2-second debounce cannot do.
- **Before the swap, it refuses.** A file whose frontmatter has no closing `---`, no canonical `id` (the hidden name needs it) or two `scope` lines is refused, unchanged. A single invalid value such as `scope: Work` is "another value": `--force` replaces it and mends the note.
- **Exit codes.** A file kept with another scope is a normal outcome of two-pass bulk triage (`set work` on the acme notes, then `set personal` on all), so it prints `kept <old>; --force replaces it` on stdout and does not fail the run. Exit 1 is for a path that is not a note, a refused file, a race and a filesystem without an atomic swap.
- **Paths only.** `set` takes file paths, not topics, so a shell glob over `notes/` works for bulk triage.

Alternatives:
- A plain rename. It loses an agent write that races it, which change 1 was built to prevent.
- Asking the agent to edit frontmatter by hand. It is error-prone in bulk, and nothing checks the name.

### No setup step or wizard question

Design B proposed a wizard question for `scope.default`. A fresh setup has no scopes, so the question has nothing to offer, and the contract fixes setup's steps. Setup only keeps the scope lines on a rewrite: `Settings` gains `scope_lines`, kept as written, like `digest_lines`. The home-manager module accepts `scope.*` keys by shape: a freeform attribute set, checked by an assertion against the name grammar and the four sub-keys. The module stays declarative and adds no option per scope.

Alternative: a structured `programs.bilbo.scopes.<name>` option. It is nicer in Nix, but a second spelling of the same keys.

### The note skill asks at most once

The skill runs `bilbo scope` before `bilbo new`. Its output is short, and it is the only way the agent learns the names and paths. The skill passes `--scope` only for a scope the user named, and otherwise lets `new` resolve one. It asks once when the result is unassigned, or when the resolved scope contradicts the subject, for example a note about an employer's deploy written from a personal repo. It applies the answer with `bilbo scope set`, never by Edit, so the name is checked. With nobody to ask, as in `codex exec`, it leaves the note unassigned and says so. Leaving it is the safe outcome, and it costs nothing. `allowed-tools` gains `Bash(bilbo scope)` and `Bash(bilbo scope set *)`.

### No new dependency

Everything uses std, `rank::words`, `config::is_local` and change 1's `swap.rs` and `versions.rs`.

## Risks / Trade-offs

- **[Declaring the first `embedder = local` scope withholds the whole untriaged store]** Every unassigned note takes `local`. With a remote embedder, `bilbo index` drops their vectors (`dropped <n>` for all of them), and `recall` and the digest rank them by keywords until triage. After triage to a scope with `embedder = any`, those notes are sent to the remote embedder again. → The README says to triage before declaring the first `embedder = local` scope, or to run the local embedder (`--embedder-local`), under which nothing is withheld. Keywords keep every note reachable meanwhile.
- **[`check` turns red when the first scope is declared]** Every existing note gets a `scope: missing` line until triaged. → `bilbo scope set` with a shell glob triages in bulk, and the `holds marks of` suffix shows where to start.
- **[A misfiled note]** A work note written with `scope: personal` is not caught unless it holds a mark of `work`. → Nothing syncs in this change. When sync arrives, the marks warn, and the skill asks when the scope looks wrong.
- **[`new` and `check` fail on a broken config]** They used to run whatever the config held. → The error names the file, the line and the key, as for every verb that reads settings.
- **[Marks are noisy]** A common word as a mark warns on unrelated notes. → Marks are the user's own list. A warning costs one line and never fails `check`.
- **[A parked write in `scope set`]** A second write between the two exchanges waits at `notes/.bilbo-restore-<id>` until a sweep records it. With no watcher running, it waits for the next restore or `scope set`. → It is named on stderr, and it is never deleted unrecorded.
- **[A tunnel on `localhost`]** `embedder = local` trusts the URL's host literally. An ssh tunnel on `localhost` counts as local, and the text leaves the machine through it. → The README says so beside the embedder settings.

## Migration Plan

Nothing to migrate. A config with no `scope.*` key changes nothing but the config errors `new` and `check` now raise. A user who declares scopes then triages with `bilbo check` and `bilbo scope set`. To roll back, remove the `scope.*` lines. The `scope:` keys left in notes then fail `check` as undeclared, until they are removed or the build is downgraded, where they fail as unknown keys.

## Decisions for the user to confirm

1. Any `scope.<name>.*` key declares a scope, with `sync` defaulting to `off`, and `default` is reserved as a name.
2. Mark lines are warnings that leave `check`'s exit code at 0. Missing and undeclared scopes are problems with exit 1.
3. For unassigned notes, marks are a `holds marks of ...` suffix on the problem line, not separate warnings.
4. Marks are searched in the file name's topic and in fenced code too. Path marks match in both `~/` and absolute form.
5. `paths` accepts `~/` and `/`. Two entries resolving to one folder are a config error; a tie that appears later matches nothing.
6. `index` drops cached vectors of withheld passages, and still sends an input that an `any` note shares. The digest admits withheld passages on its keyword gate.
7. `bilbo scope` output: fixed columns per row kind, the `(unassigned)` row, and exit 0 with a stderr hint when no scope is declared.
8. `scope set` exits 0 when it keeps a file with another scope, so two-pass bulk triage succeeds. It refuses a note with two `scope` lines, and `--force` mends a single invalid value.
9. No wizard question for `scope.default`. The home-manager module takes scope keys in `settings`, validated by shape.
10. The note skill asks once, and only when the note is unassigned or the scope looks wrong. It sets the answer through `bilbo scope set`.
