use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use bilbo_programs::{
    Fail, HOOKS_FILE, MANIFEST_AGY, MARKETPLACE_CLAUDE, MARKETPLACE_CODEX, Programs,
};
use bilbo_setup::agy::{self, Agy};
use bilbo_setup::claude::Claude;
use bilbo_setup::codex::Codex;
use bilbo_setup::plugin::{Agent, step, trust};
use bilbo_setup::program::Bounds;
use bilbo_setup::report::{Line, Status};

use super::{scratch, serial};

const VERSION: &str = "1.2.3";

fn bounds() -> Bounds {
    Bounds {
        wait: Duration::from_secs(15),
        poll: Duration::from_millis(10),
    }
}

struct World {
    folder: PathBuf,
    programs: Programs,
    _keep: Vec<super::Folder>,
}

fn world() -> World {
    let (package, bin, state) = (scratch(), scratch(), scratch());
    bilbo_programs::plugin_folder(&package).expect("a plugin folder");
    let programs = Programs::write(&bin, &state).expect("stand-ins");
    World {
        folder: package.to_path_buf(),
        programs,
        _keep: vec![package, bin, state],
    }
}

impl World {
    fn claude(&self) -> Claude {
        Claude::new(self.programs.path("claude"), bounds())
    }

    fn codex(&self) -> Codex {
        Codex::new(
            self.programs.path("codex"),
            bounds(),
            VERSION.to_owned(),
            "/stand-in/home".to_owned(),
        )
    }

    fn agy(&self) -> Agy {
        Agy::new(self.programs.path("agy"), bounds())
    }

    fn other_folder() -> super::Folder {
        let other = scratch();
        bilbo_programs::plugin_folder(&other).expect("a plugin folder");
        other
    }
}

fn run(agent: &dyn Agent, folder: &Path) -> Line {
    step(agent, folder)
}

fn lines(programs: &Programs, name: &str) -> Vec<String> {
    programs.log(name)
}

fn set_version(world: &World) {
    world
        .programs
        .set("codex", "version", VERSION)
        .expect("a version");
}

fn readable_at_mode_zero(path: &Path) -> bool {
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).expect("a mode");
    let reads = fs::read(path).is_ok();
    fs::set_permissions(path, fs::Permissions::from_mode(0o644)).expect("a mode");
    reads
}

#[test]
fn agys_copy_is_current_only_when_its_files_are_the_folders() {
    let _serial = serial();
    let (copy, folder) = (scratch(), scratch());
    for root in [&copy, &folder] {
        fs::create_dir_all(root.join("sub")).expect("a folder");
        fs::write(root.join("a.txt"), "one").expect("a file");
        fs::write(root.join("sub/b.txt"), "two").expect("a file");
    }
    assert_eq!(agy::same_files(&copy, &folder), Ok(true));
    fs::write(copy.join("sub/b.txt"), "twO").expect("a changed byte");
    assert_eq!(agy::same_files(&copy, &folder), Ok(false));
    fs::write(copy.join("sub/b.txt"), "two").expect("restored");
    fs::write(copy.join("stale.txt"), "x").expect("a file more in the copy");
    assert_eq!(agy::same_files(&copy, &folder), Ok(false));
    fs::remove_file(copy.join("stale.txt")).expect("removed");
    fs::write(folder.join("new.txt"), "x").expect("a file more in the folder");
    assert_eq!(agy::same_files(&copy, &folder), Ok(false));
    fs::remove_file(folder.join("new.txt")).expect("removed");

    if readable_at_mode_zero(&copy.join("a.txt")) {
        return;
    }
    fs::set_permissions(copy.join("a.txt"), fs::Permissions::from_mode(0o000)).expect("a mode");
    let in_copy = agy::same_files(&copy, &folder);
    fs::set_permissions(copy.join("a.txt"), fs::Permissions::from_mode(0o644)).expect("a mode");
    assert_eq!(in_copy, Ok(false), "a copy file setup cannot read differs");
    fs::set_permissions(folder.join("a.txt"), fs::Permissions::from_mode(0o000)).expect("a mode");
    let in_package = agy::same_files(&copy, &folder);
    fs::set_permissions(folder.join("a.txt"), fs::Permissions::from_mode(0o644)).expect("a mode");
    let error = in_package.expect_err("a package file setup cannot read is no answer");
    assert!(
        error.starts_with(&format!("cannot read {}: ", folder.join("a.txt").display())),
        "{error}"
    );
}

