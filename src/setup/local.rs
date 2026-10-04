//! The local embedder: its plan, the outside world it probes, and `prepare`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::facts::{Facts, timer_place};
use crate::host::{command, model, timer};
use crate::search::embed;
use crate::shared::config::Embedder;

pub const CHECK: Duration = Duration::from_secs(15);

const READY: Duration = Duration::from_secs(120);

/// One step's report line: its status and detail.
pub struct Line {
    pub status: &'static str,
    pub detail: String,
}

/// The model and server lines of the local embedder.
pub struct LocalLines {
    pub model: Line,
    pub server: Line,
}

/// What the local embedder needs, settled before anything is written.
pub struct LocalPlan {
    port: u16,
    /// `http://127.0.0.1:<port>`
    pub url: String,
    pub llama_server: PathBuf,
    pub model: PathBuf,
    /// The model file is not there with the pinned size.
    pub download: bool,
    pub platform: timer::Platform,
    pub tool: PathBuf,
    pub place: timer::Place,
    job: timer::Job,
    pub files: Vec<(PathBuf, String)>,
    pub service: timer::Current,
}

impl LocalPlan {
    pub fn log(&self) -> &Path {
        &self.job.log
    }

    pub fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// The prepare stage has work: a download, or service files to write or reload.
    pub fn prepares(&self) -> bool {
        self.download || self.service != timer::Current::Same
    }
}

/// What the prepare stage and planning reach outside the process; `Net` for real, a script in tests.
pub trait Outside {
    /// Downloads the pinned model to `path`; `progress` gets (bytes done, total).
    fn fetch(&mut self, path: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String>;
    /// Waits for the server at `url` to report ready.
    fn ready(&mut self, url: &str) -> Result<(), String>;
    /// One embed request; the vector length.
    fn check(&mut self, embedder: &Embedder) -> Result<usize, String>;
    /// Whether something accepts a connection on 127.0.0.1:`port` within 1 s.
    fn listening(&mut self, port: u16) -> bool;
}

pub struct Net;

impl Outside for Net {
    fn fetch(&mut self, path: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
        model::download(path, &model::PINNED, model::WINDOW, progress)
    }

    fn ready(&mut self, url: &str) -> Result<(), String> {
        embed::ready(url, READY)
    }

    fn check(&mut self, embedder: &Embedder) -> Result<usize, String> {
        check_embedder(embedder)
    }

