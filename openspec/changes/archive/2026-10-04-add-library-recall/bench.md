# Keyword search speed over the real library

Measured 2026-10-03 on rivendell (Apple M5), with the installed bilbo 0.5.0 (`/nix/store/a58cx909dl4k4p70zhabsrikisz500xc-bilbo-0.5.0/bin/bilbo`), keyword only, no config file, warm page cache, `hyperfine -N -w 2 -r 15`.

bilbo 0.5.0 has no library search, so the library was laid out as notes. Plain `recall` then runs the code path `recall --library` will reuse: read each file, `note::read` for the body start, `rank::passages`, `rank::keyword`, one block per file. The source frontmatter parse and the section lookup that `--library` adds are small next to the word folding.

## Stores

Built in the session scratchpad by the script below, from the notebook libraries, which were only read.

| Store | Files | Bytes |
|---|---|---|
| `notes`: the notebooks' notes, `<kind>-<notebook>-<topic>.md` | 383 | 4,974,141 |
| `lib`: every `~/Notebooks/*/library/**/*.md` as `reference-<corpus>-<path>.md` | 1,172 | 13,619,831 |
| `both`: the two together | 1,555 | 18,593,972 |

The library has 1,172 files because the 54 split sources are still split; `migrate.py` rejoins them into 441 sources and 8 guides with the same bytes.

```python
import os, re, sys, glob, pathlib
S = sys.argv[1]
lib = pathlib.Path(S, "lib/notes"); lib.mkdir(parents=True, exist_ok=True)
notes = pathlib.Path(S, "notes/notes"); notes.mkdir(parents=True, exist_ok=True)
for libdir in glob.glob(os.path.expanduser("~/Notebooks/*/library")):
    corpus = re.sub(r".*--([a-z0-9-]+)__.*", r"\1", libdir.split("/")[-2])
    for p in pathlib.Path(libdir).rglob("*.md"):
        rel = str(p.relative_to(libdir))[:-3]
        topic = re.sub(r"[^a-z0-9]+", "-", (corpus + "-" + rel).lower()).strip("-")
        (lib / f"reference-{topic}.md").write_bytes(p.read_bytes())
for p in glob.glob(os.path.expanduser("~/Notebooks/*/notes/*.md")):
    name = os.path.basename(p)
    nb = re.sub(r".*--([a-z0-9-]+)__.*", r"\1", p.split("/")[-3])
    kind, _, topic = name.partition("-")
    if kind not in "plan spec design decision gotcha research review report reference".split(): continue
    dst = notes / f"{kind}-{nb}-{topic}"
    if not dst.exists(): dst.write_bytes(open(p, "rb").read())
```

## Results

`env -i BILBO_HOME=<store> BILBO_CONFIG=<empty file> HOME=<scratch> bilbo recall <query>`:

| Store | Query | Mean | Range | User | System |
|---|---|---|---|---|---|
| notes, 5.0 MB | `goroutine leak` | 46.5 ms | 46.1 to 47.0 | 39.0 ms | 6.5 ms |
| lib, 13.6 MB | `goroutine leak` | 137.3 ms | 135.6 to 141.1 | 116.0 ms | 19.9 ms |
| lib, 13.6 MB | `wumpus` (no match) | 136.2 ms | 128.2 to 151.2 | 114.2 ms | 20.1 ms |
| both, 18.6 MB | `goroutine leak` | 181.7 ms | 174.7 to 190.5 | 151.9 ms | 28.1 ms |

A first pass with a shell loop over the queries `needless_return`, `error wrapping context` and `embedder timeout decisao` gave the same picture: about 44 ms for notes, 128 to 135 ms for the library and 170 to 180 ms for both, after subtracting the loop's 14 ms timer overhead.

Lower bounds for any scan of the library: `cat` of the 1,172 files to `/dev/null` takes 39.4 ms (16.6 ms system); `grep -c -i -w goroutine -r` takes 250 ms.

## Reading

- `recall --library` over today's library costs about 140 ms, three times plain `recall` over today's notes. Time is linear in bytes: 46.5 ms for 5.0 MB, 137 ms for 13.6 MB.
- A hint in plain `recall` that reuses this path would take plain `recall` from 46.5 ms to about 182 ms on every call. A dedicated presence scan would still read 13.6 MB, at least the 39 ms `cat` takes.
- The 500 ms budget for a generated 14 MiB library leaves more than three times the measured time, close to the room plain `recall`'s 250 ms budget leaves over its 6 MiB store (about 5 times). At this rate the library reaches 500 ms near 50 MB.

## The ignored speed tests

Measured 2026-10-04 on rivendell (Apple M5, macOS), release build, `nix develop -c cargo test --release --locked --test recall -- --ignored --nocapture`, three runs. `common::bench_library` writes a 14.0 MiB generated library: 8 corpora, each with a guide (books-a and books-b hold three books each, catalog holds a 3,800-section lint catalog and a handbook, short-a to short-e hold 215 short sources each).

| Test | Store | Query | Times | Budget |
|---|---|---|---|---|
| `recall_library_over_14_mib_is_fast` | 14.0 MiB library | `embedder timeout decisao --library` | 145, 157, 154 ms | 500 ms |
| `recall_over_a_6_mib_store_is_fast` | 6.3 MiB notes beside the 14.0 MiB library | `embedder timeout decisao` | 63, 66, 69 ms | 250 ms |

The library test is on the line of the real library's 137 ms for 13.6 MB above, and plain `recall` never reads the library beside it.