#[test]
fn claude_gets_the_marketplace_and_the_plugin_by_its_own_commands() {
    let _serial = serial();
    let world = world();
    let package = world.folder.display().to_string();
    let first = run(&world.claude(), &world.folder);
    assert_eq!(first.status, Status::Installed, "{}", first.detail);
    assert_eq!(first.detail, format!("bilbo@bilbo from {package}"));
    let looks = ["plugin marketplace list --json", "plugin list --json"];
    let mut expected: Vec<String> = looks.iter().map(|word| (*word).to_owned()).collect();
    expected.extend([
        format!("plugin marketplace add {package} --json"),
        "plugin install bilbo@bilbo --json".to_owned(),
    ]);
    expected.extend(looks.iter().map(|word| (*word).to_owned()));
    assert_eq!(lines(&world.programs, "claude"), expected);

    let before = lines(&world.programs, "claude").len();
    let again = run(&world.claude(), &world.folder);
    assert_eq!(again.status, Status::Kept, "{}", again.detail);
    assert_eq!(lines(&world.programs, "claude")[before..], looks);

    let other = World::other_folder();
    let before = lines(&world.programs, "claude").len();
    let moved = run(&world.claude(), &other);
    let other_text = other.display().to_string();
    assert_eq!(moved.status, Status::Updated, "{}", moved.detail);
    assert_eq!(
        moved.detail,
        format!("bilbo@bilbo from {other_text}; its marketplace came from {package}")
    );
    assert_eq!(
        lines(&world.programs, "claude")[before..],
        [
            "plugin marketplace list --json".to_owned(),
            "plugin list --json".to_owned(),
            "plugin uninstall bilbo@bilbo --json".to_owned(),
            "plugin marketplace remove bilbo --json".to_owned(),
            format!("plugin marketplace add {other_text} --json"),
            "plugin install bilbo@bilbo --json".to_owned(),
            "plugin marketplace list --json".to_owned(),
            "plugin list --json".to_owned(),
        ]
    );
}

fn looks(tool: &str) -> [String; 2] {
    [
        format!("{tool} plugin marketplace list --json"),
        format!("{tool} plugin list --json"),
    ]
    .map(|line| line.replacen(&format!("{tool} "), "", 1))
}

