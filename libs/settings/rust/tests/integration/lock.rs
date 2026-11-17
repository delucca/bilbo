use std::fs::{self, TryLockError};

use bilbo_settings::edit;

use crate::scratch::{Scratch, mode};

#[test]
fn the_lock_file_is_private_beside_the_config_and_outlives_its_handle() {
    let scratch = Scratch::new("lock-private");
    let config = scratch.path("bilbo/config.toml");
    drop(edit::lock_file(&config).expect("lock file"));
    let control = scratch.path("bilbo/control");
    fs::write(&control, b"").expect("control");
    assert_ne!(
        mode(&control),
        0o600,
        "the umask alone would make the lock private"
    );
    assert_eq!(mode(&scratch.path("bilbo/config.toml.lock")), 0o600);
    assert!(!config.exists());
}

#[test]
fn one_writer_holding_the_lock_file_keeps_another_out() {
    let scratch = Scratch::new("lock-turns");
    let config = scratch.path("bilbo/config.toml");
    let first = edit::lock_file(&config).expect("first");
    first.lock().expect("lock");
    let second = edit::lock_file(&config).expect("second");
    assert!(matches!(second.try_lock(), Err(TryLockError::WouldBlock)));
    first.unlock().expect("unlock");
    second.try_lock().expect("second takes its turn");
}
