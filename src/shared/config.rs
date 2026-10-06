use crate::shared::store::{self, Env};
use crate::shared::text;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const DEFAULT_MIN_SIMILARITY: f64 = 0.5;
pub const DEFAULT_DIGEST_SIMILARITY: f64 = 0.55;

pub const DEFAULT_KEEP_DAYS: u32 = 90;
pub const DEFAULT_POLL_SECONDS: u32 = 30;
pub const DEFAULT_STALE_DAYS: u32 = 180;

pub const KEYS: [&str; 12] = [
    "embedder.url",
    "embedder.model",
    "embedder.token_file",
    "embedder.token_env",
    "embedder.query_prefix",
    "embedder.min_similarity",
    "digest.enable",
    "digest.min_similarity",
    "digest.log",
    "history.keep_days",
    "sync.poll_seconds",
    "sync.stale_days",
];

/// The keys of the scope pattern, for messages.
const SCOPE_KEYS: [&str; 5] = [
    "scope.<name>.sync",
    "scope.<name>.embedder",
    "scope.<name>.paths",
    "scope.<name>.marks",
    "scope.default",
];

pub const QWEN_PREFIX: &str = "Instruct: Given a question, retrieve notes that answer it\nQuery: ";

#[derive(Debug)]
pub struct Settings {
    /// The file the settings came from, or would come from; `None` when no path can be formed.
    pub path: Option<PathBuf>,
    /// `None` without `embedder.url`: bilbo is keyword-only.
    pub embedder: Option<Embedder>,
    pub digest: Digest,
    pub history: History,
    pub sync: Sync,
    /// The digest, history and sync lines the file held, as written (unquoted), in `KEYS` order; a rewrite keeps them.
    pub kept_lines: Vec<(&'static str, String)>,
    /// The scope lines the file held, as written (unquoted), in file order; a rewrite keeps them.
    pub scope_lines: Vec<(String, String)>,
    /// The declared scopes, sorted by name.
    pub scopes: Vec<Scope>,
    /// The scope `scope.default` names, always declared.
    pub default_scope: Option<String>,
    /// The absolute `HOME`, which `~/` in a scope path expands to.
    home: Option<PathBuf>,
}

/// Whether a note's text may go to a remote embedder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Any,
    Local,
}

