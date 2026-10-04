# Smoke tests

## Real pages (task 5.4)

Date: 2026-10-04, second run, after the converter and stage fixes (tables,
content lines with trailing whitespace, final url from the redirect history,
the `existing:` scan). Binary: `bilbo 0.7.0`, built from `b6bf3af` plus the
uncommitted work of this change. Every command ran under `env -i
HOME=$T/home BILBO_HOME=$T/store BILBO_CONFIG=$T/config
XDG_STATE_HOME=$T/state PATH=/usr/bin:/bin ./target/debug/bilbo ...`, with an
empty `$T/config` and an empty `$T/store` (a fresh store is just a folder).
Paths are shortened to `$T`. An earlier run printed `content: -` for Effective
Go; the fix below removed that.

### Stage

```text
$ bilbo library stage https://go.dev/doc/effective_go      # exit 0
stage: 01M43DA4FPTD1CMCCK13BRBM3B
capture: $T/state/bilbo/staging/01M43DA4FPTD1CMCCK13BRBM3B/capture.md
raw: $T/state/bilbo/staging/01M43DA4FPTD1CMCCK13BRBM3B/raw
media type: text/html
content: 116-2266
lines: 2292
tokens: 43189
title: Effective Go
keep: 120-2266

119	# Effective Go
123	## Introduction
133	## Formatting
179	## Commentary
185	## Names
...
-- stderr --
bilbo: navigation suspect: lines 77-111, 23 lines of links only
bilbo: navigation suspect: lines 2268-2276, 5 lines of links only
bilbo: navigation suspect: lines 2280-2288, 7 lines of links only
```

```text
$ bilbo library stage https://doc.rust-lang.org/cargo/reference/manifest.html   # exit 0
stage: 01M43DA4K4W4SHDCZP2TR867XX
capture: $T/state/bilbo/staging/01M43DA4K4W4SHDCZP2TR867XX/capture.md
raw: $T/state/bilbo/staging/01M43DA4K4W4SHDCZP2TR867XX/raw
media type: text/html
content: 24-499
lines: 503
tokens: 10478
title: The Manifest Format
keep: 25-499

1	## Keyboard shortcuts
20	# The Cargo Book
24	# The Manifest Format
80	## The `[package]` section
...
-- stderr --
(empty)
```

Neither first stage printed `final url:` (no redirect) or `existing:` (empty
store). Each stage folder holds `capture.md`, `raw`, `fetch.json` and
`stage.json`; `fetch.json` has `status` 200, `media_type` `text/html`,
`converter` `bilbo 0.7.0` and `final_url` equal to `url`.

### Against design.md

| Page | design.md | This run |
| --- | --- | --- |
| Effective Go | content 116-2266 of 2,292, 153 `<pre>` all fenced, 3 navigation runs | content 116-2266 of 2292, 153 `<pre>` = 153 fenced blocks (306 fence lines), 3 navigation runs |
| Cargo manifest | content 24-499 of 503, title The Manifest Format, keep 25-499 | the same, 25 `<pre>` = 25 fenced blocks (50 fence lines) |

Fences were counted with `grep -o '<pre' raw | wc -l` and `grep -c '^```'
capture.md`. Neither page holds a `<table>`, so the table rules were not
exercised. Neither capture warned of an unclosed fence or a lost heading.

Effective Go: the site menu is lines 1-111, the breadcrumb 116-117,
`# Effective Go` is line 119 and the last prose line, `And there you have
it: ...`, is 2266. Lines 2268-2292 are the footer and cookie notice, which
`keep: 120-2266` drops. Cargo: lines 1-23 (shortcuts help, theme list, book
title) and the license footnote and prev/next links at 500-503 fall outside
`keep`.

### Land and check

```text
$ bilbo library land 01M43DA4FPTD1CMCCK13BRBM3B go/effective-go --keep 120-2266 --title 'Effective Go'
source: $T/store/library/go/effective-go.md
id: 01M43DAAEA2YXKPH6HKKB70XYM
guide: $T/store/library/go/guide.md
capture folder: $T/store/.bilbo/captures/6c78382de7deeae10173817b2c55e55aa261fc58f7fa4c3e87fedadfd838c5fd

