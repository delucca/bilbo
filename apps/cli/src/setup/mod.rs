pub mod notes;
pub mod view;

use std::cell::RefCell;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bilbo_settings::edit::read_regular;
use bilbo_settings::error::Error;
use bilbo_settings::file::File;
use bilbo_settings::folder;
use bilbo_settings::key::{Flag, Key, Origin, Resolved, Source};
use bilbo_setup::plugin::Agent;
use bilbo_setup::program::{self, Bounds};
use bilbo_setup::report::Status;
use bilbo_setup::steps::{self, Ports, Request, Taking, Timing, Tool, Way};
use bilbo_setup::timer::Scheduler;
use bilbo_store::entries::StoreError;
use bilbo_store::layout::Layout;
use bilbo_store::segments::Store;
use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::env::Env;
use crate::console::message::sanitize;
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::settings::{
    self, Group, KindKey, SETUP_AGY_ENABLED, SETUP_AGY_PROGRAM, SETUP_CLAUDE_ENABLED,
    SETUP_CLAUDE_PROGRAM, SETUP_CODEX_ENABLED, SETUP_CODEX_PROGRAM, SETUP_NOTES, SETUP_PLUGIN,
    SETUP_TIMER_LOGS, Settings, folder_variables,
};

const SUMMARY: &str = "Install the plugin and the timers on this machine";
const ABOUT: &str =
    "bilbo setup: install the plugin in Claude Code, Codex and AGY, and the timers.";
const USAGE: &str = "bilbo setup [--claude <program>] [--codex <program>]
              [--agy <program>] [--notes <folder>] [--plugin <folder>]
  bilbo setup --remove [--claude <program>] [--codex <program>]
              [--agy <program>]";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE, after_help = PAGE)]
pub struct Args {
    #[arg(
        long,
        value_name = "program",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Claude Code's program, over setup.claude.program"
    )]
    pub claude: Option<String>,
    #[arg(
        long,
        value_name = "program",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Codex's program, over setup.codex.program"
    )]
    pub codex: Option<String>,
    #[arg(
        long,
        value_name = "program",
        value_parser = NonEmptyStringValueParser::new(),
        help = "AGY's program, over setup.agy.program"
    )]
    pub agy: Option<String>,
    #[arg(
        long,
        value_name = "folder",
        value_parser = NonEmptyStringValueParser::new(),
        help = "The notes folder to make and follow, over setup.notes"
    )]
    pub notes: Option<String>,
    #[arg(
        long,
        value_name = "folder",
        value_parser = NonEmptyStringValueParser::new(),
        help = "The folder that holds the plugin, over setup.plugin"
    )]
    pub plugin: Option<String>,
    #[arg(long, help = "Take the plugin and the timers out; the notes stay")]
    pub remove: bool,
}

const PAGE: &str = "\
Output:
  <step> <status> <detail>   a line a step, its fields apart by tabs
  <step>    notes, claude, codex, codex-hooks, agy, refresh-timer, derive-timer
  <status>  installed, updated, kept, removed, absent, skipped or failed

Exit:
  0  no step failed
  1  a step failed; its line says why, and the same command tries it again
  2  usage or config error, or a store root that is bilbo 0.20's

Examples:
  bilbo setup --claude ~/.local/bin/claude

Docs: https://github.com/delucca/bilbo/wiki/Commands#setup";

pub struct Schedule<'a> {
    pub units: &'a dyn Fn() -> Result<PathBuf, Error>,
    pub program: &'static str,
    pub missing: Status,
}

pub type Agents = dyn Fn(&'static str, PathBuf, Option<&Path>, Bounds) -> Box<dyn Agent>;

pub struct Hand<'a> {
    pub kind: &'a str,
    pub save: &'a dyn Fn(&Path) -> notes::Answer,
    pub schedule: Option<Schedule<'a>>,
    pub kinds: &'a [&'static KindKey],
    pub providers: &'a [&'static Group],
    pub agent: &'a Agents,
    pub scheduler: &'a dyn Fn(PathBuf, PathBuf, Bounds) -> Box<dyn Scheduler>,
}

fn flagged<T>(
    resolved: &Resolved<T>,
    key: &Key<T>,
    source: &Source<'_>,
    name: &'static str,
    text: Option<&String>,
) -> Result<Resolved<T>, Failure>
where
    T: Clone,
{
    let flag = text
        .map(|text| Flag::parse(key, source, name, text))
        .transpose()?;
    Ok(resolved.clone().with_flag(flag))
}

