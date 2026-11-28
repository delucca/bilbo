mod steps;
mod timers;

use std::cell::RefCell;
use std::path::Path;

use bilbo_setup::plugin::{
    Act, Agent, Held, Installed, Marketplace, Other, Seen, Stale, Trusting, glance, removal, step,
};
use bilbo_setup::report::{Line, Status};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Market {
    None,
    Here,
    Elsewhere,
    NotUsed,
}

#[derive(Clone, Debug)]
struct State {
    market: Market,
    plugin: Option<(bool, bool)>,
    other: Option<Other>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Break {
    BeforeWork,
    AfterWork,
}

struct Sim {
    state: RefCell<State>,
    log: RefCell<Vec<String>>,
    fails: Option<(Act, Break)>,
    reason: &'static str,
    stays_off: bool,
    brings_other: bool,
    stays_listed: bool,
    lingers: Option<(bool, bool)>,
    commanded: RefCell<bool>,
    manifest: Result<(), String>,
}

fn word(act: Act) -> &'static str {
    match act {
        Act::Uninstall => "uninstall",
        Act::Remove => "remove",
        Act::Add => "add",
        Act::Install => "install",
    }
}

impl Sim {
    fn new(market: Market, plugin: Option<(bool, bool)>) -> Sim {
        Sim {
            state: RefCell::new(State {
                market,
                plugin,
                other: None,
            }),
            log: RefCell::new(Vec::new()),
            fails: None,
            reason: "boom",
            stays_off: false,
            brings_other: false,
            stays_listed: false,
            lingers: None,
            commanded: RefCell::new(false),
            manifest: Ok(()),
        }
    }

    fn apply(&self, act: Act) {
        let mut state = self.state.borrow_mut();
        match act {
            Act::Uninstall if self.stays_listed => {}
            Act::Uninstall => state.plugin = None,
            Act::Remove => {
                state.market = Market::None;
                state.plugin = None;
            }
            Act::Add => state.market = Market::Here,
            Act::Install => {
                state.plugin = Some((!self.stays_off, true));
                if self.brings_other {
                    state.other = Some(Other {
                        id: "other@bilbo".to_owned(),
                        scope: None,
                    });
                }
            }
        }
    }

    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

impl Agent for Sim {
    fn step(&self) -> &'static str {
        "claude"
    }

    fn name(&self) -> &'static str {
        "claude"
    }

    fn id(&self) -> &'static str {
        "bilbo@bilbo"
    }

    fn manifest(&self, _folder: &Path) -> Result<(), String> {
        self.manifest.clone()
    }

    fn look(&self, _folder: &Path) -> Result<Seen, String> {
        self.log.borrow_mut().push("look".to_owned());
        let state = self.state.borrow();
        Ok(Seen {
            marketplace: match state.market {
                Market::None => Marketplace::None,
                Market::Here => Marketplace::Here,
                Market::Elsewhere => Marketplace::Elsewhere("/old".to_owned()),
                Market::NotUsed => Marketplace::NotUsed,
            },
            plugin: state.plugin.map(|(on, current)| Installed {
                on,
                stale: (!current).then(|| Stale::Version("0.1".to_owned())),
            }),
            other: state.other.clone(),
        })
    }

    fn run(&self, act: Act, folder: &Path) -> Result<(), String> {
        self.log.borrow_mut().push(word(act).to_owned());
        *self.commanded.borrow_mut() = true;
        let words = self.words(act, folder);
        match self.fails {
            Some((failing, Break::BeforeWork)) if failing == act => {
                Err(format!("claude {words}: {}", self.reason))
            }
            Some((failing, Break::AfterWork)) if failing == act => {
                self.apply(act);
                Err(format!("claude {words}: {}", self.reason))
            }
            _ => {
                self.apply(act);
                Ok(())
            }
        }
    }

    fn words(&self, act: Act, _folder: &Path) -> String {
        word(act).to_owned()
    }

    fn hooks(&self, _folder: &Path) -> Option<Trusting> {
        None
    }

    fn held(&self) -> Result<Held, String> {
        self.log.borrow_mut().push("held".to_owned());
        if let (Some((plugin, marketplace)), true) = (self.lingers, *self.commanded.borrow()) {
            return Ok(Held {
                plugin,
                marketplace,
                other: None,
            });
        }
        let state = self.state.borrow();
        Ok(Held {
            plugin: state.plugin.is_some(),
            marketplace: matches!(state.market, Market::Here | Market::Elsewhere),
            other: state.other.clone(),
        })
    }
}