#[test]
fn codex_gets_them_and_its_hooks_trust_through_the_app_server() {
    let _serial = serial();
    let world = world();
    set_version(&world);
    world
        .programs
        .seed_trust("user:hooks.json:stop:0:0", "sha256:theirs")
        .expect("a seeded entry");
    let codex = world.codex();
    let package = world.folder.display().to_string();
    let looked = looks("codex");
    let first = run(&codex, &world.folder);
    assert_eq!(first.status, Status::Installed, "{}", first.detail);
    let mut expected: Vec<String> = looked.to_vec();
    expected.extend([
        format!("plugin marketplace add {package} --json"),
        "plugin add bilbo@bilbo --json".to_owned(),
    ]);
    expected.extend(looked.clone());
    assert_eq!(lines(&world.programs, "codex"), expected);

    let before = lines(&world.programs, "codex").len();
    let again = run(&codex, &world.folder);
    assert_eq!(again.status, Status::Kept, "{}", again.detail);
    assert_eq!(lines(&world.programs, "codex")[before..], looked);

    let other = World::other_folder();
    let other_text = other.display().to_string();
    let before = lines(&world.programs, "codex").len();
    let moved = run(&codex, &other);
    assert_eq!(moved.status, Status::Updated, "{}", moved.detail);
    assert_eq!(
        moved.detail,
        format!("bilbo@bilbo from {other_text}; its marketplace came from {package}")
    );
    let mut expected: Vec<String> = looked.to_vec();
    expected.extend([
        "plugin remove bilbo@bilbo --json".to_owned(),
        "plugin marketplace remove bilbo --json".to_owned(),
        format!("plugin marketplace add {other_text} --json"),
        "plugin add bilbo@bilbo --json".to_owned(),
    ]);
    expected.extend(looked.clone());
    assert_eq!(lines(&world.programs, "codex")[before..], expected);
    assert_eq!(run(&codex, &world.folder).status, Status::Updated);

    let mut trusting = codex
        .hooks(&world.folder)
        .expect("codex has hooks to trust");
    assert_eq!(trusting.step, "codex-hooks");
    let before = lines(&world.programs, "codex").len();
    let line = trust(trusting.step, trusting.hooks.as_mut());
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Installed, "Codex trusts 2 hooks of bilbo@bilbo"),
    );
    assert_eq!(
        lines(&world.programs, "codex")[before..],
        [
            "app-server",
            "rpc initialize",
            "rpc initialized",
            "rpc hooks/list",
            "rpc config/batchWrite",
            "rpc hooks/list"
        ]
    );
    let kept = world.programs.trust();
    assert_eq!(
        kept[0],
        (
            "user:hooks.json:stop:0:0".to_owned(),
            "sha256:theirs".to_owned()
        )
    );
    assert_eq!(kept.len(), 3);
    assert!(
        kept.iter()
            .any(|(key, hash)| key.ends_with("user_prompt_submit:0:0")
                && hash == &format!("sha256:{}", Programs::hash_of("UserPromptSubmit")))
    );

    let before = lines(&world.programs, "codex").len();
    let mut again = codex.hooks(&world.folder).expect("hooks");
    let line = trust(again.step, again.hooks.as_mut());
    assert_eq!(line.status, Status::Kept, "{}", line.detail);
    assert_eq!(
        lines(&world.programs, "codex")[before..],
        [
            "app-server",
            "rpc initialize",
            "rpc initialized",
            "rpc hooks/list"
        ]
    );

    let hooks_file = world.folder.join(HOOKS_FILE);
    fs::write(&hooks_file, "not json").expect("a hooks file that is no JSON");
    let before = lines(&world.programs, "codex").len();
    let mut broken = codex.hooks(&world.folder).expect("hooks");
    let line = trust(broken.step, broken.hooks.as_mut());
    assert_eq!(
        (line.status, line.detail),
        (
            Status::Failed,
            format!(
                "cannot read {}: it is not the hooks file bilbo reads",
                hooks_file.display()
            )
        )
    );
    assert_eq!(
        lines(&world.programs, "codex").len(),
        before,
        "no line, app-server included"
    );
    fs::remove_file(&hooks_file).expect("a plugin with no hooks file");
    let mut missing = codex.hooks(&world.folder).expect("hooks");
    let line = trust(missing.step, missing.hooks.as_mut());
    assert_eq!(line.status, Status::Failed);
    assert!(
        line.detail
            .starts_with(&format!("cannot read {}", hooks_file.display())),
        "{}",
        line.detail
    );
    assert_eq!(lines(&world.programs, "codex").len(), before);

    let second = self::world();
    second
        .programs
        .set("codex", "installed", "on")
        .expect("installed");
    set_version(&second);
    let prompt_key = "bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0";
    second
        .programs
        .seed_trust("other:hook", "sha256:other")
        .expect("seeded");
    second
        .programs
        .seed_trust(prompt_key, "sha256:old")
        .expect("seeded");
    let mut trusting = second.codex().hooks(&second.folder).expect("hooks");
    let line = trust(trusting.step, trusting.hooks.as_mut());
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Updated, "Codex trusts 2 hooks of bilbo@bilbo")
    );
    let table = second.programs.trust();
    assert_eq!(table.len(), 3, "{table:?}");
    assert_eq!(
        table[0],
        ("other:hook".to_owned(), "sha256:other".to_owned())
    );
    for key in [prompt_key, "bilbo@bilbo:hooks/hooks.json:session_start:0:0"] {
        let held: Vec<&(String, String)> = table.iter().filter(|(held, _)| held == key).collect();
        assert_eq!(held.len(), 1, "{key}: {table:?}");
        assert!(held[0].1.starts_with("sha256:") && held[0].1 != "sha256:old");
    }
}

