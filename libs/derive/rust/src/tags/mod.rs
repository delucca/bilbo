pub mod answer;
pub mod asked;
pub mod block;
pub mod instruction;
pub mod keys;
pub mod plan;
pub mod view;

use asked::Answered;
use bilbo_store::entries::Generation;
use bilbo_store::id::Id;

use crate::view::{Counter, Paragraph};

pub struct Rules<'a> {
    pub section: &'a keys::Section,
    pub instruction: &'a str,
    pub id: &'a str,
    pub model: &'a str,
    pub cap: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub answered: Option<Answered>,
    pub passed: Option<u64>,
    pub added: bool,
    pub notes: Option<u64>,
    pub state: Option<plan::State>,
    pub asking: Option<(u64, u8)>,
    pub sent: usize,
}

pub struct Known<'a> {
    pub id: Id,
    pub generation: Generation,
    pub records: u64,
    pub notes: &'a [Paragraph],
    pub digest: u64,
    pub answered: Option<&'a Answered>,
    pub passed: Option<u64>,
    pub added: bool,
}

pub struct Asking<'a> {
    pub material: String,
    pub about: asked::About,
    pub seen: plan::Seen<'a>,
    pub state: plan::State,
}

#[must_use]
pub fn asking_of<'a>(
    known: &Known<'a>,
    rules: &Rules<'_>,
    provider: &str,
    redo: bool,
    counter: &Counter<'_>,
) -> Asking<'a> {
    let list = &rules.section.pick.value;
    let fit = view::fitting(known.notes, rules.section.notes.value, counter);
    let material = view::material(fit, list, rules.section.most.value);
    let about = asked::About {
        generation: known.generation,
        records: known.records,
        notes: known.digest,
    };
    let number = asked::asking(known.id, &about, provider, rules.model, rules.id, &material);
    let seen = plan::Seen {
        answered: known.answered,
        passed: known.passed,
        added: known.added,
        asking: number,
    };
    let state = plan::state(&seen, list, (rules.id, provider, rules.model), redo);
    Asking {
        material,
        about,
        seen,
        state,
    }
}
