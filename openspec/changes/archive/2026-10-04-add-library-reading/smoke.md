# add-library-reading smoke test

## Claude Code

Run on rivendell (macOS) on 2026-10-04 with Claude Code 2.1.289, model `sonnet`. Binary: the release build of this worktree, copied from the rehearsal (`$R/rehearsal/bin/bilbo`) to `$S/bin/bilbo`. Plugin: `--plugin-dir <worktree>/plugins/bilbo` (the `bilbo:reference` skill of this worktree). `$S` is `/private/tmp/claude-501/-Users-delucca-Developer/a3d44115-3b38-427a-b7d4-1a2b3a9c0d6c/scratchpad/smoke-claude`.

Setup: each `~/Notebooks/*/library/` copied to `$S/notebooks/<folder>/library/` (8 notebooks), migrated with the library-migrate tool as in `rehearsal.md` (`BILBO_HOME=$S/store`, 8 corpora, 441 sources, 54 rejoined, `bilbo check` exit 0). Every `claude` run carried `BILBO_HOME=$S/store`, `XDG_STATE_HOME=$S/state`, `BILBO_CONFIG=$S/empty.toml` (empty) and `$S/bin` first on `PATH`, from `$S/work`: `claude -p --model sonnet --setting-sources "" --plugin-dir <worktree>/plugins/bilbo --output-format stream-json --verbose --max-turns 40 "<question>"`, with no `--allowedTools`. Raw streams: `$S/step<N>.jsonl` (the runs below), `$S/step<N>-plain.jsonl` (first attempts, see finding 1). Steps 1, 3, 3b, 4 and 5 ran in parallel against one state folder. Nothing under `~/Notebooks` was written.

All six final runs ended with `permission_denials: []`. The `bilbo cite --plan <plan> <<'EOF'` heredoc ran without a denial in every run that used it (steps 1, 2, 3, 3b), in the main session and in the readers.

## Summary

| Step | Result | Evidence |
|---|---|---|
| 1 Effective Go getters, first run | pass, led to fixes (findings 1, 2, 3) | needs a trigger phrase; `library`, `library go`, picks message, `plan`, one `read`, `cite --plan` heredoc, final 3 lines equal the last cite output |
| 2 Go panic vs error, over 60k tokens | pass | 160,628 tokens, 3 partitions, 3 `Agent` `general-purpose` calls in one assistant turn, 22 of 22 slices read |
| 3 clippy `needless_return`, first run | pass, led to a fix (finding 4) | pick resolved as `rust/clippy-lints#needless\_return 📋 style warn` after 4 failed `show` calls; the catalog was never planned or read |
| 1 re-run after SKILL.md fixes | pass | no `--slice-bytes` flags, `plan` default, 6 of 6 slices read in order, `not read: none`, last 3 lines equal the last cite output, no denial |
| 3 re-run after SKILL.md fixes | fail on one check (finding 6) | no unanchored `show rust/clippy-lints`, no guessed source name, catalog listed as lookup only; but a `grep` of `$S/store/library/rust/guide.md` ran with no denial, and the answer cites two unrelated sources |
| 3 re-run after the widened `Never` | no store access, but no answer (finding 7) | tool_use events: no Read/cat/grep/rg/sed/find on a path under `$S/store`; `bilbo library rust` printed 55 KB, the harness persisted it, and the model stopped asking the user for the source name |
| 3 re-run after the saved-guide fix | pass on the lead's checks, answer still empty (finding 8) | `Read` of the saved `bilbo library rust` output; no store path read or searched; catalog never planned; `clippy-lints` listed `lookup only, not searched` after 5 failed `show` anchors; last 3 lines equal the last cite output |
| 3b revive `add-constant` | pass | pick `go/revive-rules-descriptions#add-constant`, 1 of 1 slices |
| 4 COBOL (no corpus) | pass | only `command -v bilbo` and `bilbo library` ran; reply says no corpus covers it |
| 5 no `bilbo` on PATH | pass | `command -v bilbo` exit 1, reply is exactly `reference: bilbo is not on PATH; install the bilbo CLI first` |

