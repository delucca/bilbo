use crate::console::env::Env;
use crate::console::outcome::{Failure, Outcome};

const SUMMARY: &str = "Keep what every follow gained or changed; a timer runs it";
const ABOUT: &str = "bilbo refresh: keep what every follow gained or changed since the last run.";
const USAGE: &str = "bilbo refresh";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE, after_help = PAGE)]
pub struct Args {}

const PAGE: &str = "\
With no follow declared it keeps ~/.claude/projects and ~/.codex/sessions. When
another bilbo is writing the store, it asks that one for the pass and exits 0.

Output:
  new <n>, appended <n>, new generation <n>, unchanged <n>, refused <n>
  records <n>, raw bytes <n>, stored bytes <n>

Exit:
  0  success, also when it asked another bilbo for the pass
  1  a refused file, a follow that cannot run, a store or index failure
  2  usage or config error

Examples:
  bilbo refresh

Docs: https://github.com/delucca/bilbo/wiki/Commands#refresh";

pub type Pass<'a> = dyn Fn(&mut Env) -> Result<Outcome, Failure> + 'a;

pub fn run(_args: &Args, env: &mut Env, pass: &Pass<'_>) -> Result<Outcome, Failure> {
    pass(env)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::run;
    use crate::console::env::Env;
    use crate::console::outcome::{Exit, Failure, Outcome};
    use crate::console::view::View;

    fn usage_of(list: &[&str]) -> (String, Vec<String>, String) {
        let args: Vec<OsString> = list.iter().map(OsString::from).collect();
        let settings = Err(Failure::Config("no setting is read".to_owned()));
        match crate::run(&args, &mut Env::quiet(), settings) {
            Err(Failure::Usage {
                reason,
                usage,
                page,
            }) => (reason, usage, page),
            other => panic!("{list:?}: {other:?}"),
        }
    }

    #[test]
    fn refresh_takes_no_argument() {
        for (list, reason) in [
            (&["bilbo", "refresh", "x"][..], "unexpected argument 'x'"),
            (&["bilbo", "refresh", "--all"], "unknown option '--all'"),
        ] {
            let (said, usage, page) = usage_of(list);
            assert_eq!(said, reason, "{list:?}");
            assert_eq!(usage, ["usage: bilbo refresh"], "{list:?}");
            assert_eq!(page, "bilbo refresh --help", "{list:?}");
        }
    }

    #[test]
    fn refresh_gives_back_what_the_pass_gives() {
        let args = super::Args {};
        let given = run(&args, &mut Env::quiet(), &|_| {
            Ok(Outcome::new("new 4\n").with_exit(Exit::Refused))
        })
        .expect("the pass's outcome");
        assert_eq!(given.stdout(&View::PLAIN), "new 4\n");
        assert_eq!(given.exit_code(), 1);

        let failed = run(&args, &mut Env::quiet(), &|_| {
            Err(Failure::Refused("the store is damaged".to_owned()))
        });
        assert!(
            matches!(&failed, Err(Failure::Refused(text)) if text == "the store is damaged"),
            "{failed:?}"
        );
    }
}
