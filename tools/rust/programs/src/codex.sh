#!/bin/sh
S='@STATE@'
N=codex
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
printed() { if has "print.$1"; then lines "$S/$N.print.$1"; return 0; fi; return 1; }
first=1
emit() { if [ "$first" -eq 1 ]; then first=0; else printf ',\n'; fi; printf '    %s' "$1"; }
refused() { printf '%s\n' "$1" >&2; exit 1; }

tab=$(printf '\t')
skipws() { while :; do case $r in ' '*|"$tab"*) r=${r#?};; *) break;; esac; done; }
readstr() {
  str=''
  while [ -n "$r" ]; do
    c=${r%"${r#?}"}; r=${r#?}
    case $c in
      '"') return 0;;
      '\')
        n=${r%"${r#?}"}; r=${r#?}
        case "$n" in
          '\') str="$str\\";;
          '"') str="$str\"";;
          *) bad=1;;
        esac;;
      *) str="$str$c";;
    esac
  done
  return 1
}
takeout() {
  : > "$S/$N.trust.drop"
  bad=''
  r=${line#*'"edits"'}
  skipws
  case "$r" in ':'*) r=${r#:};; *) bad=1; return;; esac
  skipws
  case "$r" in '['*) r=${r#'['};; *) bad=1; return;; esac
  skipws
  while :; do
    case "$r" in '{'*) r=${r#'{'};; *) bad=1; return;; esac
    seen=''; path=''; merge=''
    while :; do
      skipws
      case "$r" in '"'*) r=${r#'"'};; *) bad=1; return;; esac
      readstr || { bad=1; return; }
      [ -z "$bad" ] || return
      name=$str
      skipws
      case "$r" in ':'*) r=${r#:};; *) bad=1; return;; esac
      skipws
      case "$name" in
        keyPath)
          case "$seen" in *k*) bad=1; return;; esac
          seen="${seen}k"
          case "$r" in '"'*) r=${r#'"'};; *) bad=1; return;; esac
          readstr || { bad=1; return; }
          [ -z "$bad" ] || return
          path=$str;;
        value)
          case "$seen" in *v*) bad=1; return;; esac
          seen="${seen}v"
          case "$r" in 'null'*) r=${r#null};; *) bad=1; return;; esac;;
        mergeStrategy)
          case "$seen" in *m*) bad=1; return;; esac
          seen="${seen}m"
          case "$r" in '"'*) r=${r#'"'};; *) bad=1; return;; esac
          readstr || { bad=1; return; }
          [ -z "$bad" ] || return
          merge=$str;;
        *) bad=1; return;;
      esac
      skipws
      case "$r" in
        ','*) r=${r#,};;
        '}'*) r=${r#'}'}; break;;
        *) bad=1; return;;
      esac
    done
    [ "${#seen}" -eq 3 ] || { bad=1; return; }
    [ "$merge" = replace ] || { bad=1; return; }
    case "$path" in
      'hooks.state."'*'"') ;;
      *) bad=1; return;;
    esac
    inner=${path#'hooks.state."'}; inner=${inner%'"'}
    key=''; rest=$inner
    while [ -n "$rest" ]; do
      c=${rest%"${rest#?}"}; rest=${rest#?}
      case "$c" in
        '"') bad=1; return;;
        '\')
          n=${rest%"${rest#?}"}; rest=${rest#?}
          case "$n" in
            '\') key="$key\\";;
            '"') key="$key\"";;
            *) bad=1; return;;
          esac;;
        *) key="$key$c";;
      esac
    done
    printf '%s\n' "$key" >> "$S/$N.trust.drop"
    skipws
    case "$r" in
      ','*) r=${r#,};;
      ']'*) break;;
      *) bad=1; return;;
    esac
  done
}
hook() {
  key=$1; event=$2; matcher=$3; command=$4; hash=$5; order=$6
  status=untrusted
  if [ -f "$S/$N.trust" ]; then
    while IFS="$tab" read -r tkey thash || [ -n "$tkey" ]; do
      if [ "$tkey" = "$key" ]; then
        if [ "$thash" = "$hash" ]; then status=trusted; else status=modified; fi
      fi
    done < "$S/$N.trust"
  fi
  printf '%s' "{\"key\":\"$key\",\"eventName\":\"$event\",\"handlerType\":\"command\",\"command\":\"$command\",\"async\":false,\"matcher\":$matcher,\"timeoutSec\":@TIMEOUT@,\"statusMessage\":null,\"additionalContextLimit\":null,\"sourcePath\":\"/stand-in/hooks.json\",\"source\":\"plugin\",\"pluginId\":\"bilbo@bilbo\",\"environmentId\":null,\"displayOrder\":$order,\"enabled\":true,\"isManaged\":false,\"currentHash\":\"$hash\",\"trustStatus\":\"$status\"}"
}

