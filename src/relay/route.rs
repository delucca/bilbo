//! The relay's requests: the target grammar under `/v1/`, the order of checks, signatures and the nonce cache, scope
//! reads, listings and creates, the mailbox's place, and the log lines.

use std::io::Read;

use super::State;
use super::http::{Handler, Head, Request, Response};

/// The nonces of signed requests accepted in the last 600 seconds, per key, at most 100,000.
#[derive(Default)]
pub struct Nonces {}

/// The 4xx refusals not logged one by one, counted by reason until the next minute's line.
#[derive(Default)]
pub struct Refusals {}

impl Handler for State<'_> {
    fn now(&self) -> u64 {
        (self.clock)()
    }

    fn head(&self, request: &Request) -> Head {
        let _ = request;
        unimplemented!("stream K")
    }

    fn answer(&self, request: &Request, body: &mut dyn Read) -> Response {
        let _ = (request, body);
        unimplemented!("stream K")
    }
}

/// Once a minute, logs the refusals counted since the last line, when there were any.
pub fn tick(state: &State, now: u64) {
    let _ = (state, now);
    unimplemented!("stream K")
}