impl Rule {
    pub fn as_str(self) -> &'static str {
        match self {
            Rule::Any => "any",
            Rule::Local => "local",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    pub name: String,
    /// `off` or a sync URL, as written.
    pub sync: String,
    pub embedder: Rule,
    /// Each item as written, trimmed.
    pub paths: Vec<String>,
    /// Each mark with its text as written, trimmed.
    pub marks: Vec<(Mark, String)>,
}

/// A mark, in the form the mark finder compares.
#[derive(Debug, Clone, PartialEq)]
pub enum Mark {
    /// One folded word.
    Word(String),
    /// Every spelling of a path: the absolute one, then `~/...` when it lies in the home folder.
    Path(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct History {
    /// The age in days past which `bilbo watch` prunes versions.
    pub keep_days: u32,
}

impl Default for History {
    fn default() -> History {
        History {
            keep_days: DEFAULT_KEEP_DAYS,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sync {
    /// How often `bilbo watch` looks for other devices' segments.
    pub poll_seconds: u32,
    /// How long a device may leave a segment unacknowledged before it is stale.
    pub stale_days: u32,
}

impl Default for Sync {
    fn default() -> Sync {
        Sync {
            poll_seconds: DEFAULT_POLL_SECONDS,
            stale_days: DEFAULT_STALE_DAYS,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Digest {
    /// Whether `bilbo digest` does anything; off, it prints and writes nothing.
    pub enable: bool,
    /// The similarity a note's best passage needs to enter the digest when an embedder answers.
    pub min_similarity: f64,
    /// Whether `bilbo digest` appends a line per run to its log.
    pub log: bool,
}

impl Default for Digest {
    fn default() -> Digest {
        Digest {
            enable: true,
            min_similarity: DEFAULT_DIGEST_SIMILARITY,
            log: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Embedder {
    /// As written in the file; shown in messages as written.
    pub url: String,
    pub model: String,
    pub token: Option<Token>,
    pub query_prefix: String,
    pub min_similarity: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Absolute, with `~/` already expanded.
    File(PathBuf),
    Var(String),
}

/// The config file and whether BILBO_CONFIG named it; Ok(None) without BILBO_CONFIG, XDG_CONFIG_HOME or HOME.
pub fn path(env: &Env) -> Result<Option<(PathBuf, bool)>, String> {
    if let Some(value) = env.bilbo_config.as_ref().filter(|v| !v.is_empty()) {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(format!(
                "BILBO_CONFIG must be an absolute path, got '{}'",
                path.display()
            ));
        }
        return Ok(Some((path, true)));
    }
    Ok(store::config_home(env).map(|home| (home.join("bilbo/config"), false)))
}

/// The settings, or the message for a config error, without the `bilbo: ` prefix.
pub fn load(env: &Env) -> Result<Settings, String> {
    let Some((path, explicit)) = path(env)? else {
        return Ok(defaults(None));
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e)
            if !explicit
                && matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
        {
            return Ok(defaults(Some(path)));
        }
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let text =
        String::from_utf8(bytes).map_err(|_| format!("{}: not valid UTF-8", path.display()))?;
    let home = store::absolute(&env.home);
    parse(&path, &text, home.as_deref())
}

fn defaults(path: Option<PathBuf>) -> Settings {
    Settings {
        path,
        embedder: None,
        digest: Digest::default(),
        history: History::default(),
        sync: Sync::default(),
        kept_lines: Vec::new(),
        scope_lines: Vec::new(),
        scopes: Vec::new(),
        default_scope: None,
        home: None,
    }
}

/// The settings of the file `path` holding `text`; `home` expands `~/`.
fn parse(path: &Path, text: &str, home: Option<&Path>) -> Result<Settings, String> {
    let at = path.display();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut seen: Vec<(&str, usize)> = Vec::new();
    let mut url = None;
    let mut model = None;
    let mut token_file = None;
    let mut token_env = None;
    let mut query_prefix = String::new();
    let mut min_similarity = DEFAULT_MIN_SIMILARITY;
    let mut digest = Digest::default();
    let mut history = History::default();
    let mut sync = Sync::default();
    let mut kept_lines: Vec<(&'static str, String)> = Vec::new();
    let mut scope_lines: Vec<(String, String)> = Vec::new();
    let mut declared = Declared::default();
    for (index, line) in text.split('\n').enumerate() {
        let n = index + 1;
        let line = line.strip_suffix('\r').unwrap_or(line);
        let line = trim(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("{at}:{n}: expected <key> = <value>"));
        };
        let key = trim(key);
        let fixed = KEYS.iter().find(|k| **k == key).copied();
        let kind = match fixed {
            Some(fixed) => Key::Fixed(fixed),
            None => Key::Scope(scope_key(key).map_err(|e| format!("{at}:{n}: {e}"))?),
        };
        if let Some((_, first)) = seen.iter().find(|(k, _)| *k == key) {
            return Err(format!(
                "{at}:{n}: {key} is set twice; first on line {first}"
            ));
        }
        seen.push((key, n));
        let value = unquote(trim(value)).map_err(|e| match e {
            Unquote::Unclosed => format!("{at}:{n}: {key} has an unclosed quote"),
            Unquote::TextAfter => format!("{at}:{n}: {key} has text after its closing quote"),
            Unquote::Escape(c) => {
                format!("{at}:{n}: {key} has an unknown escape '\\{c}'; use \\n, \\\\ or \\\"")
            }
            Unquote::LoneBackslash => format!("{at}:{n}: {key} ends with a lone '\\'"),
        })?;
        if value.is_empty() && key != "embedder.query_prefix" {
            return Err(format!("{at}:{n}: {key} needs a value"));
        }
        let key = match kind {
            Key::Fixed(key) => key,
            Key::Scope(scope_key) => {
                scope_lines.push((key.to_string(), value.clone()));
                declared
                    .set(scope_key, &value, n, home)
                    .map_err(|e| format!("{at}:{n}: {e}"))?;
                continue;
            }
        };
        if key.starts_with("digest.") || key.starts_with("history.") || key.starts_with("sync.") {
            kept_lines.push((key, value.clone()));
        }
        match key {
            "embedder.url" => {
                check_url(&value).map_err(|e| match e {
                    UrlError::Credentials => {
                        format!("{at}:{n}: embedder.url must not hold a user name or password")
                    }
                    UrlError::Shape => format!(
                        "{at}:{n}: embedder.url must be an http:// or https:// URL with a host, got '{value}'"
                    ),
                })?;
                url = Some(value);
            }
            "embedder.model" => model = Some(value),
            "embedder.token_file" => {
                let file = if Path::new(&value).is_absolute() {
                    PathBuf::from(&value)
                } else if let Some(rest) = value.strip_prefix("~/") {
                    match home {
                        Some(home) => home.join(rest),
                        None => {
                            return Err(format!(
                                "{at}:{n}: embedder.token_file starts with ~/ but HOME is not an absolute path"
                            ));
                        }
                    }
                } else {
                    return Err(format!(
                        "{at}:{n}: embedder.token_file must be an absolute path or start with ~/"
                    ));
                };
                token_file = Some(file);
            }
            "embedder.token_env" => {
                if !is_variable_name(&value) {
                    return Err(format!(
                        "{at}:{n}: embedder.token_env must be a variable name (letters, digits and _, not starting with a digit)"
                    ));
                }
                token_env = Some(value);
            }
            "embedder.min_similarity" => {
                min_similarity = parse_similarity(&value).ok_or_else(|| {
                    format!(
                        "{at}:{n}: embedder.min_similarity must be a number from 0 to 1, got '{value}'"
                    )
                })?;
            }
            "digest.enable" => {
                digest.enable = match value.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => {
                        return Err(format!(
                            "{at}:{n}: digest.enable must be on or off, got '{value}'"
                        ));
                    }
                };
            }
            "digest.min_similarity" => {
                digest.min_similarity = parse_similarity(&value).ok_or_else(|| {
                    format!(
                        "{at}:{n}: digest.min_similarity must be a number from 0 to 1, got '{value}'"
                    )
                })?;
            }
            "digest.log" => {
                digest.log = match value.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => {
                        return Err(format!(
                            "{at}:{n}: digest.log must be on or off, got '{value}'"
                        ));
                    }
                };
            }
            "history.keep_days" => {
                history.keep_days = parse_days(&value).ok_or_else(|| {
                    format!(
                        "{at}:{n}: history.keep_days must be a whole number of days from 1 to 3650, got '{value}'"
                    )
                })?;
            }
            "sync.poll_seconds" => {
                sync.poll_seconds = parse_whole(&value, 3600).ok_or_else(|| {
                    format!(
                        "{at}:{n}: sync.poll_seconds must be a whole number of seconds from 1 to 3600, got '{value}'"
                    )
                })?;
            }
            "sync.stale_days" => {
                sync.stale_days = parse_whole(&value, 3650).ok_or_else(|| {
                    format!(
                        "{at}:{n}: sync.stale_days must be a whole number of days from 1 to 3650, got '{value}'"
                    )
                })?;
            }
            _ => query_prefix = value,
        }
    }
    kept_lines.sort_by_key(|(key, _)| KEYS.iter().position(|k| k == key));
    if token_file.is_some() && token_env.is_some() {
        return Err(format!(
            "{at}: set embedder.token_file or embedder.token_env, not both"
        ));
    }
    let (scopes, default_scope) = declared
        .finish()
        .map_err(|(message, n)| format!("{at}:{n}: {message}"))?;
    let embedder = match url {
        None => None,
        Some(url) => {
            let Some(model) = model else {
                return Err(format!(
                    "{at}: embedder.url is set but embedder.model is not"
                ));
            };
            let token = token_file
                .map(Token::File)
                .or_else(|| token_env.map(Token::Var));
            Some(Embedder {
                url,
                model,
                token,
                query_prefix,
                min_similarity,
            })
        }
    };
    Ok(Settings {
        path: Some(path.to_path_buf()),
        embedder,
        digest,
        history,
        sync,
        kept_lines,
        scope_lines,
        scopes,
        default_scope,
        home: home.map(Path::to_path_buf),
    })
}

enum Key<'a> {
    Fixed(&'static str),
    Scope(ScopeKey<'a>),
}

enum ScopeKey<'a> {
    Default,
    Field(&'a str, &'static str),
}

/// What a `scope.` key names, or the message for a key that is not one.
fn scope_key(key: &str) -> Result<ScopeKey<'_>, String> {
    let unknown = || {
        format!(
            "unknown key '{key}'; keys: {}, {}",
            KEYS.join(", "),
            SCOPE_KEYS.join(", ")
        )
    };
    let Some(rest) = key.strip_prefix("scope.") else {
        return Err(unknown());
    };
    if rest == "default" {
        return Ok(ScopeKey::Default);
    }
    let Some((name, field)) = rest.rsplit_once('.') else {
        return Err(unknown());
    };
    let Some(field) = ["sync", "embedder", "paths", "marks"]
        .into_iter()
        .find(|f| *f == field)
    else {
        return Err(unknown());
    };
    if name == "default" {
        return Err(format!(
            "{key}: 'default' is not a scope name; use scope.default"
        ));
    }
    if !store::is_topic(name) {
        return Err(format!(
            "{key}: scope name '{name}' must be lowercase letters and digits, joined by single hyphens"
        ));
    }
    Ok(ScopeKey::Field(name, field))
}

/// The scope keys read so far.
#[derive(Default)]
struct Declared {
    scopes: Vec<Scope>,
    default: Option<(String, usize)>,
    /// Each `paths` item, resolved, with its scope and line.
    folders: Vec<(String, PathBuf, usize)>,
    marks: Vec<(String, Mark, usize)>,
}

impl Declared {
    fn scope(&mut self, name: &str) -> &mut Scope {
        let at = match self.scopes.iter().position(|s| s.name == name) {
            Some(at) => at,
            None => {
                self.scopes.push(Scope {
                    name: name.to_string(),
                    sync: "off".to_string(),
                    embedder: Rule::Any,
                    paths: Vec::new(),
                    marks: Vec::new(),
                });
                self.scopes.len() - 1
            }
        };
        &mut self.scopes[at]
    }

    /// Takes one scope line; the message of an error does not say where the line is.
    fn set(
        &mut self,
        key: ScopeKey<'_>,
        value: &str,
        n: usize,
        home: Option<&Path>,
    ) -> Result<(), String> {
        let (name, field) = match key {
            ScopeKey::Default => {
                self.default = Some((value.to_string(), n));
                return Ok(());
            }
            ScopeKey::Field(name, field) => (name, field),
        };
        let key = format!("scope.{name}.{field}");
        match field {
            "sync" => {
                if let Err(why) = sync_problem(value) {
                    return Err(match why {
                        UrlError::Credentials => format!("{key} {URL_CREDENTIALS}"),
                        UrlError::Shape => {
                            format!("{key} must be off or a sync URL, got '{value}'")
                        }
                    });
                }
                self.scope(name).sync = value.to_string();
            }
            "embedder" => {
                self.scope(name).embedder = match value {
                    "any" => Rule::Any,
                    "local" => Rule::Local,
                    _ => return Err(format!("{key} must be any or local, got '{value}'")),
                };
            }
            "paths" => {
                for item in list(&key, value)? {
                    let folder = folder(&key, &item, home)?;
                    self.scope(name).paths.push(item);
                    if let Some((other, _, line)) = self
                        .folders
                        .iter()
                        .find(|(other, f, _)| other != name && *f == folder)
                    {
                        return Err(format!(
                            "{key} names the same folder as scope.{other}.paths (line {line})"
                        ));
                    }
                    self.folders.push((name.to_string(), folder, n));
                }
            }
            _ => {
                for item in list(&key, value)? {
                    let mark = mark(&key, &item, home)?;
                    if let Some((other, _, line)) = self
                        .marks
                        .iter()
                        .find(|(other, m, _)| other != name && *m == mark)
                    {
                        return Err(format!(
                            "{key} holds '{item}', which scope.{other}.marks holds too (line {line})"
                        ));
                    }
                    self.marks.push((name.to_string(), mark.clone(), n));
                    self.scope(name).marks.push((mark, item));
                }
            }
        }
        Ok(())
    }

    /// The scopes sorted by name and the default, or the message and line of the default's error.
    fn finish(mut self) -> Result<(Vec<Scope>, Option<String>), (String, usize)> {
        if let Some((name, n)) = &self.default
            && !self.scopes.iter().any(|s| s.name == *name)
        {
            return Err((
                format!("scope.default names '{name}', which no scope.{name}.* key declares"),
                *n,
            ));
        }
        self.scopes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok((self.scopes, self.default.map(|(name, _)| name)))
    }
}

/// The trimmed items of a comma-separated list; none may be empty.
fn list(key: &str, value: &str) -> Result<Vec<String>, String> {
    value
        .split(',')
        .map(|item| match trim(item) {
            "" => Err(format!("{key} has an empty item")),
            item => Ok(item.to_string()),
        })
        .collect()
}

enum Unexpandable {
    NoHome,
    Relative,
}

/// `item` as an absolute path, with `~/` expanded.
fn expand(item: &str, home: Option<&Path>) -> Result<PathBuf, Unexpandable> {
    if let Some(rest) = item.strip_prefix("~/") {
        return home
            .map(|home| home.join(rest.trim_start_matches('/')))
            .ok_or(Unexpandable::NoHome);
    }
    if item.starts_with('/') {
        return Ok(PathBuf::from(item));
    }
    Err(Unexpandable::Relative)
}

fn expand_or_say(key: &str, item: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    expand(item, home).map_err(|e| match e {
        Unexpandable::NoHome => {
            format!("{key} item '{item}' starts with ~/ but HOME is not an absolute path")
        }
        Unexpandable::Relative => {
            format!("{key} item '{item}' must be an absolute path or start with ~/")
        }
    })
}

/// The folder a `paths` item names, with links resolved when it exists.
fn folder(key: &str, item: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    expand_or_say(key, item, home).map(|path| resolve(&path))
}

fn resolve(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn mark(key: &str, item: &str, home: Option<&Path>) -> Result<Mark, String> {
    if item.starts_with('/') || item.starts_with("~/") {
        let path: PathBuf = expand_or_say(key, item, home)?.components().collect();
        let mut forms = vec![path.display().to_string()];
        if let Some(rest) = home.and_then(|home| path.strip_prefix(home).ok()) {
            forms.push(if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.display())
            });
        }
        return Ok(Mark::Path(forms));
    }
    let words = text::words(item);
    let one_word = item
        .chars()
        .all(|c| c.is_alphanumeric() || text::is_mark(c));
    match (words.as_slice(), one_word) {
        ([word], true) => Ok(Mark::Word(word.clone())),
        _ => Err(format!(
            "{key} item '{item}' must be one word of letters and digits, or a path"
        )),
    }
}

impl Settings {
    /// The names of the declared scopes, sorted.
    pub fn scope_names(&self) -> Vec<&str> {
        self.scopes.iter().map(|s| s.name.as_str()).collect()
    }

    pub fn scope(&self, name: &str) -> Option<&Scope> {
        self.scopes.iter().find(|s| s.name == name)
    }

    /// The config path for a message: the file, or the default spelling when none can be formed.
    pub fn shown_path(&self) -> String {
        self.path.as_ref().map_or_else(
            || "$HOME/.config/bilbo/config".to_string(),
            |p| p.display().to_string(),
        )
    }

    /// The embedder rule of a note whose `scope` key holds `scope` (`None` without the key).
    /// A scope that is not declared counts as unassigned, as do all notes while none is declared.
    pub fn rule(&self, scope: Option<&str>) -> Rule {
        if let Some(scope) = scope.and_then(|name| self.scope(name)) {
            return scope.embedder;
        }
        if self.scopes.iter().any(|s| s.embedder == Rule::Local) {
            Rule::Local
        } else {
            Rule::Any
        }
    }

    /// The scope whose `paths` hold the working directory `cwd` most deeply, comparing whole folder
    /// names with links resolved on both sides; `None` when no entry holds it, two scopes tie,
    /// or `cwd` cannot be resolved.
    pub fn scope_for(&self, cwd: &Path) -> Option<&str> {
        let cwd = std::fs::canonicalize(cwd).ok()?;
        let mut best: Option<(usize, &str)> = None;
        let mut tied = false;
        for scope in &self.scopes {
            for item in &scope.paths {
                let Ok(folder) = expand(item, self.home.as_deref()).map(|p| resolve(&p)) else {
                    continue;
                };
                if !cwd.starts_with(&folder) {
                    continue;
                }
                let depth = folder.components().count();
                match best {
                    Some((d, name)) if d == depth => tied |= name != scope.name,
                    Some((d, _)) if d > depth => {}
                    _ => {
                        best = Some((depth, scope.name.as_str()));
                        tied = false;
                    }
                }
            }
        }
        best.filter(|_| !tied).map(|(_, name)| name)
    }
}

fn trim(text: &str) -> &str {
    text.trim_matches([' ', '\t'])
}

enum Unquote {
    Unclosed,
    TextAfter,
    Escape(char),
    LoneBackslash,
}

/// The value with its quotes and escapes resolved; `value` is already trimmed.
fn unquote(value: &str) -> Result<String, Unquote> {
    let quoted = value.starts_with('"');
    let body = if quoted { &value[1..] } else { value };
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted => {
                return if chars.next().is_some() {
                    Err(Unquote::TextAfter)
                } else {
                    Ok(out)
                };
            }
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some('"') if quoted => out.push('"'),
                Some(other) => return Err(Unquote::Escape(other)),
                None if quoted => return Err(Unquote::Unclosed),
                None => return Err(Unquote::LoneBackslash),
            },
            _ => out.push(c),
        }
    }
    if quoted {
        Err(Unquote::Unclosed)
    } else {
        Ok(out)
    }
}

enum UrlError {
    Credentials,
    Shape,
}

const URL_CREDENTIALS: &str = "must not hold a user name or password";
const URL_SHAPE: &str = "must be an http:// or https:// URL with a host";

/// Why `url` is not a valid embedder.url, or None: "must not hold a user name or password" | "must be an http:// or https:// URL with a host"
pub fn url_problem(url: &str) -> Option<&'static str> {
    let authority = match url.split_once("://") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next().unwrap_or(""),
        None => url,
    };
    if authority.contains('@') {
        return Some(URL_CREDENTIALS);
    }
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return Some(URL_SHAPE);
    };
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Some(URL_SHAPE);
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = match authority.rfind(':') {
        Some(at) => &authority[..at],
        None => authority,
    };
    if host.is_empty() {
        return Some(URL_SHAPE);
    }
    None
}

fn check_url(url: &str) -> Result<(), UrlError> {
    match url_problem(url) {
        None => Ok(()),
        Some(URL_CREDENTIALS) => Err(UrlError::Credentials),
        Some(_) => Err(UrlError::Shape),
    }
}

/// `off`, `file://` and an absolute path taken literally, or an `https://` URL (`http://` only when local) with a
/// valid port and no query or fragment. Credentials are tested first, so no other error repeats a password.
fn sync_problem(value: &str) -> Result<(), UrlError> {
    if value == "off" {
        return Ok(());
    }
    let problem = url_problem(value);
    if problem == Some(URL_CREDENTIALS) {
        return Err(UrlError::Credentials);
    }
    if value.chars().any(char::is_control) || value.contains(['?', '#']) {
        return Err(UrlError::Shape);
    }
    if let Some(path) = value.strip_prefix("file://") {
        return if path.starts_with('/') {
            Ok(())
        } else {
            Err(UrlError::Shape)
        };
    }
    if problem.is_some() || (value.starts_with("http://") && !is_local(value)) {
        return Err(UrlError::Shape);
    }
    let authority = value
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .unwrap_or("");
    let tail = match authority.strip_prefix('[') {
        Some(inner) => match inner.split_once(']') {
            Some((host, tail)) if !host.is_empty() => tail,
            _ => return Err(UrlError::Shape),
        },
        None => {
            if authority.matches(':').count() > 1 {
                return Err(UrlError::Shape);
            }
            authority.find(':').map_or("", |at| &authority[at..])
        }
    };
    match tail.strip_prefix(':') {
        None if tail.is_empty() => Ok(()),
        Some(digits)
            if !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_digit())
                && digits.parse::<u16>().is_ok_and(|p| p != 0) =>
        {
            Ok(())
        }
        _ => Err(UrlError::Shape),
    }
}