server() {
  while IFS= read -r line; do
    m=${line#*'"method":"'}; m=${m%%'"'*}
    printf 'rpc %s\n' "$m" >> "$S/$N.log"
    if has silent; then continue; fi
    id=''
    case "$line" in '{"id":'*) id=${line#'{"id":'}; id=${id%%,*};; esac
    [ -n "$id" ] || continue
    case "$m" in
      initialize)
        printf '{"id":%s,"result":{"userAgent":"stand-in","codexHome":"/stand-in","platformFamily":"unix","platformOs":"unix"}}\n' "$id";;
      hooks/list)
        list=''
        if has print.hooks; then
          while IFS= read -r l || [ -n "$l" ]; do list="$list${list:+,}$l"; done < "$S/$N.print.hooks"
        else
          val installed
          if [ "$v" = on ] || [ "$v" = off ]; then
            list="$(hook 'bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0' userPromptSubmit null '@PROMPT@' 'sha256:@HASH1@' 0),$(hook 'bilbo@bilbo:hooks/hooks.json:session_start:0:0' sessionStart '"compact"' '@COMPACT@' 'sha256:@HASH2@' 1)"
          fi
          if has items.hooks; then
            while IFS= read -r l || [ -n "$l" ]; do list="$list${list:+,}$l"; done < "$S/$N.items.hooks"
          fi
        fi
        printf '{"id":%s,"result":{"data":[{"cwd":"/stand-in","hooks":[%s]}]}}\n' "$id" "$list";;
      config/read)
        state=''
        if [ -f "$S/$N.trust" ]; then
          while IFS="$tab" read -r tkey thash || [ -n "$tkey" ]; do
            state="$state${state:+,}\"$(js "$tkey")\":{\"trusted_hash\":\"$(js "$thash")\"}"
          done < "$S/$N.trust"
        fi
        printf '{"id":%s,"result":{"config":{"hooks":{"state":{%s}}}}}\n' "$id" "$state";;
      config/batchWrite)
        case "$line" in
          *'"keyPath":"hooks.state"'*'"mergeStrategy":"upsert"'*) ;;
          *)
            takeout
            if [ -n "$bad" ]; then
              printf '{"id":%s,"error":{"code":-32602,"message":"this stand-in takes null replacements of hooks.state keys only"}}\n' "$id"
              continue
            fi
            : > "$S/$N.trust.next"
            if [ -f "$S/$N.trust" ]; then
              while IFS="$tab" read -r tkey thash || [ -n "$tkey" ]; do
                gone=''
                while IFS= read -r dkey || [ -n "$dkey" ]; do
                  if [ "$dkey" = "$tkey" ]; then gone=1; fi
                done < "$S/$N.trust.drop"
                if [ -z "$gone" ]; then printf '%s\t%s\n' "$tkey" "$thash" >> "$S/$N.trust.next"; fi
              done < "$S/$N.trust"
            fi
            lines "$S/$N.trust.next" > "$S/$N.trust"
            printf '{"id":%s,"result":{"status":"ok"}}\n' "$id"
            continue;;
        esac
        r=${line#*'"value"'}
        skipws; r=${r#:}; skipws; r=${r#'{'}; skipws
        : > "$S/$N.trust.pending"
        bad=''
        while :; do
          case "$r" in
            '"'*) r=${r#?};;
            *) break;;
          esac
          readstr || break
          key=$str
          skipws; r=${r#:}; skipws; r=${r#'{'}; skipws
          case "$r" in
            '"trusted_hash"'*) r=${r#'"trusted_hash"'};;
            *) break;;
          esac
          skipws; r=${r#:}; skipws
          case "$r" in
            '"'*) r=${r#?};;
            *) break;;
          esac
          readstr || break
          hash=$str
          skipws; r=${r#'}'}; skipws; r=${r#,}; skipws
          printf '%s\t%s\n' "$key" "$hash" >> "$S/$N.trust.pending"
        done
        if [ -n "$bad" ]; then
          printf '{"id":%s,"error":{"code":-32602,"message":"this stand-in reads the escapes backslash and quote only"}}\n' "$id"
          continue
        fi
        : > "$S/$N.trust.next"
        if [ -f "$S/$N.trust" ]; then
          while IFS="$tab" read -r tkey thash || [ -n "$tkey" ]; do
            held=''
            while IFS="$tab" read -r pkey phash || [ -n "$pkey" ]; do
              if [ "$pkey" = "$tkey" ]; then held=1; fi
            done < "$S/$N.trust.pending"
            if [ -z "$held" ]; then printf '%s\t%s\n' "$tkey" "$thash" >> "$S/$N.trust.next"; fi
          done < "$S/$N.trust"
        fi
        lines "$S/$N.trust.pending" >> "$S/$N.trust.next"
        lines "$S/$N.trust.next" > "$S/$N.trust"
        printf '{"id":%s,"result":{"status":"ok"}}\n' "$id";;
      *)
        printf '{"id":%s,"error":{"code":-32601,"message":"method not found"}}\n' "$id";;
    esac
  done
}

