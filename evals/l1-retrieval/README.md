# L1 retrieval eval

How well `bilbo recall` and `bilbo digest` find the right notes.

The claim is narrow: these numbers measure retrieval over curated synthetic notes. They say nothing about notes an
agent wrote, about agent behavior, or about other memory tools.

`eval.py` ranks the queries of one split of a frozen dataset with six arms and scores them with ir_measures:

| arm | what it is |
|---|---|
| oracle | the labelled notes, best grade first. It must score 1.0 or the run aborts: it proves qrels, ids and scoring |
| random | a seeded shuffle of the candidates, 20 seeds averaged |
| ripgrep | `rg -i -w -F` per query word (English and a short Portuguese stop list dropped, no accent folding); notes ordered by distinct words found, then matches |
| bm25 | Lucene BM25 (k1 1.2, b 0.75) with English stemming over whole notes; zero-score notes dropped |
| bilbo-keyword | `bilbo recall --limit 100` with no embedder |
| bilbo-full | the same after `bilbo index`, with the pinned embedder |

The headline is one comparison: **bilbo-full against bm25 on success@5 over all scored queries**, with a 95%
interval from a paired bootstrap that resamples whole fact families. The test split was sized for it. Every other
number is secondary. Metrics per stratum are success@5, reciprocal rank, nDCG@10, recall@10 and judged@10 (of what
came back, up to ten, the share somebody labelled; empty answers are left out. When it falls, bilbo surfaces notes
nobody judged, the sign a v2 dataset is due); multi-hop adds evidence@10, supersession adds new-above-old, no-answer reports whether anything came back.
Intervals are computed for success@5 only, and not printed under 10 fact families.

`bilbo digest` runs on every prompt of the split with keywords only, and at the default threshold 0.55 when
bilbo-full runs, and reports
false injection per negative label, coverage and hit given inject.

## Run

Needs a bilbo binary, `rg`, and for bilbo-full and the 0.55 digest `llama-server` and the pinned GGUF. From the
repository root:

```sh
nix develop -c cargo build --release --locked   # a baseline needs a release build: debug latency is ~6x
nix develop -c uv --directory evals/l1-retrieval sync --locked
nix shell --inputs-from . nixpkgs#llama-cpp -c nix develop -c uv --directory evals/l1-retrieval \
  run eval.py run --bilbo "$PWD/target/release/bilbo" --split dev --out results.json
nix develop -c uv --directory evals/l1-retrieval run eval.py diff baseline/0.19.0.json results.json
```

`--arms oracle,bm25,bilbo-keyword` needs no llama-server and no GGUF (the CI smoke run); `--no-digest` skips the
digest; `--baseline FILE` prints the diff after the run; `--keep-root` keeps the temp root to look at.

**Tune on dev, run test once per release.** The split is only a field; nothing enforces it.

Any guard that fires aborts the run with exit 1 and writes nothing: the oracle below 1.0, a dataset that does not
match its `SHA256SUMS`, an id or a path outside the dataset, an error exit from bilbo, `not indexed` or
`embedder unavailable` on stderr, a second `bilbo index` that embeds or drops anything, and results that contain the
temp root or `$HOME`. Scores never abort.

## The embedder

bilbo-full needs bilbo's pinned model, so an outsider can get it without bilbo's setup:

- URL: https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/370f27d7550e0def9b39c1f16d3fbaa13aa67728/Qwen3-Embedding-0.6B-Q8_0.gguf
- SHA-256: `06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439`

Pass it with `--model` (default `~/.cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf`); `eval.py` checks the hash.
llama.cpp on Metal and on a Linux CPU produce slightly different vectors, so `results.json` records the
llama-server version and backend, and a baseline is made on one named machine.

## Files

- `dataset/`: the frozen dataset and its datasheet, see `dataset/README.md`. `SHA256SUMS` verifies it with
  `shasum -a 256 -c SHA256SUMS`; the run takes the hash of that file as the dataset id, and `diff` refuses two runs
  of different datasets.
- `baseline/<version>.json`: the `results.json` committed at each release.
- `results.json`: schema 1; `per_query` and `per_prompt` hold one entry per line so git diffs stay readable.
  Rankings are not stored.

## Limits

- Synthetic notes, English and Portuguese, one author profile. The frozen `notes-synth/v1` holds 630 notes, 60 SQLite
  library pages, 366 queries (dev 186, test 180: 20 per stratum on test, multi-hop 14 and no-answer 10 on dev) and 436
  digest prompts. Tree hash of the generator's folder: `b51c2cee1801e656c6f7bb92683d766eab564e7e444830aa2d3b847478b8ac68`.
- No dense-only arm. "Does fusion beat dense alone" is a tuning question for bilbo's RRF, not a claim to users; if it
  comes back, add a hidden dense-only mode to `bilbo recall` so parity comes for free.
- ripgrep does not fold accents (`migração` is not `migracao`), as an agent's grep would not.
- The dataset cannot be regenerated bit for bit (two LLM CLIs on subscriptions). It was made by the generator at the
  git tag `evals/notes-synth-v1-generator`, which holds the code, prompts and label audit.

## Baseline: bilbo 0.19.0, test split

`baseline/0.19.0.json` and its report `baseline/0.19.0.md`, from the release build, llama.cpp 9190 on Metal (Apple
M5), run once. success@5 per stratum:

| arm | all | known-item | paraphrase | pt-en | alias | supersession | multi-hop | kind-filter | library |
|---|---|---|---|---|---|---|---|---|---|
| bilbo-full | 0.963 | 1.000 | 0.900 | 0.850 | 1.000 | 1.000 | 1.000 | 1.000 | 0.950 |
| bilbo-keyword | 0.887 | 1.000 | 0.700 | 0.500 | 1.000 | 1.000 | 0.950 | 1.000 | 0.950 |
| bm25 | 0.856 | 1.000 | 0.750 | 0.400 | 0.950 | 1.000 | 1.000 | 1.000 | 0.750 |
| ripgrep | 0.744 | 0.950 | 0.650 | 0.500 | 0.800 | 0.850 | 0.750 | 1.000 | 0.450 |

Headline, over all 160 scored queries: bilbo-full minus bm25 = +0.106 [+0.064, +0.151]. Against bilbo-keyword it is
+0.075 [+0.039, +0.116]; the gain sits in pt-en (+0.35) and paraphrase (+0.20). Supersession is the open weakness:
the new note ranks above the old one in 55% of bilbo-full's queries (bm25 40%).

Digest, 100 positive and 116 negative prompts: at 0.55 coverage is 1.0, hit given inject 0.90, and false injection
is 0.575 on noise, 0.05 on off-topic and 1.0 on near-miss; keywords only gives 0.79 hit given inject and 0.20, 0.95
and 1.0. Latency p50 is 98 ms for bilbo-full and 57 ms for bilbo-keyword.
