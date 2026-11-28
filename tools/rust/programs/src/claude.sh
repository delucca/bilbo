#!/bin/sh
S='@STATE@'
N=claude
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
    exit)
      if [ "$2" = after ]; then
        printf '{"command":"%s","outcome":"failed","message":"%s"}\n' "$3" "$(js "$text")"
      else
        printf '%s\n' "$text" >&2
      fi
      exit 1;;
    signal) kill -9 $$;;
    hang) while :; do :; done;;
  esac
}
before() { if has "fail.$1" && ! has "fail.$1.after"; then die "$1" before; fi; }
listing() { if has "fail.$1"; then die "$1" before; fi; }
after() { if has "fail.$1.after"; then die "$1" after "$2"; fi; }
printed() { if has "print.$1"; then lines "$S/$N.print.$1"; return 0; fi; return 1; }
first=1
emit() { if [ "$first" -eq 1 ]; then first=0; else printf ',\n'; fi; printf '  %s' "$1"; }
refused() { printf '{"command":"%s","outcome":"failed","message":"%s"}\n' "$1" "$(js "$2")"; exit 1; }

tab=$(printf '\t')
skipws() { while :; do case $r in ' '*|"$tab"*) r=${r#?};; *) break;; esac; done; }
idof() {
  r=$1
  while :; do
    case $r in *'"id"'*) r=${r#*'"id"'};; *) return 1;; esac
    skipws
    case $r in :*) r=${r#:};; *) continue;; esac
    skipws
    case $r in '"'*) r=${r#?};; *) return 1;; esac
    id=''
    while [ -n "$r" ]; do
      c=${r%"${r#?}"}; r=${r#?}
      case $c in
        '\') c=${r%"${r#?}"}; r=${r#?}; id="$id$c";;
        '"') return 0;;
        *) id="$id$c";;
      esac
    done
    return 1
  done
}

if [ "$1" = plugin ] && [ "$2 $3 $4" = "marketplace list --json" ] && [ $# -eq 4 ]; then
  listing mlist
  if ! printed market-list; then
    val market
    if [ -n "$v" ]; then
      p=$(js "$v")
      printf '[\n  {"name":"bilbo","source":"directory","path":"%s","installLocation":"%s"}\n]\n' "$p" "$p"
    else
      printf '[]\n'
    fi
  fi
elif [ "$1 $2 $3" = "plugin list --json" ] && [ $# -eq 3 ]; then
  listing plist
  if ! printed plugin-list; then
    val plugin; st=$v
    val market; market=$v
    printf '[\n'
    emit '{"id":"cc-plugin-privacy@builtin","version":"unknown","scope":"user","enabled":true}'
    if [ "$st" = on ] || [ "$st" = off ]; then
      on=true; if [ "$st" = off ]; then on=false; fi
      val from; from=$(js "$v")
      if has nofolder; then
        emit "{\"id\":\"bilbo@bilbo\",\"version\":\"unknown\",\"scope\":\"user\",\"enabled\":$on}"
      else
        emit "{\"id\":\"bilbo@bilbo\",\"version\":\"unknown\",\"scope\":\"user\",\"enabled\":$on,\"readFromFolder\":\"$from\"}"
      fi
    fi
    if has also; then
      val also
      if [ -n "$v" ]; then emit "{\"id\":\"$(js "$v")\",\"version\":\"unknown\",\"scope\":\"user\",\"enabled\":true}"; fi
    fi
    if has items; then while IFS= read -r line || [ -n "$line" ]; do emit "$line"; done < "$S/$N.items"; fi
    printf '\n]\n'
  fi
elif [ "$1 $2 $3" = "plugin marketplace add" ] && [ "$5" = "--json" ] && [ $# -eq 5 ]; then
  before madd
  put market "$4"
  after madd marketplace-add
  printf '{"command":"marketplace-add","outcome":"ok","marketplace":"bilbo","message":"Successfully added marketplace: bilbo (declared in user settings)"}\n'
elif [ "$1 $2 $3 $4 $5" = "plugin marketplace remove bilbo --json" ] && [ $# -eq 5 ]; then
  before mremove
  put market ''
  put plugin none
  if has also; then
    val also
    case "$v" in *@bilbo) put also '';; esac
  fi
  if has items; then
    : > "$S/$N.items.kept"
    while IFS= read -r line || [ -n "$line" ]; do
      if idof "$line"; then
        case "$id" in
          *@bilbo) ;;
          *) printf '%s\n' "$line" >> "$S/$N.items.kept";;
        esac
      else
        printf '%s\n' "$line" >> "$S/$N.items.kept"
      fi
    done < "$S/$N.items"
    lines "$S/$N.items.kept" > "$S/$N.items"
  fi
  after mremove marketplace-remove
  printf '{"command":"marketplace-remove","outcome":"ok","marketplace":"bilbo","message":"Successfully removed marketplace: bilbo"}\n'
elif [ "$1 $2 $3 $4" = "plugin install bilbo@bilbo --json" ] && [ $# -eq 4 ]; then
  before install
  val market
  if [ -z "$v" ]; then refused install 'Plugin bilbo@bilbo not found: no marketplace bilbo'; fi
  put plugin on
  put from "$v/plugins/bilbo"
  if has bring; then val bring; put also "$v"; fi
  after install install
  printf '{"command":"install","outcome":"ok","plugin":"bilbo@bilbo","pluginId":"bilbo@bilbo","scope":"user","message":"Successfully installed plugin: bilbo@bilbo (scope: user)"}\n'
elif [ "$1 $2 $3 $4" = "plugin uninstall bilbo@bilbo --json" ] && [ $# -eq 4 ]; then
  before uninstall
  put plugin none
  after uninstall uninstall
  printf '{"command":"uninstall","outcome":"ok","plugin":"bilbo@bilbo","pluginId":"bilbo@bilbo","scope":"user","message":"Successfully uninstalled plugin: bilbo@bilbo"}\n'
else
  printf 'claude: unknown arguments: %s\n' "$*" >&2
  exit 64
fi