## Step 1: Effective Go, getters

Prompt: "Check the library sources: What does Effective Go say about naming getters?" (the bare question never reaches the skill, finding 1).

```
Skill bilbo:reference
Bash  command -v bilbo
Bash  bilbo library
Bash  bilbo library go
text  picks from go (1 of 63 sources): - effective-go: the official guide; its Names chapter covers getter naming.
Bash  bilbo library plan 'go/effective-go' --slice-bytes 8000 --slice-lines 250
Bash  bilbo library read 01M42J5RJBQ8J2H5E6WQMSADS4 2
Bash  bilbo cite --plan 01M42J5RJBQ8J2H5E6WQMSADS4 <<'EOF' ... EOF     (exit 0)
```

No Read, `cat` or `grep` of a file under `$S/store/library/`. Denials: none. The reply ends with:

```
citations: 4 checked, 4 ok
coverage: plan 01M42J5RJBQ8J2H5E6WQMSADS4: read 1 of 20 slices (2410 of 40927 tokens); not read: go/effective-go lines 8-153 (slice 1), go/effective-go lines 328-3277 (slices 3-20)
picked: plan 01M42J5RJBQ8J2H5E6WQMSADS4: go 1 of 63 sources (effective-go)
```

The three lines equal the last `bilbo cite` output (checked by script). The picks block comes before the plan and again after the answer.

## Step 2: several Go sources

Prompt: "Across the Go sources (Effective Go, the Google and Uber style guides, and the Go FAQ), what do they say about when to use panic versus returning an error?" This one triggered the skill without a prefix.

```
Skill bilbo:reference; command -v bilbo; bilbo library; bilbo library go
text  picks block
Bash  bilbo library plan 'go/effective-go' 'go/go-frequently-asked-questions-faq' 'go/google-go-style-decisions' 'go/uber-go-style-guide'
Read  .../skills/reference/references/reader.md
Agent x3 in one assistant message: reader-1 (slices 1-8), reader-2 (9-15), reader-3 (16-22), all general-purpose
Bash  bilbo cite --plan 01M42J2GMZRGBT5Q42VY85DW3N <<'EOF' ... EOF     (exit 0)
```

The plan: 22 slices, 160,628 tokens, 3 partitions. Each reader prompt holds the question, the plan id, `Partition k of 3`, its slice range, one line per source, and the Reading rules, Citation rules, Support judgment and Output contract blocks. The subagent events show only `bilbo library read <plan> <n>` (slices 1-8, 9-15, 16-22 as assigned) and `bilbo cite --plan ...` calls: no Read, `cat`, `grep` or other tool, and no denial. Final lines:

```
citations: 15 checked, 15 ok
coverage: plan 01M42J2GMZRGBT5Q42VY85DW3N: read 22 of 22 slices (160628 of 160628 tokens); not read: none
picked: plan 01M42J2GMZRGBT5Q42VY85DW3N: go 4 of 63 sources (effective-go, go-frequently-asked-questions-faq, google-go-style-decisions, uber-go-style-guide)
```

Equal to the last cite output. The plan carried no `--slice-bytes` flags, as the skill says for the Agent case.

## Step 3: clippy `needless_return`

Prompt: "Check the library sources: What does clippy say about needless_return?"

```
Skill bilbo:reference; command -v bilbo; bilbo library; bilbo library rust   (55.5 KB, persisted to a file by the harness)
Bash  bilbo library show 'rust/clippy#needless_return'              exit 1: no source 'rust/clippy'
Bash  bilbo library show 'rust/clippy-lints#needless_return'        exit 1: no section 'needless_return'
Bash  bilbo library show 'rust/clippy-lints'                        282.4 KB heading list, persisted
Bash  bilbo library show 'rust/clippy-lints#needless\_return'       exit 1: no section
Bash  bilbo library show 'rust/clippy-lints#needless\_return 📋 style warn'   resolved
text  picks from rust (1 of 117 sources): - clippy-lints#needless\_return 📋 style warn: ...
Bash  bilbo library plan 'rust/clippy-lints#needless\_return 📋 style warn'   1 slice, 273 tokens
Bash  bilbo library read <plan> 1; bilbo cite x2 (exit 0 both)
```

