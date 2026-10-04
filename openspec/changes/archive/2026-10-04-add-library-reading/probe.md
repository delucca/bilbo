# Shell output cap probe

Run on rivendell (macOS) on 2026-10-04 with Claude Code 2.1.289 and Codex 0.155.1. The question was how much of one shell call's output reaches the model in each tool, and in which mode: cut inline, or persisted to a file with a short preview. That fixes the `--slice-bytes` and `--slice-lines` values the reference skill passes without an Agent tool. The scripts are in `probe/`. `$PROBE` is a scratch folder. Raw transcripts and request logs stayed in the session scratchpad and were not kept.

`probe/gen.py bytes <KB>|exact <bytes>|lines <n>` prints numbered lines (`<n>\t<text>`, the form `bilbo library read` prints) and ends with `-- end probe <label> --`:

| Output | Bytes | Lines |
|---|---|---|
| `bytes 8` | 8,178 | 103 |
| `bytes 24` | 24,541 | 305 |
| `bytes 48` | 49,165 | 609 |
| `bytes 96` | 98,302 | 1,213 |
| `lines 100` | 1,710 | 101 |
| `lines 300` | 5,510 | 301 |
| `lines 600` | 11,210 | 601 |
| `lines 1200` | 23,013 | 1,201 |

`exact <bytes>` gave the sizes used to find each cap. A reading of "gap" below means a jump in the line numbers.

## Claude Code

The probe ran `claude -p --model haiku --setting-sources "" --allowedTools Bash --permission-mode bypassPermissions --output-format json --session-id <uuid> "Run exactly this one Bash command and then reply with only the word DONE: python3 gen.py <args>"`, one session per output. `probe/claude-inspect.py` read each `tool_result` from the session transcript under `~/.claude/projects/`.

- Outputs up to `exact 29900` (29,889 characters in the tool result, the trailing newline trimmed) and `exact 30000` (29,970) arrived whole: no cut, no notice, the end marker last, no gap. That included `lines 1200` (1,201 lines, 23,012 characters), so there is no line cap.
- `exact 30100` (30,052 bytes), `exact 31000`, `exact 35000`, `bytes 48` and `bytes 96` were persisted. The tool result was 2,306 to 2,308 characters:
  - `<persisted-output>`, then `Output too large (<size>KB). Full output saved to: <path under the session's tool-results/>`, a blank line, `Preview (first 2KB):`, the first 2KB of the output, `...` and `</persisted-output>`.
  - The preview is the head only. It holds no end marker and the lines after it are not in the result.
  - `toolUseResult` carried `persistedOutputPath` and `persistedOutputSize` (49,165 for `bytes 48`).
- The cap is therefore 30,000 characters of output: 29,970 passed whole, 30,052 did not.

## Codex

The probe used a throwaway `HOME` and `CODEX_HOME` and `probe/codex-config.toml` (a model provider pointing at `probe/mock-shell.py` on 127.0.0.1:18799, with `<probe-work>` replaced by a trusted working folder). `probe/mock-shell.py` logs each request. Codex's own `tools` list, read from the first request, shows the shell tool: `exec_command`, with `cmd`, `yield_time_ms`, `max_output_tokens` ("Output token budget. Defaults to 10000 tokens") and no line limit. The mock answers the turn's first request with an `exec_command` function call (`cmd` the generator, `yield_time_ms` 30000) and the request that carries a `function_call_output` with a plain `DONE`. `probe/run-codex.sh <label> <gen args>` runs `codex exec --skip-git-repo-check --json "probe <label>" </dev/null` (without `</dev/null`, `codex exec` waits on stdin) and keeps the log. `probe/cx-inspect.py` read the `function_call_output` of the last request. The command ran under `/bin/zsh -lc`.

The cap depends on the model's metadata, which the mock chooses with `model` in `config.toml`:

- `mock-model` (also `gpt-5-codex`, `gpt-5`, `gpt-5.1-codex-max`, for which Codex 0.155.1 printed "Model metadata ... not found" and used its fallback): a cap of 10,000 bytes.
  - `exact 9000`, `9900` and `10000` arrived whole. `exact 10100` was cut. `bytes 8` (8,178) arrived whole; `bytes 24`, `48`, `96` and `lines 600`, `lines 1200` (11,210 and 23,013) were cut, `lines 100` and `lines 300` were not.
  - The result was about 10,200 characters: head and tail kept, the middle cut. A line `Warning: truncated output (original token count: <n>)` and `Total output lines: <n>` opened the output, and the cut sat inside one line as `<head of line>…<k> chars truncated…<tail of line>`. For `bytes 24`, lines 1 to 63 and 244 to 304 survived, with the end marker; the gap was 63 to 244.
- `gpt-5.5` (known to Codex, no warning): a cap of 10,000 tokens, counted at about 4 bytes a token, so about 40,000 bytes.
  - `exact 38000` and `39900` arrived whole, `exact 40100` was cut. `bytes 8` and `bytes 24` (24,541) arrived whole, `lines 600` and `lines 1200` (23,013, 1,201 lines) too, so there is no line cap.
  - `bytes 48`, `96` and `exact 44000` were cut to about 40,200 characters in the same shape, with the notice `…<k> tokens truncated…`. For `bytes 48`, lines 1 to 249 and 363 to 608 survived with the end marker.
- Every output opened with a wrapper of about 100 characters (`Chunk ID`, `Wall time`, `Process exited with code`, `Original token count`, `Output:`) that the cap does not seem to count.
- Codex never persisted an output to a file in these runs.

## What the modes mean for the checks

- Claude Code persists above 30,000 characters. The tool result then holds a preview of the first 2KB and a path, with no end marker and no later lines. A reader that sees no `-- end slice` has not read the slice, which is what the end-marker check says. A slice that stays under the cap arrives whole.
- Codex cuts inline and keeps both ends. The end marker survives a cut, so the marker alone says nothing: a gap in the line numbers (or the `truncated output` notice) is the only sign, which is the reason the line-gap check exists. Both checks are needed, one per tool.
- Neither tool has a line cap, so `--slice-lines` is a guard on slices of very short lines, not a measured limit. Short lines are cut by bytes first: 600 lines of 18 bytes were cut under the 10,000-byte cap and passed under 40,000.

## Conclusions

- Claude Code's cap is 30,000 characters of output, inline up to there and persisted above. The default 24,000 printed bytes holds: it is 80% of the cap, and `bytes 24` (24,541 bytes with its numbers and marker) arrived whole.
- Codex's cap is 40,000 bytes for a model Codex knows and 10,000 bytes for one it does not, and the skill cannot tell which it runs under. The smaller cap is 10,000 bytes, so under Codex the skill passes `--slice-bytes 8000`: 80% of 10,000. At 40,000 the same value is 20%, which only costs more slices.
- `--slice-lines 250` stays: no line cap was seen, so the value only bounds slices of short lines, and 8,000 / 250 = 32 bytes a line is the average at which the byte bound and the line bound meet (the probe's short lines were 18 bytes, a numbered text line is 40 to 90).
- Step 4 of the reference skill passes `--slice-bytes 8000 --slice-lines 250` when it has no Agent tool, and nothing when it has one (the readers are Claude Code subagents, under the 30,000 cap and the 24,000 default).