fn inside(
    layout: &Layout,
    folder: &Path,
    sentence: impl Fn(&Path, &Path) -> String,
) -> Result<(), Failure> {
    if Store::is_inside_root(layout, folder).map_err(unlooked)? {
        return Err(Failure::Config(sanitize(&sentence(folder, &layout.root))));
    }
    Ok(())
}

fn unlooked(error: StoreError) -> Failure {
    match Failure::from(error) {
        Failure::Refused(text) | Failure::Output(text) => Failure::Config(text),
        other => other,
    }
}

fn canonical_or_given(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

fn clause(settings: &Settings) -> Option<String> {
    let said = if !settings.derive.enabled.value {
        format!("{} is false", bilbo_derive::keys::ENABLED.about.name)
    } else if settings.llm.provider.value.is_empty() {
        format!("{} is empty", bilbo_llm::keys::PROVIDER.about.name)
    } else if settings.derive.named.value.is_empty() {
        format!("{} is empty", bilbo_derive::keys::IN.about.name)
    } else {
        return None;
    };
    Some(format!("{said} in the config and the unit"))
}

type Programs<'a> = [&'a Resolved<PathBuf>; 3];

fn tools_of(
    path: &OsString,
    settings: &Settings,
    programs: Programs<'_>,
    hand: &Hand<'_>,
    codex_home: Option<&Path>,
    bounds: Bounds,
) -> [Tool; 3] {
    let [claude, codex, agy] = programs;
    [
        (
            "claude",
            None,
            settings.setup.claude_enabled.value,
            claude,
            (
                SETUP_CLAUDE_ENABLED.about.name,
                SETUP_CLAUDE_PROGRAM.about.name,
            ),
        ),
        (
            "codex",
            Some("codex-hooks"),
            settings.setup.codex_enabled.value,
            codex,
            (
                SETUP_CODEX_ENABLED.about.name,
                SETUP_CODEX_PROGRAM.about.name,
            ),
        ),
        (
            "agy",
            None,
            settings.setup.agy_enabled.value,
            agy,
            (SETUP_AGY_ENABLED.about.name, SETUP_AGY_PROGRAM.about.name),
        ),
    ]
    .map(|(step, hooks, enabled, program, (switch, program_key))| {
        let way = if enabled {
            let value = program.value.to_string_lossy().into_owned();
            match program::find(&program.value, path) {
                Some(found) => Way::Found {
                    folder: found.parent().map(Path::to_path_buf).unwrap_or_default(),
                    agent: (hand.agent)(step, found, codex_home, bounds),
                },
                None => Way::Missing {
                    value,
                    default: matches!(program.origin, Origin::Default),
                },
            }
        } else {
            Way::Off
        };
        Tool {
            step,
            switch,
            program_key,
            hooks,
            way,
        }
    })
}

fn taken_out(settings: &Settings, env: &Env, hand: &Hand<'_>, programs: Programs<'_>) -> Outcome {
    let path = (env.var)("PATH").unwrap_or_default();
    let bounds = Bounds {
        wait: settings.setup_timer.wait.value,
        poll: settings.setup_timer.poll.value,
    };
    let tools = tools_of(&path, settings, programs, hand, None, bounds);
    let scheduler = match &hand.schedule {
        None => Timing::Absent,
        Some(schedule) => match (schedule.units)() {
            Err(error) => Timing::Refused(sanitize(&error.to_string())),
            Ok(units) => match program::find(schedule.program, &path) {
                Some(found) => Timing::Has((hand.scheduler)(found, units, bounds)),
                None => Timing::Missing {
                    program: schedule.program,
                    status: schedule.missing,
                },
            },
        },
    };
    let report = steps::remove(&Taking { tools, scheduler });
    let failed = view::failed(&report, true);
    let rows = report.clone();
    let mut outcome = Outcome::new(view::rows(&report, &crate::console::view::View::PLAIN))
        .with_human(move |view| view::rows(&rows, view))
        .on_every_terminal();
    if let Some(message) = failed {
        outcome = outcome
            .with_diagnostics(vec![message])
            .with_exit(Exit::Refused);
    }
    outcome
}

pub fn removal_alone(root: &Command, args: &Args) -> Result<(), Failure> {
    if !args.remove {
        return Ok(());
    }
    for (given, flag, reason) in [
        (&args.notes, "--notes", "a removal leaves the notes folder"),
        (&args.plugin, "--plugin", "a removal reads no plugin folder"),
    ] {
        if given.is_some() {
            return Err(usage(
                root,
                format!("{flag} does not stand with --remove: {reason}"),
                Some("setup"),
            ));
        }
    }
    Ok(())
}

