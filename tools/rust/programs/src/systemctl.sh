#!/bin/sh
S='@STATE@'
N=systemctl
MKDIR='@MKDIR@'
LN='@LN@'
RM='@RM@'
printf '%s\n' "$*" >> "$S/$N.log"
has() { [ -f "$S/$N.$1" ]; }
armed() { [ -s "$S/$N.$1" ]; }
pick() {
  fkey=''
  if [ -n "$2" ] && armed "fail.$1.for.$2"; then fkey="fail.$1.for.$2"
  elif armed "fail.$1"; then fkey="fail.$1"; fi
}
die() {
  { read -r how; read -r text; read -r once; } < "$S/$N.$1"
  if [ "$once" = once ]; then : > "$S/$N.$1"; fi
  case "$how" in
    exit) printf '%s\n' "$text" >&2; exit 1;;
    signal) kill -9 $$;;
    hang) while :; do :; done;;
  esac
}
before() {
  pick "$1" "$2"
  if [ -n "$fkey" ] && [ ! -f "$S/$N.$fkey.after" ]; then die "$fkey"; fi
}
after() {
  pick "$1" "$2"
  if [ -n "$fkey" ] && [ -f "$S/$N.$fkey.after" ]; then die "$fkey"; fi
}
units=''
if has units; then read -r units < "$S/$N.units"; fi
seen_files() {
  base=${1%.*}; there=''
  for f in "$base.service" "$base.timer"; do
    if [ -e "$units/$f" ] || [ -L "$units/$f" ]; then there="$there${there:+ }$f"; fi
  done
  printf '%s\t%s\n' "$2" "$there" >> "$S/$N.seen"
}
quiet() { has "silent.$1"; }
line() {
  if has "value.$1"; then read -r v < "$S/$N.value.$1"; printf '%s\n' "$v"; else printf '%s\n' "$2"; fi
}

if [ "$1" != --user ]; then printf 'systemctl: unknown arguments: %s\n' "$*" >&2; exit 64; fi
cmd=$2
unit=$3
case "$cmd" in
  is-system-running)
    [ $# -eq 2 ] || { printf 'systemctl: unknown arguments: %s\n' "$*" >&2; exit 64; }
    before is-system-running ''
    quiet is-system-running || line is-system-running running
    exit 1;;
  is-active)
    [ $# -eq 3 ] || { printf 'systemctl: unknown arguments: %s\n' "$*" >&2; exit 64; }
    before is-active "$unit"
    if quiet is-active; then exit 3; fi
    if has skip.is-active; then
      read -r n < "$S/$N.skip.is-active"
      if [ "$n" -le 0 ]; then exit 3; fi
      printf '%s\n' "$((n - 1))" > "$S/$N.skip.is-active"
    fi
    if has "value.is-active.$unit"; then read -r v < "$S/$N.value.is-active.$unit"; printf '%s\n' "$v"; exit 0; fi
    if has "active.$unit"; then read -r v < "$S/$N.active.$unit"; printf '%s\n' "$v"; else printf 'inactive\n'; fi
    exit 0;;
  is-enabled)
    [ $# -eq 3 ] || { printf 'systemctl: unknown arguments: %s\n' "$*" >&2; exit 64; }
    before is-enabled "$unit"
    if quiet is-enabled; then exit 1; fi
    if [ ! -e "$units/$unit" ]; then exit 1; fi
    if [ -L "$units/timers.target.wants/$unit" ]; then printf 'enabled\n'; else printf 'disabled\n'; exit 1; fi
    exit 0;;
  show)
    [ "$3 $4" = "--property=LoadState --value" ] && [ $# -eq 5 ] || { printf 'systemctl: unknown arguments: %s\n' "$*" >&2; exit 64; }
    unit=$5
    before show "$unit"
    if quiet show; then exit 0; fi
    if [ -e "$units/$unit" ] || has "held.$unit"; then printf 'loaded\n'; else printf 'not-found\n'; fi
    exit 0;;
  daemon-reload)
    [ $# -eq 2 ] || { printf 'systemctl: unknown arguments: %s\n' "$*" >&2; exit 64; }
    before daemon-reload ''
    for file in "$units"/*.service "$units"/*.timer; do
      [ -e "$file" ] || continue
      printf 'held\n' > "$S/$N.held.${file##*/}"
    done
    for held in "$S/$N".held.*; do
      [ -e "$held" ] || continue
      name=${held#"$S/$N.held."}
      if [ ! -e "$units/$name" ] && ! has "keep.$name"; then "$RM" -f "$held" || exit 1; fi
    done
    after daemon-reload ''
    exit 0;;
  enable)
    [ $# -eq 3 ] || { printf 'systemctl: unknown arguments: %s\n' "$*" >&2; exit 64; }
    before enable "$unit"
    if [ ! -e "$units/$unit" ]; then printf 'Failed to enable unit: Unit file %s does not exist.\n' "$unit" >&2; exit 1; fi
    "$MKDIR" -p "$units/timers.target.wants" || exit 1
    "$LN" -sf "$units/$unit" "$units/timers.target.wants/$unit" || exit 1
    printf 'held\n' > "$S/$N.held.$unit"
    after enable "$unit"
    exit 0;;
  disable)
    [ $# -eq 3 ] || { printf 'systemctl: unknown arguments: %s\n' "$*" >&2; exit 64; }
    seen_files "$unit" "$*"
    before disable "$unit"
    "$RM" -f "$units/timers.target.wants/$unit" || exit 1
    after disable "$unit"
    exit 0;;
  restart)
    [ $# -eq 3 ] || { printf 'systemctl: unknown arguments: %s\n' "$*" >&2; exit 64; }
    before restart "$unit"
    if [ ! -e "$units/$unit" ]; then printf 'Failed to restart %s: Unit not found.\n' "$unit" >&2; exit 5; fi
    printf 'active\n' > "$S/$N.active.$unit"
    printf 'held\n' > "$S/$N.held.$unit"
    after restart "$unit"
    exit 0;;
  stop)
    [ $# -eq 3 ] || { printf 'systemctl: unknown arguments: %s\n' "$*" >&2; exit 64; }
    seen_files "$unit" "$*"
    before stop "$unit"
    if ! has "stuck.$unit"; then printf 'inactive\n' > "$S/$N.active.$unit"; fi
    service=${unit%.timer}.service
    case "$unit" in *.timer) if has "starts.$service"; then printf 'activating\n' > "$S/$N.active.$service"; fi;; esac
    after stop "$unit"
    exit 0;;
  *)
    printf 'systemctl: unknown arguments: %s\n' "$*" >&2
    exit 64;;
esac
