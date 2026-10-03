#!/bin/sh
# usage: hook.sh <label> plain|json|none <event>
input=$(cat)
printf '%s %s\n' "$1" "$input" >> $PROBE/hooks.log
case "$2" in
plain) printf '%s: the word of this hook is %s-WORD\n' "$1" "$1" ;;
json) printf '{"hookSpecificOutput":{"hookEventName":"%s","additionalContext":"%s: the word of this hook is %s-WORD"}}\n' "$3" "$1" "$1" ;;
none) ;;
esac
exit 0
