use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const CLAUDE: &str = r##"#!/bin/sh
D='@STATE@'
printf '%s\n' "$*" >>"$D/claude.log"
m=
[ -s "$D/claude.marketplace" ] && read -r m <"$D/claude.marketplace"
p=
[ -s "$D/claude.plugin" ] && read -r p <"$D/claude.plugin"
fail() {
  printf '{"command":"%s","outcome":"failed","message":"%s","failureCode":"%s"}\n' "$1" "$2" "$3"
  printf '\342\234\230 %s\n' "$2" >&2
  exit 1
}
injected() {
  if [ -s "$D/claude.fail-$1" ]; then
    read -r line <"$D/claude.fail-$1"
    printf '%s\n' "$line"
    printf '\342\234\230 injected failure\n' >&2
    exit 1
  fi
}
case "$*" in
"plugin marketplace list --json")
  case "$m" in
  "") printf '[]\n' ;;
  github\ *)
    set -- $m
    printf '[{"name":"bilbo","source":"github","repo":"%s","ref":"%s","installLocation":"%s/claude-bilbo"}]\n' "$2" "$3" "$D"
    ;;
  *) printf '[{"name":"bilbo","source":"directory","path":"%s","installLocation":"%s"}]\n' "${m#directory }" "${m#directory }" ;;
  esac
  ;;
"plugin list --json")
  case "$p" in
  "") printf '[]\n' ;;
  *) printf '[{"id":"bilbo@bilbo","version":"92076707d856","scope":"user","enabled":%s,"installPath":"%s/claude-cache","projectEnabled":false}]\n' "$p" "$D" ;;
  esac
  ;;
"plugin marketplace add "*" --json")
  injected marketplace-add
  src=$4
  case "$src" in
  *"#"*) new="github ${src%%#*} ${src#*#}" ;;
  *)
    [ -f "$src/.claude-plugin/marketplace.json" ] || fail marketplace-add "Marketplace file not found at $src/.claude-plugin/marketplace.json" manifest_missing
    new="directory $src"
    ;;
  esac
  if [ -n "$m" ] && [ "$m" != "$new" ]; then
    fail marketplace-add "Cannot add marketplace \\\"bilbo\\\": its source doesn't match its extraKnownMarketplaces entry in user or managed settings; add it from the source that entry lists, or change the entry." declared
  fi
  printf '%s\n' "$new" >"$D/claude.marketplace"
  printf '{"command":"marketplace-add","outcome":"ok","marketplace":"bilbo","message":"Successfully added marketplace: bilbo (declared in user settings)"}\n'
  ;;
"plugin install bilbo@bilbo --json")
  injected install
  [ -n "$m" ] || fail install "Plugin \\\"bilbo\\\" not found in marketplace \\\"bilbo\\\"" not_found
  printf 'true\n' >"$D/claude.plugin"
  printf '{"command":"install","outcome":"ok","plugin":"bilbo@bilbo","pluginId":"bilbo@bilbo","scope":"user","message":"Successfully installed plugin: bilbo@bilbo (scope: user)"}\n'
  ;;
"plugin uninstall bilbo@bilbo --json")
  [ -n "$p" ] || fail uninstall "Plugin \\\"bilbo@bilbo\\\" not found in installed plugins" not_installed
  : >"$D/claude.plugin"
  printf '{"command":"uninstall","outcome":"ok","plugin":"bilbo@bilbo","pluginId":"bilbo@bilbo","scope":"user","keptData":false,"message":"Successfully uninstalled plugin: bilbo (scope: user)"}\n'
  ;;
"plugin marketplace remove bilbo --json")
  [ -n "$m" ] || fail marketplace-remove "Marketplace 'bilbo' not found" not_configured
  : >"$D/claude.marketplace"
  : >"$D/claude.plugin"
  printf '{"command":"marketplace-remove","outcome":"ok","marketplace":"bilbo","message":"Successfully removed marketplace: bilbo"}\n'
  ;;
