//! Sync: the transport and its segments, the owner's scopes on it, one scope's replica and manifests, applying what
//! other devices wrote to the notes, and the `sync` verb.

pub mod cli;
pub mod integrate;
pub mod manifests;
pub mod replica;
pub mod scopes;
pub mod segment;
pub mod transport;
