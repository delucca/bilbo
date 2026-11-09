use std::fs;
use std::path::PathBuf;

const PLIST: &str =
    include_str!("../../../docs/reference/timer/io.github.delucca.bilbo.remember.plist");
const SERVICE: &str = include_str!("../../../docs/reference/timer/bilbo-remember.service");
const TIMER: &str = include_str!("../../../docs/reference/timer/bilbo-remember.timer");
const COMMANDS: &str = include_str!("../../../docs/reference/commands.md");

fn plist_value<'a>(name: &str) -> Vec<&'a str> {
    let key = format!("<key>{name}</key>");
    let mut lines = PLIST.lines().skip_while(|line| line.trim() != key).skip(1);
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
fn the_launchd_job_runs_a_bare_remember_every_10_minutes() {
    assert_eq!(
        plist_value("Label"),
        ["<string>io.github.delucca.bilbo.remember</string>"]
    );
    assert_eq!(plist_value("ProgramArguments"), ["@BILBO@", "remember"]);
    assert_eq!(plist_value("StartInterval"), ["<integer>600</integer>"]);
    assert_eq!(plist_value("RunAtLoad"), ["<false/>"]);
    assert_eq!(plist_value("StandardErrorPath"), ["<string>@LOG@</string>"]);
    assert_eq!(plist_value("StandardOutPath"), ["<string>@LOG@</string>"]);
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
    let path = scratch().join(format!("remember-{}.plist", std::process::id()));
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
        "ExecStart=\"@BILBO@\" remember",
        "StandardOutput=append:@LOG@",
        "StandardError=append:@LOG@",
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
fn the_install_steps_name_each_unit_file() {
    for needle in [
        "docs/reference/timer/io.github.delucca.bilbo.remember.plist",
        "launchctl bootstrap",
        "docs/reference/timer/bilbo-remember.service",
        "docs/reference/timer/bilbo-remember.timer",
        "systemctl --user enable --now bilbo-remember.timer",
        "\"command\": \"command -v bilbo >/dev/null 2>&1 || exit 0; bilbo remember --hook; exit 0\"",
    ] {
        assert!(COMMANDS.contains(needle), "{needle}");
    }
}
