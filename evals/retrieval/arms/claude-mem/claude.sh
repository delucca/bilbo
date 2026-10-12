#!/bin/sh
# The `claude` claude-mem finds on PATH: Claude Code itself, run with exec so it keeps the pid claude-mem tracks and
# signals. With BILBO_CLAUDE_IO set, what it writes on stdout also goes to $BILBO_CLAUDE_IO/<start ns>-<pid>.jsonl (a pid
# alone repeats across reruns), after a first line with its working directory (observer-sessions/<session db id> for an
# observer) and its --model: the build reads the LLM usage and the account's limits from Claude Code's own reports there.
real=/opt/claude-code/claude
last=
for a in "$@"; do last=$a; done
if [ -z "${BILBO_CLAUDE_IO:-}" ] || [ "$last" = --version ]; then exec "$real" "$@"; fi
model= prev=
for a in "$@"; do
  [ "$prev" = --model ] && model=$a
  case $a in --model=*) model=${a#--model=} ;; esac
  prev=$a
done
log=$BILBO_CLAUDE_IO/$(date +%s%N)-$$.jsonl fifo=$BILBO_CLAUDE_IO/.$$.fifo
printf '{"type":"bilbo_process","pid":%s,"cwd":"%s","model":"%s","started":%s}\n' "$$" "$PWD" "$model" "$(date +%s)" \
  > "$log"
rm -f "$fifo" && mkfifo "$fifo" || exit 1
(exec 3<"$fifo"; rm -f "$fifo"; exec tee -a "$log" <&3) &
exec "$real" "$@" > "$fifo"