    fn listening(&mut self, port: u16) -> bool {
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        std::net::TcpStream::connect_timeout(&address, Duration::from_secs(1)).is_ok()
    }
}

/// The model and server lines of a local embedder that needs no preparing.
pub fn kept_lines(local: &LocalPlan) -> LocalLines {
    LocalLines {
        model: Line {
            status: "kept",
            detail: local.model.display().to_string(),
        },
        server: Line {
            status: "kept",
            detail: local.address(),
        },
    }
}

/// An embed request to a local server that needs no preparing, once it reports ready.
pub fn check_kept(
    local: &LocalPlan,
    embedder: &Embedder,
    outside: &mut impl Outside,
) -> Result<(LocalLines, usize), (LocalLines, String)> {
    match outside
        .ready(&local.url)
        .and_then(|()| outside.check(embedder))
    {
        Ok(dims) => Ok((kept_lines(local), dims)),
        Err(message) => Err((kept_lines(local), message)),
    }
}

/// The platform and its service manager; on Linux a live user session too.
fn local_manager(facts: &Facts) -> Result<(timer::Platform, PathBuf), String> {
    let t = &facts.timer;
    let Some(platform) = t.platform else {
        return Err("the local embedder runs only on macOS and Linux".into());
    };
    let no_session = "the local embedder needs a systemd user session; none answers here";
    match (platform, &t.tool) {
        (timer::Platform::Launchd, None) => {
            Err("the local embedder needs launchctl, which is not on PATH".into())
        }
        (timer::Platform::Systemd, None) => Err(no_session.into()),
        (timer::Platform::Systemd, Some(tool)) if !timer::session(&command::System, tool) => {
            Err(no_session.into())
        }
        (_, Some(tool)) => Ok((platform, tool.clone())),
    }
}

/// The service files' place, the state folder and the model path.
fn local_folders(facts: &Facts) -> Result<(timer::Place, PathBuf, PathBuf), String> {
    let Some(model) = &facts.model else {
        return Err(
            "cannot find the cache folder: set XDG_CACHE_HOME, or HOME, to an absolute path".into(),
        );
    };
    let Some(state_dir) = &facts.timer.state_dir else {
        return Err(
            "cannot find the state folder: set XDG_STATE_HOME, or HOME, to an absolute path".into(),
        );
    };
    let Some(place) = timer_place(&facts.timer) else {
        return Err(match facts.timer.platform {
            Some(timer::Platform::Systemd) => {
                "cannot find the config folder: set XDG_CONFIG_HOME, or HOME, to an absolute path"
            }
            _ => "cannot find the home folder: set HOME to an absolute path",
        }
        .into());
    };
    Ok((place, state_dir.clone(), model.clone()))
}

/// Something answers on the port and no embedder service of ours is installed.
fn port_busy(
    platform: timer::Platform,
    place: &timer::Place,
    port: u16,
    outside: &mut impl Outside,
) -> Option<String> {
    (!timer::installed(platform, place, timer::Name::Embedder) && outside.listening(port)).then(
        || format!("127.0.0.1:{port} is already in use; pick another port with --embedder-port"),
    )
}

/// Why the local embedder cannot run here, or None: platform, service manager (on Linux a live user
/// session), state and home or config folders, and the port (busy and no embedder service file).
pub fn local_unavailable(facts: &Facts, port: u16, outside: &mut impl Outside) -> Option<String> {
    let (platform, _) = match local_manager(facts) {
        Ok(found) => found,
        Err(message) => return Some(message),
    };
    let (place, _, _) = match local_folders(facts) {
        Ok(found) => found,
        Err(message) => return Some(message),
    };
    port_busy(platform, &place, port, outside)
}

/// Settles the local embedder. Reads files, may run `systemctl --user is-system-running`, connects to the port; writes nothing.
pub fn plan_local(
    facts: &Facts,
    port: u16,
    llama_server: Option<PathBuf>,
    outside: &mut impl Outside,
) -> Result<LocalPlan, String> {
    let (platform, tool) = local_manager(facts)?;
    let Some(llama_server) = llama_server else {
        return Err("llama-server not found on PATH; install it (brew install llama.cpp, your distribution's llama.cpp package, or Nix's llama-cpp) or pass --llama-server <path>".into());
    };
    let (place, state_dir, model) = local_folders(facts)?;
    let job = timer::Job {
        kind: timer::Kind::Service,
        program: llama_server.clone(),
        args: model::server_args(&model, port),
        log: state_dir.join("bilbo/embedder.log"),
        env: Vec::new(),
    };
    let files = timer::files(platform, &place, &job)?;
    if let Some(message) = port_busy(platform, &place, port, outside) {
        return Err(message);
    }
    Ok(LocalPlan {
        port,
        url: format!("http://127.0.0.1:{port}"),
        llama_server,
        download: !model::kept(&model, model::PINNED.size),
        model,
        platform,
        tool,
        place,
        service: timer::current(&files),
        job,
        files,
    })
}

/// Download, service, readiness, check, in that order. On failure, unloads and deletes a service
/// this call wrote and returns the two report lines and the message for stderr.
pub fn prepare(
    local: &LocalPlan,
    embedder: &Embedder,
    outside: &mut impl Outside,
) -> Result<(LocalLines, usize), (LocalLines, String)> {
    let line = |status, detail: String| Line { status, detail };
    let model_line = if local.download {
        if let Err(message) = outside.fetch(&local.model, &mut |_, _| {}) {
            return Err((
                LocalLines {
                    model: line("failed", message.clone()),
                    server: line("skipped", "no model".into()),
                },
                message,
            ));
        }
        line("installed", local.model.display().to_string())
    } else {
        line("kept", local.model.display().to_string())
    };
    let wrote = local.service != timer::Current::Same;
    let log = local.log().display();
    let failed = |model: Line, detail: String, message: String| {
        Err((
            LocalLines {
                model,
                server: line("failed", detail),
            },
            message,
        ))
    };
    let loaded = if wrote {
        timer::install(
            local.platform,
            &command::System,
            &local.tool,
            &local.job,
            &local.files,
        )
    } else {
        timer::reload(
            local.platform,
            &command::System,
            &local.tool,
            &local.place,
            timer::Name::Embedder,
        )
    };
    if let Err(message) = loaded {
        return failed(model_line, format!("{message}; see {log}"), message);
    }
    let unload = || {
        if wrote {
            let _ = timer::uninstall(
                local.platform,
                &command::System,
                Some(&local.tool),
                &local.place,
                timer::Name::Embedder,
            );
        }
    };
    if let Err(message) = outside.ready(&local.url) {
        unload();
        return failed(model_line, format!("{message}; see {log}"), message);
    }
    let dims = match outside.check(embedder) {
        Ok(dims) => dims,
        Err(message) => {
            unload();
            let removed = if wrote {
                "; the service was removed"
            } else {
                ""
            };
            return failed(
                model_line,
                format!("{message}{removed}; see {log}"),
                message,
            );
        }
    };
    let status = match local.service {
        timer::Current::Missing => "installed",
        timer::Current::Different => "updated",
        timer::Current::Same => "kept",
    };
    Ok((
        LocalLines {
            model: model_line,
            server: line(status, local.address()),
        },
        dims,
    ))
}

/// One embed request, with the 15 s limit; the vector length, or the message.
pub fn check_embedder(embedder: &Embedder) -> Result<usize, String> {
    let client = embed::Client::new(embedder, |name| std::env::var_os(name), CHECK)?;
    vector_length(&client)
}

pub fn vector_length(client: &embed::Client) -> Result<usize, String> {
    let vectors = client.embed(&["bilbo setup check".to_string()])?;
    Ok(vectors.first().map_or(0, Vec::len))
}

#[cfg(test)]
mod tests {
    use super::super::facts::{ConfigState, TimerFacts};
    use super::super::fakes::*;
    use super::*;