The whole catalog (356,459 tokens) was never planned or read. The reply cites anchor `needless\_return 📋 style warn > What it does`, two claims, and says a third claim (MachineApplicable) was dropped as `too_short`. Final lines: `citations: 2 checked, 2 ok`, a one-slice coverage line and `picked: ... rust 1 of 117 sources (clippy-lints#needless\_return 📋 style warn)`, equal to the last cite output.

## Step 3b: control, revive `add-constant`

Prompt: "Check the library sources: What does revive's add-constant rule check?" Picks block: `revive-rules-descriptions#add-constant`; `bilbo library show 'go/revive-rules-descriptions#add-constant'` resolved at once, plan 1 slice (349 tokens), `read: 1 of 1`, `citations: 3 checked, 3 ok`, `picked: ... go 1 of 63 sources (revive-rules-descriptions#add-constant)`.

## Step 4: no corpus covers it

Prompt: "Check the library sources: What does the COBOL 2014 standard say about the PERFORM statement?" Commands: `command -v bilbo`, `bilbo library`. No `plan`, `read` or `cite`. The reply: "The library has no corpus that covers COBOL, so I can't answer from its sources. The skill says not to answer from memory ..." and lists the 8 corpora. Denials: none.

## Step 5: no bilbo on PATH

`PATH=/Users/delucca/.local/bin:/usr/bin:/bin` (`command -v bilbo` there exits 1; it holds `claude`). Commands: `Skill bilbo:reference`, `Bash command -v bilbo` (exit 1). The reply is exactly `reference: bilbo is not on PATH; install the bilbo CLI first`. Denials: none.

## Re-runs after the SKILL.md fixes

The lead changed SKILL.md after the first runs (step 4 checks for a tool named `Agent` and adds the slice flags only without it; step 5 reads every slice you read yourself, in order; `Never` gains "run `bilbo library show` on a catalog without an anchor" and "guess a source name"). Same setup and prefix. Streams: `$S/step1.jsonl`, `$S/step3.jsonl`; the first runs are `step1-first.jsonl` and `step3-first.jsonl`.

### Step 1 re-run: pass

Commands: `Skill bilbo:reference`, `command -v bilbo`, `bilbo library`, `bilbo library go`, the picks message, `bilbo library plan 'go/effective-go'` (no flags), "One partition, so I'll read it myself, in order.", `bilbo library read <plan> 1` to `6`, one `bilbo cite --plan` heredoc (exit 0). Denials: none; no file under the store read. The reply ends with:

```
citations: 4 checked, 4 ok
coverage: plan 01M42JAWWAA8VEDFR0R9YF97RB: read 6 of 6 slices (40922 of 40922 tokens); not read: none
picked: plan 01M42JAWWAA8VEDFR0R9YF97RB: go 1 of 63 sources (effective-go)
```

Equal to the last cite output. Findings 2 and 3 are fixed.

### Step 3 re-run: one check fails

```
Bash  bilbo library; bilbo library rust                         (55 KB, persisted)
Bash  grep -i -A3 "^## .*clippy" $S/store/library/rust/guide.md   <- a source-store file read by grep
Bash  bilbo library show 'rust/clippy-lints#needless_return'    exit 1: no section
Bash  bilbo library show 'rust/clippy-lints#Needless return'    exit 1: no section
Bash  bilbo library plan 'rust/effective-rust-clippy' 'rust/introduction-clippy-documentation'
Bash  read 1, read 2; bilbo cite x2 (exit 0 both)
```

No unanchored `show rust/clippy-lints`, no guessed source name, the catalog was never planned or read, and the reply says it "was left as lookup only, not searched, because no section matched". Last 3 lines equal the last cite output. But the answer rests on two unrelated sources (`effective-rust-clippy`, `introduction-clippy-documentation`) that do not mention `needless_return`, and the model said so; it never narrowed the lookup to the full heading `needless\_return 📋 style warn`, so the user gets no answer the catalog holds.