if [ "$1 $2 $3 $4" = "plugin marketplace list --json" ] && [ $# -eq 4 ]; then
  listing mlist
  if ! printed market-list; then
    val market
    if [ -n "$v" ]; then
      p=$(js "$v")
      printf '{\n  "marketplaces": [\n    {"name":"bilbo","root":"%s","marketplaceSource":{"sourceType":"local","source":"%s"}}\n  ]\n}\n' "$p" "$p"
    else
      printf '{"marketplaces":[]}\n'
    fi
  fi
elif [ "$1 $2 $3" = "plugin list --json" ] && [ $# -eq 3 ]; then
  listing plist
  if ! printed plugin-list; then
    val installed; st=$v
    val market; market=$v
    printf '{"installed":[\n'
    if [ "$st" = on ] || [ "$st" = off ]; then
      on=true; if [ "$st" = off ]; then on=false; fi
      val from; from=$(js "$v")
      val version; version=$v; if [ -z "$version" ]; then version=0.0.0; fi
      version=$(js "$version"); market=$(js "$market")
      emit "{\"pluginId\":\"bilbo@bilbo\",\"name\":\"bilbo\",\"marketplaceName\":\"bilbo\",\"version\":\"$version\",\"installed\":true,\"enabled\":$on,\"source\":{\"source\":\"local\",\"path\":\"$from\"},\"marketplaceSource\":{\"sourceType\":\"local\",\"source\":\"$market\"},\"installPolicy\":\"AVAILABLE\",\"authPolicy\":\"ON_INSTALL\"}"
    fi
    if has also; then val also; emit "{\"pluginId\":\"$(js "$v")\",\"name\":\"other\",\"marketplaceName\":\"bilbo\",\"version\":\"1\",\"installed\":true,\"enabled\":true,\"source\":{\"source\":\"local\",\"path\":\"/stand-in\"}}"; fi
    if has items; then while IFS= read -r line || [ -n "$line" ]; do emit "$line"; done < "$S/$N.items"; fi
    printf '\n],"available":[]}\n'
  fi
elif [ "$1 $2 $3" = "plugin marketplace add" ] && [ "$5" = "--json" ] && [ $# -eq 5 ]; then
  before madd
  put market "$4"
  after madd
  printf '{"marketplaceName":"bilbo","installedRoot":"%s","alreadyAdded":false}\n' "$(js "$4")"
elif [ "$1 $2 $3 $4 $5" = "plugin marketplace remove bilbo --json" ] && [ $# -eq 5 ]; then
  before mremove
  put market ''
  put installed none
  after mremove
  printf '{"marketplaceName":"bilbo","installedRoot":null}\n'
elif [ "$1 $2 $3" = "plugin add bilbo@bilbo" ] && [ "$4" = "--json" ] && [ $# -eq 4 ]; then
  before add
  val market
  if [ -z "$v" ]; then refused 'plugin bilbo@bilbo not found: no marketplace bilbo'; fi
  put installed on
  put from "$v/plugins/bilbo"
  if has bring; then val bring; put also "$v"; fi
  val version; if [ -z "$v" ]; then v=0.0.0; fi
  after add
  printf '{"pluginId":"bilbo@bilbo","name":"bilbo","marketplaceName":"bilbo","version":"%s","installedPath":"/stand-in","authPolicy":"ON_INSTALL"}\n' "$(js "$v")"
elif [ "$1 $2 $3" = "plugin remove bilbo@bilbo" ] && [ "$4" = "--json" ] && [ $# -eq 4 ]; then
  before remove
  put installed none
  after remove
  printf '{"pluginId":"bilbo@bilbo","name":"bilbo","marketplaceName":"bilbo"}\n'
elif [ "$*" = "app-server" ]; then
  server
else
  printf 'codex: unknown arguments: %s\n' "$*" >&2
  exit 64
fi
