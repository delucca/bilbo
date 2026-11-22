use bilbo_settings::error::Error;
use bilbo_settings::key::Value;

use crate::writer::Variables;

pub const PROGRAM_ALLOWED: &str = "a program's name or path, not empty";
pub const EFFORT_ALLOWED: &str = "a word of lower-case letters";
pub const ARGUMENTS_ALLOWED: &str = "a list of texts";
pub const INHERIT_ALLOWED: &str = "a list of variables' names: letters, digits and _";
pub const ENVIRONMENT_ALLOWED: &str = "a table of texts whose names are letters, digits and _";

fn variable_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub fn arguments(value: &Value<'_>) -> Result<Vec<String>, Error> {
    value
        .items()
        .ok_or_else(|| value.refuse())?
        .iter()
        .map(|item| item.text().map(str::to_owned).ok_or_else(|| item.refuse()))
        .collect()
}

pub fn program(value: &Value<'_>) -> Result<String, Error> {
    value
        .text()
        .filter(|program| !program.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| value.refuse())
}

pub fn effort(value: &Value<'_>) -> Result<String, Error> {
    value
        .text()
        .filter(|word| !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase()))
        .map(str::to_owned)
        .ok_or_else(|| value.refuse())
}

pub fn inherit(value: &Value<'_>) -> Result<Vec<String>, Error> {
    let items = value
        .items()
        .ok_or_else(|| value.refused("must be a list of variables' names"))?;
    items
        .iter()
        .map(|item| {
            item.text()
                .filter(|name| variable_name(name))
                .map(str::to_owned)
                .ok_or_else(|| {
                    value.refused("must hold variables' names alone: letters, digits and _")
                })
        })
        .collect()
}

pub fn environment(value: &Value<'_>) -> Result<Vec<(String, String)>, Error> {
    let fields = value
        .fields()
        .ok_or_else(|| value.refused("must be a table of texts"))?;
    fields
        .into_iter()
        .map(|(name, item)| match item.text() {
            Some(_) if !variable_name(name) => {
                Err(value.refused("must name its variables with letters, digits and _"))
            }
            Some(text) => Ok((name.to_owned(), text.to_owned())),
            None => Err(value.refused("must hold texts alone")),
        })
        .collect()
}

pub fn inherited(
    inherit: &[String],
    environment: &[(String, String)],
    variables: &Variables,
) -> Vec<(String, String)> {
    let mut inherited: Vec<(String, String)> = Vec::new();
    for name in inherit {
        if let Some(value) = variables(name) {
            inherited.push((name.clone(), value));
        }
    }
    for (name, value) in environment {
        inherited.retain(|(kept, _)| kept != name);
        inherited.push((name.clone(), value.clone()));
    }
    inherited
}