*)
  printf 'fake claude: unexpected arguments: %s\n' "$*" >&2
  exit 64
  ;;
esac
"##;

const CODEX: &str = r##"#!/bin/sh
D='@STATE@'
printf '%s\n' "$*" >>"$D/codex.log"
m=
[ -s "$D/codex.marketplace" ] && read -r m <"$D/codex.marketplace"
p=
[ -s "$D/codex.plugin" ] && read -r p <"$D/codex.plugin"
fail() {
  printf 'Error: %s\n' "$1" >&2
  exit 1
}
injected() {
  if [ -s "$D/codex.fail-$1" ]; then
    read -r line <"$D/codex.fail-$1"
    printf '%s\n' "$line" >&2
    exit 1
  fi
}
source_json() {
  case "$m" in
  git\ *)
    set -- $m
    printf '{"sourceType":"git","source":"%s"}' "$2"
    ;;
  *) printf '{"sourceType":"local","source":"%s"}' "${m#local }" ;;
  esac
}
if [ "$1" = app-server ]; then
  injected app-server
  hash=sha256:fake
  [ -s "$D/codex.hook-hash" ] && read -r hash <"$D/codex.hook-hash"
  key='bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0'
  n=0
  while IFS= read -r line; do
    case "$line" in
    *'"id":'*) n=$((n+1)) ;;
    *) continue ;;
    esac
    trust=
    [ -s "$D/codex.trust" ] && read -r trust <"$D/codex.trust"
    case "$line" in
    *'"method":"initialize"'*)
      printf 'app-server initialize\n' >>"$D/codex.log"
      printf '{"id":%s,"result":{}}\n' "$n"
      ;;
    *'"method":"hooks/list"'*)
      printf 'app-server hooks/list\n' >>"$D/codex.log"
      if [ -z "$p" ] || [ -s "$D/codex.no-hook" ]; then
        printf '{"id":%s,"result":{"data":[{"cwd":"/","hooks":[],"warnings":[],"errors":[]}]}}\n' "$n"
      else
        status=untrusted
        if [ "$trust" = "$hash" ]; then
          status=trusted
        elif [ -n "$trust" ]; then
          status=modified
        fi
        printf '{"id":%s,"result":{"data":[{"cwd":"/","hooks":[{"key":"%s","pluginId":"bilbo@bilbo","source":"plugin","isManaged":false,"currentHash":"%s","trustStatus":"%s"}],"warnings":[],"errors":[]}]}}\n' "$n" "$key" "$hash" "$status"
      fi
      ;;
    *'"method":"config/batchWrite"'*)
      printf 'app-server config/batchWrite\n' >>"$D/codex.log"
      printf '%s\n' "$line" >>"$D/codex.batch-writes"
      if [ -s "$D/codex.readonly" ]; then
        printf '{"id":%s,"error":{"code":-32603,"message":"failed to persist config.toml: failed to persist config at %s/config.toml"}}\n' "$n" "$D"
        continue
      fi
      case "$line" in
      *'"value":null'*) printf '\n' >"$D/codex.trust" ;;
      *)
        t=${line#*trusted_hash\":\"}
        printf '%s\n' "${t%%\"*}" >"$D/codex.trust"
        ;;
      esac
      printf '{"id":%s,"result":{"status":"ok"}}\n' "$n"
      ;;
    *'"method":"config/read"'*)
      printf 'app-server config/read\n' >>"$D/codex.log"
      if [ -n "$trust" ]; then
        printf '{"id":%s,"result":{"config":{"hooks":{"state":{"%s":{"trusted_hash":"%s"},"other@plugin:hooks/hooks.json:stop:0:0":{"trusted_hash":"sha256:other"}}}}}}\n' "$n" "$key" "$trust"
      else
        printf '{"id":%s,"result":{"config":{"hooks":{"state":{"other@plugin:hooks/hooks.json:stop:0:0":{"trusted_hash":"sha256:other"}}}}}}\n' "$n"
      fi
      ;;
    *)
      printf '{"id":%s,"error":{"code":-32601,"message":"unknown method"}}\n' "$n"
      ;;
    esac
  done
  exit 0
