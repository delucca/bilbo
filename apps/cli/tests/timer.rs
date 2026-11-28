use std::fs;
use std::path::PathBuf;

const PLIST: &str =
    include_str!("../../../docs/reference/timer/io.github.delucca.bilbo.refresh.plist");
const SERVICE: &str = include_str!("../../../docs/reference/timer/bilbo-refresh.service");
const TIMER: &str = include_str!("../../../docs/reference/timer/bilbo-refresh.timer");
const DERIVE_PLIST: &str =
    include_str!("../../../docs/reference/timer/io.github.delucca.bilbo.derive.plist");
const DERIVE_SERVICE: &str = include_str!("../../../docs/reference/timer/bilbo-derive.service");
const DERIVE_TIMER: &str = include_str!("../../../docs/reference/timer/bilbo-derive.timer");
const COMMANDS: &str = include_str!("../../../docs/reference/commands.md");

fn plist_value<'a>(name: &str) -> Vec<&'a str> {
    value_in(PLIST, name)
}

fn value_in<'a>(plist: &'a str, name: &str) -> Vec<&'a str> {
    let key = format!("<key>{name}</key>");
    let mut lines = plist.lines().skip_while(|line| line.trim() != key).skip(1);
    let first = lines.next().unwrap_or_default().trim();
    if first == "<array>" {
        lines
            .take_while(|line| line.trim() != "</array>")
            .map(|line| {
                line.trim()
                    .trim_start_matches("<string>")
                    .trim_end_matches("</string>")
            })
            .collect()
    } else {
        vec![first]
    }
}

#[test]
fn the_launchd_job_runs_refresh_every_10_minutes() {
    assert_eq!(
        plist_value("Label"),
        ["<string>io.github.delucca.bilbo.refresh</string>"]
    );
    assert_eq!(
        plist_value("ProgramArguments"),
        ["/opt/bilbo/bin/bilbo", "refresh"]
    );
    assert_eq!(plist_value("StartInterval"), ["<integer>600</integer>"]);
    assert_eq!(plist_value("RunAtLoad"), ["<false/>"]);
    for line in [
        "<key>HOME</key>",
        "<string>/home/me</string>",
        "<string>/home/me/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>",
    ] {
        assert!(PLIST.lines().any(|found| found.trim() == line), "{line}");
    }
    assert_eq!(
        plist_value("StandardErrorPath"),
        ["<string>/home/me/.local/state/bilbo/timer/refresh.log</string>"]
    );
    assert_eq!(
        plist_value("StandardOutPath"),
        ["<string>/home/me/.local/state/bilbo/timer/refresh.log</string>"]
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel gives each test its scratch folder in TEST_TMPDIR"
)]
fn scratch() -> PathBuf {
    std::env::var_os("TEST_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

#[cfg(target_os = "macos")]
#[test]
fn the_launchd_job_is_a_property_list_plutil_accepts() {
    let path = scratch().join(format!("refresh-{}.plist", std::process::id()));
    fs::write(&path, PLIST).expect("write the plist");
    let output = std::process::Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(&path)
        .output()
        .expect("plutil runs");
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn the_systemd_timer_starts_the_oneshot_service_every_10_minutes() {
    let lines = |text: &'static str| text.lines().collect::<Vec<_>>();
    let service = lines(SERVICE);
    for line in [
        "[Service]",
        "Type=oneshot",
        "Environment=\"HOME=/home/me\"",
        "Environment=\"PATH=/home/me/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin\"",
        "ExecStart=\"/opt/bilbo/bin/bilbo\" refresh",
        "StandardOutput=append:/home/me/.local/state/bilbo/timer/refresh.log",
        "StandardError=append:/home/me/.local/state/bilbo/timer/refresh.log",
    ] {
        assert!(service.contains(&line), "{line}");
    }
    let timer = lines(TIMER);
    for line in [
        "[Timer]",
        "OnActiveSec=10min",
        "OnUnitActiveSec=10min",
        "[Install]",
        "WantedBy=timers.target",
    ] {
        assert!(timer.contains(&line), "{line}");
    }
}

#[test]
fn the_reference_names_each_unit_setup_writes() {
    let section = COMMANDS
        .split("\n## setup\n")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("a setup section");
    for needle in [
        "bilbo setup",
        "io.github.delucca.bilbo.refresh.plist",
        "io.github.delucca.bilbo.derive.plist",
        "bilbo-refresh.service",
        "bilbo-refresh.timer",
        "bilbo-derive.service",
        "bilbo-derive.timer",
    ] {
        assert!(section.contains(needle), "{needle}");
    }
}

#[test]
fn the_launchd_job_runs_a_bare_derive_every_30_minutes() {
    assert_eq!(
        value_in(DERIVE_PLIST, "Label"),
        ["<string>io.github.delucca.bilbo.derive</string>"]
    );
    assert_eq!(
        value_in(DERIVE_PLIST, "ProgramArguments"),
        ["/opt/bilbo/bin/bilbo", "derive"]
    );
    assert_eq!(
        value_in(DERIVE_PLIST, "StartInterval"),
        ["<integer>1800</integer>"]
    );
    assert_eq!(value_in(DERIVE_PLIST, "RunAtLoad"), ["<false/>"]);
    for line in [
        "<key>HOME</key>",
        "<string>/home/me</string>",
        "<string>/home/me/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>",
    ] {
        assert!(
            DERIVE_PLIST.lines().any(|found| found.trim() == line),
            "{line}"
        );
    }
    assert_eq!(
        value_in(DERIVE_PLIST, "StandardErrorPath"),
        ["<string>/home/me/.local/state/bilbo/timer/derive.log</string>"]
    );
    assert_eq!(
        value_in(DERIVE_PLIST, "StandardOutPath"),
        ["<string>/home/me/.local/state/bilbo/timer/derive.log</string>"]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_derive_launchd_job_is_a_property_list_plutil_accepts() {
    let path = scratch().join(format!("derive-{}.plist", std::process::id()));
    fs::write(&path, DERIVE_PLIST).expect("write the plist");
    let output = std::process::Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(&path)
        .output()
        .expect("plutil runs");
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn the_systemd_timer_starts_the_derive_service_every_30_minutes() {
    let lines = |text: &'static str| text.lines().collect::<Vec<_>>();
    let service = lines(DERIVE_SERVICE);
    for line in [
        "[Service]",
        "Type=oneshot",
        "Environment=\"HOME=/home/me\"",
        "Environment=\"PATH=/home/me/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin\"",
        "ExecStart=\"/opt/bilbo/bin/bilbo\" derive",
        "StandardOutput=append:/home/me/.local/state/bilbo/timer/derive.log",
        "StandardError=append:/home/me/.local/state/bilbo/timer/derive.log",
    ] {
        assert!(service.contains(&line), "{line}");
    }
    let timer = lines(DERIVE_TIMER);
    for line in [
        "[Timer]",
        "OnActiveSec=30min",
        "OnUnitActiveSec=30min",
        "[Install]",
        "WantedBy=timers.target",
    ] {
        assert!(timer.contains(&line), "{line}");
    }
}

#[test]
fn the_derive_timers_part_sends_the_keys_to_the_config_file() {
    let section = COMMANDS
        .split("\n## derive\n")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("a derive section");
    let timer = section
        .split("\n### The timer\n")
        .nth(1)
        .and_then(|rest| rest.split("\n### ").next())
        .expect("a timer part");
    for needle in [
        "bilbo setup",
        "Set `llm.*`, `derive.*` and `paths.*` in the config file.",
    ] {
        assert!(timer.contains(needle), "{needle}");
    }
}