/// The URL's host (lowercased, [ ] stripped) is localhost, 127.0.0.1 or ::1.
pub fn is_local(url: &str) -> bool {
    let Some((_, rest)) = url.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = match authority.strip_prefix('[') {
        Some(inner) => inner.split(']').next().unwrap_or(""),
        None => match authority.rfind(':') {
            Some(at) => &authority[..at],
            None => authority,
        },
    };
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

/// QWEN_PREFIX when the model, lowercased, holds "qwen3-embedding"; "" otherwise.
pub fn default_query_prefix(model: &str) -> &'static str {
    if model.to_lowercase().contains("qwen3-embedding") {
        QWEN_PREFIX
    } else {
        ""
    }
}

/// Writes `text` to a temporary file in the folder, then renames it over `path`, after moving an old file to `<name>.bak` when `backup`.
pub fn write_config(path: &Path, text: &str, backup: bool) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    let dir = path.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(dir).map_err(fail)?;
    let temp = dir.join(format!(".config.tmp-{}", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if backup {
            let name = path
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
            std::fs::rename(path, dir.join(format!("{name}.bak")))?;
        }
        std::fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written.map_err(fail)
}

/// Sets each key in the file at `path`: the first line that assigns it is replaced in place, else a line is appended; every other line stays as written, and an old file is kept as `<name>.bak`.
pub fn set_keys(path: &Path, keys: &[(String, String)]) -> Result<(), String> {
    let (old, existed) = match std::fs::read_to_string(path) {
        Ok(text) => (text, true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), false),
        Err(e) => return Err(format!("cannot write {}: {e}", path.display())),
    };
    let mut lines: Vec<String> = old.split_inclusive('\n').map(String::from).collect();
    for (key, value) in keys {
        let line = format!("{key} = {}", quote(value));
        let found = lines
            .iter()
            .position(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == key));
        match found {
            Some(i) => {
                let ending = &lines[i][lines[i].trim_end_matches(['\r', '\n']).len()..];
                lines[i] = format!("{line}{ending}");
            }
            None => {
                if let Some(last) = lines.last_mut()
                    && !last.ends_with('\n')
                {
                    last.push('\n');
                }
                lines.push(format!("{line}\n"));
            }
        }
    }
    write_config(path, &lines.concat(), existed)
}

