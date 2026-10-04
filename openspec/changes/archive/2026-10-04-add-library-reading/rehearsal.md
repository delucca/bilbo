# Rehearsal of the cutover's citation step (task 8.1)

Date: 2026-10-04. Binary: `$T/bin/bilbo`, the release build of this worktree. `$T` is
`/private/tmp/claude-501/-Users-delucca-Developer/a3d44115-3b38-427a-b7d4-1a2b3a9c0d6c/scratchpad/rehearsal`.
Every command ran with `BILBO_HOME=$T/store`, `XDG_STATE_HOME=$T/state`, and also
`XDG_CACHE_HOME=$T/cache` and an empty `BILBO_CONFIG=$T/empty-config`. Raw outputs sit in `$T`
(`mig.out`, `cite1.out`, `plan1.out`, `read1-*.out`, `cite2.out`, `p5*.out`, `land*.out`).

## Summary

| Step | Result |
| --- | --- |
| 1 Copy, migrate, `bilbo check` | pass |
| 2 Rewritten draft, `bilbo cite` | pass (3 `ok`, exit 0) |
| 3 `plan go/effective-go`, slice sizes | pass (6 slices, largest 23,529 bytes) |
| 4 Partial read, `cite --plan` | pass (`ok` + `unread`, exit 1, coverage names slices 2-6) |
| 5 Catalog plan | partial: the catalog is refused as specified, but `#needless_return` does not resolve (finding 1) |
| 6 `land --replace` vs `--force` | pass (exit 1 naming the note; `--force` exit 0) |
| 7 `find ... -newer` | pass (prints nothing; `find ~/Notebooks -newer $T/started` also nothing) |

## Step 1: copy, migrate, check

- `cp -R ~/Notebooks/*/library $T/notebooks/<folder>/library` for the 8 notebooks that have one
  (ai-tooling, cognition, go, haskell, lisp, rust, software-architecture, writing); 15 MB.
