use crate::store::{self, Env};
use std::path::{Path, PathBuf};

pub const DEFAULT_MIN_SIMILARITY: f64 = 0.5;

const KEYS: [&str; 6] = [
    "embedder.url",
    "embedder.model",
    "embedder.token_file",
    "embedder.token_env",
    "embedder.query_prefix",
    "embedder.min_similarity",
];

#[derive(Debug)]
pub struct Settings {
    /// The file the settings came from, or would come from; `None` when no path can be formed.
    pub path: Option<PathBuf>,
    /// `None` without `embedder.url`: bilbo is keyword-only.
    pub embedder: Option<Embedder>,
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

/// The settings, or the message for a config error, without the `bilbo: ` prefix.
pub fn load(env: &Env) -> Result<Settings, String> {
    let (path, explicit) = if let Some(value) = env.bilbo_config.as_ref().filter(|v| !v.is_empty())
    {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(format!(
                "BILBO_CONFIG must be an absolute path, got '{}'",
                path.display()
            ));
        }
        (path, true)
    } else if let Some(xdg) = store::absolute(&env.xdg_config_home) {
        (xdg.join("bilbo/config"), false)
    } else if let Some(home) = store::absolute(&env.home) {
        (home.join(".config/bilbo/config"), false)
    } else {
        return Ok(Settings {
            path: None,
            embedder: None,
        });
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
            return Ok(Settings {
                path: Some(path),
                embedder: None,
            });
        }
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let text =
        String::from_utf8(bytes).map_err(|_| format!("{}: not valid UTF-8", path.display()))?;
    let home = store::absolute(&env.home);
    let embedder = parse(&path, &text, home.as_deref())?;
    Ok(Settings {
        path: Some(path),
        embedder,
    })
}

/// The embedder of the file `path` holding `text`; `home` expands `~/`.
fn parse(path: &Path, text: &str, home: Option<&Path>) -> Result<Option<Embedder>, String> {
    let at = path.display();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut seen: Vec<(&str, usize)> = Vec::new();
    let mut url = None;
    let mut model = None;
    let mut token_file = None;
    let mut token_env = None;
    let mut query_prefix = String::new();
    let mut min_similarity = DEFAULT_MIN_SIMILARITY;
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
        let Some(key) = KEYS.iter().find(|k| **k == key).copied() else {
            return Err(format!(
                "{at}:{n}: unknown key '{key}'; keys: {}",
                KEYS.join(", ")
            ));
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
            _ => query_prefix = value,
        }
    }
    if token_file.is_some() && token_env.is_some() {
        return Err(format!(
            "{at}: set embedder.token_file or embedder.token_env, not both"
        ));
    }
    let Some(url) = url else {
        return Ok(None);
    };
    let Some(model) = model else {
        return Err(format!(
            "{at}: embedder.url is set but embedder.model is not"
        ));
    };
    let token = token_file
        .map(Token::File)
        .or_else(|| token_env.map(Token::Var));
    Ok(Some(Embedder {
        url,
        model,
        token,
        query_prefix,
        min_similarity,
    }))
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

fn check_url(url: &str) -> Result<(), UrlError> {
    let authority = match url.split_once("://") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next().unwrap_or(""),
        None => url,
    };
    if authority.contains('@') {
        return Err(UrlError::Credentials);
    }
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or(UrlError::Shape)?;
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(UrlError::Shape);
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = match authority.rfind(':') {
        Some(at) => &authority[..at],
        None => authority,
    };
    if host.is_empty() {
        return Err(UrlError::Shape);
    }
    Ok(())
}

fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
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

    const BASE: &str = "embedder.url = http://bagend:8081\nembedder.model = m\n";

    fn parsed(text: &str) -> Result<Option<Embedder>, String> {
        parse(Path::new("/c"), text, Some(Path::new("/home/a")))
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
        assert_eq!(settings.embedder.unwrap().url, "http://bagend:8081");
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
            "# the home server\n\nembedder.url = http://bagend:8081\nembedder.model = qwen3\n",
        );
        assert_eq!(e.url, "http://bagend:8081");
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
            "/c:3: unknown key 'embeder.url'; keys: embedder.url, embedder.model, embedder.token_file, embedder.token_env, embedder.query_prefix, embedder.min_similarity"
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
}
