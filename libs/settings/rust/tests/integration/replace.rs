use std::fs;

use bilbo_settings::edit;

use crate::scratch::{Scratch, chmod, mode, names};

#[test]
fn replace_writes_the_text_keeps_the_permissions_and_leaves_no_temp() {
    let scratch = Scratch::new("replace-keeps");
    let folder = scratch.path("bilbo");
    fs::create_dir(&folder).expect("folder");
    let config = folder.join("config.toml");
    fs::write(&config, b"old\n").expect("write");
    chmod(&config, 0o750);
    assert_eq!(edit::replace(&config, b"new\n").expect("replace"), None);
    assert_eq!(fs::read(&config).expect("read"), b"new\n");
    assert_eq!(mode(&config), 0o750);
    assert_eq!(names(&folder), ["config.toml"]);
}

#[test]
fn replace_writes_a_file_that_was_not_there() {
    let scratch = Scratch::new("replace-new");
    let folder = scratch.path("bilbo");
    fs::create_dir(&folder).expect("folder");
    let config = folder.join("config.toml");
    assert_eq!(
        edit::replace(&config, b"[[follow]]\n").expect("replace"),
        None
    );
    assert_eq!(fs::read(&config).expect("read"), b"[[follow]]\n");
    assert_eq!(names(&folder), ["config.toml"]);
}

#[test]
fn a_replace_that_cannot_rename_removes_its_temp_and_leaves_the_target() {
    let scratch = Scratch::new("replace-rename");
    let folder = scratch.path("bilbo");
    let config = folder.join("config.toml");
    fs::create_dir_all(&config).expect("config as a folder");
    fs::write(config.join("kept"), b"").expect("kept");
    assert!(edit::replace(&config, b"new\n").is_err());
    assert_eq!(names(&config), ["kept"]);
    assert_eq!(names(&folder), ["config.toml"]);
}

#[test]
fn a_replace_in_a_folder_it_cannot_write_leaves_the_file_as_it_was() {
    let scratch = Scratch::new("replace-folder");
    let folder = scratch.path("bilbo");
    fs::create_dir(&folder).expect("folder");
    let config = folder.join("config.toml");
    fs::write(&config, b"old\n").expect("write");
    chmod(&folder, 0o555);
    let checked = edit::check(&config);
    let replaced = edit::replace(&config, b"new\n");
    chmod(&folder, 0o755);
    assert_eq!(checked, Ok(true));
    assert!(replaced.is_err());
    assert_eq!(fs::read(&config).expect("read"), b"old\n");
    assert_eq!(names(&folder), ["config.toml"]);
}