/// The value as written in the file: quoted when it is empty, starts or ends with a space or tab, starts with `"`, or holds a line break; `\` doubled either way.
pub fn quote(value: &str) -> String {
    let edge = |c: char| c == ' ' || c == '\t';
    let quoted = value.is_empty()
        || value.starts_with(edge)
        || value.ends_with(edge)
        || value.starts_with('"')
        || value.contains(['\n', '\r']);
    if !quoted {
        return value.replace('\\', "\\\\");
    }
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// (key, value) pairs of `e` in KEYS order: url, model, token_file (absolute path) or token_env, query_prefix if not empty, min_similarity if != 0.5 (written with `{}`).
pub fn settings(e: &Embedder) -> Vec<(&'static str, String)> {
    let mut out = vec![
        ("embedder.url", e.url.clone()),
        ("embedder.model", e.model.clone()),
    ];
    match &e.token {
        Some(Token::File(path)) => out.push(("embedder.token_file", path.display().to_string())),
        Some(Token::Var(name)) => out.push(("embedder.token_env", name.clone())),
        None => {}
    }
    if !e.query_prefix.is_empty() {
        out.push(("embedder.query_prefix", e.query_prefix.clone()));
    }
    if e.min_similarity != DEFAULT_MIN_SIMILARITY {
        out.push(("embedder.min_similarity", format!("{}", e.min_similarity)));
    }
    out
}

/// `header` (one comment line, no newline), then `key = quote(value)` lines, or the example block when `settings` is empty.
pub fn render(header: &str, settings: &[(String, String)]) -> String {
    let mut out = format!("{header}\n");
    if settings.is_empty() {
        out.push_str(
            "# One <key> = <value> per line; the keys are under Configuration in bilbo's README.\n\
             # To search by meaning as well as by keywords, set an embedder:\n\
             # embedder.url = http://localhost:11434\n\
             # embedder.model = nomic-embed-text\n",
        );
    }
    for (key, value) in settings {
        out.push_str(&format!("{key} = {}\n", quote(value)));
    }
    out
}

pub fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A whole number of days from 1 to 3650 written as digits.
fn parse_days(value: &str) -> Option<u32> {
    parse_whole(value, 3650)
}

/// A whole number from 1 to `max` written as digits.
fn parse_whole(value: &str, max: u32) -> Option<u32> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<u32>().ok().filter(|d| (1..=max).contains(d))
}

