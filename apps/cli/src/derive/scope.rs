use bilbo_derive::scope::{Named, Scope, Standing, Untrusted};
use bilbo_search::position::{Manifest, Position};
use bilbo_search::recall::scope::{self as recall_scope, Unnamed};
use bilbo_search::recall::{Stand, stand};
use bilbo_store::entries::{Entries, StoreError};
use bilbo_store::id::Id;
use bilbo_store::labels::Labels;

pub struct Labelled<'a> {
    pub entries: &'a dyn Entries,
    pub labels: &'a dyn Labels,
    pub position: Option<Position>,
    pub manifest: Manifest,
}

impl Labelled<'_> {
    pub fn unnamed(&self, values: &[String]) -> Result<Vec<(String, Unnamed)>, StoreError> {
        Ok(recall_scope::resolve(values, self.entries, self.labels)?.unnamed)
    }
}

impl Scope for Labelled<'_> {
    fn named(&self, values: &[String]) -> Result<Named, StoreError> {
        let found = recall_scope::resolve(values, self.entries, self.labels)?;
        let mut several: Vec<Id> = found
            .unnamed
            .iter()
            .filter_map(|(_, why)| match why {
                Unnamed::Several(ids) => Some(ids.iter().copied()),
                Unnamed::Nothing => None,
            })
            .flatten()
            .collect();
        several.sort();
        several.dedup();
        Ok(Named {
            entries: found.entries,
            several,
        })
    }

    fn standing(&self) -> Result<Standing, StoreError> {
        match stand(self.position.as_ref(), &self.manifest, self.entries)? {
            Stand::NoIndex => return Ok(Standing::Untrusted(Untrusted::NoIndex)),
            Stand::OtherManifest => return Ok(Standing::Untrusted(Untrusted::OtherBilbo)),
            Stand::Current => {}
            Stand::Behind(_) | Stand::Unknown | Stand::Withheld => {
                return Ok(Standing::Untrusted(Untrusted::Behind));
            }
        }
        let mut failed: Vec<Id> = self
            .position
            .iter()
            .flat_map(|found| found.failed.iter().map(|(id, _)| *id))
            .collect();
        failed.sort();
        failed.dedup();
        let mut stale = self.labels.belonging_to(&failed)?;
        stale.extend(failed);
        stale.sort();
        stale.dedup();
        Ok(Standing::NotCurrent(stale))
    }
}