    /// An `Outside` that only answers whether a port is taken.
    struct Taken(bool);

    impl Outside for Taken {
        fn fetch(&mut self, _: &Path, _: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
            Err("fetch is not scripted".into())
        }
        fn ready(&mut self, _: &str) -> Result<(), String> {
            Err("ready is not scripted".into())
        }
        fn check(&mut self, _: &Embedder) -> Result<usize, String> {
            Err("check is not scripted".into())
        }
        fn listening(&mut self, _: u16) -> bool {
            self.0
        }
    }

    /// Facts for a box whose manager answers: a script that exits 0 stands in for `launchctl` or `systemctl`.
    fn local_facts(dir: &Path) -> Facts {
        let tool = dir.join("manager");
        write_script(&tool, LIVE_MANAGER);
        let mut facts = seen(dir, None, ConfigState::Absent);
        facts.timer = TimerFacts {
            platform: timer::platform(),
            home: Some(dir.join("home")),
            config_home: Some(dir.join("home/.config")),
            state_dir: Some(dir.join("state")),
            tool: Some(tool),
            locations: Ok(Vec::new()),
        };
        facts.llama_server = Some(dir.join("llama-server"));
        facts.model = Some(dir.join("cache").join(model::FILE));
        facts
    }

    fn place_model(facts: &Facts, size: u64) {
        let path = facts.model.as_ref().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(path).unwrap().set_len(size).unwrap();
    }

    fn local_plan_of(facts: &Facts) -> LocalPlan {
        plan_local(facts, 9100, facts.llama_server.clone(), &mut Taken(false))
            .ok()
            .unwrap()
    }

    fn write_service(plan: &LocalPlan) {
        for (path, text) in &plan.files {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    fn lines_of(lines: &LocalLines) -> [String; 2] {
        [
            format!("{}: {}", lines.model.status, lines.model.detail),
            format!("{}: {}", lines.server.status, lines.server.detail),
        ]
    }

    #[test]
    fn local_plan_states() {
        let dir = scratch("local-plan");
        let facts = local_facts(&dir);
        let plan = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(false))
            .ok()
            .unwrap();
        assert!(plan.download);
        assert_eq!(plan.service, timer::Current::Missing);
        assert!(plan.prepares());
        assert_eq!(plan.url, "http://127.0.0.1:9100");
        assert_eq!(plan.address(), "127.0.0.1:9100");
        assert_eq!(plan.log(), dir.join("state/bilbo/embedder.log"));
        assert_eq!(plan.job.args, model::server_args(&plan.model, 9100));

        place_model(&facts, model::PINNED.size - 1);
        let short = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(false))
            .ok()
            .unwrap();
        assert!(short.download, "a file of another size is not the model");

        place_model(&facts, model::PINNED.size);
        let placed = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(false))
            .ok()
            .unwrap();
        assert!(!placed.download);
        assert!(placed.prepares(), "the service is missing");

        for (path, text) in &placed.files {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let same = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(true))
            .ok()
            .unwrap();
        assert_eq!(same.service, timer::Current::Same);
        assert!(
            !same.prepares(),
            "bilbo's own server on the port is no conflict"
        );

