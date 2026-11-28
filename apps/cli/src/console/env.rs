use std::ffi::OsString;
use std::io::{self, Read};
use std::path::PathBuf;
use std::rc::Rc;

use crate::console::message::Message;

pub type Var = Rc<dyn Fn(&str) -> Option<OsString>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    Person,
    Nobody,
}

pub struct Env {
    pub var: Var,
    pub stdin: Box<dyn Read>,
    pub who: Who,
    pub program: Option<PathBuf>,
    /// Written to stderr at once, not after the run.
    pub notice: Box<dyn FnMut(&Message)>,
}

impl Env {
    pub fn quiet() -> Env {
        Env {
            var: Rc::new(|_| None),
            stdin: Box::new(io::empty()),
            who: Who::Nobody,
            program: None,
            notice: Box::new(|_| {}),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::{Env, Who};
    use crate::console::message::Message;

    #[test]
    fn a_quiet_env_has_no_variables_no_stdin_and_drops_notices() {
        let mut env = Env::quiet();
        assert_eq!((env.var)("HOME"), None);
        assert_eq!(env.who, Who::Nobody);
        let mut text = String::new();
        env.stdin.read_to_string(&mut text).expect("empty stdin");
        assert_eq!(text, "");
        (env.notice)(&Message::error("dropped"));
    }

    #[test]
    fn a_quiet_env_names_no_program() {
        assert_eq!(Env::quiet().program, None);
    }
}
