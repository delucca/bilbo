//! The pairing mailbox, the relay's one area a device with no manifest entry may write: who opens a nameplate and who
//! writes next, the size and count limits, expiry, and the per-address limits on unsigned requests.

use std::net::IpAddr;

use super::admit;
use super::http::Response;
use super::store::{Data, Staged};

/// A message's place, `pair/<nameplate>/<name>.msg`, as the route read it from the target.
pub struct Message<'a> {
    pub nameplate: &'a str,
    pub name: &'a str,
}

/// The open nameplates, who opened each, and the unsigned requests of each peer address, in memory only.
pub struct Mailbox {}

impl Mailbox {
    /// Counts the open nameplates under `<data>/pair/` and removes those opened 30 minutes or more before `now`.
    pub fn open(data: &Data, now: u64) -> Result<Mailbox, String> {
        let _ = (data, now);
        unimplemented!("stream M")
    }

    /// The checks before a mailbox request's body: who may write (`signer` is the key whose signature, time and
    /// nonce the route verified, `None` when unsigned), the per-address limits of an unsigned request, and room for
    /// a new message. `length` is the `PUT`'s `Content-Length`, `None` for a `GET`.
    pub fn head(
        &self,
        scopes: &admit::Scopes,
        at: &Message,
        signer: Option<&[u8; 32]>,
        peer: IpAddr,
        length: Option<u64>,
        now: u64,
    ) -> Result<(), Response> {
        let _ = (scopes, at, signer, peer, length, now);
        unimplemented!("stream M")
    }

    /// Creates a message whose `head` passed, from its received body: 201, 200 for the same bytes, 409 `exists`.
    pub fn put(
        &self,
        data: &Data,
        at: &Message,
        signer: Option<&[u8; 32]>,
        body: Staged,
        now: u64,
    ) -> Response {
        let _ = (data, at, signer, body, now);
        unimplemented!("stream M")
    }

    /// A message's bytes, or 404 `not-found`, an expired nameplate included.
    pub fn get(&self, data: &Data, at: &Message, now: u64) -> Response {
        let _ = (data, at, now);
        unimplemented!("stream M")
    }

    /// Removes the nameplates opened 30 minutes or more before `now`, and forgets idle peer addresses.
    pub fn sweep(&self, data: &Data, now: u64) {
        let _ = (data, now);
        unimplemented!("stream M")
    }
}