- `BILBO_HOME=$T/store CARGO_TARGET_DIR=$T/migrate-target nix develop <worktree> -c cargo run --release --manifest-path <library-migrate>/Cargo.toml -- --notebooks $T/notebooks --report $T/report`
  (the README's flags match the task text; no difference). Exit 0.
- Counts: corpora 8, sources 441, rejoined sources 54, moved headers 26, map rows 1172, headings listed 8
  (`report/headings.tsv`: 1 haskell, 6 lisp, 1 writing). Note printed:
  `lisp__shared/library/jonase-eastwood/index.md: chapter 'Usage' repeats, its anchor is ambiguous`.
- `bilbo check`: no output, exit 0, 0.03 s.

## Step 2: draft with `bilbo:` citations

`$T/draft.md` is the estate note with its 3 citations rewritten to `bilbo:01M3EZ7Z37X9TVGNTXJ1CHNYHP#<heading> "<quote>"`
(the id of `extend-claude-with-skills-claude-code-docs`, from `map.tsv`).
`bilbo cite $T/draft.md`, exit 0:

```
41  ok  01M3EZ7Z37X9TVGNTXJ1CHNYHP#Available string substitutions  .../store/library/ai-tooling/extend-claude-with-skills-claude-code-docs.md  lines 366-366
60  ok  01M3EZ7Z37X9TVGNTXJ1CHNYHP#Control who invokes a skill  ...  lines 459-459
78  ok  01M3EZ7Z37X9TVGNTXJ1CHNYHP#Pre-approve tools for a skill  ...  lines 473-473
citations: 3 checked, 3 ok
```

## Step 3: plan and slices

`bilbo library plan go/effective-go`: exit 0, plan `01M42HTXWW0X5VVGERBCXK5X5G`, picks 1, slices 6, tokens 40922, partitions 1.
`bilbo library read <plan> <i> | wc -c`, all exit 0:

| Slice | Lines | Tokens | Bytes |
| --- | --- | --- | --- |
| 1 | 8-702 | 8230 | 23365 |
| 2 | 703-1308 | 7442 | 21479 |
| 3 | 1309-1941 | 7706 | 22552 |
| 4 | 1942-2556 | 8124 | 23529 |
| 5 | 2557-3173 | 8049 | 23337 |
| 6 | 3174-3277 | 1371 | 4066 |

All at most 24,000. Because this plan was fully read, step 4 used a fresh plan.

## Step 4: partial read

Fresh plan `01M42HV2P9VVADEVYF39CGADWW` (6 slices), `library read <plan> 1` only. `$T/draft2.md` cites
`#Introduction "focuses on simplicity, reliability, and efficiency, specifically designed"` (slice 1) and
`#A web server "Let's finish with a complete Go program, a web server."` (slice 6).
`bilbo cite --plan <plan> draft2.md`, exit 1:

```
3  ok      01M3EZ8NVEC2KJQNGK5DTK349R#Introduction   .../go/effective-go.md  lines 19-20
7  unread  01M3EZ8NVEC2KJQNGK5DTK349R#A web server   .../go/effective-go.md  lines 3176-3176 not read (slice 6)
citations: 2 checked, 1 ok
coverage: plan 01M42HV2P9VVADEVYF39CGADWW: read 1 of 6 slices (8230 of 40922 tokens); not read: go/effective-go lines 703-3277 (slices 2-6)
picked: plan 01M42HV2P9VVADEVYF39CGADWW: go 1 of 63 sources (effective-go)
```

## Step 5: catalog

- `bilbo library plan rust/clippy-lints`, exit 1:
  `bilbo: rust/clippy-lints is a catalog, too big to read whole: pick a section as 'rust/clippy-lints#<anchor>'; bilbo library show rust/clippy-lints lists them`.
  The catalog is 892 KB, 356459 tokens, 3813 headings, `catalog: yes`.
- `bilbo library plan 'rust/clippy-lints#needless_return'`, exit 1:
  `bilbo: no section 'needless_return' in rust/clippy-lints`. See finding 1.
- What does resolve (exit 0, 1 slice, 273 tokens, 1022 bytes when read):
  `bilbo library plan 'rust/clippy-lints#needless\_return 📋 style warn'`.
  `bilbo cite` with `#needless\_return 📋 style warn > What it does "Checks for return statements at the end of a block."` gives `ok`.

## Step 6: land with `--replace`

- `bilbo new gotcha claude-code-skill-frontmatter` made `$T/store/notes/gotcha-claude-code-skill-frontmatter.md`
  (id `01M42HWKQCAHTYBJF60SKWEFXQ`); the draft's body, title included, replaced the generated title.
  `bilbo check` then exited 0.
- The cited source's body (store file from line 8, after the 7-line frontmatter) with the `##### Available string substitutions`
  section (store lines 358-404) deleted is `$T/src-trim.md`, 935 lines.
  `bilbo library stage $T/src-trim.md --origin "url: https://code.claude.com/docs/en/skills"` printed `stage: 01M42HXMQC4KZCJ3TJ9A2FXWW3`, `lines: 935`, `keep: 2-935`.
- `bilbo library land 01M42HXMQC4KZCJ3TJ9A2FXWW3 ai-tooling/extend-claude-with-skills-claude-code-docs --keep 1-935 --replace`, exit 1:
  ```
  bilbo: 1 citation would degrade; nothing was written and the stage is kept; pass --force to replace anyway
  bilbo: notes/gotcha-claude-code-skill-frontmatter.md:36: ok -> anchor_missing
  ```
  The source file was byte-identical afterwards (`cmp`).
- The same with `--replace --force`, exit 0:
  ```
  bilbo: the kept lines hold a level-1 heading, so every heading in them moved one level down
  bilbo: notes/gotcha-claude-code-skill-frontmatter.md:36: ok -> anchor_missing
  source: .../store/library/ai-tooling/extend-claude-with-skills-claude-code-docs.md
  id: 01M3EZ7Z37X9TVGNTXJ1CHNYHP
  ```
  The id stayed the old one, `capture: external`, `fetched: 2026-10-04`. A `bilbo check` afterwards exits 1 on
  `library/ai-tooling/guide.md: entry 'extend-claude-with-skills-claude-code-docs': stale; ... (line 98)`, as the README says.
- An earlier attempt staged the file from line 7, which swallowed the frontmatter's closing `---` as body line 1. It gave the same
  exit 1 and the same citation line; the run above is the clean one. The stage's own `keep: 2-935` would not have moved the headings; I used `1-935` to keep all lines as asked.

## Step 7: nothing written under `~/Notebooks`

`find ~/Notebooks/*/library ~/Notebooks/*estate__shared/notes -newer $T/started` printed nothing, and neither did `find ~/Notebooks -newer $T/started`.

## Timings (migrated store, wall clock, warm)

| Command | Time |
| --- | --- |
| `bilbo check` (441 sources, 1 note) | 0.02-0.03 s |
| `bilbo library` | 0.02 s |
| `bilbo library rust` | under 0.01 s |
| `bilbo library show rust/clippy-lints` (356k tokens, 289 KB output) | under 0.01 s |
| `bilbo library plan go/effective-go` | 0.004 s |
| `bilbo library plan go/common-go-mistakes-100-go-mistakes` (6 slices, 47686 tokens) | under 0.01 s |
| `bilbo library plan 'rust/clippy-lints#...'` | 0.01 s |
| `bilbo library read <plan> 3` | 0.01 s |
| `bilbo cite draft.md` | 0.014 s |
| `bilbo cite --plan <plan> draft2.md` | 0.018 s |

Nothing is slow on the largest catalog.

## Findings

1. **Catalog anchors in the spec do not match the migrated clippy headings.** The migrated `rust/clippy-lints` headings are
   `## needless\_return  📋 style warn` (backslash-escaped underscores, a double space, an emoji and a level/group suffix), so
   `plan 'rust/clippy-lints#needless_return'` exits 1 with `bilbo: no section 'needless_return' in rust/clippy-lints`.
   `needless\_return`, `needless_return > What it does`, `needless\_return  > What it does` fail the same way, and
   `cite` gives `anchor_missing` with the detail `no section matches 'needless_return > What it does'; the quote is under needless\_r...`.
   Only the whole heading works: `needless\_return 📋 style warn`. This contradicts tasks.md 8.1, `library-reading` spec lines 32 and 40,
   `citations` spec line 43, `library-store` spec line 9, and `agent-plugin` spec lines 56-57 (the skill "picks `rust/clippy-lints#needless_return`").
   The tail match works on whole heading texts only, so a lint name is not a tail. Either the migration strips the escapes and badge from
   catalog headings, or the anchor rule matches a heading's leading word, or the specs use the real heading text.
2. `#What it does` on the catalog is ambiguous across all lints and prints a list of `absolute\_paths 📋 restriction allow > What it does (line 55)` and so on (expected, listed for the record).
3. The migration reported one repeated chapter, `jonase-eastwood` `Usage`, whose anchor is ambiguous, and 8 headings bilbo does not read as headings (listed in `report/headings.tsv`).
4. `land` suggests `keep: 2-935` (the title line excluded) while its message about a level-1 heading appears when the title is kept; this is consistent with the README, not a defect.
