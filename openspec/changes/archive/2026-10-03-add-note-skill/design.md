# Design

## Context

The plugin (`plugins/bilbo/`) holds the `recall` skill and `hooks/hooks.json` with the digest hook. Everything a note skill needs from the CLI already exists:

- `bilbo new <kind> <topic> [--title]` prints the new file's absolute path. On a taken topic, whatever its kind, it exits 1 with `bilbo: topic '<topic>' already has a note: <absolute path>` (`src/new.rs`, `taken`). It exits 2 on an unknown kind (`bilbo: unknown kind '<k>'; kinds: …`), an invalid topic (`bilbo: invalid topic '<t>': …`), an empty or multi-line `--title`, and a relative `BILBO_HOME` (a `Failure::Config`, exit 2 with no usage text).
- `bilbo check` prints `notes/<file>: <message>` lines, sorted, and exits 1 on any problem. It exits 1 with `bilbo: no store at <root>` when `notes/` is missing.
- `bilbo recall` prints absolute paths, so the skill never has to work out the store root.

The legacy dnix skill (`dnix/modules/ai/common/skills/note/SKILL.md`) gives the flow to keep: pick a kind, look for the note on the topic, update or create, never invent a source. It also brings notebook lookup, a ULID script, `date`, `kind` and `supersedes` keys and an Org link, which this change drops (proposal, Non-goals).

The compaction nudge was probed on 2026-10-03 (`probe.md`, scripts in `probe/`). In short:

- In both tools, an auto-compaction runs before the agent gets a turn. PreCompact and PostCompact output never reaches the model then.
- Claude Code 2.1.288 rejects `additionalContext` from both PreCompact and PostCompact.
- A SessionStart hook with the matcher `compact` puts its plain stdout in front of the next model request, in both tools, for a plugin hook as well. Codex runs it once `bilbo setup` has trusted it.

## Goals / Non-Goals

**Goals:**
- An agent that writes a note, on request or because the session settled something durable, does so in bilbo's store in the `note-store` format, without a second note on the same subject, and leaves `bilbo check` clean for that note. The description names the unprompted case; whether a model acts on it unasked is observed in the smoke test, not promised.
- After a compaction, the agent is reminded once to save what the session settled.
- The skill and the hook work the same in Claude Code and Codex.

**Non-Goals:**
- Fixing notes the skill did not touch. `bilbo check` lines for other notes are reported, not fixed.
- A bilbo verb for renames or edits.
- Any change to `src/agents.rs`'s trust logic, which already handles any number of bilbo hooks.

## Decisions

### The skill file

`plugins/bilbo/skills/note/SKILL.md`, frontmatter keys in this order, as `recall`'s:

```yaml
---
name: note
description: Keeps what a later session should know as a bilbo note, such as a decision, gotcha, plan or finding. Use for "note this", "save this as a decision/gotcha/plan", "keep this for later sessions", or when this session settled something durable. NOT for a passing "note that..." in conversation, or for searching notes (recall).
license: Apache-2.0
allowed-tools: Bash(command -v bilbo), Bash(bilbo recall *), Bash(bilbo new *), Bash(bilbo check), Bash(mv -n *), Read, Edit
---
```

- The description opens with "Keeps what a later session should know" and the four kinds an agent meets most, then the explicit requests ("note this", "save this as a decision/gotcha/plan"). A tool that shortens descriptions still keeps the trigger. "note this" asks for a note; a passing "note that…" inside another request does not, and the description says so.
- "remember" is left out on purpose: Claude Code routes "remember X" to its own memory.
- `allowed-tools` covers every command in the skill's `bash` blocks, each as one unchained command. `tests/plugin.rs` checks that for both skills.
- `Read` and `Edit` are listed because Claude Code's Edit needs a Read of the file first. `Write` is not, because the file always exists by the time the skill writes: `bilbo new` made it, or it is being updated.

The body follows `recall`'s shape: a one-line purpose, then numbered steps with an exit-code table per command, then a `Never` list. Its content, which the implementer writes as prose in that shape:

1. **Missing binary.** Run `command -v bilbo`. When it prints nothing, stop with exactly `note: bilbo is not on PATH; install the bilbo CLI first`.
2. **When and which kind.** Write a note when the user asks to keep something for later sessions, or when the session settled something a later one would otherwise work out again. A passing "note that…" inside another request is not one. The kinds are the nine in `note-store`, and a note that fits none is a `report`. Use the kind the user named, or infer it: a decision taken is a `decision`, a trap found is a `gotcha`, steps for later work are a `plan`. When two kinds fit equally, ask the user once.
3. **Find the note on the subject.** Run `bilbo recall --limit 5 -- '<words that name the subject>'`, quoted as `recall`'s skill says. Read each hit whose file name or heading path may be on the subject. The same subject is the same question, decision or component, under any kind or wording. A related but different subject gets its own note. The table:

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty or warning lines | Read the plausible hits. Update the one on the subject (step 5), or create (step 4) |
   | 1 | last line `bilbo: no notes match` | run at most one more query in the words the note would likely use, then create (step 4) |
   | 1 | `bilbo: no store at <root>` | no notes yet: create (step 4); `bilbo new` makes the store |
   | 1 | anything else | print bilbo's first stderr line and stop |
   | 2 | anything | print bilbo's first stderr line and stop |

