# Smoke test against a real embedder

Run on 2026-10-02 from rivendell with a release build of the branch, against bagend's production llama-server: llama.cpp build 9190, Qwen3-Embedding-0.6B Q8_0, `--embedding --pooling last --parallel 1 -c 4096 -ub 512`. The server is reached through `ssh -N -L 28081:127.0.0.1:8081 bagend`. `BILBO_HOME`, `BILBO_CONFIG` and `XDG_CACHE_HOME` point into a fresh temp folder, shown below as `<tmp>`. Each note is created with `bilbo new` and gets one `##` section of text appended before the next command.

What it shows:

- The second `index` embeds nothing.
- Paraphrases that share no word with the note are found (`bicycle drivetrain upkeep`, `fish tank filtration`).
- A query with nothing near it exits 1.
- A note written after the last index ranks by keywords, and `recall` says it is not indexed.
- A dead embedder falls back to keywords with one stderr line.

```
$ cat $BILBO_CONFIG
embedder.url = http://127.0.0.1:28081
embedder.model = qwen3-embedding-0.6b
embedder.query_prefix = "Instruct: Given a question, retrieve notes that answer it\nQuery: "

$ bilbo new decision kitchen-sink-leak --title Kitchen sink leak
<tmp>/store/notes/decision-kitchen-sink-leak.md
[exit 0]

$ bilbo new gotcha bike-chain --title Bike chain
<tmp>/store/notes/gotcha-bike-chain.md
[exit 0]

$ bilbo index
embedded 2, kept 0, dropped 0
[exit 0]

$ bilbo index
embedded 0, kept 2, dropped 0
[exit 0]

$ bilbo recall bicycle drivetrain upkeep
<tmp>/store/notes/gotcha-bike-chain.md:8	gotcha	2026-10-02T18:53-03:00
Bike chain > Maintenance
Clean the chain with degreaser every 300 km and oil each link afterwards.
[exit 0]

$ bilbo recall water dripping below the bathroom washbasin
<tmp>/store/notes/decision-kitchen-sink-leak.md:8	decision	2026-10-02T18:53-03:00
Kitchen sink leak > Fix
The plumber replaced the trap under the basin and resealed the drain with silicone.

<tmp>/store/notes/gotcha-bike-chain.md:8	gotcha	2026-10-02T18:53-03:00
Bike chain > Maintenance
Clean the chain with degreaser every 300 km and oil each link afterwards.
[exit 0]

$ bilbo recall wumpus
bilbo: no notes match
[exit 1]

$ bilbo new plan pond-pump --title Pond pump
<tmp>/store/notes/plan-pond-pump.md
[exit 0]

$ bilbo recall koi
bilbo: 1 passages not indexed; run bilbo index
<tmp>/store/notes/plan-pond-pump.md:8	plan	2026-10-02T18:53-03:00
Pond pump > Choice
A 2,000 L/h submersible pump keeps the koi pond clear.
[exit 0]

$ bilbo index
embedded 1, kept 2, dropped 0
[exit 0]

$ bilbo recall fish tank filtration
<tmp>/store/notes/plan-pond-pump.md:8	plan	2026-10-02T18:53-03:00
Pond pump > Choice
A 2,000 L/h submersible pump keeps the koi pond clear.
[exit 0]


$ bilbo recall koi   # embedder.url pointed at a closed port
bilbo: embedder unavailable (embedder http://127.0.0.1:28099 unreachable: Connection refused (os error 61)); keyword results only
<tmp>/store/notes/plan-pond-pump.md:8	plan	2026-10-02T18:53-03:00
Pond pump > Choice
A 2,000 L/h submersible pump keeps the koi pond clear.
[exit 0]
```
