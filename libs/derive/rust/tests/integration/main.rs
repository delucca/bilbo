mod anthropic_api;
mod anthropic_tool;
mod child;
mod http;
mod openai_api;
mod openai_tool;
mod secret;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn untrusted_address() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("an address");
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        if let Ok((mut stream, _)) = listener.accept() {
            let mut hello = [0u8; 16];
            let _ = stream.read(&mut hello);
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n");
        }
    });
    format!("https://{address}")
}

pub fn signalable(target: &str) -> bool {
    std::process::Command::new("/bin/sh")
        .args(["-c", "kill -0 -- \"$1\" 2>/dev/null", "sh", target])
        .status()
        .expect("sh")
        .success()
}

pub fn gone(pid: i32) -> bool {
    for _ in 0..200 {
        if !signalable(&pid.to_string()) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

static COUNT: AtomicUsize = AtomicUsize::new(0);

pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(test: &str) -> Scratch {
        let path = std::env::temp_dir().join(format!(
            "bilbo-derive-{test}-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).expect("a scratch folder");
        Scratch(fs::canonicalize(path).expect("a canonical scratch folder"))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn executable(&self, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = self.script(name, body);
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("a mode");
        script
    }

    pub fn script(&self, name: &str, body: &str) -> PathBuf {
        let script = self.0.join(name);
        fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("a script");
        script
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
