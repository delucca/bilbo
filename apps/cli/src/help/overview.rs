pub const DESCRIPTION: &str = "\
bilbo keeps durable memory for coding agents: it ingests their sessions and
the pages they cite, and recalls them when a later session asks.";

pub const USAGE: &str = "bilbo <verb> [<args>]...";

/// clap's default template minus `{all-args}`: the verbs are listed in groups below and the
/// overview names no option. clap puts a blank line before `{after-help}` itself.
pub const TEMPLATE: &str = "{about-with-newline}\n{usage-heading} {usage}{after-help}";

/// The Paths block repeats `bilbo_store::paths`; nothing checks it.
pub const AFTER: &str = "\
No verbs yet.

Exit codes: 0 success; 1 refused, problems found or nothing matched; 2 usage
or config error.
Results go to stdout; diagnostics to stderr, prefixed 'bilbo: ' off a terminal.

Paths, the first that applies:
  root    $BILBO_HOME, $XDG_DATA_HOME/bilbo, ~/.local/share/bilbo
  config  $BILBO_CONFIG, $XDG_CONFIG_HOME/bilbo/config, ~/.config/bilbo/config
  cache   $XDG_CACHE_HOME/bilbo, ~/.cache/bilbo
  state   $XDG_STATE_HOME/bilbo, ~/.local/state/bilbo

Run 'bilbo <verb> --help' or 'bilbo help <verb>' for a verb's options, output
and exit codes.";

#[cfg(test)]
mod tests {
    use clap::Command;

    use super::{TEMPLATE, USAGE};
    use crate::bilbo;
    use crate::console::args::verbs;
    use crate::help::page::overview;
    use crate::probe::{self, with_verb};

    fn named_verbs(overview: &str) -> Vec<String> {
        overview
            .lines()
            .skip_while(|line| !line.starts_with("Usage: "))
            .take_while(|line| !line.starts_with("Exit codes:"))
            .filter_map(|line| line.strip_prefix("  "))
            .filter_map(|row| row.split_once("  ").map(|(name, _)| name))
            .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase()))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_overview_has_its_sections_in_order() {
        let text = overview(&bilbo());
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle:?} missing"))
        };
        assert!(text.starts_with("bilbo keeps durable memory for coding agents: "));
        assert!(at("\nUsage: bilbo <verb> [<args>]...\n") < at("\nExit codes: "));
        assert!(at("\nExit codes: ") < at("\nResults go to stdout; "));
        assert!(at("\nResults go to stdout; ") < at("\nPaths, the first that applies:\n"));
        assert!(at("\nPaths, the first that applies:\n") < at("\nRun 'bilbo <verb> --help' or "));
        assert!(text.ends_with("and exit codes.\n"));
        assert!(!text.contains("Options:"));
    }

    #[test]
    fn the_overview_names_exactly_the_registered_verbs() {
        let root = bilbo();
        assert_eq!(named_verbs(&overview(&root)), verbs(&root));
    }

    #[test]
    fn the_drift_guard_sees_a_verb_the_groups_leave_out_or_invent() {
        let named = |after: &'static str| {
            let bare = Command::new("bilbo")
                .help_template(TEMPLATE)
                .override_usage(USAGE)
                .disable_help_subcommand(true)
                .after_help(after);
            let root = with_verb(bare, probe::verb());
            (named_verbs(&overview(&root)), verbs(&root))
        };
        let (groups, registered) = named("\nProbing:\n  probe  Shows a verb page\n\nExit codes: x");
        assert_eq!(groups, registered);
        let (groups, registered) = named("\nExit codes: x");
        assert_ne!(groups, registered);
        let (groups, registered) = named("\n  probe  a\n  ghost  b\n\nExit codes: x");
        assert_ne!(groups, registered);
    }
}
