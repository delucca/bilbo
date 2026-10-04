//! `bilbo library read`.

use std::path::Path;

use super::reference::{label, source_by_id};
use super::{Args, Output, grouped, io_failure, refused, root, sections, state_failure, usage};
use crate::Failure;
use crate::citation;
use crate::library::corpus::SourceFile;
use crate::library::reading;
use crate::shared::frontmatter;
use crate::shared::markdown;
use crate::shared::store;

/// `<k>/<n>` with `n` from 2 to the most parts and `k` from 1 to `n`.
fn parse_part(value: &str) -> Result<(usize, usize), Failure> {
    let number = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| s.parse::<usize>().ok())
            .flatten()
    };
    value
        .split_once('/')
        .and_then(|(k, n)| Some((number(k)?, number(n)?)))
        .filter(|&(k, n)| (2..=reading::MAX_PARTS).contains(&n) && (1..=n).contains(&k))
        .ok_or_else(|| {
            usage(format!(
                "--part '{value}' is not <k>/<n> with n from 2 to {} and k from 1 to n",
                reading::MAX_PARTS
            ))
        })
}

pub fn run(args: &Args, env: &store::Env) -> Result<Output, Failure> {
    let operands = &args.positional[1..];
    let Some(plan_id) = operands.first() else {
        return Err(usage("missing <plan>"));
    };
    if !frontmatter::is_ulid(plan_id) {
        return Err(usage(format!("'{plan_id}' is not a plan id")));
    }
    if operands.len() == 1 {
        return Err(usage("missing <slice>"));
    }
    let numbers = operands[1..]
        .iter()
        .map(|value| {
            value
                .bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| value.parse::<usize>().ok().filter(|n| *n >= 1))
                .flatten()
                .ok_or_else(|| usage(format!("'{value}' is not a slice number")))
        })
        .collect::<Result<Vec<usize>, Failure>>()?;
    let part = args.one("--part").map(parse_part).transpose()?;

    let root = root(env)?;
    let plans = store::plans_dir(env).ok_or_else(state_failure)?;
    let plan = reading::load(&plans, plan_id)
        .map_err(refused)?
        .ok_or_else(|| refused(format!("no plan '{plan_id}' in {}", plans.display())))?;
    if Path::new(&plan.root) != root {
        return Err(refused(format!(
            "plan {plan_id} was made for the store {}, not {}",
            plan.root,
            root.display()
        )));
    }
    if let Some(n) = numbers.iter().find(|n| **n > plan.slices.len()) {
        return Err(usage(format!(
            "slice {n} is not in plan {plan_id}, which has {} slices",
            plan.slices.len()
        )));
    }

    let ids = citation::Ids::scan(&root);
    let mut sources: Vec<(usize, SourceFile)> = Vec::new();
    let mut rendered = Vec::new();
    for &number in &numbers {
        let pick_index = plan.slices[number - 1].pick;
        let pick = &plan.picks[pick_index];
        if !sources.iter().any(|(i, _)| *i == pick_index) {
            let file = source_by_id(&ids, &pick.id).map_err(|failure| match failure {
                Failure::Refused(message) => refused(format!(
                    "{}: {message}; make a new plan with bilbo library plan",
                    pick.label()
                )),
                other => other,
            })?;
            if file.source.digest.as_deref() != Some(pick.digest.as_str())
                || file.source.body_start != pick.body_start
            {
                return Err(refused(format!(
                    "{} changed since plan {plan_id}; make a new plan with bilbo library plan",
                    pick.label()
                )));
            }
            sources.push((pick_index, file));
        }
        let file = &sources.iter().find(|(i, _)| *i == pick_index).unwrap().1;
        let lines = markdown::lines(&file.text);
        let outline = sections(file);
        let material = reading::Material {
            lines: &lines,
            sections: &outline,
        };
        rendered
            .push(reading::render(&plan, number, part, &label(file), &material).map_err(usage)?);
    }

    let bytes: usize = rendered.iter().map(reading::Rendered::bytes).sum();
    if numbers.len() > 1 && bytes > plan.options.slice_bytes {
        let named: Vec<String> = numbers.iter().map(usize::to_string).collect();
        return Err(usage(format!(
            "slices {} print {} bytes together, over the limit of {} bytes for one read; read fewer slices per call",
            named.join(", "),
            grouped(bytes),
            grouped(plan.options.slice_bytes)
        )));
    }

    let entries: Vec<reading::LogEntry> = numbers
        .iter()
        .zip(&rendered)
        .map(|(&number, run)| {
            reading::entry(
                number,
                &plan.picks[plan.slices[number - 1].pick],
                run.start,
                run.end,
            )
        })
        .collect();
    reading::append_log(&plans, plan_id, &entries)
        .map_err(io_failure("write", &reading::log_path(&plans, plan_id)))?;
    Ok(Output::lines(
        rendered.into_iter().flat_map(|run| run.lines).collect(),
    ))
}
