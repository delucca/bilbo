use std::ffi::OsString;
use std::io::{self, Read};

use crate::console::message::Message;

type Var = Box<dyn Fn(&str) -> Option<OsString>>;

pub struct Env {
    pub var: Var,
    pub stdin: Box<dyn Read>,
    /// Written to stderr at once, not after the run.
    pub notice: Box<dyn FnMut(&Message)>,
}

impl Env {
    pub fn quiet() -> Env {
        Env {
            var: Box::new(|_| None),
            stdin: Box::new(io::empty()),
            notice: Box::new(|_| {}),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::Env;
    use crate::console::message::Message;

    #[test]
    fn a_quiet_env_has_no_variables_no_stdin_and_drops_notices() {
        let mut env = Env::quiet();
        assert_eq!((env.var)("HOME"), None);
        let mut text = String::new();
        env.stdin.read_to_string(&mut text).expect("empty stdin");
        assert_eq!(text, "");
        (env.notice)(&Message::error("dropped"));
    }
}