fi
case "$1 $2 $3" in
"plugin marketplace list")
  if [ -z "$m" ]; then
    printf '{\n  "marketplaces": []\n}\n'
  else
    printf '{\n  "marketplaces": [\n    {\n      "name": "bilbo",\n      "root": "%s/codex-bilbo",\n      "marketplaceSource": %s\n    }\n  ]\n}\n' "$D" "$(source_json)"
  fi
  ;;
"plugin list --json")
  if [ -z "$p" ]; then
    printf '{\n  "installed": [],\n  "available": []\n}\n'
  else
    set -- $p
    printf '{\n  "installed": [\n    {\n      "pluginId": "bilbo@bilbo",\n      "name": "bilbo",\n      "marketplaceName": "bilbo",\n      "version": "%s",\n      "installed": true,\n      "enabled": %s,\n      "marketplaceSource": %s\n    }\n  ],\n  "available": []\n}\n' "$1" "$2" "$(source_json)"
  fi
  ;;
"plugin marketplace add")
  injected marketplace-add
  src=$4
  if [ "$5" = "--ref" ]; then
    new="git https://github.com/$src.git $6"
  else
    [ -f "$src/.agents/plugins/marketplace.json" ] || fail "marketplace file not found in $src"
    new="local $src"
  fi
  already=false
  if [ -n "$m" ]; then
    [ "$m" = "$new" ] || fail "marketplace 'bilbo' is already added from a different source; remove it before adding this source"
    already=true
  fi
  printf '%s\n' "$new" >"$D/codex.marketplace"
  printf '{\n  "marketplaceName": "bilbo",\n  "installedRoot": "%s/codex-bilbo",\n  "alreadyAdded": %s\n}\n' "$D" "$already"
  ;;
"plugin add bilbo@bilbo")
  injected add
  case "$m" in
  "") fail "plugin \`bilbo\` was not found in marketplace \`bilbo\`" ;;
  git\ *)
    set -- $m
    version=${3#v}
    ;;
  *)
    version=
    [ -f "$D/codex.local-version" ] && read -r version <"$D/codex.local-version"
    version=${version:-@VERSION@}
    ;;
  esac
  printf '%s true\n' "$version" >"$D/codex.plugin"
  printf '{\n  "pluginId": "bilbo@bilbo",\n  "name": "bilbo",\n  "marketplaceName": "bilbo",\n  "version": "%s",\n  "installedPath": "%s/codex-cache",\n  "authPolicy": "ON_INSTALL"\n}\n' "$version" "$D"
  ;;
"plugin remove bilbo@bilbo")
  : >"$D/codex.plugin"
  printf '{\n  "pluginId": "bilbo@bilbo",\n  "name": "bilbo",\n  "marketplaceName": "bilbo"\n}\n'
  ;;
"plugin marketplace remove")
  [ -n "$m" ] || fail "marketplace \`bilbo\` is not configured or installed"
  : >"$D/codex.marketplace"
  : >"$D/codex.plugin"
  printf '{\n  "marketplaceName": "bilbo",\n  "installedRoot": "%s/codex-bilbo"\n}\n' "$D"
  ;;
*)
  printf 'fake codex: unexpected arguments: %s\n' "$*" >&2
  exit 64
  ;;
esac
"##;

const LAUNCHCTL: &str = r##"#!/bin/sh
D='@STATE@'
printf '%s\n' "$*" >>"$D/launchctl.log"
case "$1" in
bootout)
  [ "$2" = --wait ] && set -- bootout "$3"
  label=${2##*/}
  ;;
bootstrap)
  label=${3##*/}
  label=${label%.plist}
  ;;
*) label= ;;
esac
case "$label" in
*.embedder)
  f="$D/launchctl.loaded-embedder"
  fail="$D/launchctl.fail-$1-embedder"
  ;;