#[test]
fn agy_gets_one_copy_of_the_plugin() {
    let _serial = serial();
    let world = world();
    let package = world.folder.display().to_string();
    let copy_of = format!("{package}/plugins/bilbo");
    let first = run(&world.agy(), &world.folder);
    assert_eq!(first.status, Status::Installed, "{}", first.detail);
    assert_eq!(
        lines(&world.programs, "agy"),
        [
            "plugin list --json".to_owned(),
            format!("plugin install {copy_of}"),
            "plugin list --json".to_owned()
        ]
    );
    let stale = scratch();
    fs::create_dir_all(stale.join("skills")).expect("a folder");
    fs::write(stale.join("old.txt"), "old").expect("an old file");
    world
        .programs
        .set("agy", "copyat", stale.to_str().expect("UTF-8"))
        .expect("a copy that differs");
    let before = lines(&world.programs, "agy").len();
    let changed = run(&world.agy(), &world.folder);
    assert_eq!(changed.status, Status::Updated, "{}", changed.detail);
    assert!(
        changed.detail.ends_with("; its copy differed"),
        "{}",
        changed.detail
    );
    assert_eq!(
        lines(&world.programs, "agy")[before..],
        [
            "plugin list --json".to_owned(),
            "plugin uninstall bilbo".to_owned(),
            format!("plugin install {copy_of}"),
            "plugin list --json".to_owned()
        ]
    );
    let before = lines(&world.programs, "agy").len();
    let again = run(&world.agy(), &world.folder);
    assert_eq!(again.status, Status::Kept, "{}", again.detail);
    assert_eq!(
        lines(&world.programs, "agy")[before..],
        ["plugin list --json"]
    );
}

#[test]
fn a_plugin_turned_off_is_taken_out_and_put_back() {
    let _serial = serial();
    let world = world();
    set_version(&world);
    for name in ["claude", "codex", "agy"] {
        let agent: Box<dyn Agent> = match name {
            "claude" => Box::new(world.claude()),
            "codex" => Box::new(world.codex()),
            _ => Box::new(world.agy()),
        };
        assert_eq!(
            run(agent.as_ref(), &world.folder).status,
            Status::Installed,
            "{name}"
        );
        match name {
            "claude" => world.programs.set("claude", "plugin", "off"),
            "codex" => world.programs.set("codex", "installed", "off"),
            _ => world.programs.flag("agy", "off"),
        }
        .expect("turned off");
        let before = lines(&world.programs, name).len();
        let line = run(agent.as_ref(), &world.folder);
        assert_eq!(line.status, Status::Updated, "{name}: {}", line.detail);
        assert!(
            line.detail.ends_with("; it was turned off"),
            "{name}: {}",
            line.detail
        );
        let commands: Vec<String> = lines(&world.programs, name)[before..]
            .iter()
            .filter(|line| {
                line.contains("install") || line.contains("remove") || line.contains(" add ")
            })
            .cloned()
            .collect();
        assert_eq!(commands.len(), 2, "{name}: {commands:?}");
        assert_eq!(
            run(agent.as_ref(), &world.folder).status,
            Status::Kept,
            "{name}"
        );
    }
}

fn rewrite(folder: &Path, relative: &str, from: &str, to: &str) {
    let path = folder.join(relative);
    let text = fs::read_to_string(&path).expect("a file");
    assert!(text.contains(from), "{relative}");
    fs::write(path, text.replace(from, to)).expect("rewritten");
}

