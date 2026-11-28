use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::keys::Section;
use crate::plugin::{self, Agent};
use crate::report::{Line, Report, Status};
use crate::timer::{self, Job, Scheduler};

pub struct Request {
    pub plugin: Option<PathBuf>,
    pub program: Option<PathBuf>,
    pub logs: PathBuf,
    pub variables: Vec<(String, OsString)>,
    pub clause: Option<String>,
    pub section: Section,
}

pub enum Way {
    Off,
    Found {
        agent: Box<dyn Agent>,
        folder: PathBuf,
    },
    Missing {
        value: String,
        default: bool,
    },
}

pub struct Tool {
    pub step: &'static str,
    pub hooks: Option<&'static str>,
    pub way: Way,
}

pub enum Timing {
    Absent,
    Refused(String),
    Missing {
        program: &'static str,
        status: Status,
    },
    Has(Box<dyn Scheduler>),
}

pub trait Notes {
    fn see_to(&self) -> Line;
}

pub struct Ports<'a> {
    pub notes: Box<dyn Notes + 'a>,
    pub tools: [Tool; 3],
    pub scheduler: Timing,
}

fn line(step: &'static str, status: Status, detail: String) -> Line {
    Line {
        step,
        status,
        detail,
    }
}

fn unavailable(tool: &Tool) -> Option<Line> {
    let step = tool.step;
    match &tool.way {
        Way::Off => Some(line(
            step,
            Status::Skipped,
            format!("setup.{step}.enabled is false"),
        )),
        Way::Missing { value, default } => Some(if *default {
            line(step, Status::Skipped, format!("{step} is not on PATH"))
        } else if value.contains('/') {
            line(
                step,
                Status::Failed,
                format!("setup.{step}.program: {value} is not a program this user can run"),
            )
        } else {
            line(
                step,
                Status::Failed,
                format!("setup.{step}.program: {value} is not on PATH"),
            )
        }),
        Way::Found { .. } => None,
    }
}

fn tool_line(request: &Request, tool: &Tool) -> Line {
    if let Some(row) = unavailable(tool) {
        return row;
    }
    match (&tool.way, &request.plugin) {
        (Way::Found { agent, .. }, Some(folder)) => plugin::step(agent.as_ref(), folder),
        _ => line(
            tool.step,
            Status::Failed,
            "cannot find this bilbo's package; set setup.plugin to the folder that holds the plugin"
                .to_owned(),
        ),
    }
}

fn hooks_line(
    request: &Request,
    tool: &Tool,
    hooks: &'static str,
    tool_row: &Line,
) -> Option<Line> {
    let step = tool.step;
    match (tool_row.status, &tool.way, &request.plugin) {
        (
            Status::Installed | Status::Updated | Status::Kept,
            Way::Found { agent, .. },
            Some(folder),
        ) => agent
            .hooks(folder)
            .map(|mut trusting| plugin::trust(hooks, trusting.hooks.as_mut())),
        (Status::Failed, _, _) => Some(line(
            hooks,
            Status::Skipped,
            format!("the {step} step failed"),
        )),
        _ => Some(line(
            hooks,
            Status::Skipped,
            format!("the {step} step was skipped"),
        )),
    }
}

fn job(
    request: &Request,
    folders: &[PathBuf],
    step: &'static str,
    verb: &'static str,
    interval: Duration,
    clause: Option<String>,
) -> Job {
    let mut path = folders.to_vec();
    path.extend(
        request
            .section
            .timer_path
            .value
            .split(':')
            .filter(|folder| !folder.is_empty())
            .map(PathBuf::from),
    );
    Job {
        step,
        verb,
        interval,
        program: request.program.clone(),
        log: request.logs.join(format!("{verb}.log")),
        variables: request.variables.clone(),
        path,
        clause,
    }
}

pub fn run(request: &Request, ports: &Ports<'_>) -> Report {
    let mut report = Report::default();
    report.lines.push(ports.notes.see_to());
    let mut folders = Vec::new();
    for tool in &ports.tools {
        let row = tool_line(request, tool);
        let hooks = tool
            .hooks
            .and_then(|hooks| hooks_line(request, tool, hooks, &row));
        report.lines.push(row);
        report.lines.extend(hooks);
        if let Way::Found { folder, .. } = &tool.way {
            folders.push(folder.clone());
        }
    }
    let jobs = [
        job(
            request,
            &folders,
            "refresh-timer",
            "refresh",
            request.section.refresh_every.value,
            None,
        ),
        job(
            request,
            &folders,
            "derive-timer",
            "derive",
            request.section.derive_every.value,
            request.clause.clone(),
        ),
    ];
    for job in &jobs {
        let row = match &ports.scheduler {
            Timing::Refused(sentence) => line(job.step, Status::Failed, sentence.clone()),
            Timing::Absent => line(
                job.step,
                Status::Skipped,
                "this platform has neither launchd nor systemd".to_owned(),
            ),
            Timing::Missing { program, status } => {
                line(job.step, *status, format!("{program} is not on PATH"))
            }
            Timing::Has(scheduler) => timer::step(scheduler.as_ref(), job),
        };
        report.lines.push(row);
    }
    report
}

pub struct Taking {
    pub tools: [Tool; 3],
    pub scheduler: Timing,
}

fn taking_job(step: &'static str, verb: &'static str) -> Job {
    Job {
        step,
        verb,
        interval: Duration::ZERO,
        program: None,
        log: PathBuf::new(),
        variables: Vec::new(),
        path: Vec::new(),
        clause: None,
    }
}

fn skipped_hooks(tool: &Tool, hooks: &'static str, failed: bool) -> Line {
    let step = tool.step;
    line(
        hooks,
        Status::Skipped,
        format!(
            "the {step} step {}",
            if failed { "failed" } else { "was skipped" }
        ),
    )
}

pub fn remove(taking: &Taking) -> Report {
    let mut report = Report::default();
    for tool in &taking.tools {
        if let Some(row) = unavailable(tool) {
            if let Some(hooks) = tool.hooks {
                report
                    .lines
                    .push(skipped_hooks(tool, hooks, row.status == Status::Failed));
            }
            report.lines.push(row);
            continue;
        }
        let Way::Found { agent, .. } = &tool.way else {
            continue;
        };
        let held = match plugin::glance(agent.as_ref()) {
            Ok(held) => held,
            Err(row) => {
                if let Some(hooks) = tool.hooks {
                    report.lines.push(skipped_hooks(tool, hooks, true));
                }
                report.lines.push(row);
                continue;
            }
        };
        if let Some(hooks) = tool.hooks {
            report.lines.push(match agent.hooks(Path::new("")) {
                Some(mut trusting) => plugin::forget(hooks, trusting.hooks.as_mut()),
                None => skipped_hooks(tool, hooks, false),
            });
        }
        report.lines.push(plugin::removal(agent.as_ref(), &held));
    }
    for (step, verb) in [("refresh-timer", "refresh"), ("derive-timer", "derive")] {
        let job = taking_job(step, verb);
        report.lines.push(match &taking.scheduler {
            Timing::Refused(sentence) => line(step, Status::Failed, sentence.clone()),
            Timing::Absent => line(
                step,
                Status::Skipped,
                "this platform has neither launchd nor systemd".to_owned(),
            ),
            Timing::Missing { program, status } => {
                line(step, *status, format!("{program} is not on PATH"))
            }
            Timing::Has(scheduler) => timer::removal(scheduler.as_ref(), &job),
        });
    }
    report
}