type Taken = (
    Market,
    Option<(bool, bool)>,
    Status,
    &'static str,
    Vec<&'static str>,
);
type Row = (Market, Option<(bool, bool)>, Status, Vec<&'static str>);
type Failure = (
    &'static str,
    Market,
    Option<(bool, bool)>,
    Act,
    Break,
    Status,
    Vec<&'static str>,
);

fn run(sim: &Sim) -> Line {
    step(sim, Path::new("/pkg"))
}

fn commands(sim: &Sim) -> Vec<String> {
    sim.log()
        .into_iter()
        .filter(|word| word != "look")
        .collect()
}

#[test]
fn a_plugin_step_looks_changes_and_looks_again() {
    let rows: Vec<Row> = vec![
        (Market::Here, Some((true, true)), Status::Kept, vec![]),
        (Market::NotUsed, Some((true, true)), Status::Kept, vec![]),
        (
            Market::None,
            None,
            Status::Installed,
            vec!["add", "install"],
        ),
        (Market::Here, None, Status::Installed, vec!["install"]),
        (Market::NotUsed, None, Status::Installed, vec!["install"]),
        (
            Market::Here,
            Some((false, true)),
            Status::Updated,
            vec!["uninstall", "install"],
        ),
        (
            Market::Here,
            Some((true, false)),
            Status::Updated,
            vec!["uninstall", "install"],
        ),
        (
            Market::NotUsed,
            Some((false, true)),
            Status::Updated,
            vec!["uninstall", "install"],
        ),
        (
            Market::None,
            Some((true, true)),
            Status::Updated,
            vec!["uninstall", "add", "install"],
        ),
        (
            Market::Elsewhere,
            None,
            Status::Updated,
            vec!["remove", "add", "install"],
        ),
        (
            Market::Elsewhere,
            Some((true, true)),
            Status::Updated,
            vec!["uninstall", "remove", "add", "install"],
        ),
    ];
    for (market, plugin, status, asked) in rows {
        let sim = Sim::new(market.clone(), plugin);
        let line = run(&sim);
        assert_eq!(
            line.status, status,
            "{market:?} {plugin:?}: {}",
            line.detail
        );
        assert_eq!(commands(&sim), asked, "{market:?} {plugin:?}");
        let mut expected = vec!["look".to_owned()];
        expected.extend(asked.iter().map(|word| (*word).to_owned()));
        if !asked.is_empty() {
            expected.push("look".to_owned());
        }
        assert_eq!(
            sim.log(),
            expected,
            "a second look follows the commands, and only them"
        );
        assert!(
            line.detail.starts_with("bilbo@bilbo from /pkg"),
            "{}",
            line.detail
        );
        assert_eq!(
            line.detail.contains("; "),
            status == Status::Updated,
            "{}",
            line.detail
        );
    }
}

#[test]
fn a_step_whose_second_look_is_not_as_wanted_is_failed() {
    let mut sim = Sim::new(Market::Here, None);
    sim.stays_off = true;
    let line = run(&sim);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            "claude lists the plugin turned off after install"
        )
    );
    let mut sim = Sim::new(Market::Here, None);
    sim.brings_other = true;
    let line = run(&sim);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            "claude lists other@bilbo, also from the marketplace bilbo after install"
        )
    );
}

#[test]
fn a_failed_command_ends_its_tool_and_names_the_command() {
    let mut sim = Sim::new(Market::Elsewhere, None);
    sim.fails = Some((Act::Add, Break::BeforeWork));
    let line = run(&sim);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Failed, "claude add: boom")
    );
    assert_eq!(commands(&sim), ["remove", "add"]);
}