$ bilbo library land 01M43DA4K4W4SHDCZP2TR867XX cargo/manifest-format --keep 25-499 --title 'The Manifest Format'
source: $T/store/library/cargo/manifest-format.md
id: 01M43DAAF6V91WD2QT6X2TTY6E
guide: $T/store/library/cargo/guide.md
capture folder: $T/store/.bilbo/captures/e6cccefbc88c2be617d4235bcc3225922b40cfbfe72c8dc52966ba2e706ba00f
```

Both guides were new; the corpus leads and entries were written by hand in
an editor over the `TODO` lines. Then `bilbo check` exited 0 with empty output.

Frontmatter of the landed sources, with no `capture` key (the digests equal
the first run's, so the conversion is unchanged for these ranges):

```text
---
id: 01M43DAAEA2YXKPH6HKKB70XYM
fetched: 2026-10-04
origin: "url: https://go.dev/doc/effective_go"
digest: sha256:0acbb3a8b0de1b19e3cd1c5f19d6454431a64a29787c25f1fc067564d571acfc
kept: 120-2266
---
# Effective Go
```

```text
---
id: 01M43DAAF6V91WD2QT6X2TTY6E
fetched: 2026-10-04
origin: "url: https://doc.rust-lang.org/cargo/reference/manifest.html"
digest: sha256:062d043ce8329418ba61a7933f9c0390b82825e3779f38e67ee9f101f0362fbf
kept: 25-499
---
# The Manifest Format
```

Each capture folder holds `capture.md`, `fetch.json`, `landed` and `raw`.

### Staging Effective Go again

```text
$ bilbo library stage https://go.dev/doc/effective_go      # exit 0
stage: 01M43DAB7JQ9K4VFNQ0RVW8FHE
capture: ...
raw: ...
media type: text/html
content: 116-2266
existing: go/effective-go
lines: 2292
tokens: 43189
title: Effective Go
keep: 120-2266
```

`existing:` sits after `content:` and before `lines:`. The second stage
folder was removed afterwards, leaving the two landed stages' folders gone and
`staging/` empty.

### Findings

- The `content_lines` fix works: Effective Go now shows `content: 116-2266`,
  as design.md measured, and `keep` stops before the footer.
- `existing: go/effective-go` appears on the re-stage and not on the first.
- No redirect occurred on either page, so `final url:` stayed unexercised on
  real pages; the tests cover it.
- No tables on either page.
- The shell's `zoxide` doctor message appears on stderr of every command in
  this environment; it comes from the profile, not from bilbo.

## Skill in Claude Code (task 7.1)

Run on rivendell (macOS) on 2026-10-04 with Claude Code 2.1.289, model `sonnet`, `bilbo 0.7.0` (`/Users/delucca/Developer/delucca/.worktrees/bilbo.add-library-fetch/target/debug/bilbo`, not rebuilt). `$T` is `/private/tmp/claude-501/-Users-delucca-Developer/f8d9b729-c5be-4051-adfb-0e8834cac27b/scratchpad/smoke-skill`. Every run carried `BILBO_HOME=$T/store`, `BILBO_CONFIG=$T/config` (empty), `XDG_STATE_HOME=$T/state`, `target/debug` first on `PATH` followed by `/opt/homebrew/bin` (pdftotext), `~/.local/bin` (claude) and `/usr/bin:/bin`; step 4 dropped only `target/debug`. From `$T/work`: `claude -p --model sonnet --setting-sources "" --plugin-dir <worktree>/plugins/bilbo --output-format stream-json --verbose --max-turns 30 "<prompt>"`, no `--allowedTools`. The user's login was used (a throwaway `CLAUDE_CONFIG_DIR` is not logged in, per the earlier smokes). The store started as an empty folder. Streams: `$T/run-1.jsonl` (step 1), `run-2.jsonl` (step 2), `run-3a-404.jsonl` (step 3, first URL), `run-3.jsonl` (step 3), `run-4.jsonl` (step 4); copies are kept in the planning notebook's `work/add-library-fetch/smoke-skill/`. All five runs ended `subtype: success` with `permission_denials: []`, and no tool_result errors other than the ones listed.

| Step | Result |
|---|---|
| 1 effective_go | pass |
| 2 same prompt again | pass |
| 3 PDF | pass (after a bad test URL, see below) |
| 4 no bilbo on PATH | pass |

### Step 1: `Ingest https://go.dev/doc/effective_go into the library.`

Commands (20 turns): `Skill bilbo:ingest`; `command -v bilbo`; `bilbo library stage 'https://go.dev/doc/effective_go'`; two `Read` of the stage's `capture.md`; `bilbo library`; `bilbo library land 01M43DAZXPAAP72P123K0TJN3J go/effective-go --keep 119-2266 --title 'Effective Go'`; `bilbo library show go/effective-go`; `Read` of `store/library/go/guide.md` (for the Edit); `bilbo library plan go/effective-go`; `bilbo library read <plan> 1` to `6`; `Edit` of `go/guide.md`; `bilbo check`. Denials: none.

