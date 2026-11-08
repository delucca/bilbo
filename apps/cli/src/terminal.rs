//! Probes the real streams and the real environment, once per stream.

use std::io::IsTerminal;

use crate::view::{self, View};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

pub fn open(stream: Stream) -> View {
    let terminal = match stream {
        Stream::Stdout => std::io::stdout().is_terminal(),
        Stream::Stderr => std::io::stderr().is_terminal(),
    };
    view::decide(terminal, cfg!(target_os = "macos"), |name| {
        std::env::var_os(name)
    })
}