#[test]
fn a_folder_of_another_marketplace_is_never_added_and_takes_nothing_out() {
    let _serial = serial();
    let world = world();
    set_version(&world);
    let agents: Vec<(&str, Box<dyn Agent>)> = vec![
        ("claude", Box::new(world.claude())),
        ("codex", Box::new(world.codex())),
        ("agy", Box::new(world.agy())),
    ];
    for (_, agent) in &agents {
        assert_eq!(run(agent.as_ref(), &world.folder).status, Status::Installed);
    }
    let personal = World::other_folder();
    rewrite(
        &personal,
        MARKETPLACE_CLAUDE,
        "\"name\": \"bilbo\",\n  \"plugins\"",
        "\"name\": \"personal\",\n  \"plugins\"",
    );
    rewrite(
        &personal,
        MARKETPLACE_CODEX,
        "\"name\": \"bilbo\",\n  \"plugins\"",
        "\"name\": \"personal\",\n  \"plugins\"",
    );
    rewrite(&personal, MANIFEST_AGY, "\"bilbo\"", "\"personal\"");
    let folder = personal.display().to_string();
    for (name, agent) in &agents {
        let before = lines(&world.programs, name);
        let line = run(agent.as_ref(), &personal);
        assert_eq!(line.status, Status::Failed, "{name}");
        let (file, what) = match *name {
            "claude" => (MARKETPLACE_CLAUDE, "marketplace"),
            "codex" => (MARKETPLACE_CODEX, "marketplace"),
            _ => (MANIFEST_AGY, "plugin"),
        };
        assert_eq!(
            line.detail,
            format!("no plugin at {folder}: {file} names the {what} 'personal', not bilbo")
        );
        assert_eq!(
            lines(&world.programs, name),
            before,
            "{name}: no command was run"
        );
    }
    let from = |name: &str, key: &str| world.programs.get(name, key).unwrap_or_default();
    let first = world.folder.display().to_string();
    assert_eq!(from("claude", "from"), format!("{first}/plugins/bilbo"));
    assert_eq!(from("codex", "from"), format!("{first}/plugins/bilbo"));
    assert_eq!(from("agy", "copy"), format!("{first}/plugins/bilbo"));

    let depends = World::other_folder();
    rewrite(
        &depends,
        MARKETPLACE_CLAUDE,
        "\"source\": \"./plugins/bilbo\"",
        "\"source\": \"./plugins/bilbo\",\n      \"dependencies\": [\"extra\"]",
    );
    let empty = world_with_nothing();
    let line = run(&empty.claude(), &depends);
    assert_eq!(line.status, Status::Failed);
    assert!(
        line.detail.ends_with(
            "makes the plugin bilbo depend on other plugins, which bilbo setup installs none of"
        ),
        "{}",
        line.detail
    );
    assert!(lines(&empty.programs, "claude").is_empty());

    let bringing = world_with_nothing();
    bringing
        .programs
        .set("claude", "bring", "other@bilbo")
        .expect("bring");
    let line = run(&bringing.claude(), &bringing.folder);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            "claude lists other@bilbo, also from the marketplace bilbo after plugin install bilbo@bilbo --json"
        )
    );
}

fn world_with_nothing() -> World {
    world()
}

fn fifo_at(path: &Path) {
    fs::remove_file(path).expect("the file to replace");
    let made = Command::new("mkfifo").arg(path).status().expect("mkfifo");
    assert!(made.success());
}