Checks:
- stage and land in the stream, no WebFetch: pass.
- source `go/effective-go.md` frontmatter is `id`, `fetched`, `origin: "url: https://go.dev/doc/effective_go"`, `digest`, `kept: 119-2266`; no `capture` key: pass.
- `kept` 119-2266 drops the menu (nav warning 77-111) and the footer (2268-2276, 2280-2288) and sits inside the suggested 120-2292 minus the footer; the landed body opens at `# Effective Go` and ends at "a useful web server in a few lines": pass. The agent began at 119, one line before stage's suggested 120; the line is the title heading's blank-adjacent line, harmless.
- guide entry is three sentences of prose (covers, when to consult, 2009 date, no generics or modules, predates `context` and `errgroup`); the new corpus lead replaced its TODO; `grep TODO guide.md` finds none: pass.
- `bilbo check` exit 0 (run by the agent and again by hand): pass.
- report gives the absolute path, id `01M43DB5MQA577MZV8FRNPXQP1`, `go/effective-go`, fetched (not external), the keep range, the warnings and the check count: pass.
- body read only through `plan`/`read` (6 slices, in order); the only `Read` of a store file was `go/guide.md`, which the skill needs for Edit: pass.

### Step 2: the same prompt again

Commands: `Skill`, `command -v bilbo`, `bilbo library stage 'https://go.dev/doc/effective_go'`. No `land`. The agent said Effective Go is already in the library as `go/effective-go`, staged the capture as `01M43DCA9EB3J7N7QB0F93S00Z`, and asked "Do you want me to re-ingest it over `go/effective-go`?" before using `--replace`. `find $T/store -type f -exec shasum {} +` before and after were identical (7 files, including the `.bilbo/captures` folder). Pass. The run never read the capture or planned a keep, which is fine for the ask.

### Step 3: a PDF

First attempt, `https://www.rfc-editor.org/rfc/pdfrfc/rfc2119.txt.pdf` (`run-3a-404.jsonl`): the URL does not exist (404 with curl too; rfc-editor serves no `pdfrfc/` for RFC 2119). The agent ran `command -v bilbo` and `bilbo library stage '<url>'` (exit 1, `answered 404`), then followed the exit-1 row: said bilbo could not fetch it, offered a text URL or a saved file, wrote nothing, did not use `curl` or WebFetch. A pass for the "cannot fetch" row, not for the step.

Second attempt, `Ingest this PDF: https://www.irs.gov/pub/irs-pdf/fw9.pdf` (140,815 bytes, `run-3.jsonl`). Commands: `command -v bilbo`, `command -v pdftotext`, `mktemp -d`, `curl -fsSL -o <dir>/source.pdf <url>`, `pdftotext -layout <dir>/source.pdf <dir>/source.txt`, `bilbo library stage <dir>/source.txt --origin 'url: https://www.irs.gov/pub/irs-pdf/fw9.pdf'`, `Read` of `capture.md`, `bilbo library`, `bilbo library land <stage> irs-forms/form-w9-request-for-taxpayer-id --keep 1-445 --title '...'`, `plan`, `read` 1 to 3, `Read` + `Edit` of `irs-forms/guide.md`, `bilbo check`. Denials: none (the `mktemp -d` folder under `/var/folders`, `curl` and `pdftotext -layout` all matched `allowed-tools`).

Checks: source frontmatter has `origin: "url: https://www.irs.gov/pub/irs-pdf/fw9.pdf"` and `capture: external`: pass. New corpus `irs-forms` with its lead and entry written, no TODO: pass. `bilbo check` exit 0: pass. The report says `capture: external`, gives path, id `01M43DDVSHXCX66FA6X5B7DRCV`, keep 1-445, and notes that `-layout` interleaves the form's columns.

### Step 4: no `bilbo` on PATH

Commands: `Skill`, `command -v bilbo` (exit 1). The reply is exactly `ingest: bilbo is not on PATH; install the bilbo CLI first`. The store hashes were identical before and after, and no stage was made. Pass.

### Findings

1. No skill-text failure and no bilbo failure in any run. The `Agent` check (plan with defaults) worked: no slice flags were used.
2. The prompt in 7.1 step 3 needs a real URL: rfc-editor.org has no `rfc2119.txt.pdf`. The IRS Form W-9 PDF is stable and small; use it in the task text.
3. `Read` of the corpus `guide.md` is needed for `Edit`, so the skill's "Read is for `capture.md` only" in the spec's guide-entry requirement is slightly narrower than what the agent must do; the agent read only `guide.md` and never a source. If the spec is to be exact, say "Read of a source is never used".
4. Steps 1 and 3 each started a new corpus because the store was empty; the corpus-fit branch (an existing guide that fits) was not exercised.
