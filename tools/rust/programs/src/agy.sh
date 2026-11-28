#!/bin/sh
S='@STATE@'
N=agy
printf '%s\n' "$*" >> "$S/$N.log"
has() { [ -f "$S/$N.$1" ]; }
val() { v=''; if [ -f "$S/$N.$1" ]; then read -r v < "$S/$N.$1"; fi; }
put() { printf '%s\n' "$2" > "$S/$N.$1"; }
lines() { while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "$1"; }
js() {
  rest=$1; out=''
  while [ -n "$rest" ]; do
    c=${rest%"${rest#?}"}; rest=${rest#?}
    case $c in
      '\') out="$out\\\\";;
      '"') out="$out\\\"";;
      *[[:cntrl:]]*)
        n=$(printf '%d' "'$c")
        case $n in
          8) out="$out\\b";;
          9) out="$out\\t";;
          10) out="$out\\n";;
          12) out="$out\\f";;
          13) out="$out\\r";;
          *)
            if [ "$n" -ge 0 ] && [ "$n" -lt 32 ]; then
              out="$out$(printf '\\u%04x' "$n")"
            else
              out="$out$c"
            fi;;
        esac;;
      *) out="$out$c";;
    esac
  done
  printf '%s' "$out"
}
die() {
  { read -r how; read -r text; } < "$S/$N.fail.$1"
  case "$how" in
    exit) printf '%s\n' "$text" >&2; exit 1;;
    signal) kill -9 $$;;
    hang) while :; do :; done;;
  esac
}
before() { if has "fail.$1" && ! has "fail.$1.after"; then die "$1"; fi; }
after() { if has "fail.$1.after"; then die "$1"; fi; }
listing() { if has "fail.$1"; then die "$1"; fi; }

if [ "$1 $2 $3" = "plugin list --json" ] && [ $# -eq 3 ]; then
  listing plist
  if has print.list; then
    lines "$S/$N.print.list"
  else
    val copy
    if [ -n "$v" ]; then
      path=$v
      val copyat
      if [ -n "$v" ]; then path=$v; fi
      on=true; val off; if [ "$v" = on ]; then on=false; fi
      printf '[\n  {"name":"bilbo","enabled":%s,"source":"antigravity","path":"%s","skills":["note","recall"],"hooks":["bilbo-digest"]}\n]\n' "$on" "$(js "$path")"
    else
      printf '[]\n'
    fi
  fi
elif [ "$1 $2" = "plugin install" ] && [ $# -eq 3 ]; then
  before install
  put copy "$3"
  put copyat ''
  put off ''
  after install
  printf '\033[32m[ok]\033[0m    bilbo\n      \033[32m✔\033[0m skills      : 2 processed\n      \033[32m✔\033[0m hooks       : 1 processed\n'
elif [ "$1 $2 $3" = "plugin uninstall bilbo" ] && [ $# -eq 3 ]; then
  before uninstall
  put copy ''
  put copyat ''
  after uninstall
  printf 'Uninstalled plugin "bilbo"\n'
else
  printf 'agy: unknown arguments: %s\n' "$*" >&2
  exit 64
fi
