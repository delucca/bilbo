use std::fmt;
use std::fs;
use std::path::Path;

use bilbo_settings::duration;

use crate::child::{self, Bounds, End, Environment};

pub struct Secret(String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn hides(&self, texts: &[&str]) -> bool {
        texts.iter().any(|text| text.contains(&self.0))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Source<'a> {
    Variable(&'a str),
    Program(&'a [String]),
}

impl<'a> Source<'a> {
    pub fn choose(variable: &'a str, command: &'a [String]) -> Source<'a> {
        if command.is_empty() {
            Source::Variable(variable)
        } else {
            Source::Program(command)
        }
    }
}

fn bare(text: &str) -> Option<&str> {
    let key = text.trim_ascii();
    (!key.is_empty()).then_some(key)
}

pub fn read(
    source: Source<'_>,
    key: &str,
    variables: &dyn Fn(&str) -> Option<String>,
    folder: &Path,
    bounds: Bounds,
) -> Result<Secret, String> {
    match source {
        Source::Variable(name) => {
            let value = variables(name);
            match value.as_deref().and_then(bare) {
                None => Err(format!("{name} is not set ({key})")),
                Some(found) if found.chars().any(char::is_control) => {
                    Err(format!("{name} holds a control character ({key})"))
                }
                Some(found) => Ok(Secret(found.to_owned())),
            }
        }
        Source::Program(command) => {
            let (program, args) = command
                .split_first()
                .ok_or_else(|| format!("{key} printed no key"))?;
            fs::create_dir_all(folder)
                .map_err(|error| format!("{key} could not be started: {error}"))?;
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let run = child::run(
                Path::new(program),
                &args,
                Environment::Inherit,
                folder,
                b"",
                bounds,
            )
            .map_err(|error| format!("{key} could not be started: {error}"))?;
            match run.end {
                End::NotStarted(error) => Err(format!("{key} could not be started: {error}")),
                End::Exited(0) => {
                    let text = String::from_utf8_lossy(&run.stdout);
                    match text.lines().next().and_then(bare) {
                        None => Err(format!("{key} printed no key")),
                        Some(line) if line.chars().any(char::is_control) => {
                            Err(format!("{key} printed a key with a control character"))
                        }
                        Some(line) => Ok(Secret(line.to_owned())),
                    }
                }
                End::Exited(code) => Err(format!("{key} failed: exit {code}")),
                End::NoExitCode => Err(format!("{key} ended with no exit code")),
                End::TimedOut => Err(format!(
                    "{key} did not end in {}",
                    duration::show(bounds.timeout)
                )),
                End::OverLimit => Err(format!("{key} printed more than derive.response allows")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn a_secret_prints_as_stars_in_every_form() {
        let secret = Secret("sk-canary-123".to_owned());
        assert_eq!(format!("{secret:?}"), "***");
        assert_eq!(format!("{secret}"), "***");
        assert_eq!(format!("{secret:#?}"), "***");
    }

    #[test]
    fn a_text_that_holds_the_key_is_found() {
        let secret = Secret("sk-canary-123".to_owned());
        assert!(secret.hides(&["clean", "see sk-canary-123 here"]));
        assert!(secret.hides(&["sk-canary-123 first"]));
        assert!(!secret.hides(&["clean", "sk-canary-12", ""]));
        assert!(!secret.hides(&[]));
    }
}