### Step 3 third run, after the widened `Never` line: no store access, no answer

Stream `$S/step3.jsonl` (the second run is `step3-second.jsonl`). Commands: `Skill bilbo:reference`, `command -v bilbo`, `bilbo library`, `bilbo library rust`, `bilbo library --help`; then the final reply. A script over the tool_use events found no Read, `cat`, `grep`, `rg`, `sed` or `find` on a path under `$S/store`, and `permission_denials` is `[]`. No `show`, `plan`, `read` or `cite` ran. The reply says `bilbo library rust` printed 55 KB, the harness saved it to a file, the skill forbids opening a saved output, so it read no source and "won't answer from memory"; it asks the user for the Clippy source name, offers to be allowed to read the saved file, or asks the user to paste the entry.

### Step 3 fourth run, after the saved-guide fix: pass on the checks, no clippy answer

Stream `$S/step3.jsonl` (the third run is `step3-third.jsonl`). Commands: `command -v bilbo`, `bilbo library`, the picks sentence, `bilbo library rust`, then `Read` of the harness's saved output (`~/.claude/projects/.../tool-results/bet343ut6.txt`, not a store path); five `bilbo library show 'rust/clippy-lints#<anchor>'` calls, all exit 1 (`needless_return`, `clippy::needless_return`, `needless return`, `Needless Return`, `needless_return_with_question_mark`); `plan 'rust/effective-rust-clippy'` (1 slice), `read 1`, two `bilbo cite --plan` (first exit 1 `anchor_missing`, fixed to the heading `Item 29: Listen to Clippy`, then exit 0). A script found no Read, `cat`, `grep`, `rg`, `sed` or `find` on a path under `$S/store`; denials none; the catalog was never planned or read, and no unanchored `show` ran. The picks block lists `clippy-lints` as `lookup only, not searched`. The last 3 reply lines equal the last cite output (`citations: 2 checked, 2 ok`, `read 1 of 1 slices ... not read: none`, `picked: ... rust 1 of 117 sources (effective-rust-clippy)`).

The reply says the sources read do not mention `needless_return`; the clippy answer is still not given.

## Findings

