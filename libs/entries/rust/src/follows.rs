use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;

use bilbo_store::entries::{Capture, CaptureError, Captured, Entries};
use bilbo_store::id::Id;
use bilbo_store::labels::Tags;

use crate::files::{At, Files, Node, at};
use crate::kind::Kind;
use crate::remember::{self, Error, Item, Outcome, Output, Plan, Request, Tagging};

/// A `None` kind is detected as a target's is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Follow {
    pub path: PathBuf,
    pub kind: Option<String>,
}

pub enum Planned<'k> {
    /// The follow's folder does not exist: it is skipped.
    Missing(Follow),
    /// Unknown kind, several kinds claiming, or an unreadable target.
    Failed(Follow, Error),
    Ready(Follow, Plan<'k>),
}

/// Opens no store.
pub fn plan<'k>(
    follows: &[Follow],
    kinds: &'k [Box<dyn Kind>],
    files: &dyn Files,
) -> Vec<Planned<'k>> {
    follows
        .iter()
        .map(|follow| {
            let request = Request {
                target: follow.path.clone(),
                kind: follow.kind.clone(),
                tags: Tagging::default(),
            };
            match remember::plan(&request, kinds, files) {
                Ok(plan) => Planned::Ready(follow.clone(), plan),
                Err(Error::Target { error, .. }) if error.kind() == io::ErrorKind::NotFound => {
                    Planned::Missing(follow.clone())
                }
                Err(error) => Planned::Failed(follow.clone(), error),
            }
        })
        .collect()
}

pub fn keep(
    planned: &[Planned<'_>],
    files: &dyn Files,
    entries: &mut dyn Entries,
) -> Result<Vec<Output>, Error> {
    let mut plans = Vec::new();
    let mut outputs = Vec::new();
    for planned in planned {
        if let Planned::Ready(_, plan) = planned {
            outputs.push(remember::keep(plan, files, entries)?);
            plans.push(plan);
        }
    }
    let protected: BTreeSet<Id> = outputs
        .iter()
        .flat_map(|output| &output.items)
        .filter_map(|item| item.carried)
        .collect();
    for (plan, output) in plans.into_iter().zip(&mut outputs) {
        withdraw_gone(plan, output, &protected, files, entries)?;
    }
    Ok(outputs)
}

fn withdraw_gone(
    plan: &Plan<'_>,
    output: &mut Output,
    protected: &BTreeSet<Id>,
    files: &dyn Files,
    entries: &mut dyn Entries,
) -> Result<(), Error> {
    let Some(kind) = plan.kind.filter(|kind| kind.withdraws_gone()) else {
        return Ok(());
    };
    let seen = output
        .items
        .iter()
        .any(|item| !matches!(item.outcome, Outcome::Refused(_)));
    let Some(target) = plan.target.to_str().filter(|_| seen) else {
        return Ok(());
    };
    if !matches!(files.node(&plan.target), Ok(Node::Folder)) {
        return Ok(());
    }
    let claimed: BTreeSet<&OsStr> = plan
        .claims
        .iter()
        .map(|claim| claim.label().as_os_str())
        .collect();
    let mut gone = Vec::new();
    for (id, label) in entries.captures_with(target)? {
        let Some(path) = label.path() else {
            continue;
        };
        if !path
            .strip_prefix(&plan.target)
            .is_ok_and(|rest| !rest.as_os_str().is_empty())
            || claimed.contains(path.as_os_str())
            || protected.contains(&id)
        {
            continue;
        }
        if entries
            .meta(id)?
            .is_none_or(|meta| meta.kind != kind.name())
        {
            continue;
        }
        let Some(head) = entries.by_capture(&label)?.filter(|head| head.records > 0) else {
            continue;
        };
        if matches!(at(files, path), At::Nothing | At::Elsewhere(_)) {
            gone.push((path.to_path_buf(), label, head));
        }
    }
    gone.sort_by(|a, b| {
        a.0.as_os_str()
            .as_encoded_bytes()
            .cmp(b.0.as_os_str().as_encoded_bytes())
    });
    for (path, label, head) in gone {
        let input = Capture {
            label: &label,
            kind: kind.name(),
            id: None,
            id_label: None,
            tags: &Tags::default(),
            stamp: None,
            bytes: &[],
            ends: &[],
        };
        match entries.capture(&input) {
            Ok(Captured::NewGeneration(kept)) => {
                output.stored_bytes += kept.stored_bytes;
                output.items.push(Item {
                    path,
                    outcome: Outcome::NewGeneration(kept.id),
                    moved: None,
                    carried: None,
                    withdrew: Some(head.generation),
                });
            }
            Ok(_) => output.items.push(refusal(
                path,
                "its file is gone, and the store did not withdraw its entry".to_owned(),
            )),
            Err(CaptureError::Store(error)) => return Err(error.into()),
            Err(error) => output.items.push(refusal(path, error.to_string())),
        }
    }
    Ok(())
}

fn refusal(path: PathBuf, reason: String) -> Item {
    Item {
        path,
        outcome: Outcome::Refused(reason),
        moved: None,
        carried: None,
        withdrew: None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Follow, Planned, plan};
    use crate::fake::Suffix;
    use crate::files::{Memory, Node};
    use crate::kind::Kind;

    fn follow(path: &str, kind: Option<&str>) -> Follow {
        Follow {
            path: PathBuf::from(path),
            kind: kind.map(str::to_owned),
        }
    }

    fn kinds() -> Vec<Box<dyn Kind>> {
        vec![
            Box::new(Suffix::new("one", ".one")),
            Box::new(Suffix::new("two", ".two")),
        ]
    }

    fn shape(planned: &Planned<'_>) -> String {
        match planned {
            Planned::Missing(follow) => format!("missing {}", follow.path.display()),
            Planned::Failed(follow, error) => format!("failed {}: {error}", follow.path.display()),
            Planned::Ready(follow, plan) => format!(
                "ready {} {} {}",
                follow.path.display(),
                plan.kind.map_or("none", Kind::name),
                plan.claims.len()
            ),
        }
    }

    #[test]
    fn follows_are_planned_in_the_order_given_and_a_missing_folder_is_skipped() {
        let files = Memory::new()
            .file("/b/x.two", "x")
            .file("/a/x.one", "x")
            .file("/a/y.one", "y");
        let kinds = kinds();
        let follows = [
            follow("/b", None),
            follow("/gone", Some("one")),
            follow("/a", Some("one")),
        ];
        let planned = plan(&follows, &kinds, &files);
        let shapes: Vec<String> = planned.iter().map(shape).collect();
        assert_eq!(
            shapes,
            ["ready /b two 1", "missing /gone", "ready /a one 2"]
        );
    }

    #[test]
    fn a_follow_that_cannot_run_fails_alone_and_the_others_still_plan() {
        let files = Memory::new()
            .file("/both/x.one", "x")
            .file("/both/x.two", "x")
            .file("/a/x.one", "x")
            .unreadable("/locked", Node::Folder);
        let kinds = kinds();
        let follows = [
            follow("/both", None),
            follow("/a", Some("pdf")),
            follow("/a", None),
            follow("/locked", Some("one")),
        ];
        let shapes: Vec<String> = plan(&follows, &kinds, &files).iter().map(shape).collect();
        assert_eq!(
            shapes,
            [
                "failed /both: several kinds claim files under /both: one and two; choose one with --kind",
                "failed /a: unknown kind 'pdf'; kinds: one, two",
                "ready /a one 1",
                "ready /locked one 0",
            ]
        );
    }
}
