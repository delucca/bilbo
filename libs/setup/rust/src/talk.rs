use std::io;
use std::path::Path;
use std::time::Instant;

use serde_json::{Value, json};

use crate::program::{Bounds, Lost, Peer};

#[derive(Debug, PartialEq, Eq)]
pub enum Fault {
    NotTaken {
        method: String,
    },
    Silent {
        method: String,
    },
    Ended {
        method: String,
        line: Option<String>,
    },
    Answered {
        method: String,
        message: String,
    },
}

pub trait Calls {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, Fault>;
    fn notice(&mut self, method: &str, params: Option<Value>) -> Result<(), Fault>;
}

pub struct Talk {
    peer: Peer,
    next: u64,
    bounds: Bounds,
}

impl Talk {
    pub fn start(program: &Path, args: &[&str], bounds: Bounds) -> io::Result<Talk> {
        Ok(Talk {
            peer: Peer::start(program, args, bounds)?,
            next: 1,
            bounds,
        })
    }

    pub fn close(self) {
        self.peer.close();
    }
}

fn fault(lost: Lost, method: &str) -> Fault {
    let method = method.to_owned();
    match lost {
        Lost::NotTaken => Fault::NotTaken { method },
        Lost::Silent => Fault::Silent { method },
        Lost::Ended(line) => Fault::Ended { method, line },
    }
}

fn line(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

impl Calls for Talk {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, Fault> {
        let id = self.next;
        self.next += 1;
        let deadline = Instant::now() + self.bounds.wait;
        let bytes = line(&json!({"id": id, "method": method, "params": params}));
        let mut is_reply = |line: &[u8]| {
            serde_json::from_slice::<Value>(line).is_ok_and(|reply| {
                reply.get("id").and_then(Value::as_u64) == Some(id) && reply.get("method").is_none()
            })
        };
        let found = self
            .peer
            .exchange(&bytes, deadline, Some(&mut is_reply))
            .map_err(|lost| fault(lost, method))?;
        let reply = found
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or(Value::Null);
        if let Some(error) = reply.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| error.to_string(), str::to_owned);
            return Err(Fault::Answered {
                method: method.to_owned(),
                message,
            });
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }

    fn notice(&mut self, method: &str, params: Option<Value>) -> Result<(), Fault> {
        let deadline = Instant::now() + self.bounds.wait;
        let body = match params {
            Some(params) => json!({"method": method, "params": params}),
            None => json!({"method": method}),
        };
        self.peer
            .exchange(&line(&body), deadline, None)
            .map(|_| ())
            .map_err(|lost| fault(lost, method))
    }
}
