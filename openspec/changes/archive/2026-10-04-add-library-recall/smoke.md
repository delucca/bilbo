# Smoke tests

Run on rivendell (macOS) on 2026-10-04. `$T` is the session scratchpad's `smoke/` folder. The store was built with bilbo 0.8.0 (`main` at `500dd06`) from three real pages: `bilbo library stage <url>` and `land` for `https://go.dev/doc/effective_go` (`go/effective-go`, keep 119-2266), `https://go.dev/ref/mod` (`go/modules-reference`, keep 116-2382) and `https://doc.rust-lang.org/cargo/reference/manifest.html` (`rust/cargo-manifest`, keep 24-499). The guides were written by hand, and the store got two notes (`decision-release-tags`, `gotcha-goroutine-leak-in-tests`). `bilbo check` exited 0. `library-migrate` was not used: it runs only at the cutover. The full record, with every stream, is in the planning notebook's `work/add-library-recall/` (`smoke-cli.md`, `smoke-skills.md`, `smoke-skill/*.jsonl`).

## CLI

The binary was this change's debug build. Every run used `env -i HOME BILBO_HOME BILBO_CONFIG=<empty file> XDG_STATE_HOME PATH=/usr/bin:/bin`.

```text
$ bilbo recall goroutine leak --library        # exit 0
$T/store/library/go/effective-go.md:1681	source	go/effective-go	1681-1707
Effective Go > Concurrency > Goroutines
They're called *goroutines* because the existing terms—threads, coroutines, ...
```

The page's own `## Effective Go` heading sits below the `# Effective Go` title that `land --title` added, so `Effective Go` here is a real heading below the title.

| Check | Result |
|---|---|
| 20 queries with `--limit 10`: each source block's `<start>-<end>` equals the row `library show <ref>#<heading path>` prints, and `<line>` falls inside it | 36 of 36, 0 mismatches |
| Split part: `manifest features --corpus rust` | `cargo-manifest.md:16 ... 10-485`, inside the `## The Manifest Format` section that holds every subsection |
| Guide hits | `rust/guide.md:10 guide rust 10-12` with path `cargo-manifest`; `go/guide.md:10 guide go 10-13` with path `effective-go` |
| `wumpus --library` | `bilbo: no sources match`, exit 1 |
| `--corpus python` | `bilbo: no corpus 'python' in <root>/library`, exit 1 |
| A library holding only `.hidden/`, and a missing root | `bilbo: no library at <root>`, exit 1 |
| `--corpus Go`, `--corpus show`, `--library=go`, `--kind gotcha --library`, `--corpus go --kind gotcha` | usage error naming the option or corpus, exit 2 |
| `--corpus go -- --library` | searches the word `library` in `go`, exit 0 |
| Plain `goroutine leak`, `release tags` | only the note blocks |
| Plain `GOFLAGS` (in a source only) | `bilbo: no notes match`, exit 1 |
| Paths, modification times and sizes under the store | identical before and after the whole run |

## Skills in Claude Code

Claude Code 2.1.289, model `sonnet`, `claude -p --setting-sources "" --plugin-dir plugins/bilbo --output-format stream-json --verbose --max-turns 40`, no `--allowedTools`, a fresh copy of the store per run. There were 22 runs over five rounds, with the skill text revised between rounds; two of them used `main`'s unchanged plugin as a baseline. Every run ended `success`, with `permission_denials: []` and no Read, `cat` or `grep` of a store file. The CLI checks above were rerun on the final build with the same 36 of 36.

### Final text (round 5)

| Run | Prompt | Result |
|---|---|---|
| f1b | Use the reference skill: what does Effective Go say about goroutines? | pass: no `bilbo recall --library`, since the `effective-go` guide entry covers goroutines; it picked `go/effective-go` by its entry, posted `picks from go (1 of 2 sources):` before the plan, and read 6 of 6 slices with 11 citations ok |
| f2 | What does GOFLAGS do, according to the library? | pass: `reference` ran `bilbo recall --library -- 'GOFLAGS'` with no `--corpus` (no guide names it), picked `go/modules-reference#Go Modules Reference > Module-aware commands > ...` with the heading path whole, then planned, read and cited (3 ok) |
| f3 | Use the recall skill to search the library for what it says about defer and recover. | pass: `recall` ran `bilbo recall --library -- 'defer and recover'` and handed `go/effective-go#Effective Go > Errors > Recover` and `go/effective-go` (a guide hit) to `reference`, which ran no second lookup, then planned, read and cited |
| f4 | Find the passages in the library about version selection. | pass: `recall` ran `bilbo recall --library -- 'version selection'`, showed the four blocks, offered `reference` and ran nothing else |
| f1 | What does Effective Go say about goroutines? | no skill ran; the model answered from memory in one turn. A trigger miss: the `reference` description is unchanged by this change, and `add-library-reading`'s report already noted that a bare question does not trigger it |

### Earlier rounds

The earlier rounds ran on texts that changed afterwards. They drove these fixes: posting the picks became a numbered step of its own; a pick keeps the hit's heading path whole (run 1b widened it to `#Concurrency`); the GOFLAGS example went under the `--corpus` rule (run 2 had passed `--corpus go`); and the wording that ended a `-p` run on the picks message was removed (run 1c). Round 1's runs 3 (`Did we decide anything about release tags?`: plain `bilbo recall`, the decision note shown) and 4 (`Look up "wumpusfrobnicate" in the library.`: `no sources match`, query named, no notes search) cover paths the later edits did not touch. The final review found that the round-4 `--corpus` bullets sent questions a guide entry covers to a lookup. In new-1 and new-2, the lookup read only the 609-token Goroutines section, while the baseline read the whole 41k-token source and also cited Channels, Parallelization and Recover. The final text limits lookups to the spec's three cases.

### Not met

- **Picks message.** The agent skipped posting the picks as text before `bilbo library plan` in most runs, the final round included. A baseline with `main`'s unchanged skill skipped it in 1 of 2 runs. The model goes from a tool result straight to the next tool call. This predates the change, and a prose rule did not fix it. A fix that holds needs the picks in the tool flow. The picks block still ends every answer.
- **Common words.** `and` in `defer and recover` matched the go guide's `effective-go` entry. That guide hit became the whole-source pick `go/effective-go`, as the spec maps a guide hit, so f3 read the whole source. Keyword ranking has no stopwords, the same as in note recall.
- **Trigger.** A question that names a document but not the library (f1) did not trigger `reference`. That predates this change.