#[test]
fn a_manifest_that_is_a_pipe_fails_the_step_and_holds_nothing_up() {
    let _serial = serial();
    for (name, file) in [
        ("claude", MARKETPLACE_CLAUDE),
        ("codex", MARKETPLACE_CODEX),
        ("agy", MANIFEST_AGY),
    ] {
        let world = world();
        fifo_at(&world.folder.join(file));
        let agent: Box<dyn Agent> = match name {
            "claude" => Box::new(world.claude()),
            "codex" => Box::new(world.codex()),
            _ => Box::new(world.agy()),
        };
        let line = run(agent.as_ref(), &world.folder);
        assert_eq!(
            (line.status, line.detail),
            (
                Status::Failed,
                format!(
                    "cannot read {}: not a regular file",
                    world.folder.join(file).display()
                )
            ),
            "{name}"
        );
        assert!(
            lines(&world.programs, name).is_empty(),
            "{name}: no program was started"
        );
    }
    let world = world();
    fifo_at(&world.folder.join(HOOKS_FILE));
    let mut trusting = world.codex().hooks(&world.folder).expect("hooks");
    let line = trust(trusting.step, trusting.hooks.as_mut());
    assert_eq!(
        (line.status, line.detail),
        (
            Status::Failed,
            format!(
                "cannot read {}: not a regular file",
                world.folder.join(HOOKS_FILE).display()
            )
        )
    );
    assert!(lines(&world.programs, "codex").is_empty());

    let world = self::world();
    let skill = world.folder.join("plugins/bilbo/skills/recall/SKILL.md");
    fifo_at(&skill);
    let line = run(&world.agy(), &world.folder);
    assert_eq!(
        (line.status, line.detail),
        (
            Status::Failed,
            format!("cannot read {}: not a regular file", skill.display())
        )
    );
    assert!(lines(&world.programs, "agy").is_empty());

    let (copy, folder) = (scratch(), scratch());
    fs::write(copy.join("a.txt"), "x").expect("a file");
    fs::write(folder.join("a.txt"), "x").expect("a file");
    assert_eq!(agy::same_files(&copy, &folder), Ok(true));
    fifo_at(&copy.join("a.txt"));
    assert_eq!(agy::same_files(&copy, &folder), Ok(false));
}

#[test]
fn a_failed_command_of_a_stand_in_names_the_command_and_its_line() {
    let _serial = serial();
    let world = world();
    world
        .programs
        .fail("claude", "install", Fail::Exit("no space left"), false)
        .expect("marked");
    let line = run(&world.claude(), &world.folder);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            "claude plugin install bilbo@bilbo --json: no space left"
        )
    );
}

fn agent_of(world: &World, name: &str) -> Box<dyn Agent> {
    match name {
        "claude" => Box::new(world.claude()),
        "codex" => Box::new(world.codex()),
        _ => Box::new(world.agy()),
    }
}

#[test]
fn a_listed_folder_is_the_packages_only_when_both_are_made_canonical() {
    let _serial = serial();
    for name in ["claude", "codex"] {
        let world = world();
        set_version(&world);
        let links = scratch();
        let link = links.join("link");
        symlink(&world.folder, &link).expect("a link to the package");
        let agent = agent_of(&world, name);
        let first = run(agent.as_ref(), &link);
        assert_eq!(first.status, Status::Installed, "{name}: {}", first.detail);
        let before = lines(&world.programs, name).len();
        let second = run(agent.as_ref(), &world.folder);
        assert_eq!(second.status, Status::Kept, "{name}: {}", second.detail);
        assert_eq!(
            lines(&world.programs, name).len(),
            before + 2,
            "{name}: the two lists alone"
        );
        fs::remove_dir_all(world.folder.join("plugins/bilbo")).expect("the plugin is gone");
        let before = lines(&world.programs, name).len();
        let third = run(agent.as_ref(), &world.folder);
        assert_eq!(third.status, Status::Failed, "{name}");
        assert!(
            third.detail.starts_with(&format!(
                "cannot read {}: ",
                world.folder.join("plugins/bilbo").display()
            )),
            "{name}: {}",
            third.detail
        );
        assert_eq!(
            lines(&world.programs, name).len(),
            before,
            "{name}: no line"
        );
    }
}

