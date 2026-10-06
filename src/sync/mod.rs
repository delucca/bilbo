//! Sync: the transport, its `file://` folder and its relay client, the segments, the owner's scopes on the transport,
//! one scope's replica and manifests, applying what other devices wrote to the notes, and the `sync` verb.

pub mod cli;
pub mod integrate;
pub mod manifests;
pub mod remote;
pub mod replica;
pub mod scopes;
pub mod segment;
pub mod transport;
