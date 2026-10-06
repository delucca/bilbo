//! The relay transport: `https://` URLs, and `http://` to a loopback host, reach a bilbo relay's API under
//! `<url>/v1/`, with every request signed as `sign` says.

pub mod sign;