#[test]
fn an_agy_package_that_cannot_be_read_uninstalls_nothing() {
    let _serial = serial();
    let world = world();
    assert_eq!(run(&world.agy(), &world.folder).status, Status::Installed);
    let file = world.folder.join("plugins/bilbo/skills/recall/SKILL.md");
    if readable_at_mode_zero(&file) {
        return;
    }
    let before = lines(&world.programs, "agy").len();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).expect("a mode");
    let line = run(&world.agy(), &world.folder);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).expect("a mode");
    assert_eq!(line.status, Status::Failed, "{}", line.detail);
    assert!(
        line.detail
            .starts_with(&format!("cannot read {}: ", file.display())),
        "{}",
        line.detail
    );
    assert_eq!(
        lines(&world.programs, "agy").len(),
        before,
        "no line, uninstall least"
    );
}

#[test]
fn a_command_that_fails_after_its_work_is_failed_and_the_next_run_keeps_it() {
    let _serial = serial();
    for (name, command) in [("claude", "install"), ("codex", "add"), ("agy", "install")] {
        let world = world();
        set_version(&world);
        world
            .programs
            .fail(name, command, Fail::Exit("no space left"), true)
            .expect("marked");
        let agent = agent_of(&world, name);
        let first = run(agent.as_ref(), &world.folder);
        let package = world.folder.display();
        let sentence = match name {
            "claude" => "claude plugin install bilbo@bilbo --json: no space left".to_owned(),
            "codex" => "codex plugin add bilbo@bilbo --json: no space left".to_owned(),
            _ => format!("agy plugin install {package}/plugins/bilbo: no space left"),
        };
        assert_eq!(
            (first.status, first.detail),
            (Status::Failed, sentence),
            "{name}"
        );
        world.programs.unfail(name, command).expect("taken away");
        let before = lines(&world.programs, name).len();
        let second = run(agent.as_ref(), &world.folder);
        assert_eq!(second.status, Status::Kept, "{name}: {}", second.detail);
        let rest = &lines(&world.programs, name)[before..];
        assert!(
            rest.iter().all(|line| line.contains("list")),
            "{name}: the lists alone: {rest:?}"
        );
    }
}

#[test]
fn a_package_folder_with_a_quote_and_a_backslash_goes_through_each_list() {
    let _serial = serial();
    for name in ["claude", "codex", "agy"] {
        let world = world();
        set_version(&world);
        let holder = scratch();
        let odd = holder.join("q\"b\\s\tt");
        fs::create_dir(&odd).expect("a folder with a quote, a backslash and a tab");
        bilbo_programs::plugin_folder(&odd).expect("a plugin folder");
        let agent = agent_of(&world, name);
        let first = run(agent.as_ref(), &odd);
        assert_eq!(first.status, Status::Installed, "{name}: {}", first.detail);
        assert!(
            first.detail.contains(&odd.display().to_string()),
            "{name}: {}",
            first.detail
        );
        let second = run(agent.as_ref(), &odd);
        assert_eq!(second.status, Status::Kept, "{name}: {}", second.detail);
    }
}

#[test]
fn a_list_that_cannot_be_read_is_said_before_the_next_list_runs() {
    let _serial = serial();
    for name in ["claude", "codex"] {
        let world = world();
        world
            .programs
            .print(name, "market-list", "not json")
            .expect("a list that is no JSON");
        world
            .programs
            .fail(name, "plist", Fail::Exit("late"), false)
            .expect("marked");
        let agent = agent_of(&world, name);
        let line = run(agent.as_ref(), &world.folder);
        assert_eq!(
            (line.status, line.detail),
            (
                Status::Failed,
                format!(
                    "cannot read {name} plugin marketplace list --json: its output is not the JSON bilbo reads"
                )
            ),
            "{name}"
        );
        assert_eq!(
            lines(&world.programs, name),
            ["plugin marketplace list --json"],
            "{name}"
        );
    }
}