/// A number from 0 to 1 written as digits with an optional `.` and digits.
fn parse_similarity(value: &str) -> Option<f64> {
    let (whole, fraction) = match value.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (value, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || fraction.is_some_and(|f| !digits(f)) {
        return None;
    }
    value.parse::<f64>().ok().filter(|v| *v <= 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn env() -> Env {
        Env::from_vars(|_| None)
    }

    fn os(value: &str) -> Option<OsString> {
        Some(OsString::from(value))
    }

    const BASE: &str = "embedder.url = http://embedder.example:8081\nembedder.model = m\n";

    fn parsed(text: &str) -> Result<Option<Embedder>, String> {
        parse(Path::new("/c"), text, Some(Path::new("/home/a"))).map(|s| s.embedder)
    }

    fn digest(text: &str) -> Result<Digest, String> {
        parse(Path::new("/c"), text, Some(Path::new("/home/a"))).map(|s| s.digest)
    }

    #[test]
    fn digest_defaults() {
        assert_eq!(
            digest(BASE).unwrap(),
            Digest {
                enable: true,
                min_similarity: 0.55,
                log: false
            }
        );
        assert_eq!(digest("").unwrap(), Digest::default());
    }

    #[test]
    fn digest_keys_without_an_embedder() {
        let d =
            digest("digest.enable = off\ndigest.min_similarity = 0.7\ndigest.log = on\n").unwrap();
        assert_eq!(
            d,
            Digest {
                enable: false,
                min_similarity: 0.7,
                log: true
            }
        );
    }

    #[test]
    fn digest_lines_keep_what_the_file_wrote_in_keys_order() {
        let text = "digest.log = \"off\"\nembedder.url = http://h\nembedder.model = m\ndigest.enable = on\ndigest.min_similarity = 0.55\n";
        let lines = parse(Path::new("/c"), text, None).unwrap().kept_lines;
        assert_eq!(
            lines,
            [
                ("digest.enable", "on".to_string()),
                ("digest.min_similarity", "0.55".to_string()),
                ("digest.log", "off".to_string())
            ]
        );
        assert!(
            parse(
                Path::new("/c"),
                "embedder.url = http://h\nembedder.model = m\n",
                None
            )
            .unwrap()
            .kept_lines
            .is_empty()
        );
    }

    #[test]
    fn history_defaults_and_values() {
        let history = |text: &str| parse(Path::new("/c"), text, None).map(|s| s.history);
        assert_eq!(history("").unwrap().keep_days, 90);
        assert_eq!(history(BASE).unwrap().keep_days, 90);
        assert_eq!(history("history.keep_days = 30\n").unwrap().keep_days, 30);
        assert_eq!(history("history.keep_days = 1\n").unwrap().keep_days, 1);
        assert_eq!(
            history("history.keep_days = 3650\n").unwrap().keep_days,
            3650
        );
    }

    #[test]
    fn history_bad_values() {
        for value in ["0", "3651", "2w", "-5", "+5", "1.5", "99999999999"] {
            let message = parse(
                Path::new("/c"),
                &format!("\nhistory.keep_days = {value}\n"),
                None,
            )
            .unwrap_err();
            assert_eq!(
                message,
                format!(
                    "/c:2: history.keep_days must be a whole number of days from 1 to 3650, got '{value}'"
                )
            );
        }
    }

    #[test]
    fn history_lines_are_kept_beside_the_digest_lines() {
        let text = "history.keep_days = 30\ndigest.log = on\nembedder.url = http://h\nembedder.model = m\n";
        let lines = parse(Path::new("/c"), text, None).unwrap().kept_lines;
        assert_eq!(
            lines,
            [
                ("digest.log", "on".to_string()),
                ("history.keep_days", "30".to_string())
            ]
        );
    }

    #[test]
    fn sync_defaults_and_values() {
        let sync = |text: &str| parse(Path::new("/c"), text, None).map(|s| s.sync);
        assert_eq!(sync("").unwrap(), Sync::default());
        assert_eq!(sync(BASE).unwrap().poll_seconds, 30);
        assert_eq!(sync(BASE).unwrap().stale_days, 180);
        let both = sync("sync.poll_seconds = 1\nsync.stale_days = 3650\n").unwrap();
        assert_eq!((both.poll_seconds, both.stale_days), (1, 3650));
        assert_eq!(
            sync("sync.poll_seconds = 3600\n").unwrap().poll_seconds,
            3600
        );
        assert_eq!(sync("sync.stale_days = 1\n").unwrap().stale_days, 1);
    }

    #[test]
    fn sync_keys_need_no_embedder_or_scope() {
        let settings = parse(Path::new("/c"), "sync.poll_seconds = 60\n", None).unwrap();
        assert!(settings.embedder.is_none());
        assert!(settings.scopes.is_empty());
        assert_eq!(settings.sync.poll_seconds, 60);
    }

    #[test]
    fn sync_bad_values() {
        for (key, max, values) in [
            (
                "sync.poll_seconds",
                "seconds from 1 to 3600",
                ["0", "3601", "1y", "-5", "1.5"],
            ),
            (
                "sync.stale_days",
                "days from 1 to 3650",
                ["0", "3651", "1y", "+5", "99999999999"],
            ),
        ] {
            for value in values {
                let message =
                    parse(Path::new("/c"), &format!("\n{key} = {value}\n"), None).unwrap_err();
                assert_eq!(
                    message,
                    format!("/c:2: {key} must be a whole number of {max}, got '{value}'")
                );
            }
        }
    }

    #[test]
    fn sync_lines_are_kept_after_the_history_lines() {
        let text =
            "sync.stale_days = 9\nsync.poll_seconds = 5\nhistory.keep_days = 30\ndigest.log = on\n";
        let lines = parse(Path::new("/c"), text, None).unwrap().kept_lines;
        assert_eq!(
            lines,
            [
                ("digest.log", "on".to_string()),
                ("history.keep_days", "30".to_string()),
                ("sync.poll_seconds", "5".to_string()),
                ("sync.stale_days", "9".to_string())
            ]
        );
    }

    #[test]
    fn digest_bad_values() {
        assert_eq!(
            digest("digest.enable = no\n").unwrap_err(),
            "/c:1: digest.enable must be on or off, got 'no'"
        );
        assert_eq!(
            digest("digest.log = yes\n").unwrap_err(),
            "/c:1: digest.log must be on or off, got 'yes'"
        );
        assert_eq!(
            digest("\ndigest.min_similarity = 1.5\n").unwrap_err(),
            "/c:2: digest.min_similarity must be a number from 0 to 1, got '1.5'"
        );
    }

    fn embedder(text: &str) -> Embedder {
        parsed(text).unwrap().unwrap()
    }

    fn err(text: &str) -> String {
        parsed(text).unwrap_err()
    }

    #[test]
    fn default_location() {
        let e = Env {
            home: os("/nonexistent-bilbo-home/a"),
            ..env()
        };
        let settings = load(&e).unwrap();
        assert_eq!(
            settings.path,
            Some(PathBuf::from(
                "/nonexistent-bilbo-home/a/.config/bilbo/config"
            ))
        );
        assert!(settings.embedder.is_none());
    }

    #[test]
    fn absolute_xdg_config_home_is_used() {
        let e = Env {
            xdg_config_home: os("/nonexistent-bilbo-xdg"),
            home: os("/nonexistent-bilbo-home"),
            ..env()
        };
        assert_eq!(
            load(&e).unwrap().path,
            Some(PathBuf::from("/nonexistent-bilbo-xdg/bilbo/config"))
        );
    }

    #[test]
    fn relative_xdg_config_home_is_ignored() {
        let e = Env {
            xdg_config_home: os("conf"),
            home: os("/nonexistent-bilbo-home"),
            ..env()
        };
        assert_eq!(
            load(&e).unwrap().path,
            Some(PathBuf::from(
                "/nonexistent-bilbo-home/.config/bilbo/config"
            ))
        );
    }

    #[test]
    fn bilbo_config_wins() {
        let dir = scratch("wins");
        let file = dir.0.join("mine");
        std::fs::write(&file, BASE).unwrap();
        let e = Env {
            bilbo_config: Some(file.clone().into_os_string()),
            xdg_config_home: os("/nonexistent-bilbo-xdg"),
            home: os("/nonexistent-bilbo-home"),
            ..env()
        };
        let settings = load(&e).unwrap();
        assert_eq!(settings.path, Some(file));
        assert_eq!(
            settings.embedder.unwrap().url,
            "http://embedder.example:8081"
        );
    }

    #[test]
    fn empty_bilbo_config_counts_as_unset() {
        let e = Env {
            bilbo_config: os(""),
            home: os("/nonexistent-bilbo-home"),
            ..env()
        };
        assert_eq!(
            load(&e).unwrap().path,
            Some(PathBuf::from(
                "/nonexistent-bilbo-home/.config/bilbo/config"
            ))
        );
    }

    #[test]
    fn relative_bilbo_config_is_refused() {
        let e = Env {
            bilbo_config: os("conf"),
            ..env()
        };
        assert_eq!(
            load(&e).unwrap_err(),
            "BILBO_CONFIG must be an absolute path, got 'conf'"
        );
    }

    #[test]
    fn missing_default_file_gives_defaults() {
        let dir = scratch("missing");
        let e = Env {
            home: Some(dir.0.clone().into_os_string()),
            ..env()
        };
        let settings = load(&e).unwrap();
        assert!(settings.embedder.is_none());
        assert!(settings.path.is_some());
    }

    #[test]
    fn file_in_place_of_config_folder_gives_defaults() {
        let dir = scratch("notdir");
        std::fs::create_dir_all(dir.0.join(".config")).unwrap();
        std::fs::write(dir.0.join(".config/bilbo"), "").unwrap();
        let e = Env {
            home: Some(dir.0.clone().into_os_string()),
            ..env()
        };
        assert!(load(&e).unwrap().embedder.is_none());
    }

    #[test]
    fn missing_explicit_file_names_it() {
        let e = Env {
            bilbo_config: os("/nonexistent-bilbo/config"),
            ..env()
        };
        let message = load(&e).unwrap_err();
        assert!(
            message.starts_with("cannot read /nonexistent-bilbo/config: "),
            "{message}"
        );
    }

    #[test]
    fn no_location_gives_defaults() {
        let settings = load(&env()).unwrap();
        assert!(settings.path.is_none());
        assert!(settings.embedder.is_none());
    }

    #[test]
    fn not_utf8_is_refused() {
        let dir = scratch("utf8");
        let file = dir.0.join("config");
        std::fs::write(&file, b"embedder.url = \xff").unwrap();
        let e = Env {
            bilbo_config: Some(file.clone().into_os_string()),
            ..env()
        };
        assert_eq!(
            load(&e).unwrap_err(),
            format!("{}: not valid UTF-8", file.display())
        );
    }

    #[test]
    fn valid_file() {
        let e = embedder(
            "# embedder on another machine\n\nembedder.url = http://embedder.example:8081\nembedder.model = qwen3\n",
        );
        assert_eq!(e.url, "http://embedder.example:8081");
        assert_eq!(e.model, "qwen3");
    }

    #[test]
    fn escaped_newline_in_quotes() {
        let e = embedder(&format!(
            "{BASE}embedder.query_prefix = \"Instruct: find notes\\nQuery: \"\n"
        ));
        assert_eq!(e.query_prefix, "Instruct: find notes\nQuery: ");
    }

    #[test]
    fn unquoted_value_is_trimmed() {
        let e = embedder("embedder.url =   http://x  \nembedder.model =\tqwen3 \t\n");
        assert_eq!(e.url, "http://x");
        assert_eq!(e.model, "qwen3");
    }

    #[test]
    fn unknown_key_names_file_and_line() {
        assert_eq!(
            err("# c\n\nembeder.url = http://x\n"),
            "/c:3: unknown key 'embeder.url'; keys: embedder.url, embedder.model, embedder.token_file, embedder.token_env, embedder.query_prefix, embedder.min_similarity, digest.enable, digest.min_similarity, digest.log, history.keep_days, sync.poll_seconds, sync.stale_days, scope.<name>.sync, scope.<name>.embedder, scope.<name>.paths, scope.<name>.marks, scope.default"
        );
    }

    #[test]
    fn repeated_key() {
        assert_eq!(
            err("embedder.model = a\n\nembedder.model = b\n"),
            "/c:3: embedder.model is set twice; first on line 1"
        );
    }

    #[test]
    fn line_without_equals() {
        assert_eq!(err("embedder.url\n"), "/c:1: expected <key> = <value>");
    }

    #[test]
    fn unclosed_quote() {
        assert_eq!(
            err("embedder.model = \"a\n"),
            "/c:1: embedder.model has an unclosed quote"
        );
        assert_eq!(
            err("embedder.model = \"a\\\"\n"),
            "/c:1: embedder.model has an unclosed quote"
        );
    }

    #[test]
    fn text_after_closing_quote() {
        let message = "/c:1: embedder.model has text after its closing quote";
        assert_eq!(err("embedder.model = \"a\"b\"\n"), message);
        assert_eq!(err("embedder.model = \"x\" # c\n"), message);
    }

    #[test]
    fn unknown_escape() {
        assert_eq!(
            err("embedder.model = \"a\\tb\"\n"),
            "/c:1: embedder.model has an unknown escape '\\t'; use \\n, \\\\ or \\\""
        );
        assert_eq!(
            err("embedder.model = a\\tb\n"),
            "/c:1: embedder.model has an unknown escape '\\t'; use \\n, \\\\ or \\\""
        );
        assert_eq!(
            err("embedder.model = a\\\"b\n"),
            "/c:1: embedder.model has an unknown escape '\\\"'; use \\n, \\\\ or \\\""
        );
    }

    #[test]
    fn lone_trailing_backslash() {
        assert_eq!(
            err("embedder.model = a\\\n"),
            "/c:1: embedder.model ends with a lone '\\'"
        );
    }

    #[test]
    fn escaped_backslash() {
        assert_eq!(
            embedder("embedder.url = http://x\nembedder.model = a\\\\n\n").model,
            "a\\n"
        );
        assert_eq!(
            embedder("embedder.url = http://x\nembedder.model = \"a\\\\\"\n").model,
            "a\\"
        );
    }

    #[test]
    fn hash_inside_a_value_is_kept() {
        assert_eq!(
            embedder("embedder.url = http://x\nembedder.model = a #b\n").model,
            "a #b"
        );
    }

    #[test]
    fn crlf_and_bom_are_tolerated() {
        let e = embedder("\u{feff}embedder.url = http://x\r\nembedder.model = m\r\n");
        assert_eq!(e.url, "http://x");
        assert_eq!(e.model, "m");
    }

    #[test]
    fn no_spaces_around_equals() {
        let e = embedder("embedder.url=http://x\nembedder.model=m\n");
        assert_eq!(e.url, "http://x");
        assert_eq!(e.model, "m");
    }

    #[test]
    fn empty_value_is_refused() {
        assert_eq!(
            err("embedder.model =\n"),
            "/c:1: embedder.model needs a value"
        );
        assert_eq!(
            err("embedder.model = \"\"\n"),
            "/c:1: embedder.model needs a value"
        );
        assert_eq!(
            embedder(&format!("{BASE}embedder.query_prefix =\n")).query_prefix,
            ""
        );
    }

    #[test]
    fn url_without_model() {
        assert_eq!(
            err("embedder.url = http://x\n"),
            "/c: embedder.url is set but embedder.model is not"
        );
    }

    #[test]
    fn both_token_keys() {
        assert_eq!(
            err(&format!(
                "{BASE}embedder.token_file = /t\nembedder.token_env = T\n"
            )),
            "/c: set embedder.token_file or embedder.token_env, not both"
        );
    }

    #[test]
    fn similarity_out_of_range() {
        assert_eq!(
            err(&format!("{BASE}embedder.min_similarity = 1.5\n")),
            "/c:3: embedder.min_similarity must be a number from 0 to 1, got '1.5'"
        );
    }

    #[test]
    fn similarity_shapes() {
        for good in ["0", "1", "0.35", "1.0"] {
            let e = embedder(&format!("{BASE}embedder.min_similarity = {good}\n"));
            assert_eq!(e.min_similarity, good.parse::<f64>().unwrap(), "{good}");
        }
        for bad in [".5", "5.", "+0.3", "-0", "1e-1", "nan", "inf", "1.0001"] {
            assert_eq!(
                err(&format!("{BASE}embedder.min_similarity = {bad}\n")),
                format!("/c:3: embedder.min_similarity must be a number from 0 to 1, got '{bad}'"),
                "{bad}"
            );
        }
    }

    #[test]
    fn url_shapes() {
        for good in ["https://x", "http://127.0.0.1:8081/"] {
            let e = embedder(&format!("embedder.url = {good}\nembedder.model = m\n"));
            assert_eq!(e.url, good);
        }
        for bad in ["ftp://x", "http://", "http://:80", "HTTP://x", "http://a b"] {
            assert_eq!(
                err(&format!("embedder.url = {bad}\nembedder.model = m\n")),
                format!(
                    "/c:1: embedder.url must be an http:// or https:// URL with a host, got '{bad}'"
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn url_with_credentials_is_not_echoed() {
        let message = err("embedder.url = http://u:sekrit@x\nembedder.model = m\n");
        assert_eq!(
            message,
            "/c:1: embedder.url must not hold a user name or password"
        );
        assert!(!message.contains("sekrit"));
        for url in [
            "http:/u:sekrit@x",
            "u:sekrit@x",
            "u:sekrit@x:8081",
            "//u:sekrit@x",
        ] {
            let message = err(&format!("embedder.url = {url}\nembedder.model = m\n"));
            assert_eq!(
                message,
                "/c:1: embedder.url must not hold a user name or password"
            );
        }
        for url in ["http://x/path@y", "http://x?q=a@b", "http://x#a@b"] {
            assert_eq!(
                embedder(&format!("embedder.url = {url}\nembedder.model = m\n")).url,
                url
            );
        }
    }

    #[test]
    fn token_file_absolute() {
        let e = embedder(&format!("{BASE}embedder.token_file = /run/secrets/t\n"));
        assert_eq!(e.token, Some(Token::File(PathBuf::from("/run/secrets/t"))));
    }

    #[test]
    fn token_file_tilde_expands() {
        let e = embedder(&format!("{BASE}embedder.token_file = ~/.tok\n"));
        assert_eq!(e.token, Some(Token::File(PathBuf::from("/home/a/.tok"))));
    }

    #[test]
    fn token_file_tilde_without_home() {
        let message = parse(
            Path::new("/c"),
            &format!("{BASE}embedder.token_file = ~/.tok\n"),
            None,
        )
        .unwrap_err();
        assert_eq!(
            message,
            "/c:3: embedder.token_file starts with ~/ but HOME is not an absolute path"
        );
    }

    #[test]
    fn token_file_relative_is_refused() {
        assert_eq!(
            err(&format!("{BASE}embedder.token_file = tok\n")),
            "/c:3: embedder.token_file must be an absolute path or start with ~/"
        );
    }

    #[test]
    fn token_env_names() {
        for good in ["EMBED_TOKEN", "_x1"] {
            let e = embedder(&format!("{BASE}embedder.token_env = {good}\n"));
            assert_eq!(e.token, Some(Token::Var(good.into())));
        }
        for bad in ["1X", "A-B"] {
            assert_eq!(
                err(&format!("{BASE}embedder.token_env = {bad}\n")),
                "/c:3: embedder.token_env must be a variable name (letters, digits and _, not starting with a digit)"
            );
        }
        assert!(!err(&format!("{BASE}embedder.token_env = sk-abc123\n")).contains("sk-abc123"));
    }

    #[test]
    fn keys_without_url_are_ignored() {
        assert_eq!(parsed("embedder.model = m\n"), Ok(None));
        assert_eq!(
            err("embedder.min_similarity = 2\n"),
            "/c:1: embedder.min_similarity must be a number from 0 to 1, got '2'"
        );
    }

    #[test]
    fn defaults() {
        let e = embedder(BASE);
        assert_eq!(e.query_prefix, "");
        assert_eq!(e.min_similarity, 0.5);
        assert_eq!(e.token, None);
    }

    #[test]
    fn write_config_replaces_atomically_and_keeps_a_backup() {
        let dir = std::env::temp_dir().join(format!("bilbo-setup-unit-{}", std::process::id()));
        let path = dir.join("nested/config");
        write_config(&path, "one\n", false).unwrap();
        write_config(&path, "two\n", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two\n");
        assert_eq!(
            std::fs::read_to_string(dir.join("nested/config.bak")).unwrap(),
            "one\n"
        );
        let leftovers = std::fs::read_dir(dir.join("nested"))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp-")
            })
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn keys(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn set_keys_replaces_a_line_in_place() {
        let dir = scratch("set-replace");
        let path = dir.0.join("config");
        std::fs::write(&path, "a = 1\nscope.x.sync = off\nb = 2\n").unwrap();
        set_keys(&path, &keys(&[("scope.x.sync", "file:///f")])).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "a = 1\nscope.x.sync = file:///f\nb = 2\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.0.join("config.bak")).unwrap(),
            "a = 1\nscope.x.sync = off\nb = 2\n"
        );
    }

    #[test]
    fn set_keys_appends_a_missing_key() {
        let dir = scratch("set-append");
        let path = dir.0.join("config");
        std::fs::write(&path, "a = 1").unwrap();
        set_keys(&path, &keys(&[("b", " pad "), ("c", "3")])).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "a = 1\nb = \" pad \"\nc = 3\n"
        );
    }

    #[test]
    fn set_keys_keeps_comments_and_blank_lines() {
        let dir = scratch("set-keep");
        let path = dir.0.join("config");
        let old = "# a = 9\n\n  a = 1  \r\n\n# end\n";
        std::fs::write(&path, old).unwrap();
        set_keys(&path, &keys(&[("a", "2")])).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# a = 9\n\na = 2\r\n\n# end\n"
        );
    }

    #[test]
    fn set_keys_creates_a_missing_file_without_a_backup() {
        let dir = scratch("set-new");
        let path = dir.0.join("nested/config");
        set_keys(&path, &keys(&[("a", "1")])).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 1\n");
        assert!(!dir.0.join("nested/config.bak").exists());
    }

    #[test]
    fn set_keys_names_a_path_it_cannot_write() {
        let dir = scratch("set-fail");
        let blocker = dir.0.join("blocker");
        std::fs::write(&blocker, "").unwrap();
        let path = blocker.join("config");
        let err = set_keys(&path, &keys(&[("a", "1")])).unwrap_err();
        assert!(
            err.starts_with(&format!("cannot write {}:", path.display())),
            "{err}"
        );
    }

    #[test]
    fn quote_round_trips() {
        for value in [
            "http://embedder.example:8081",
            "  spaced  ",
            "C:\\x\\y",
            "\"starts with quote",
            "a \"middle\" quote",
            "#hash",
            "a=b",
            QWEN_PREFIX,
        ] {
            let text = format!(
                "embedder.url = http://x\nembedder.model = m\nembedder.query_prefix = {}\n",
                quote(value)
            );
            assert_eq!(embedder(&text).query_prefix, value, "{value:?}");
        }
    }

    #[test]
    fn render_empty_is_comments_only() {
        let text = render("# bilbo config", &[]);
        assert!(text.starts_with("# bilbo config\n# One <key> = <value> per line"));
        assert!(text.ends_with("# embedder.model = nomic-embed-text\n"));
        assert_eq!(parsed(&text), Ok(None));
    }

    #[test]
    fn render_settings_parse_back() {
        let e = Embedder {
            url: "http://embedder.example:8081".into(),
            model: "qwen3-embedding-0.6b".into(),
            token: Some(Token::File(PathBuf::from("/home/a/.config/bilbo/token"))),
            query_prefix: QWEN_PREFIX.into(),
            min_similarity: 0.6,
        };
        let pairs = settings(&e);
        assert_eq!(pairs[0].0, "embedder.url");
        assert_eq!(pairs.last().unwrap().0, "embedder.min_similarity");
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let text = render("# h", &owned);
        assert_eq!(embedder(&text), e);
    }

    #[test]
    fn default_query_prefix_cases() {
        assert_eq!(default_query_prefix("Qwen3-Embedding-0.6B"), QWEN_PREFIX);
        assert_eq!(default_query_prefix("qwen3-embedding-0.6b"), QWEN_PREFIX);
        assert_eq!(default_query_prefix("text-embedding-3-small"), "");
    }

    #[test]
    fn is_local_cases() {
        for yes in [
            "http://localhost:11434",
            "http://127.0.0.1:1",
            "http://[::1]:1",
            "HTTP://LocalHost",
        ] {
            assert!(is_local(yes), "{yes}");
        }
        for no in [
            "http://embedder.example:8081",
            "https://api.openai.com",
            "http://localhost.evil.com",
            "http://0.0.0.0:8081",
        ] {
            assert!(!is_local(no), "{no}");
        }
    }

    #[test]
    fn path_rules() {
        let explicit = Env {
            bilbo_config: os("/etc/b"),
            home: os("/home/a"),
            ..env()
        };
        assert_eq!(path(&explicit), Ok(Some((PathBuf::from("/etc/b"), true))));
        let xdg = Env {
            xdg_config_home: os("/x"),
            home: os("/home/a"),
            ..env()
        };
        assert_eq!(
            path(&xdg),
            Ok(Some((PathBuf::from("/x/bilbo/config"), false)))
        );
        let home = Env {
            home: os("/home/a"),
            ..env()
        };
        assert_eq!(
            path(&home),
            Ok(Some((PathBuf::from("/home/a/.config/bilbo/config"), false)))
        );
        assert_eq!(path(&env()), Ok(None));
        let relative = Env {
            bilbo_config: os("conf"),
            ..env()
        };
        assert!(path(&relative).is_err());
    }

    fn scopes(text: &str) -> Result<Settings, String> {
        parse(Path::new("/c"), text, Some(Path::new("/home/a")))
    }

    fn scope_err(text: &str) -> String {
        scopes(text).unwrap_err()
    }

    #[test]
    fn a_full_declaration() {
        let text = "scope.personal.sync = off\nscope.work.embedder = local\nscope.work.paths = ~/Developer/acme\nscope.work.marks = acme, ~/Developer/acme\nscope.default = personal\n";
        let s = scopes(text).unwrap();
        assert_eq!(s.scope_names(), ["personal", "work"]);
        assert_eq!(s.default_scope.as_deref(), Some("personal"));
        let work = s.scope("work").unwrap();
        assert_eq!(work.embedder, Rule::Local);
        assert_eq!(work.sync, "off");
        assert_eq!(work.paths, ["~/Developer/acme"]);
        assert_eq!(
            work.marks,
            [
                (Mark::Word("acme".to_string()), "acme".to_string()),
                (
                    Mark::Path(vec![
                        "/home/a/Developer/acme".to_string(),
                        "~/Developer/acme".to_string()
                    ]),
                    "~/Developer/acme".to_string()
                )
            ]
        );
        assert_eq!(s.scope("personal").unwrap().embedder, Rule::Any);
        assert_eq!(s.scope_lines.len(), 5);
    }

    #[test]
    fn one_key_declares_a_scope() {
        let s = scopes("scope.work.marks = acme\n").unwrap();
        let work = s.scope("work").unwrap();
        assert_eq!((work.sync.as_str(), work.embedder), ("off", Rule::Any));
        assert!(work.paths.is_empty());
        assert_eq!(s.default_scope, None);
    }

    #[test]
    fn scopes_need_no_embedder() {
        assert!(
            scopes("scope.work.embedder = local\n")
                .unwrap()
                .embedder
                .is_none()
        );
    }

    #[test]
    fn scope_lines_keep_file_order_unquoted() {
        let text = "scope.work.paths = \"~/a\"\ndigest.log = on\nscope.default = work\nscope.work.marks = acme\n";
        let s = scopes(text).unwrap();
        assert_eq!(
            s.scope_lines,
            [
                ("scope.work.paths".to_string(), "~/a".to_string()),
                ("scope.default".to_string(), "work".to_string()),
                ("scope.work.marks".to_string(), "acme".to_string())
            ]
        );
        assert_eq!(s.kept_lines, [("digest.log", "on".to_string())]);
    }

    #[test]
    fn rendered_scope_lines_parse_back_to_the_same_settings() {
        let text = "embedder.model = a\nembedder.url = http://x:1\ndigest.log = on\nscope.work.paths = ~/a, /srv/b\nscope.default = work\nscope.work.marks = \"  acme, ~/Developer/acme\"\nscope.home.sync = off\n";
        let old = scopes(text).unwrap();
        let mut lines: Vec<(String, String)> = old
            .kept_lines
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        lines.extend(old.scope_lines.iter().cloned());
        let rendered = render("# h", &lines);
        assert!(
            rendered.contains("scope.work.marks = \"  acme, ~/Developer/acme\"\n"),
            "{rendered}"
        );
        let new = scopes(&rendered).unwrap();
        assert_eq!(new.scope_lines, old.scope_lines);
        assert_eq!(new.kept_lines, old.kept_lines);
        assert_eq!(new.scope_names(), old.scope_names());
        assert_eq!(new.default_scope, old.default_scope);
        let order: Vec<&str> = rendered
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| l.split(" =").next().unwrap())
            .collect();
        assert_eq!(
            order,
            [
                "digest.log",
                "scope.work.paths",
                "scope.default",
                "scope.work.marks",
                "scope.home.sync"
            ]
        );
    }

    fn sync_of(value: &str) -> Result<String, String> {
        let s = scopes(&format!("scope.personal.sync = {value}\n"))?;
        Ok(s.scope("personal").unwrap().sync.clone())
    }

    #[test]
    fn sync_takes_a_url() {
        for ok in [
            "off",
            "https://relay.example.net",
            "https://relay.example.net:8443/bilbo",
            "http://127.0.0.1:8740",
            "http://localhost",
            "http://LOCALHOST:8740",
            "http://[::1]:8740",
            "https://[::1]",
            "https://relay.example.net/",
            "file:///Users/a/Library/Mobile Documents/bilbo",
            "file:///a%20b",
        ] {
            assert_eq!(sync_of(ok).as_deref(), Ok(ok), "{ok}");
        }
    }

    #[test]
    fn sync_refuses_what_is_not_a_url() {
        for bad in [
            "http://relay.example.net",
            "file://Sync/bilbo",
            "~/Sync/bilbo",
            "ftp://relay.example.net",
            "https://relay.example.net/?token=1",
            "https://relay.example.net/a#b",
            "https://relay.example.net:0",
            "https://relay.example.net:65536",
            "https://relay.example.net:",
            "https://relay.example.net:80a",
            "https://relay .example.net",
            "https://",
            "file:///a?b",
            "https://[]",
            "https://[::1]x",
            "https://[::1",
            "https://::1",
            "http://::1",
            "http://localhost.evil.com",
            "HTTPS://relay.example.net",
            "FILE:///x",
            "https://:8443",
        ] {
            let err = sync_of(bad).unwrap_err();
            assert!(
                err.starts_with("/c:1: scope.personal.sync "),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn sync_credentials_are_not_echoed() {
        for bad in [
            "https://u:sekrit@relay.example.net",
            "https://u:sekrit@relay.example.net/?token=1",
            "https://u:sekrit@relay.example.net/a#b",
            "\"https://u:sekrit@relay.example.net\\n\"",
        ] {
            assert_eq!(
                sync_of(bad).unwrap_err(),
                "/c:1: scope.personal.sync must not hold a user name or password",
                "{bad}"
            );
        }
    }

    #[test]
    fn scope_value_errors_name_the_key() {
        assert_eq!(
            scope_err("scope.personal.sync = on\n"),
            "/c:1: scope.personal.sync must be off or a sync URL, got 'on'"
        );
        assert_eq!(
            scope_err("\nscope.work.embedder = remote\n"),
            "/c:2: scope.work.embedder must be any or local, got 'remote'"
        );
        assert!(scope_err("scope.work.sync =\n").contains("scope.work.sync needs a value"));
    }

    #[test]
    fn bad_scope_names_and_sub_keys() {
        assert!(scope_err("scope.Work.sync = off\n").starts_with("/c:1: scope.Work.sync: "));
        assert!(scope_err("scope.a--b.sync = off\n").contains("scope.a--b.sync"));
        assert_eq!(
            scope_err("scope.default.sync = off\n"),
            "/c:1: scope.default.sync: 'default' is not a scope name; use scope.default"
        );
        for key in ["scope.work.colour", "scope.work", "scope.sync", "scope."] {
            let message = scope_err(&format!("{key} = red\n"));
            assert!(
                message.starts_with(&format!("/c:1: unknown key '{key}'; keys: embedder.url")),
                "{message}"
            );
            assert!(
                message.ends_with("scope.<name>.marks, scope.default"),
                "{message}"
            );
        }
    }

    #[test]
    fn a_default_that_is_not_declared() {
        assert_eq!(
            scope_err("\nscope.default = acme\n"),
            "/c:2: scope.default names 'acme', which no scope.acme.* key declares"
        );
        assert!(scopes("scope.default = acme\nscope.acme.sync = off\n").is_ok());
    }

    #[test]
    fn scope_keys_repeat_like_any_key() {
        assert_eq!(
            scope_err("scope.work.sync = off\nscope.work.sync = off\n"),
            "/c:2: scope.work.sync is set twice; first on line 1"
        );
    }

    #[test]
    fn path_and_mark_lists() {
        let s = scopes("scope.work.paths = ~/Developer/acme, /srv/acme/\n").unwrap();
        assert_eq!(
            s.scope("work").unwrap().paths,
            ["~/Developer/acme", "/srv/acme/"]
        );
        assert_eq!(
            scope_err("scope.work.paths = /a,,/b\n"),
            "/c:1: scope.work.paths has an empty item"
        );
        assert_eq!(
            scope_err("scope.work.marks = acme,,beta\n"),
            "/c:1: scope.work.marks has an empty item"
        );
        assert_eq!(
            scope_err("scope.work.paths = Developer/acme\n"),
            "/c:1: scope.work.paths item 'Developer/acme' must be an absolute path or start with ~/"
        );
        assert!(scopes("scope.a.paths = ~/\nscope.b.paths = /\n").is_ok());
        let message = parse(Path::new("/c"), "scope.a.paths = ~/x\n", None).unwrap_err();
        assert!(
            message.contains("HOME is not an absolute path"),
            "{message}"
        );
    }

    #[test]
    fn a_mark_is_one_word_or_a_path() {
        for item in [
            "acme corp",
            "acme.",
            "Acme's",
            "a",
            "ac-me",
            "~acme",
            "acme_corp",
        ] {
            let message = scope_err(&format!("scope.work.marks = {item}\n"));
            assert!(
                message.starts_with(&format!(
                    "/c:1: scope.work.marks item '{item}' must be one word"
                )),
                "{message}"
            );
        }
        let s = scopes("scope.work.marks = Ação, acme2, /srv/acme/\n").unwrap();
        assert_eq!(
            s.scope("work").unwrap().marks,
            [
                (Mark::Word("acao".to_string()), "Ação".to_string()),
                (Mark::Word("acme2".to_string()), "acme2".to_string()),
                (
                    Mark::Path(vec!["/srv/acme".to_string()]),
                    "/srv/acme/".to_string()
                )
            ]
        );
        assert_eq!(
            scope_err("scope.work.marks = /srv/acme, acme\nscope.x.marks = Acme\n"),
            "/c:2: scope.x.marks holds 'Acme', which scope.work.marks holds too (line 1)"
        );
        assert!(scopes("scope.work.marks = acme, acme\n").is_ok());
        let message = scope_err("scope.a.marks = ~/x\nscope.b.marks = /home/a/x\n");
        assert!(
            message.starts_with("/c:2: scope.b.marks holds '/home/a/x'"),
            "{message}"
        );
    }

    #[test]
    fn path_marks_come_with_their_forms() {
        let forms = |item: &str| {
            let s = scopes(&format!("scope.w.marks = {item}\n")).unwrap();
            match s.scope("w").unwrap().marks[0].0.clone() {
                Mark::Path(forms) => forms,
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(
            forms("~/Developer/acme"),
            ["/home/a/Developer/acme", "~/Developer/acme"]
        );
        assert_eq!(
            forms("/home/a/Developer/acme/"),
            ["/home/a/Developer/acme", "~/Developer/acme"]
        );
        assert_eq!(forms("~/"), ["/home/a", "~"]);
        assert_eq!(forms("~//x"), ["/home/a/x", "~/x"]);
        assert_eq!(forms("/srv/acme"), ["/srv/acme"]);
        assert_eq!(forms("/home/ab/x"), ["/home/ab/x"]);
        assert_eq!(forms("/"), ["/"]);
    }

    #[test]
    fn one_path_in_two_scopes() {
        assert_eq!(
            scope_err(
                "scope.work.paths = ~/Developer/acme\nscope.personal.paths = ~/Developer/acme/\n"
            ),
            "/c:2: scope.personal.paths names the same folder as scope.work.paths (line 1)"
        );
        assert!(scopes("scope.work.paths = /a, /a/\n").is_ok());
    }

    #[test]
    fn one_folder_under_two_spellings() {
        let dir = scratch("spellings");
        let home = dir.0.join("home");
        std::fs::create_dir_all(home.join("Developer/acme")).unwrap();
        std::os::unix::fs::symlink(home.join("Developer/acme"), home.join("src")).unwrap();
        let text = format!(
            "scope.work.paths = ~/Developer/acme\nscope.personal.paths = {}\n",
            home.join("src").display()
        );
        let message = parse(Path::new("/c"), &text, Some(&home)).unwrap_err();
        assert_eq!(
            message,
            "/c:2: scope.personal.paths names the same folder as scope.work.paths (line 1)"
        );
    }

    fn tree(name: &str, text: &str) -> (Scratch, Settings) {
        let dir = scratch(name);
        for folder in ["acme/api", "acme-tools", "other"] {
            std::fs::create_dir_all(dir.0.join("home").join(folder)).unwrap();
        }
        let home = dir.0.join("home");
        let settings = parse(Path::new("/c"), text, Some(&home)).unwrap();
        (dir, settings)
    }

    #[test]
    fn the_longest_path_wins_by_whole_folder_names() {
        let text = "scope.personal.paths = ~/\nscope.work.paths = ~/acme, /nonexistent-bilbo-x/\n";
        let (dir, s) = tree("longest", text);
        let at = |p: &str| s.scope_for(&dir.0.join("home").join(p)).map(str::to_string);
        assert_eq!(at("acme/api").as_deref(), Some("work"));
        assert_eq!(at("acme").as_deref(), Some("work"));
        assert_eq!(at("acme-tools").as_deref(), Some("personal"));
        assert_eq!(at("other").as_deref(), Some("personal"));
        assert_eq!(s.scope_for(&dir.0), None);
        assert_eq!(s.scope_for(&dir.0.join("gone")), None);
    }

    #[test]
    fn a_root_entry_and_a_trailing_slash_match() {
        let (dir, s) = tree(
            "root-slash",
            "scope.all.paths = /\nscope.work.paths = ~/acme/\n",
        );
        let home = dir.0.join("home");
        assert_eq!(s.scope_for(&home.join("acme/api")), Some("work"));
        assert_eq!(s.scope_for(&home.join("other")), Some("all"));
        assert_eq!(s.scope_for(&dir.0), Some("all"));
    }

    #[test]
    fn a_double_slash_after_the_tilde_is_one() {
        let s = scopes("scope.a.paths = ~//etc\n").unwrap();
        assert_eq!(s.scope("a").unwrap().paths, ["~//etc"]);
        assert!(scopes("scope.a.paths = ~//etc\nscope.b.paths = /home/a/etc\n").is_err());
    }

    #[test]
    fn a_working_directory_through_a_link_matches() {
        let (dir, s) = tree("through-link", "scope.work.paths = ~/acme\n");
        let link = dir.0.join("link");
        std::os::unix::fs::symlink(dir.0.join("home/acme/api"), &link).unwrap();
        assert_eq!(s.scope_for(&link), Some("work"));
    }

    #[test]
    fn a_tie_through_a_link_made_after_load_matches_nothing() {
        let text = "scope.x.paths = ~/acme\nscope.y.paths = ~/later\n";
        let (dir, s) = tree("late-link", text);
        let acme = dir.0.join("home/acme");
        assert_eq!(s.scope_for(&acme), Some("x"));
        std::os::unix::fs::symlink(&acme, dir.0.join("home/later")).unwrap();
        assert_eq!(s.scope_for(&acme), None);
        assert_eq!(s.scope_for(&acme.join("api")), None);
    }

    #[test]
    fn the_home_folder_as_a_path() {
        let text = "scope.personal.paths = ~/\nscope.work.paths = ~/acme\n";
        let (dir, s) = tree("home-path", text);
        let home = dir.0.join("home");
        assert_eq!(s.scope_for(&home.join("acme/api")), Some("work"));
        assert_eq!(s.scope_for(&home.join("other")), Some("personal"));
    }

    #[test]
    fn the_rule_of_a_scope_value() {
        let s = scopes("scope.work.embedder = local\nscope.personal.sync = off\n").unwrap();
        assert_eq!(s.rule(Some("work")), Rule::Local);
        assert_eq!(s.rule(Some("personal")), Rule::Any);
        assert_eq!(s.rule(None), Rule::Local);
        assert_eq!(s.rule(Some("acme")), Rule::Local);
        let s = scopes("scope.personal.sync = off\n").unwrap();
        assert_eq!(s.rule(None), Rule::Any);
        let s = scopes("").unwrap();
        assert_eq!(s.rule(None), Rule::Any);
        assert_eq!(s.rule(Some("work")), Rule::Any);
        assert_eq!(Rule::Local.as_str(), "local");
    }
}
