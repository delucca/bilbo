#!/bin/sh
# usage: run-codex.sh <label> <gen args...>   (needs mock-shell.py on 18799 reading cx/cmd.txt)
D=/private/tmp/claude-501/-Users-delucca-Developer/a3d44115-3b38-427a-b7d4-1a2b3a9c0d6c/scratchpad/probe
label=$1; shift
echo "python3 $D/gen.py $*" > $D/cx/cmd.txt
: > $D/cx/log.jsonl
cd $D/cx/work && HOME=$D/cx/home CODEX_HOME=$D/cx/codex MOCK_KEY=x codex exec --skip-git-repo-check --json "probe $label" </dev/null > $D/cx/run-$label.json 2>&1
cp $D/cx/log.jsonl $D/cx/log-$label.jsonl