#[test]
fn a_run_after_a_failed_command_goes_on_from_what_the_tool_lists() {
    let rows: Vec<Failure> = vec![
        (
            "its first command failed and changed nothing",
            Market::Elsewhere,
            Some((true, true)),
            Act::Uninstall,
            Break::BeforeWork,
            Status::Updated,
            vec!["uninstall", "remove", "add", "install"],
        ),
        (
            "the uninstall worked and the removal did not happen",
            Market::Elsewhere,
            Some((true, true)),
            Act::Uninstall,
            Break::AfterWork,
            Status::Updated,
            vec!["remove", "add", "install"],
        ),
        (
            "the removal worked and the add did not happen",
            Market::Elsewhere,
            Some((true, true)),
            Act::Remove,
            Break::AfterWork,
            Status::Installed,
            vec!["add", "install"],
        ),
        (
            "the add worked and the install did not happen",
            Market::None,
            None,
            Act::Add,
            Break::AfterWork,
            Status::Installed,
            vec!["install"],
        ),
    ];
    for (name, market, plugin, failing, when, status, again) in rows {
        let mut sim = Sim::new(market, plugin);
        sim.fails = Some((failing, when));
        let first = run(&sim);
        assert_eq!(first.status, Status::Failed, "{name}");
        let command = match failing {
            Act::Uninstall => "uninstall",
            Act::Remove => "remove",
            Act::Add => "add",
            Act::Install => "install",
        };
        assert_eq!(first.detail, format!("claude {command}: boom"), "{name}");
        let logged = sim.log().len();
        let mut next = sim;
        next.fails = None;
        let second = run(&next);
        assert_eq!(second.status, status, "{name}: {}", second.detail);
        assert_eq!(
            commands(&next)[logged_commands(&next, logged)..],
            again,
            "{name}"
        );
    }
    for (how, reason) in [
        ("it exits with an error", "boom"),
        ("a signal ends it", "a signal ended it"),
        ("its wait runs out", "it did not end within 30s"),
    ] {
        for changed in [false, true] {
            let mut sim = Sim::new(Market::Here, None);
            sim.reason = reason;
            sim.fails = Some((
                Act::Install,
                if changed {
                    Break::AfterWork
                } else {
                    Break::BeforeWork
                },
            ));
            let first = run(&sim);
            assert_eq!(first.status, Status::Failed, "{how}");
            assert_eq!(first.detail, format!("claude install: {reason}"), "{how}");
            let mut next = sim;
            next.fails = None;
            let before = next.log().len();
            let second = run(&next);
            let expected = if changed {
                Status::Kept
            } else {
                Status::Installed
            };
            assert_eq!(
                second.status, expected,
                "{how} {changed}: {}",
                second.detail
            );
            let after: Vec<String> = next.log()[before..].to_vec();
            let wanted: Vec<&str> = if changed {
                vec!["look"]
            } else {
                vec!["look", "install", "look"]
            };
            assert_eq!(after, wanted, "{how} {changed}");
        }
    }
}

fn logged_commands(sim: &Sim, before: usize) -> usize {
    sim.log()[..before]
        .iter()
        .filter(|word| *word != "look")
        .count()
}

#[test]
fn another_installation_stops_the_step_before_any_command() {
    let other = Other {
        id: "other@bilbo".to_owned(),
        scope: Some("user".to_owned()),
    };
    for (market, plugin) in [
        (Market::Elsewhere, Some((true, true))),
        (Market::Here, Some((true, true))),
    ] {
        let sim = Sim::new(market, plugin);
        sim.state.borrow_mut().other = Some(other.clone());
        let line = run(&sim);
        assert_eq!(
            (line.status, line.detail.as_str()),
            (
                Status::Failed,
                "claude has other@bilbo (user scope) installed from the marketplace bilbo; bilbo setup changes no marketplace another installation uses"
            )
        );
        assert_eq!(sim.log(), ["look"], "no command was asked");
    }
}

#[test]
fn a_plugin_folder_that_lacks_the_tools_file_starts_nothing() {
    for sentence in [
        "no plugin at /pkg: it holds no .claude-plugin/marketplace.json",
        "no plugin at /pkg: .claude-plugin/marketplace.json names the marketplace 'personal', not bilbo",
    ] {
        let mut sim = Sim::new(Market::Here, None);
        sim.manifest = Err(sentence.to_owned());
        let line = run(&sim);
        assert_eq!(
            (line.status, line.detail.as_str()),
            (Status::Failed, sentence)
        );
        assert!(sim.log().is_empty(), "no look was asked");
    }
}