        std::fs::write(&placed.files[0].0, "changed").unwrap();
        let moved = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(false))
            .ok()
            .unwrap();
        assert_eq!(moved.service, timer::Current::Different);
        assert!(moved.prepares());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_prepare_downloads_then_installs() {
        let dir = scratch("local-prepare-fresh");
        let facts = local_facts(&dir);
        let plan = local_plan_of(&facts);
        let mut outside = Script::working();
        let (lines, dims) = prepare(&plan, &embedder("m"), &mut outside).ok().unwrap();
        assert_eq!(dims, 1024);
        assert_eq!(outside.calls, ["fetch", "ready", "check"]);
        assert_eq!(
            lines_of(&lines),
            [
                format!("installed: {}", plan.model.display()),
                "installed: 127.0.0.1:9100".to_string()
            ]
        );
        assert!(plan.files.iter().all(|(path, _)| path.exists()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_prepare_model_failure() {
        let dir = scratch("local-prepare-model");
        let facts = local_facts(&dir);
        let plan = local_plan_of(&facts);
        let mut outside = Script::working();
        let sha = "the download's SHA-256 is aa, expected bb";
        outside.fetch = Err(sha.into());
        let (lines, message) = prepare(&plan, &embedder("m"), &mut outside).err().unwrap();
        assert_eq!(message, sha);
        assert_eq!(
            lines_of(&lines),
            [format!("failed: {sha}"), "skipped: no model".to_string()]
        );
        assert_eq!(outside.calls, ["fetch"]);
        assert!(plan.files.iter().all(|(path, _)| !path.exists()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_prepare_not_ready_removes_the_service() {
        let dir = scratch("local-prepare-not-ready");
        let facts = local_facts(&dir);
        place_model(&facts, model::PINNED.size);
        let plan = local_plan_of(&facts);
        let mut outside = Script::working();
        outside.ready = Err("embedder http://127.0.0.1:9100 was not ready within 120 s".into());
        let (lines, message) = prepare(&plan, &embedder("m"), &mut outside).err().unwrap();
        assert!(message.contains("was not ready"));
        let [model_line, server] = lines_of(&lines);
        assert_eq!(model_line, format!("kept: {}", plan.model.display()));
        assert!(server.starts_with("failed: embedder http://127.0.0.1:9100 was not ready"));
        assert!(server.ends_with(&format!("; see {}", plan.log().display())));
        assert_eq!(outside.calls, ["ready"]);
        assert!(plan.files.iter().all(|(path, _)| !path.exists()));
        assert!(plan.model.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_prepare_check_failure_removes_the_service() {
        let dir = scratch("local-prepare-check");
        let facts = local_facts(&dir);
        place_model(&facts, model::PINNED.size);
        let plan = local_plan_of(&facts);
        let mut outside = Script::working();
        outside.check = Err("http://127.0.0.1:9100 answered 500".into());
        let (lines, message) = prepare(&plan, &embedder("m"), &mut outside).err().unwrap();
        assert_eq!(message, "http://127.0.0.1:9100 answered 500");
        assert_eq!(
            lines_of(&lines)[1],
            format!(
                "failed: http://127.0.0.1:9100 answered 500; the service was removed; see {}",
                plan.log().display()
            )
        );
        assert!(plan.files.iter().all(|(path, _)| !path.exists()));
        assert!(plan.model.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_prepare_kept_service_is_reloaded_after_a_download() {
        let dir = scratch("local-prepare-kept");
        let facts = local_facts(&dir);
        write_service(&local_plan_of(&facts));
        let plan = local_plan_of(&facts);
        assert!(plan.download);
        assert_eq!(plan.service, timer::Current::Same);
        let mut outside = Script::working();
        let (lines, _) = prepare(&plan, &embedder("m"), &mut outside).ok().unwrap();
        assert_eq!(
            lines_of(&lines),
            [
                format!("installed: {}", plan.model.display()),
                "kept: 127.0.0.1:9100".to_string()
            ]
        );
        assert_eq!(outside.calls, ["fetch", "ready", "check"]);

        let mut outside = Script::working();
        outside.check = Err("http://127.0.0.1:9100 answered 500".into());
        let (lines, _) = prepare(&plan, &embedder("m"), &mut outside).err().unwrap();
        assert_eq!(
            lines_of(&lines)[1],
            format!(
                "failed: http://127.0.0.1:9100 answered 500; see {}",
                plan.log().display()
            )
        );
        assert!(
            plan.files.iter().all(|(path, _)| path.exists()),
            "a service this run did not write stays"
        );
        assert_eq!(timer::current(&plan.files), timer::Current::Same);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_plan_refusals() {
        let dir = scratch("local-plan-refusals");
        let facts = local_facts(&dir);
        let busy = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(true));
        assert_eq!(
            busy.err().unwrap(),
            "127.0.0.1:9100 is already in use; pick another port with --embedder-port"
        );
        let missing = plan_local(&facts, 9100, None, &mut Taken(false));
        assert!(
            missing
                .err()
                .unwrap()
                .starts_with("llama-server not found on PATH;")
        );
        let mut no_cache = local_facts(&dir);
        no_cache.model = None;
        let message = plan_local(
            &no_cache,
            9100,
            no_cache.llama_server.clone(),
            &mut Taken(false),
        );
        assert!(
            message
                .err()
                .unwrap()
                .starts_with("cannot find the cache folder")
        );
        assert!(local_unavailable(&facts, 9100, &mut Taken(true)).is_some());
        assert!(local_unavailable(&facts, 9100, &mut Taken(false)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