4. **Create.** Run `bilbo new <kind> <topic> [--title '<title>']`. The topic is lowercase kebab-case ASCII, two to five words that name the subject, never a date or a task (`release-tags`, not `2026-10-03-task`). Add `--title` when the default (hyphens as spaces, first letter uppercase) reads badly, for example with names or acronyms. The table:

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty | stdout is the new file's absolute path: write the body (step 6) |
   | 1 | `bilbo: topic '<topic>' already has a note: <path>` | that note is the one on the subject: update it (step 5) |
   | 1 | anything else | print bilbo's first stderr line and stop |
   | 2 | names the kind, the topic or `--title` | fix that argument and run `bilbo new` once more. A second exit 2: print the first stderr line and stop |
   | 2 | anything else, such as `BILBO_HOME` | print bilbo's first stderr line and stop |

5. **Update.** Read the note, then Edit its body in place: add what is new, correct what is wrong, and remove what no longer holds. Keep `id` and `created`. When the note now records another kind, rename it first (next section).
6. **Body and frontmatter.** Read the file, then Edit below the `# ` title line. Keep the prose short: what was found or decided and why, then pointers (paths, commands, URLs) that show it. Use `##` sections and never a second `# ` line outside a code fence. The frontmatter keys are only `id`, `created` and `sources`, so never `kind`, `supersedes` or another key. For sources, see "Sources" below.
7. **Check.** Run `bilbo check`. The table:

   | Exit | output | What to do |
   |---|---|---|
   | 0 | empty | report (step 8) |
   | 1 | stdout lines | fix every line that starts with `notes/<the note's file name>: `, except a `topic:` or `id:` line naming another file, which goes in the report unfixed. Then run `bilbo check` again, at most three runs in all. Leave lines for other notes alone and count them |
   | 1 | stderr `bilbo: no store at <root>` | print it and stop |
   | 2 | anything | print bilbo's first stderr line and stop |

   If lines still name the note after the third run, report them.
8. **Report.** Say `created`, `updated` or `renamed` and give the absolute path (both paths for a rename), and how many `bilbo check` lines were left for other notes. When the skill stopped before writing, say what stopped it and claim no note.

The `Never` list:

- Write any file but the note's own.
- Create a note without `bilbo new`.
- Create a second note on a subject that has one.
- Set an environment variable in a command: a `BILBO_HOME=… bilbo …` prefix falls outside `allowed-tools`, as in `recall`'s skill.
- Chain commands with `;`, `&&` or a pipe.
- Invent a source.

### Finding the note on the subject through `recall`

`bilbo new` refuses only an exact topic, so `release-tags` and `release-tagging` would both be created. A `recall` query catches the near miss by words, and by meaning when an embedder is configured.

- `--limit 5` keeps the hits to read small.
- One retry, against `recall`'s two: a note that two queries miss is unlikely to be the same subject, and `bilbo new` still catches the exact topic.
- A `recall` that fails for any other reason (a bad config exits 2, for example) stops the skill. Creating blind would risk a duplicate, and the content is still in the conversation.

### Changing a note's kind: rename with `mv -n`

A plan that becomes a decision cannot go through `bilbo new decision <topic>`: the topic is taken. With the supersede step gone, the choice is between renaming the file and keeping the old kind for good. Renaming is safe, by the code and by a run of the built binary:

