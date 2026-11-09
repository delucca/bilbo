pub const DESCRIPTION: &str = "\
bilbo keeps durable memory for coding agents: it remembers their sessions and
the pages they cite, and recalls them when a later session asks.";

pub const USAGE: &str = "bilbo <verb> [<args>]";

pub const TEMPLATE: &str =
    "{about-with-newline}\n{usage-heading} {usage}\n\nVerbs:\n{subcommands}{after-help}";

pub const AFTER: &str = "\
Run 'bilbo <verb> --help' for its options, output and exit codes.
Docs: https://github.com/delucca/bilbo/wiki";

#[cfg(test)]
mod tests {
    use clap::Command;

    use super::{AFTER, TEMPLATE, USAGE};
    use crate::bilbo;
    use crate::console::args::verbs;
    use crate::help::page::overview;
    use crate::probe::{self, with_verb};

    fn named_verbs(overview: &str) -> Vec<String> {
        overview
            .lines()
            .skip_while(|line| !line.starts_with("Usage: "))
            .take_while(|line| !line.starts_with("Run '"))
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
        assert!(at("\n\nUsage: bilbo <verb> [<args>]\n\n") < at("\nVerbs:\n  "));
        assert!(at("\nVerbs:\n  ") < at("\n\nRun 'bilbo <verb> --help' for "));
        assert!(at("\n\nRun 'bilbo <verb> --help' for ") < at("\nDocs: https://"));
        assert!(
            text.lines()
                .last()
                .is_some_and(|line| line.starts_with("Docs: "))
        );
        assert!(!text.contains("Options:"));
    }

    #[test]
    fn the_overview_names_exactly_the_registered_verbs() {
        let root = bilbo();
        assert_eq!(named_verbs(&overview(&root)), verbs(&root));
    }

    #[test]
    fn the_drift_guard_sees_a_verb_the_overview_leaves_out_or_invents() {
        let named = |verb: Command, after: &'static str| {
            let bare = Command::new("bilbo")
                .help_template(TEMPLATE)
                .override_usage(USAGE)
                .disable_help_subcommand(true)
                .after_help(after);
            let root = with_verb(bare, verb);
            (named_verbs(&overview(&root)), verbs(&root))
        };
        let (rows, registered) = named(probe::verb(), AFTER);
        assert_eq!(rows, registered);
        let (rows, registered) = named(probe::verb().hide(true), AFTER);
        assert_ne!(rows, registered);
        let (rows, registered) = named(probe::verb(), "  ghost  b\n\nRun 'x'");
        assert_ne!(rows, registered);
    }
}