1. The skill is not invoked by a bare question. With the exact prompts of task 9.1 ("What does Effective Go say about naming getters?", the clippy, revive and COBOL questions, and step 5's), Sonnet made no `Skill` call and answered from memory in 1 turn (`$S/step{1,3,3b,4,5}-plain.jsonl`; step 1's reply said "quoted from memory, not fetched", step 4's answered the COBOL question from memory), so `bilbo library` never ran, nor `command -v bilbo`. Only step 2's prompt, which says "Go sources", triggered it. The runs above prefix "Check the library sources: ". The description's trigger phrases ("what does the Go book say", "check the sources on X") do not reach a question that names a source but no "library".
2. (Fixed in the re-run) Step 1 passed `--slice-bytes 8000 --slice-lines 250` to `plan` although the Agent tool was available (step 2's run had it and added no flags). SKILL.md says to add them only without the Agent tool. The plan came out as 20 slices instead of 6.
3. (Fixed in the re-run) Step 1 read 1 of 20 slices (slice 2, "Getters") and said so in the coverage line; the skill does not say whether a one-partition plan must be read in full, and the run chose the slice that held the answer. The coverage line is accurate; "read whole" in the skill's summary line is not what ran.
4. (First run; the re-run no longer dumps the catalog, see finding 6) Step 3: with the migrated headings (`needless\_return 📋 style warn`, as in the rehearsal), the anchor `needless_return` does not resolve. The skill did not list it as `lookup only, not searched`; it found the full heading by running `show` on the whole catalog (282.4 KB of headings dumped into the context, persisted by the harness) and then retried with the literal heading, which resolved. Result differs from tasks.md 9.1 in the pick's spelling: `rust/clippy-lints#needless\_return 📋 style warn`, not `rust/clippy-lints#needless_return`. The `bilbo cite` anchors passed with the backslash and emoji. It also ran `show 'rust/clippy#needless_return'` first, a guess at the source name taken before reading the `rust` guide (55.5 KB, persisted).
5a. Step 2: the readers also ran `bilbo cite` themselves, as reader.md's output contract says; the main session then ran its own. No finding beyond that: no reader read a source another way.
5b. Task text 9.1 step 4 still names the Haskell report with no `haskell` corpus; the migrated store has one, so step 4 used COBOL as the lead decided.
6. Step 3 re-run: the model ran `grep -i -A3 "^## .*clippy" $S/store/library/rust/guide.md` through Bash. That is a read of a file under the store, which the skill's `Never` forbids, and `grep` is not in `allowed-tools`, yet `permission_denials` is `[]`: Claude Code's `-p` mode allows a read-only `grep` without a prompt, so the "no denial" check cannot catch it; only the tool_use events do. The lookup also failed to reach the catalog section (`needless_return` and `Needless return` both miss; the heading is `needless\_return 📋 style warn`), so the skill falls back to tangential sources instead of listing the catalog only as `lookup only, not searched` in the picks block. The guess `rust/clippy` is gone.

Whether the `Bash(bilbo cite *)` heredoc ran without a permission denial: yes, in all runs.

7. Step 3 third run: the `rust` guide is 55.5 KB, over the harness's tool-output limit, so `bilbo library rust` is always persisted to a file. The skill forbids both reading the store and opening a saved output file, so with the widened `Never` the model has no way to see the guide's entries and stops without an answer. A guide this size needs a narrower listing verb (or bilbo's output kept under the limit) for the skill to work on `rust` and `ai-tooling`.

8. Step 3 fourth run: the guide was read, but the catalog's heading style was never found. The model tried five guessed anchors, none the full heading `needless\_return 📋 style warn` that the first run reached, and gave up with `lookup only, not searched`. That outcome is allowed by the skill, but it leaves the question unanswered while the section exists. The skill's `show` step does not tell the model to list the catalog's real heading style (only unanchored `show`, now forbidden, shows it).
9. Codex has no Read tool and cuts the middle of a large output, so the saved-guide Read does not apply there: guides over about 30,000 characters (rust, cognition, lisp, ai-tooling) stay a Codex limitation for a follow-up.

## Codex

`codex --version`: codex-cli 0.155.1. Run 2026-10-04 on rivendell. Scratch `$S` = `/private/tmp/claude-501/-Users-delucca-Developer/a3d44115-3b38-427a-b7d4-1a2b3a9c0d6c/scratchpad/smoke-codex`; throwaway `HOME=$S/home`, `CODEX_HOME=$S/home/.codex`, `BILBO_HOME=$S/store`, `XDG_STATE_HOME=$S/state`, empty `BILBO_CONFIG`. The env reached Codex's `/bin/zsh -lc` shell (the first `bilbo library` call listed the migrated store). `cwd` was `$S/work`, `--skip-git-repo-check`.

### Setup

Store: each `~/Notebooks/*/library/` copied to `$S/notebooks/<folder>/library/`, then the migration tool (8 corpora, 441 sources, 54 rejoined, 1 note: `jonase-eastwood` chapter 'Usage' repeats). The rehearsal `bilbo` was copied into `$HOME/.nix-profile/bin`.

```
$ bilbo setup --yes --no-timer --plugin-source <worktree>
codex installed: <worktree>
hook installed: trusted in Codex
```

### Summary

| Check | Result |
|---|---|
| `codex debug prompt-input hi` lists `bilbo:reference` | pass |
| Run 1: plan used `--slice-bytes 8000 --slice-lines 250` | FAIL: the model ran `bilbo library plan 'go/effective-go'` with neither flag (the finding that led to the step 4 fix) |
| Run 2 (fixed step 4): plan carried both flags | pass: `bilbo library plan --slice-bytes 8000 --slice-lines 250 'go/effective-go'`, 20 slices, 40,927 tokens, 1 partition |
| Every source read through `bilbo library read` | pass in both runs (6 reads, then 20), no cat/sed/rg of `$S/store/library/` |
| `bilbo cite --plan` ran, coverage line | pass: ran once, `citations: 2 checked, 2 ok` |
| Read output truncated, re-read in parts | no, none cut, no `--part` needed |
| Auth hash | same before and after; copy deleted; nothing written to `~/.codex` |

### Evidence

`codex debug prompt-input hi`: `- bilbo:reference: Answers a question from the library's sources in the bilbo store, read in full through a plan and cited by id. ... (file: r1/reference/SKILL.md)`.

`codex exec --skip-git-repo-check --sandbox danger-full-access --json "What does Effective Go say about naming getters?"` (stream in `$S/step1.jsonl`, exit 0). Commands in order: `cat .../skills/reference/SKILL.md`, `command -v bilbo`, `bilbo library`, `bilbo library go`, a picks message (`picks from go (1 of 63 sources): - effective-go`), `bilbo library plan 'go/effective-go'`, `cat .../skills/reference/references/reader.md`, `bilbo library read <plan> 1` to `6`, one `bilbo cite --plan <plan>` with a heredoc. The `cat`s read plugin files only (Codex has no Read tool).

The plan: 6 slices, 40,922 tokens, `partitions: 1`, slices of 7,400 to 8,200 tokens. The six reads were 23,365, 21,479, 22,552, 23,529, 23,337 and 4,066 bytes: every one whole, `-- end slice k/6 --` last, no `truncated output` notice. They are all above the 10,000-byte cap the probe found for models Codex does not know, so this session's model has the larger cap (about 40,000 bytes), which the skill cannot tell in advance. With `--slice-bytes 8000` the plan would have had more, smaller slices.

Reply: the claim that Effective Go omits `Get` (`Owner()`, not `GetOwner()`), two citations `bilbo:01M3EZ8NVEC2KJQNGK5DTK349R#Getters "..."`, the picks block, then the three lines word for word:

```
citations: 2 checked, 2 ok
coverage: plan 01M42J3NBS7Y10145VR7EWRC6Y: read 6 of 6 slices (40922 of 40922 tokens); not read: none
picked: plan 01M42J3NBS7Y10145VR7EWRC6Y: go 1 of 63 sources (effective-go)
```

### Run 2, after the step 4 fix (`$S/step1b.jsonl`)

`bilbo setup` reported `codex kept` and left the old SKILL.md in the plugin cache (same version 0.6.0), so a first run 2 attempt still used the old text and no flags (discarded). `bilbo setup --remove --yes` then a fresh `setup` refreshed the cache (byte-identical to the worktree file, step 4 now has the concrete command). Same `codex exec` flags, auth copied and deleted again, hash unchanged.

The model ran `bilbo library plan --slice-bytes 8000 --slice-lines 250 'go/effective-go'`: 20 slices, each under 8,000 bytes (3,539 to 7,934), every read whole with its end marker and no truncation notice, 20 `bilbo library read` calls and no other source read. One `bilbo cite --plan` run; the reply ended with `citations: 2 checked, 2 ok`, `coverage: ... read 20 of 20 slices (40927 of 40927 tokens); not read: none`, `picked: ... go 1 of 63 sources (effective-go)`.

### Findings

- Run 1: step 4's Codex branch ("Without the Agent tool, add `--slice-bytes 8000 --slice-lines 250`") was skipped by the model. It read the whole 40,922-token source in six slices without the Agent tool, which stayed correct only because Codex's cap here was large enough. Under the 10,000-byte cap every slice would have been cut. The instruction does not say how the model knows it has no Agent tool, and the skill's frontmatter names `Agent(general-purpose)`, `SendMessage` and `Read`, none of which Codex has.
- The truncation path (line-number gap, `--part`) was not exercised.
- The heredoc in the `cite` call quoted the draft with the model's own curly quotes in the reply but straight quotes in the check, and the check passed.

- A changed SKILL.md needs `bilbo setup --remove` then `setup` to reach the Codex plugin cache: a plain `setup` keeps the cached copy at the same version.