- `bilbo check` (`src/check.rs`) checks the name, frontmatter, title and the uniqueness of topic and id. After `mv plan-x.md decision-x.md`, one file holds the topic and the id. The scratch run reported nothing and exited 0.
- `store::read_notes` takes the kind from the file name. `recall` on the scratch store listed `decision-release-steps.md` as `decision`, with the same `created`.
- The vector cache is keyed by FNV-1a over `rank::input` (the passage's heading path and text). The heading path starts at the `# ` title, not the file name, so `bilbo index` keeps every vector of a renamed note, and nothing is embedded again.
- The digest's session memory holds paths, so a session may be shown the renamed note once more. That is harmless.
- `bilbo new <other kind> <topic>` after the rename still exits 1 and names the new file.

`mv -n` never overwrites. In a valid store the new name cannot exist, because the topic is unique. In a store that already holds both files there is nothing to rename: the skill updates the note of the requested kind, leaves the other file's bytes alone, deletes and renames neither, and reports the shared topic that `bilbo check` prints, for the user to decide. Were `mv -n` run anyway, it would leave both files in place.

Rejected:
- Forbidding a kind change: the note's kind then misleads every later `recall --kind` and digest line.
- `bilbo new` under the new kind plus a copy of the body: impossible while the old file holds the topic. Deleting it first would lose the id.
- A `bilbo rename` verb: a new verb, spec and tests for what one `mv` does.

### Sources

`bilbo new` writes no `sources` key, so the skill adds it with Edit between `created` and the closing `---`:

```
sources:
  - "code: src/new.rs"
  - "url: https://example.org/page"
```

- The types are `url`, `code`, `doc` and `search`. The value is the URL, the path (with `:line` when one line matters), the document's name, or the query.
- Only what the agent read or ran in this session and the note rests on counts as a source. Never one recalled from training or from another note. With none, the key is omitted, never `sources: []`.
- The legacy skill's rule stands, without its notebook paths.

### The compaction hook: SessionStart `compact`

`plugins/bilbo/hooks/hooks.json` becomes:

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "command -v bilbo >/dev/null 2>&1 || exit 0; bilbo digest; exit 0",
            "timeout": 5
          }
        ]
      }
    ],
    "SessionStart": [
      {
        "matcher": "compact",
        "hooks": [
          {
            "type": "command",
            "command": "command -v bilbo >/dev/null 2>&1 || exit 0; echo 'Context was compacted. If this session settled something later sessions should know, such as a decision, a gotcha or a plan, save it with the bilbo note skill once the current task allows.'",
            "timeout": 5
          }
        ]
      }
    ]
  }
}
```

- **Why SessionStart `compact`.** It is the only compaction-time output that reached the model in both tools on an auto-compaction (`probe.md`).
  - PreCompact cannot help: nobody gets a turn before an auto-compaction, and the output is dropped. The legacy dnix nudge reaches the model only on a manual `/compact`, and only after the summary.
  - PostCompact: plain stdout is dropped on auto-compaction in both tools. Claude Code 2.1.288 rejects its `additionalContext`, and Codex documents no `additionalContext` for it.
  - A reminder after compaction still works: the summary keeps what the session settled, and the agent can write it down then.
- **Wording.** "once the current task allows" keeps the nudge from hijacking the user's request. The line names "the bilbo note skill" rather than `/bilbo:note` or `$bilbo:note`, because the two tools spell the call differently.
- **Command shape.** `command -v bilbo … || exit 0` keeps it silent without bilbo, as the digest hook is: the skill it points to needs bilbo. It never runs `bilbo`, so no bilbo exit code can reach the tool. `echo` exits 0, so a trailing `; exit 0` adds nothing. The text holds no `'`, `\` or `$`, so `/bin/sh -c` (Claude Code) and `$SHELL -lc` (Codex, often zsh) print it alike.
- **No `${CLAUDE_PLUGIN_ROOT}`**, for the same reason as the digest hook: Codex's trust hash covers the command text, which then stays the same across releases.
- **Cost.** One line per compaction. Neither tool runs it on startup, resume or clear.

### Codex trust of two hooks

`agents::trust_hooks` already collects every `hooks/list` entry whose `pluginId` is `bilbo@bilbo`, trusts each `untrusted` or `modified` one in one `config/batchWrite`, and returns `Kept` only when all are trusted. `agents::forget_hooks` deletes every `hooks.state` key that starts with `bilbo@bilbo:`. The probe ran the built `bilbo setup` against a plugin with both hooks:

- It reported `hook installed: trusted in Codex`, then `hook kept: trusted in Codex`.
- Codex's `config.toml` held `bilbo@bilbo:hooks/hooks.json:session_start:0:0` and `…:user_prompt_submit:0:0`.
- The digest hook's hash was the same as with one hook (`sha256:25e2d1e2…`). So on an upgrade, only the new hook is untrusted, and the line says `installed`, as the `setup` delta's new scenario says.

No code change, then, beyond:

- The wizard summary in `src/setup.rs` (`" and trust its digest hook"`, around line 1878) becomes `" and trust its hooks"`, and `summary_names_the_hook_trust_when_codex_gets_the_plugin` expects the new string.
- Unit tests in `src/agents.rs` on a new recorded fixture, `tests/fixtures/agents/codex-app-server-list-two-hooks.txt`. It is a `hooks/list` reply from Codex 0.155.1 with the digest hook `trusted` and the compaction hook `untrusted`, the upgrade case. The tests check that `trust_hooks` writes exactly the `session_start` key and returns `Wrote { changed: false }`, and that a reply with both trusted returns `Kept`.
- The fake `codex` in `tests/common/fakes.rs` keeps listing one hook. `hook_step` only maps a `Trust` to a line, so a second hook in the fake would test `trust_hooks` again through shell parsing that is harder to keep right.

