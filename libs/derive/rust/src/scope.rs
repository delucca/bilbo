use bilbo_store::entries::StoreError;
use bilbo_store::id::Id;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Named {
    pub entries: Vec<Id>,
    pub several: Vec<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Untrusted {
    NoIndex,
    OtherBilbo,
    Behind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    Untrusted(Untrusted),
    NotCurrent(Vec<Id>),
}

pub trait Scope {
    fn named(&self, values: &[String]) -> Result<Named, StoreError>;
    fn standing(&self) -> Result<Standing, StoreError>;
}