#[test]
fn a_package_whose_plugin_is_a_file_starts_no_program() {
    let _serial = serial();
    for (name, installed) in [("claude", "plugin"), ("codex", "installed")] {
        let broken = scratch();
        bilbo_programs::plugin_folder(&broken).expect("a plugin folder");
        fs::remove_dir_all(broken.join("plugins/bilbo")).expect("the plugin folder is gone");
        fs::write(broken.join("plugins/bilbo"), "not a folder\n").expect("a regular file");
        let sentence = format!(
            "cannot read {}: not a folder",
            broken.join("plugins/bilbo").display()
        );

        let file = scratch();
        bilbo_programs::plugin_folder(&file).expect("a plugin folder");
        let flat = file.join("package");
        fs::write(&flat, "not a folder\n").expect("a regular file");
        let world_of_file = world();
        set_version(&world_of_file);
        let agent = agent_of(&world_of_file, name);
        let line = run(agent.as_ref(), &flat);
        assert_eq!(
            (line.status, line.detail.as_str()),
            (
                Status::Failed,
                format!("cannot read {}: not a folder", flat.display()).as_str()
            ),
            "{name}: a package folder that is a file"
        );
        assert!(lines(&world_of_file.programs, name).is_empty(), "{name}");

        let fresh = world();
        set_version(&fresh);
        let agent = agent_of(&fresh, name);
        let line = run(agent.as_ref(), &broken);
        assert_eq!(
            (line.status, line.detail.as_str()),
            (Status::Failed, sentence.as_str()),
            "{name}"
        );
        assert!(lines(&fresh.programs, name).is_empty(), "{name}");

        let held = world();
        set_version(&held);
        let agent = agent_of(&held, name);
        let first = run(agent.as_ref(), &held.folder);
        assert_eq!(first.status, Status::Installed, "{name}: {}", first.detail);
        let before = lines(&held.programs, name);
        let state = |world: &World| {
            (
                world.programs.get(name, installed),
                world.programs.get(name, "from"),
                world.programs.get(name, "market"),
            )
        };
        let was = state(&held);
        let line = run(agent.as_ref(), &broken);
        assert_eq!(
            (line.status, line.detail.as_str()),
            (Status::Failed, sentence.as_str()),
            "{name}"
        );
        assert_eq!(lines(&held.programs, name), before, "{name}: no line");
        assert_eq!(state(&held), was, "{name}: the first plugin stands");
        assert_eq!(was.0.as_deref(), Some("on"), "{name}");
    }
}

#[test]
fn a_trust_key_is_one_component_of_the_path_whatever_it_holds() {
    let _serial = serial();
    let world = world();
    set_version(&world);
    let odd = [
        "bilbo@bilbo:with \"quote\"",
        "bilbo@bilbo:back\\slash",
        "bilbo@bilbo:a.b.c",
    ];
    for key in odd {
        world
            .programs
            .seed_trust(key, "sha256:x")
            .expect("a seeded entry");
    }
    world
        .programs
        .seed_trust("bilbo@other:hook", "sha256:other")
        .expect("another plugin's");
    world
        .programs
        .seed_trust("user:hooks.json:stop:0:0", "sha256:theirs")
        .expect("the user's");
    let mut trusting = world
        .codex()
        .hooks(&world.folder)
        .expect("codex has hooks to trust");
    let line = bilbo_setup::plugin::forget(trusting.step, trusting.hooks.as_mut());
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Removed, "3 trust entries of bilbo@bilbo"),
        "{}",
        line.detail
    );
    assert_eq!(
        world.programs.trust(),
        [
            ("bilbo@other:hook".to_owned(), "sha256:other".to_owned()),
            (
                "user:hooks.json:stop:0:0".to_owned(),
                "sha256:theirs".to_owned()
            ),
        ]
    );
    let mut again = world
        .codex()
        .hooks(&world.folder)
        .expect("codex has hooks to trust");
    let line = bilbo_setup::plugin::forget(again.step, again.hooks.as_mut());
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Absent, "Codex keeps no trust of bilbo@bilbo")
    );
}
