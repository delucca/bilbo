use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use bilbo_cli::console::settings;
use bilbo_store::layout::Layout;

pub fn layout(vars: &HashMap<String, OsString>) -> Layout {
    settings::load(
        &|name| vars.get(name).cloned(),
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
        &bilbo_cli::provider_keys(),
    )
    .expect("settings")
    .layout()
}

pub fn backdate(state: &Path, minutes: u64) {
    let at = SystemTime::now() - Duration::from_secs(minutes * 60);
    let millis = at
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("after 1970")
        .as_millis();
    fs::write(state.join("open.since"), format!("{millis}\n")).expect("backdate");
}
