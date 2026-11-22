pub mod answer;
pub mod asked;
pub mod block;
pub mod instruction;
pub mod keys;
pub mod plan;
pub mod view;

use asked::Answered;

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