pub fn run(
    _root: &Command,
    args: &Args,
    env: &mut Env,
    settings: &Settings,
    hand: &Hand<'_>,
) -> Result<Outcome, Failure> {
    let none = File::default();
    let source = Source {
        var: &*env.var,
        file: &none,
    };
    let claude = flagged(
        &settings.setup.claude_program,
        &SETUP_CLAUDE_PROGRAM,
        &source,
        "--claude",
        args.claude.as_ref(),
    )?;
    let codex = flagged(
        &settings.setup.codex_program,
        &SETUP_CODEX_PROGRAM,
        &source,
        "--codex",
        args.codex.as_ref(),
    )?;
    let agy = flagged(
        &settings.setup.agy_program,
        &SETUP_AGY_PROGRAM,
        &source,
        "--agy",
        args.agy.as_ref(),
    )?;
    if args.remove {
        return Ok(taken_out(settings, env, hand, [&claude, &codex, &agy]));
    }
    let plugin = flagged(
        &settings.setup.plugin,
        &SETUP_PLUGIN,
        &source,
        "--plugin",
        args.plugin.as_ref(),
    )?;

    let notes = flagged(
        &settings.setup.notes,
        &SETUP_NOTES,
        &source,
        "--notes",
        args.notes.as_ref(),
    )?;

    let layout = settings.layout();
    Store::check(&layout).map_err(unlooked)?;
    let decision = notes::decide(
        &notes,
        &settings.follow.value,
        hand.kind,
        &settings.config.value,
    );
    if !matches!(decision.plan, notes::Plan::Skip | notes::Plan::Failed(_)) {
        let followed = decision.followed;
        inside(&layout, &decision.folder, |folder, root| {
            if followed {
                format!(
                    "the followed notes folder {} is inside the store root {}, which holds only segments; follow a notes folder outside the root",
                    folder.display(),
                    root.display()
                )
            } else {
                format!(
                    "{key} names {}, inside the store root {}, which holds only segments; set {key} to a folder outside the root",
                    folder.display(),
                    root.display(),
                    key = SETUP_NOTES.about.name
                )
            }
        })?;
    }
    if decision.plan == notes::Plan::Save
        && let Some(folder) = settings.config.value.parent()
    {
        inside(&layout, folder, |folder, root| {
            format!(
                "the config file's folder {} is inside the store root {}, which holds only segments; set {key} to a folder outside it",
                folder.display(),
                root.display(),
                key = settings::ROOT.about.name
            )
        })?;
    }
    let logs = settings
        .setup
        .timer_logs
        .value
        .clone()
        .unwrap_or_else(|| settings.state.value.join("timer"));
    if hand.schedule.is_some() {
        inside(&layout, &logs, |folder, root| {
            format!(
                "{key} names {}, inside the store root {}, which holds only segments; set {key} to a folder outside the root",
                folder.display(),
                root.display(),
                key = SETUP_TIMER_LOGS.about.name
            )
        })?;
    }
    let units = match &hand.schedule {
        Some(schedule) => {
            let units = (schedule.units)()?;
            inside(&layout, &units, |folder, root| {
                format!(
                    "the units' folder {} is inside the store root {}, which holds only segments; set {key} to a folder outside it",
                    folder.display(),
                    root.display(),
                    key = settings::ROOT.about.name
                )
            })?;
            Some(units)
        }
        None => None,
    };
    let codex_home = if settings.setup.codex_enabled.value {
        Some(folder::home(&*env.var)?)
    } else {
        None
    };

    let own = env
        .program
        .as_ref()
        .and_then(|program| fs::canonicalize(program).ok());
    let package = match &plugin.value {
        Some(folder) => Some(canonical_or_given(folder.clone())),
        None => own
            .as_deref()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .map(|prefix| canonical_or_given(prefix.join("share/bilbo"))),
    };

    let path = (env.var)("PATH").unwrap_or_default();
    let section = settings.setup_timer.clone();
    let bounds = Bounds {
        wait: section.wait.value,
        poll: section.poll.value,
    };
    let tools = tools_of(
        &path,
        settings,
        [&claude, &codex, &agy],
        hand,
        codex_home.as_deref(),
        bounds,
    );

    let scheduler = match &hand.schedule {
        None => Timing::Absent,
        Some(schedule) => match program::find(schedule.program, &path) {
            Some(found) => Timing::Has((hand.scheduler)(
                found,
                units.clone().unwrap_or_default(),
                bounds,
            )),
            None => Timing::Missing {
                program: schedule.program,
                status: schedule.missing,
            },
        },
    };

    let carried: Vec<(String, OsString)> = folder_variables()
        .into_iter()
        .filter_map(|name| {
            (env.var)(name)
                .filter(|value| !value.is_empty())
                .map(|value| (name.to_owned(), value))
        })
        .collect();
    let unit_var = |name: &str| {
        carried
            .iter()
            .find(|(carried, _)| carried == name)
            .map(|(_, value)| value.clone())
    };
    let unit = settings::load(&unit_var, &read_regular, hand.kinds, hand.providers)?;

    let request = Request {
        plugin: package,
        plugin_key: SETUP_PLUGIN.about.name,
        program: own,
        logs,
        variables: carried.clone(),
        clause: clause(&unit),
        section,
    };
    let said = Rc::new(RefCell::new(Vec::new()));
    let keeper = notes::Keeper {
        decision,
        kind: hand.kind,
        config: settings.config.value.clone(),
        save: hand.save,
        said: Rc::clone(&said),
    };
    let ports = Ports {
        notes: Box::new(keeper),
        tools,
        scheduler,
    };
    let report = steps::run(&request, &ports);
    let failed = view::failed(&report, false);
    let rows = report.clone();
    let mut outcome = Outcome::new(view::rows(&report, &crate::console::view::View::PLAIN))
        .with_human(move |view| view::rows(&rows, view))
        .on_every_terminal();
    let mut diagnostics = said.borrow().clone();
    diagnostics.extend(failed.iter().cloned());
    if !diagnostics.is_empty() {
        outcome = outcome.with_diagnostics(diagnostics);
    }
    if failed.is_some() {
        outcome = outcome.with_exit(Exit::Refused);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use crate::console::args::{Parsed, parse, typed};
    use crate::console::outcome::Failure;
    use crate::{Cli, bilbo};

    fn usage_error(words: &[&str]) -> Failure {
        let root = bilbo();
        let args: Vec<OsString> = ["bilbo", "setup"]
            .into_iter()
            .chain(words.iter().copied())
            .map(OsString::from)
            .collect();
        match parse(&root, &args) {
            Err(failure) => failure,
            Ok(Parsed::Matches(matches)) => typed::<Cli>(&root, &matches).expect_err("an error"),
            Ok(other) => panic!("{other:?}"),
        }
    }

    #[test]
    fn setup_takes_only_its_options() {
        for (words, reason) in [
            (&["x"][..], "unexpected argument 'x'"),
            (&["--all"], "unknown option '--all'"),
            (&["--claude"], "--claude needs a value"),
        ] {
            match usage_error(words) {
                Failure::Usage {
                    reason: found,
                    usage,
                    page,
                } => {
                    assert_eq!(found, reason, "{words:?}");
                    assert_eq!(
                        usage,
                        [
                            "usage: bilbo setup [--claude <program>] [--codex <program>]",
                            "       [--agy <program>] [--notes <folder>] [--plugin <folder>]",
                            "       bilbo setup --remove [--claude <program>] [--codex <program>]",
                            "       [--agy <program>]"
                        ],
                        "{words:?}"
                    );
                    assert_eq!(page, "bilbo setup --help");
                }
                other => panic!("{words:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn removal_takes_only_what_goes_with_it() {
        let setup = |words: &[&str]| {
            let root = bilbo();
            let args: Vec<OsString> = ["bilbo", "setup"]
                .into_iter()
                .chain(words.iter().copied())
                .map(OsString::from)
                .collect();
            let Ok(Parsed::Matches(matches)) = parse(&root, &args) else {
                panic!("{words:?} parses");
            };
            let Cli {
                verb: Some(crate::Verb::Setup(setup)),
            } = typed::<Cli>(&root, &matches).expect("typed")
            else {
                panic!("{words:?} is setup");
            };
            super::removal_alone(&root, &setup)
        };
        for (words, reason) in [
            (
                &["--remove", "--notes", "x"][..],
                "--notes does not stand with --remove: a removal leaves the notes folder",
            ),
            (
                &["--remove", "--plugin", "x"],
                "--plugin does not stand with --remove: a removal reads no plugin folder",
            ),
        ] {
            match setup(words) {
                Err(Failure::Usage {
                    reason: found,
                    page,
                    ..
                }) => {
                    assert_eq!(found, reason, "{words:?}");
                    assert_eq!(page, "bilbo setup --help");
                }
                other => panic!("{words:?}: {other:?}"),
            }
        }
        assert!(setup(&["--remove", "--codex", "x"]).is_ok());
        assert!(setup(&["--notes", "x", "--plugin", "y"]).is_ok());
    }
}
