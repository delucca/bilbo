use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use bilbo_settings::edit;

use crate::scratch::{Scratch, chmod};

#[test]
fn read_regular_reads_a_file_or_a_link_to_one() {
    let scratch = Scratch::new("read-regular");
    let config = scratch.path("config.toml");
    fs::write(&config, b"[store]\n").expect("write");
    let link = scratch.path("link.toml");
    symlink(&config, &link).expect("link");
    assert_eq!(edit::read_regular(&config).expect("file"), b"[store]\n");
    assert_eq!(edit::read_regular(&link).expect("link"), b"[store]\n");
    let missing = edit::read_regular(&scratch.path("none.toml")).expect_err("missing");
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
}

#[test]
fn read_regular_refuses_a_folder_and_a_device() {
    let scratch = Scratch::new("read-regular-refuses");
    let folder = scratch.path("folder");
    fs::create_dir(&folder).expect("folder");
    for path in [folder, PathBuf::from("/dev/null")] {
        let error = edit::read_regular(&path).expect_err("refused");
        assert_eq!(
            (error.kind(), error.to_string()),
            (io::ErrorKind::InvalidInput, "not a regular file".to_owned())
        );
    }
}

#[test]
fn check_finds_a_writable_file_and_no_file() {
    let scratch = Scratch::new("check-finds");
    let config = scratch.path("config.toml");
    assert_eq!(edit::check(&config), Ok(false));
    assert_eq!(edit::check(&scratch.path("none/config.toml")), Ok(false));
    fs::write(&config, b"").expect("write");
    assert_eq!(edit::check(&config), Ok(true));
}

#[test]
fn check_refuses_a_link_a_folder_and_a_read_only_file_with_the_reason() {
    let scratch = Scratch::new("check-refuses");
    let target = scratch.path("target.toml");
    fs::write(&target, b"").expect("write");
    let link = scratch.path("link.toml");
    symlink(&target, &link).expect("link");
    let dangling = scratch.path("dangling.toml");
    symlink(scratch.path("gone.toml"), &dangling).expect("dangling");
    let folder = scratch.path("folder");
    fs::create_dir(&folder).expect("folder");
    let read_only = scratch.path("read-only.toml");
    fs::write(&read_only, b"").expect("write");
    chmod(&read_only, 0o444);
    for (path, reason) in [
        (link, "it is a symbolic link"),
        (dangling, "it is a symbolic link"),
        (folder, "it is not a regular file"),
        (read_only, "it is read-only"),
    ] {
        assert_eq!(edit::check(&path), Err(reason.to_owned()), "{path:?}");
    }
}

#[test]
fn read_gives_the_bytes_none_for_no_file_and_the_reason_check_gives() {
    let scratch = Scratch::new("read");
    let config = scratch.path("config.toml");
    assert_eq!(edit::read(&config), Ok(Vec::new()));
    fs::write(&config, b"# mine\n").expect("write");
    assert_eq!(edit::read(&config), Ok(b"# mine\n".to_vec()));
    let link = scratch.path("link.toml");
    symlink(&config, &link).expect("link");
    assert_eq!(edit::read(&link), Err("it is a symbolic link".to_owned()));
}
