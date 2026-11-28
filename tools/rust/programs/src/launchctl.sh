#!/bin/sh
S='@STATE@'
N=launchctl
printf '%s\n' "$*" >> "$S/$N.log"
armed() { [ -s "$S/$N.$1" ]; }
pick() {
  fkey=''
  if [ -n "$3" ] && armed "$1.$2.for.$3"; then fkey="$1.$2.for.$3"
  elif armed "$1.$2"; then fkey="$1.$2"; fi
}
spent_fail() {
  { read -r x1; read -r x2; read -r x3; } < "$S/$N.$1"
  if [ "$x3" = once ]; then : > "$S/$N.$1"; fi
}
spent_code() {
  { read -r x1; read -r x2; } < "$S/$N.$1"
  if [ "$x2" = once ]; then : > "$S/$N.$1"; fi
}
die() {
  { read -r how; read -r text; } < "$S/$N.$1"
  spent_fail "$1"
  case "$how" in
    exit) printf '%s\n' "$text" >&2; exit 1;;
    signal) kill -9 $$;;
    hang) while :; do :; done;;
  esac
}
before() {
  pick code "$1" "$2"
  if [ -n "$fkey" ]; then
    read -r c < "$S/$N.$fkey"
    spent_code "$fkey"
    exit "$c"
  fi
  pick fail "$1" "$2"
  if [ -n "$fkey" ] && [ ! -f "$S/$N.$fkey.after" ]; then die "$fkey"; fi
}
after() {
  pick fail "$1" "$2"
  if [ -n "$fkey" ] && [ -f "$S/$N.$fkey.after" ]; then die "$fkey"; fi
}
holds() { [ -f "$S/$N.loaded" ] && while IFS= read -r l || [ -n "$l" ]; do [ "$l" = "$1" ] && return 0; done < "$S/$N.loaded"; return 1; }
label_of() {
  want=0
  while IFS= read -r l || [ -n "$l" ]; do
    case "$l" in
      *'<key>Label</key>'*) want=1;;
      *'<string>'*) if [ "$want" -eq 1 ]; then l=${l#*'<string>'}; printf '%s' "${l%%'</string>'*}"; return 0; fi;;
    esac
  done < "$1"
  return 1
}
without() {
  : > "$S/$N.loaded.new"
  if [ -f "$S/$N.loaded" ]; then
    while IFS= read -r l || [ -n "$l" ]; do [ "$l" = "$1" ] || printf '%s\n' "$l" >> "$S/$N.loaded.new"; done < "$S/$N.loaded"
  fi
  while IFS= read -r l || [ -n "$l" ]; do printf '%s\n' "$l"; done < "$S/$N.loaded.new" > "$S/$N.loaded"
}

case "$1" in
  print)
    if [ $# -ne 2 ]; then printf 'launchctl: unknown arguments: %s\n' "$*" >&2; exit 64; fi
    before print "${2##*/}"
    if holds "$2"; then after print "${2##*/}"; exit 0; fi
    domain=${2%/*}
    printf 'Could not find service "%s" in domain for user gui: %s\n' "${2##*/}" "${domain##*/}" >&2
    exit 113;;
  bootout)
    if [ "$2" != --wait ] || [ $# -ne 3 ]; then printf 'launchctl: unknown arguments: %s\n' "$*" >&2; exit 64; fi
    plist=''
    if [ -f "$S/$N.plist.${3##*/}" ]; then read -r plist < "$S/$N.plist.${3##*/}"; fi
    if [ -n "$plist" ] && { [ -e "$plist" ] || [ -L "$plist" ]; }; then there=there; else there=gone; fi
    printf '%s\t%s\n' "$*" "$there" >> "$S/$N.seen"
    before bootout "${3##*/}"
    if armed leave; then read -r gone < "$S/$N.leave"; without "$gone"; : > "$S/$N.leave"; fi
    if holds "$3"; then without "$3"; after bootout "${3##*/}"; exit 0; fi
    printf 'Boot-out failed: 3: No such process\n' >&2
    exit 3;;
  bootstrap)
    if [ $# -ne 3 ]; then printf 'launchctl: unknown arguments: %s\n' "$*" >&2; exit 64; fi
    label=$(label_of "$3") || { printf 'Bootstrap failed: 5: Input/output error\n' >&2; exit 5; }
    before bootstrap "$label"
    if holds "$2/$label"; then printf 'Bootstrap failed: 5: Input/output error\n' >&2; exit 5; fi
    printf '%s\n' "$2/$label" >> "$S/$N.loaded"
    printf '%s\n' "$3" > "$S/$N.plist.$label"
    after bootstrap "$label"
    exit 0;;
  *)
    printf 'launchctl: unknown arguments: %s\n' "$*" >&2
    exit 64;;
esac