*)
  f="$D/launchctl.loaded"
  fail="$D/launchctl.fail-$1"
  ;;
esac
loaded=
[ -s "$f" ] && read -r loaded <"$f"
if [ -s "$fail" ]; then
  read -r line <"$fail"
  printf '%s\n' "$line" >&2
  exit 5
fi
case "$1" in
bootout)
  if [ -z "$loaded" ]; then
    printf 'Boot-out failed: 3: No such process\n' >&2
    exit 3
  fi
  : >"$f"
  ;;
bootstrap)
  if [ -n "$loaded" ]; then
    printf 'Bootstrap failed: 5: Input/output error\nTry re-running the command as root for richer errors.\n' >&2
    exit 5
  fi
  printf '%s\n' "$3" >"$f"
  case "$label" in *.embedder) : >"$D/embedder.started" ;; esac
  ;;
*)
  printf 'fake launchctl: unexpected arguments: %s\n' "$*" >&2
  exit 64
  ;;
esac
"##;

const SYSTEMCTL: &str = r##"#!/bin/sh
D='@STATE@'
printf '%s\n' "$*" >>"$D/systemctl.log"
state=running
[ -s "$D/systemctl.state" ] && read -r state <"$D/systemctl.state"
for fail in "$D/systemctl.fail-$2-$3" "$D/systemctl.fail-$2"; do
  if [ -s "$fail" ]; then
    read -r line <"$fail"
    printf '%s\n' "$line" >&2
    exit 1
  fi
done
case "$*" in
"--user is-system-running")
  printf '%s\n' "$state"
  [ "$state" = running ]
  exit
  ;;
"--user daemon-reload" | "--user restart bilbo-index.timer") ;;
"--user enable bilbo-index.timer") printf 'enabled\n' >"$D/systemctl.enabled" ;;
"--user disable --now bilbo-index.timer") : >"$D/systemctl.enabled" ;;
"--user enable bilbo-embedder.service") printf 'enabled\n' >"$D/systemctl.embedder-enabled" ;;
"--user restart bilbo-embedder.service") : >"$D/embedder.started" ;;
"--user disable --now bilbo-embedder.service") : >"$D/systemctl.embedder-enabled" ;;
*)
  printf 'fake systemctl: unexpected arguments: %s\n' "$*" >&2
  exit 64
  ;;
esac
"##;

const LLAMA_SERVER: &str =
    "#!/bin/sh\nprintf 'fake llama-server: tests never run it\\n' >&2\nexit 64\n";

/// Writes executable fakes named `tools` into `bin`, keeping state and logs in `state`.
pub fn install(bin: &Path, state: &Path, tools: &[&str]) {
    std::fs::create_dir_all(bin).unwrap();
    std::fs::create_dir_all(state).unwrap();
    for tool in tools {
        let template = match *tool {
            "claude" => CLAUDE,
            "codex" => CODEX,
            "launchctl" => LAUNCHCTL,
            "systemctl" => SYSTEMCTL,
            "llama-server" => LLAMA_SERVER,
            other => panic!("no fake for {other}"),
        };
        let script = template
            .replace("@STATE@", &state.display().to_string())
            .replace("@VERSION@", env!("CARGO_PKG_VERSION"));
        let path = bin.join(tool);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// The fake's argument log, one call per line, empty when it never ran.
pub fn log(state: &Path, tool: &str) -> Vec<String> {
    std::fs::read_to_string(state.join(format!("{tool}.log")))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Replaces the fake's state file `<tool>.<name>` (e.g. "claude", "marketplace", "github delucca/bilbo v0.0.9").
pub fn set(state: &Path, tool: &str, name: &str, line: &str) {
    std::fs::create_dir_all(state).unwrap();
    std::fs::write(state.join(format!("{tool}.{name}")), format!("{line}\n")).unwrap();
}

/// The state file's first line; "" when missing or empty.
pub fn get(state: &Path, tool: &str, name: &str) -> String {
    std::fs::read_to_string(state.join(format!("{tool}.{name}")))
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
}