fn taking(sim: &Sim) -> Line {
    match glance(sim) {
        Ok(held) => removal(sim, &held),
        Err(line) => line,
    }
}

fn commands_but_held(sim: &Sim) -> Vec<String> {
    sim.log()
        .into_iter()
        .filter(|word| word != "held")
        .collect()
}

#[test]
fn a_removal_takes_out_what_the_tool_lists() {
    let rows: Vec<Taken> = vec![
        (
            Market::None,
            None,
            Status::Absent,
            "no plugin bilbo@bilbo",
            vec![],
        ),
        (
            Market::NotUsed,
            None,
            Status::Absent,
            "no plugin bilbo@bilbo",
            vec![],
        ),
        (
            Market::NotUsed,
            Some((true, true)),
            Status::Removed,
            "bilbo@bilbo",
            vec!["uninstall"],
        ),
        (
            Market::Here,
            None,
            Status::Removed,
            "the marketplace bilbo",
            vec!["remove"],
        ),
        (
            Market::Here,
            Some((false, false)),
            Status::Removed,
            "bilbo@bilbo and the marketplace bilbo",
            vec!["uninstall", "remove"],
        ),
        (
            Market::Elsewhere,
            Some((true, true)),
            Status::Removed,
            "bilbo@bilbo and the marketplace bilbo",
            vec!["uninstall", "remove"],
        ),
    ];
    for (market, plugin, status, detail, acts) in rows {
        let sim = Sim::new(market.clone(), plugin);
        let line = taking(&sim);
        assert_eq!(
            (line.status, line.detail.as_str(), commands_but_held(&sim)),
            (
                status,
                detail,
                acts.iter().map(|a| (*a).to_owned()).collect()
            ),
            "{market:?} {plugin:?}"
        );
        let left = sim.held().expect("held");
        assert!(
            !left.plugin && !left.marketplace,
            "{market:?} {plugin:?} left something"
        );
    }
    for (lingers, said) in [
        (
            (true, false),
            "claude still lists the plugin bilbo@bilbo after remove",
        ),
        (
            (false, true),
            "claude still lists the marketplace bilbo after remove",
        ),
        (
            (true, true),
            "claude still lists the plugin bilbo@bilbo and the marketplace bilbo after remove",
        ),
    ] {
        let mut sim = Sim::new(Market::Here, Some((true, true)));
        sim.lingers = Some(lingers);
        let line = taking(&sim);
        assert_eq!(line.status, Status::Failed, "{lingers:?}");
        assert_eq!(line.detail, said, "{lingers:?}");
        assert_eq!(commands_but_held(&sim), ["uninstall", "remove"]);
    }
}

#[test]
fn a_removal_that_leaves_something_listed_fails_with_what_is_left() {
    let mut sim = Sim::new(Market::Here, Some((true, true)));
    sim.fails = Some((Act::Remove, Break::BeforeWork));
    let line = taking(&sim);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(line.detail, "claude remove: boom");
    assert_eq!(commands_but_held(&sim), ["uninstall", "remove"]);

    let mut sim = Sim::new(Market::Here, Some((true, true)));
    sim.fails = Some((Act::Uninstall, Break::BeforeWork));
    let line = taking(&sim);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(commands_but_held(&sim), ["uninstall"]);
}

#[test]
fn a_removal_that_works_but_leaves_the_plugin_listed_says_what_it_still_lists() {
    let mut sim = Sim::new(Market::NotUsed, Some((true, true)));
    sim.stays_listed = true;
    let line = taking(&sim);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(
        line.detail,
        "claude still lists the plugin bilbo@bilbo after uninstall"
    );
}

#[test]
fn a_removal_stops_at_another_installation() {
    let sim = Sim::new(Market::Here, Some((true, true)));
    sim.state.borrow_mut().other = Some(Other {
        id: "other@bilbo".to_owned(),
        scope: Some("user".to_owned()),
    });
    let line = taking(&sim);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(
        line.detail,
        "claude has other@bilbo (user scope) installed from the marketplace bilbo; bilbo setup changes no marketplace another installation uses"
    );
    assert_eq!(commands_but_held(&sim), Vec::<String>::new());
}
