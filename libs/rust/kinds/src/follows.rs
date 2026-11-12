use std::io;
use std::path::PathBuf;

use bilbo_store::entries::Entries;

use crate::files::Files;
use crate::kind::Kind;
use crate::remember::{self, Error, Output, Plan, Request};

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
    let mut outputs = Vec::new();
    for planned in planned {
        if let Planned::Ready(_, plan) = planned {
            outputs.push(remember::keep(plan, files, entries)?);
        }
    }
    Ok(outputs)
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
                plan.files.len()
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