Recording the fixture, in a throwaway `HOME` and `CODEX_HOME`:

1. Install the one-hook plugin with the built bilbo: `bilbo setup --yes --no-timer --plugin-source <a checkout of origin/main>`. This trusts the digest hook.
2. Point the marketplace at the worktree: `codex plugin marketplace remove bilbo`, `codex plugin marketplace add <worktree>`, then `codex plugin add bilbo@bilbo`.
3. Pipe `initialize`, `initialized` and `{"id":2,"method":"hooks/list","params":{"cwds":["<HOME>"]}}` into `codex app-server`. Keep the `"id":2` line.
4. Replace the `CODEX_HOME` path with `<codex-home>` and `HOME` with `/fixture/home`, as the existing `codex-app-server-list-*.txt` do.
5. Write it in their format: `$ <request>`, `exit 0`, `--- stdout`, the reply, `--- stderr`.

`codex app-server` needs `PATH` to hold `codex`, and the pipe must stay open a few seconds (`; sleep 4`) for the reply to arrive. The probe ran these steps.

### Manifests and README

| File | Field | New value |
|---|---|---|
| `plugins/bilbo/.claude-plugin/plugin.json` | `description` | `Write and recall the notes agent sessions keep, through the bilbo CLI.` |
| `plugins/bilbo/.codex-plugin/plugin.json` | `description` | the same |
| `.claude-plugin/marketplace.json` | `plugins[0].description` | the same |
| `.codex-plugin` `interface` | `shortDescription` | `Write and recall notes across sessions` |
| `.codex-plugin` `interface` | `longDescription` | `Write Markdown notes that later agent sessions find again, and search them best passage first, with the bilbo CLI. Install the bilbo binary separately.` |
| `.codex-plugin` `interface` | `capabilities` | `["Read", "Write"]` |
| `.codex-plugin` `interface` | `defaultPrompt` | `["Recall what we decided about the release steps", "Keep this decision as a note for later sessions"]` |

- `capabilities`: Codex's validator (`validate_plugin.py`) accepts any non-empty strings. The 91 Codex plugin manifests under `~/.codex` use only `Interactive` (61), `Read` (38) and `Write` (65). The note skill writes files, so `Write` joins `Read`. Nothing in bilbo asks for `Interactive`.
- `defaultPrompt`: at most 3 entries, each 128 characters at most and ideally about 50 (`plugin-json-spec.md`). Both entries fit.
- `.agents/plugins/marketplace.json` has no description and does not change.
- `recall`'s description ends "NOT for writing a note." and becomes "NOT for writing a note (note).", so each skill names the other.
- README: the intro and "Agent plugin" bullet name the note skill. "From an agent" gains a paragraph on `note` and one on the compaction hook. "Set up" says setup trusts the plugin's hooks, not only the digest hook. "Remove" says the trust of the hooks.

## Risks / Trade-offs

- [Two note skills on rivendell until the cutover: dnix's `note` and the plugin's `bilbo:note`, with near descriptions] → Accepted by the user. The plugin's description names bilbo, and the legacy one names the notebook.
- [The agent writes notes unprompted when it judges something durable] → That is the point of the trigger. Every write ends in a report with the path, so the user sees each one. The smoke test found Sonnet does not act on the description alone after a debugging answer (explicit asks and the compaction nudge work); a stronger nudge is a follow-up.
- [`recall` misses a note on the subject under other words, and a near duplicate is created] → `bilbo new` still refuses the exact topic, and a later `recall` shows both. Merging them is a manual edit.
- [`mv -n`'s exit code when the target exists is not the same on every platform and coreutils release] → The skill does not trust the exit code. It runs `bilbo check`, which names both files when the move did not happen, and reports that.
- [The compaction line fires in every session with the plugin, including sessions with nothing worth keeping] → One line per compaction, phrased as conditional. It replaces nothing the user relies on, since the legacy PreCompact nudge never reached the model on an auto-compaction.
- [Codex's login shell can rebuild `PATH` without `bilbo`] → The hook then stays silent, as designed. A normal install puts `bilbo` in a profile folder the login shell keeps. The smoke test links it into `$HOME/.nix-profile/bin`.
- [Claude Code's PreCompact and PostCompact JSON schema may gain `additionalContext` later, as the docs already claim] → It would still arrive after an auto-compaction, no earlier than SessionStart. Nothing to change.

## Migration Plan

- A bilbo release with this change brings the skill and the hook to every machine where `bilbo setup` installed the plugin. Re-running `bilbo setup` (dnix activation does) trusts the new hook in Codex.
- On rivendell, dnix's `note` skill and its PreCompact hook keep running until the cutover. Until then, notes the legacy skill writes go to notebooks, and notes `bilbo:note` writes go to bilbo's store.
- Rollback: pin the previous release. A store written by the skill is a valid `note-store` store either way.
