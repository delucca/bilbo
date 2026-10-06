//! Who may use a scope on the relay: the access rule, scope admission and the manifest chain, seq contiguity, the
//! quotas, and each scope's state behind its own mutex.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::identity::manifest;

/// Whether the relay serves a scope it holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Standing {
    Valid,
    /// Its owner was not passed with `--owner`: 403 `not-admitted`.
    NotAdmitted,
    /// It failed the start-up check, for this reason: 403 `invalid`.
    Invalid(String),
}

/// A scope the relay holds.
pub struct Held {
    /// Its valid versions.
    pub chain: manifest::Scope,
    pub standing: Standing,
    /// The bytes of its stored objects.
    pub bytes: u64,
    /// The bytes of bodies being received, booked against `--max-scope-mb`.
    pub reserved: u64,
    /// Each device's highest seq.
    pub highest: BTreeMap<String, u64>,
}

/// Every scope the relay holds, each behind its own mutex.
pub struct Scopes {
    /// The admitted owners, as `keys::owner_fingerprint` spells them.
    owners: Vec<String>,
    held: Mutex<BTreeMap<String, Arc<Mutex<Held>>>>,
}

impl Scopes {
    pub fn new(owners: &[String], held: BTreeMap<String, Held>) -> Scopes {
        Scopes {
            owners: owners.to_vec(),
            held: Mutex::new(
                held.into_iter()
                    .map(|(id, h)| (id, Arc::new(Mutex::new(h))))
                    .collect(),
            ),
        }
    }

    /// Scope `id`, when the relay holds it.
    pub fn get(&self, id: &str) -> Option<Arc<Mutex<Held>>> {
        self.held
            .lock()
            .expect("the scope map is not poisoned")
            .get(id)
            .cloned()
    }

    /// Whether `key` is the `sign` key of a device that the latest manifest of a valid scope of an admitted owner
    /// lists: who may open a pairing nameplate.
    pub fn enrolled(&self, key: &[u8; 32]) -> bool {
        let _ = (key, &self.owners);
        unimplemented!("stream A")
    }
}
