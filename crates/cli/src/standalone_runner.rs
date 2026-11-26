//! Install a verified canary payload and let launchd own its lifecycle.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Write};
use std::os::unix::{fs::symlink, process::CommandExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use farcooler_daemon_contracts::standalone_macos::{INSTALL_LOCK, LABEL, install_root};
use farcooler_daemon::landing_update;
use farcooler_protocol::{BUILD, CHANNEL, Channel, build_identity::same_build};
use farcooler_transport::ClientError;

use crate::Fallible;
use crate::daemon_link::{self, Link};

const LOGIN_LABEL: &str = "com.farcooler.daemon.canary.bootstrap";
const APP_LABEL: &str = "com.farcooler.daemon.canary";
const APP_ID: &str = "com.farcooler.FarCooler.canary";
const CLI: &str = "farcooler-canary";
const DAEMON: &str = "farcoolerd-canary";
const INSTALLER: &str = "install-mac-runner.sh";
const POLL: Duration = Duration::from_millis(250);

struct Install {
    home: PathBuf,
    root: PathBuf,
    domain: String,
}

impl Install {
    fn local(runner: Option<&str>) -> Result<Self, Box<dyn std::error::Error>> {
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { libc::geteuid() };
        if runner.is_some() || CHANNEL != Channel::Canary || uid == 0 {
            return Err("The standalone updater runs locally as the canary runner's Mac user.".into());
        }
        if farcooler_daemon_contracts::paths::selection()?.is_explicit() {
            return Err("Unset FARCOOLER_HOME before installing or updating the standalone runner.".into());
        }
        let home = PathBuf::from(std::env::var("HOME")?);
        if !home.is_absolute() || home.to_string_lossy().chars().any(char::is_control) {
            return Err("The account needs an absolute home directory without control characters.".into());
        }
        let root = install_root(&home);
        Ok(Self { home, root, domain: format!("user/{uid}") })
    }

    fn job(&self) -> String {
        format!("{}/{LABEL}", self.domain)
    }

    fn gui_domain(&self) -> String {
        self.domain.replacen("user/", "gui/", 1)
    }

    fn plist_path(&self) -> PathBuf {
        self.home.join(format!("Library/LaunchAgents/{LABEL}.plist"))
    }

    fn login_plist_path(&self) -> PathBuf {
        self.home.join(format!("Library/LaunchAgents/{LOGIN_LABEL}.plist"))
    }

    fn login_plist(&self) -> String {
        // The durable Aqua job repopulates the background user domain on
        // login, including after a reboot. It can exit as soon as it loads it.
        let domain = &self.domain;
        let background = xml(&self.plist_path());
        format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LOGIN_LABEL}</string>
    <key>ProgramArguments</key>
    <array><string>/bin/launchctl</string><string>bootstrap</string><string>{domain}</string><string>{background}</string></array>
    <key>LimitLoadToSessionType</key><string>Aqua</string>
    <key>RunAtLoad</key><true/>
</dict>
</plist>
"#)
    }

    fn plist(&self) -> String {
        let executable = xml(&self.root.join("current").join(DAEMON));
        let home = xml(&self.home);
        let path = xml(&self.home.join(".local/bin"));
        let cargo = xml(&self.home.join(".cargo/bin"));
        format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LABEL}</string>
    <key>ProgramArguments</key><array><string>{executable}</string></array>
    <key>LimitLoadToSessionType</key><string>Background</string>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
    <key>AbandonProcessGroup</key><true/>
    <key>ProcessType</key><string>Interactive</string>
    <key>EnvironmentVariables</key><dict>
        <key>HOME</key><string>{home}</string>
        <key>PATH</key><string>{path}:{cargo}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
    </dict>
</dict>
</plist>
"#)
    }

    fn links(&self) -> [(PathBuf, &'static str); 3] {
        [
            (self.home.join(".local/bin").join(CLI), CLI),
            (self.home.join(".local/bin").join(DAEMON), DAEMON),
            (self.home.join(".config/farcooler/canary/bin/farcooler"), CLI),
        ]
    }
}

fn xml(path: &Path) -> String {
    path.to_string_lossy().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub(crate) fn update(runner: Option<&str>, wait: bool) -> Fallible {
    let install = Install::local(runner)?;
    let script = install.root.join("current").join(INSTALLER);
    if !script.is_file() {
        return Err("Install the standalone canary runner first. See docs/runners.md.".into());
    }
    let mut command = Command::new("/bin/bash");
    command.arg(script);
    if wait { command.arg("--wait"); }
    Err(command.exec().into())
}

pub(crate) async fn activate(
    runner: Option<&str>, build: u64, payload: &Path, app: Option<&Path>, wait: bool,
) -> Fallible {
    let install = Install::local(runner)?;
    let current = install.root.join("current");
    let mutex = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(install.root.join(INSTALL_LOCK))?;
    if let Err(err) = mutex.try_lock() {
        return Err(format!("Cannot lock the standalone runner installation: {err}").into());
    }
    let previous = match fs::read_link(&current) {
        Ok(path) => Some(path),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };
    let installed = previous.as_deref().map(|path| {
        release_build(path).ok_or("Invalid standalone runner installation pointer.")
    }).transpose()?;
    if installed.is_some_and(|old| build < old) {
        return Err("The canary feed is older than the installed runner.".into());
    }

    let background = LaunchAgent::inspect(&install, LABEL, install.domain.clone(),
        install.plist_path(), install.plist())?;
    let login = LaunchAgent::inspect(&install, LOGIN_LABEL, install.gui_domain(),
        install.login_plist_path(), install.login_plist())?;
    let app = app.map(|path| migration_app(path, build)).transpose()?;
    let links = install.links();
    for (link, binary) in &links {
        check_link(link, &current.join(binary), app.as_deref())?;
    }
    let runtime = farcooler_daemon_contracts::paths::ensure_runtime_dir()?;
    check_app_stopped(&install, &runtime, app.is_some())?;
    let socket = runtime.join("farcoolerd.sock");
    let database = runtime.join("farcooler.db");
    background.is_loaded()?;
    login.is_loaded()?;

    if installed == Some(build) && app.is_none() && !background.changed() && !login.changed() {
        // Recovery also covers an interrupted bootstrap after the current link moved.
        publish_links(&links, &current)?;
        background.bootstrap()?;
        login.bootstrap_at_login()?;
        wait_for_job(&install, &socket, &installed_version(&current)?).await?;
        println!("Standalone canary runner build {build} is up to date.");
        return Ok(());
    }

    // Allocate and stage the final version before reserving the landing fence
    // or interrupting the old daemon.
    // Keep previous releases: already-running panes can refer to their CLI.
    let releases = install.root.join("releases");
    fs::create_dir_all(&releases)?;
    let release = tempfile::Builder::new().prefix(&format!("{build}-")).tempdir_in(&releases)?;
    for name in [CLI, DAEMON, "farcooler-lfs-filter", INSTALLER] {
        fs::rename(payload.join(name), release.path().join(name))?;
    }
    check_app_stopped(&install, &runtime, app.is_some())?;
    let reservation = reserve(&runtime, &socket, &database, wait).await?;
    drain(reservation.link, &socket, &database, reservation.legacy).await?;
    let activation = async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while job_pid(&install.job())?.is_some() {
            if tokio::time::Instant::now() >= deadline {
                return Err("The previous LaunchAgent process is still running.".into());
            }
            tokio::time::sleep(POLL).await;
        }

        if let Some(app) = app {
            unregister_app(&app).await?;
        }
        check_app_stopped(&install, &runtime, false)?;
        // Unload before replacing the plist so a retry can bootstrap the
        // correct configuration even if this process is interrupted.
        login.replace()?;
        background.replace()?;
        let path = release.path().strip_prefix(&install.root)?.to_owned();
        atomic_link(&current, &path)?;
        let _ = release.keep();
        publish_links(&links, &current)?;
        background.bootstrap()?;
        login.bootstrap_at_login()?;
        wait_for_job(&install, &socket, BUILD).await
    }.await;
    if let Err(error) = activation {
        if let Some(previous) = previous {
            let rollback = async {
                if login.changed() {
                    login.bootout()?;
                }
                background.bootout()?;
                wait_for_runtime_stop(&socket).await?;
                atomic_link(&current, &previous)?;
                background.restore()?;
                login.restore()?;
                background.bootstrap()?;
                if login.previous.is_some() {
                    login.bootstrap_at_login()?;
                }
                wait_for_job(&install, &socket, &installed_version(&current)?).await
            }.await;
            return match rollback {
                Ok(()) => Err(format!("Standalone runner update failed: {error} Previous release restored.").into()),
                Err(rollback) => Err(format!("Standalone runner update failed: {error} Could not restart the previous release: {rollback}").into()),
            };
        }
        return Err(error);
    }
    println!("Standalone canary runner build {build} is running.");
    Ok(())
}

fn release_build(path: &Path) -> Option<u64> {
    if path.parent()? != Path::new("releases") { return None; }
    let (build, _) = path.file_name()?.to_str()?.split_once('-')?;
    build.parse().ok()
}

fn migration_app(path: &Path, build: u64) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = path.canonicalize()?;
    let info = path.join("Contents/Info.plist");
    let value = |key| -> Result<String, Box<dyn std::error::Error>> {
        let out = Command::new("/usr/bin/plutil")
            .args(["-extract", key, "raw", "-o", "-"]).arg(&info).output()?;
        success(out, key)
    };
    if value("CFBundleIdentifier")?.trim() != APP_ID || value("FarCoolerAgentLabel")?.trim() != APP_LABEL {
        return Err("The migration path must name the Far Cooler Canary app bundle.".into());
    }
    let app_build: u64 = value("CFBundleVersion")?.trim().parse()?;
    if build < app_build {
        return Err(format!("Canary app build {app_build} is newer than standalone build {build}.").into());
    }
    Ok(path)
}

fn app_owns_runtime(runtime: &Path) -> std::io::Result<bool> {
    // AppControl.swift holds this flock while the app answers on app.sock.
    // A copy of the app using another runtime has a different lock.
    match File::open(runtime.join("app.sock.lock")) {
        Ok(lock) => match lock.try_lock() {
            Ok(()) => Ok(false),
            Err(TryLockError::WouldBlock) => Ok(true),
            Err(TryLockError::Error(err)) => Err(err),
        },
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

fn check_app_stopped(install: &Install, runtime: &Path, migrating: bool) -> Fallible {
    if app_owns_runtime(runtime)? {
        return Err("Quit Far Cooler Canary before continuing.".into());
    }
    if !migrating {
        let uid = install.domain.trim_start_matches("user/");
        for domain in [&install.domain, &format!("gui/{uid}")] {
            if launch_info(&format!("{domain}/{APP_LABEL}"))?.is_some() {
                return Err("The app still owns its LaunchAgent. Run the bootstrap installer with --migrate-app.".into());
            }
        }
    }
    Ok(())
}

async fn unregister_app(app: &Path) -> Fallible {
    let output = tokio::time::timeout(Duration::from_secs(15), tokio::process::Command::new(
        app.join("Contents/MacOS/FarCooler")
    ).env("FARCOOLER_SERVICE_PROBE", "unregister").kill_on_drop(true).output()).await??;
    let state = success(output, "unregister the app's LaunchAgent")?;
    if !state.lines().any(|line| line == "state:  notRegistered") {
        return Err(format!("The app's LaunchAgent was not unregistered. App response: {}", state.trim()).into());
    }
    Ok(())
}

fn check_link(link: &Path, target: &Path, app: Option<&Path>) -> Fallible {
    match fs::read_link(link) {
        Ok(old) if old == target => Ok(()),
        Ok(old) if app.is_some_and(|app| {
            let binary = if target.ends_with(DAEMON) { "farcoolerd" } else { "farcooler" };
            let bundled = app.join("Contents/Resources").join(binary);
            old == bundled || old.canonicalize().is_ok_and(|path| path == bundled)
        }) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        _ => Err(format!("{} already exists and is managed by another installation.", link.display()).into()),
    }
}

fn atomic_link(link: &Path, target: &Path) -> std::io::Result<()> {
    let dir = link.parent().expect("installation links have a parent");
    fs::create_dir_all(dir)?;
    let temp = tempfile::tempdir_in(dir)?;
    let staged = temp.path().join("link");
    symlink(target, &staged)?;
    fs::rename(staged, link)
}

struct LaunchAgent {
    label: &'static str,
    domain: String,
    path: PathBuf,
    desired: String,
    previous: Option<Vec<u8>>,
}

impl LaunchAgent {
    fn inspect(
        install: &Install, label: &'static str, domain: String, path: PathBuf, desired: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let conflict = || format!("LaunchAgent conflict at {}", path.display());
        let previous = match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => {
                let content = fs::read(&path)?;
                if content != desired.as_bytes() && !owns_plist(install, label, &path)? {
                    return Err(conflict().into());
                }
                Some(content)
            }
            Ok(_) => return Err(conflict().into()),
            Err(err) if err.kind() == ErrorKind::NotFound => None,
            Err(err) => return Err(err.into()),
        };
        Ok(Self { label, domain, path, desired, previous })
    }

    fn changed(&self) -> bool {
        self.previous.as_deref() != Some(self.desired.as_bytes())
    }

    fn replace(&self) -> Fallible {
        if self.changed() {
            self.bootout()?;
            write_plist(&self.path, self.desired.as_bytes(), self.previous.is_some())?;
        }
        Ok(())
    }

    fn restore(&self) -> Fallible {
        if self.changed() {
            if let Some(previous) = &self.previous {
                write_plist(&self.path, previous, true)?;
            } else if self.label == LABEL {
                // An earlier interrupted install may have left only the
                // release pointer. The background job still needs a plist.
                write_plist(&self.path, self.desired.as_bytes(), true)?;
            } else {
                match fs::remove_file(&self.path) {
                    Ok(()) => {}
                    Err(err) if err.kind() == ErrorKind::NotFound => {}
                    Err(err) => return Err(err.into()),
                }
            }
        }
        Ok(())
    }

    fn job(&self) -> String {
        format!("{}/{}", self.domain, self.label)
    }

    fn is_loaded(&self) -> Result<bool, Box<dyn std::error::Error>> {
        let job = self.job();
        let Some(info) = launch_info(&job)? else { return Ok(false) };
        let path = info.lines().find_map(|line| line.trim_start().strip_prefix("path = ")).map(Path::new);
        let owns_job = path.is_some_and(|path| {
            path == self.path || path.canonicalize().is_ok_and(|loaded| {
                self.path.canonicalize().is_ok_and(|expected| loaded == expected)
            })
        });
        if !owns_job {
            return Err(format!("LaunchAgent {job} is loaded from a different location.").into());
        }
        Ok(true)
    }

    fn bootout(&self) -> Fallible {
        let job = self.job();
        if self.is_loaded()? {
            let output = Command::new("/bin/launchctl").args(["bootout", &job]).output()?;
            if !output.status.success() && self.is_loaded()? {
                success(output, &format!("unload LaunchAgent {job}"))?;
            }
        }
        Ok(())
    }

    fn bootstrap(&self) -> Fallible {
        let job = self.job();
        if !self.is_loaded()? {
            let output = Command::new("/bin/launchctl").args(["bootstrap", &self.domain])
                .arg(&self.path).output()?;
            if !output.status.success() && !self.is_loaded()? {
                success(output, &format!("register LaunchAgent {job}"))?;
            }
        }
        Ok(())
    }

    fn bootstrap_at_login(&self) -> Fallible {
        if launch_info(&self.domain)?.is_some() {
            self.bootstrap()?;
        }
        Ok(())
    }
}

fn owns_plist(install: &Install, label: &str, path: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    // Read the identity fields alone. Other keys can use plist types (such
    // as dates and data) that plutil cannot convert to JSON.
    let extract = |key, format, kind| -> std::io::Result<Option<Vec<u8>>> {
        let output = Command::new("/usr/bin/plutil")
            .args(["-extract", key, format, "-expect", kind, "-n", "-o", "-", "--"])
            .arg(path).output()?;
        Ok(output.status.success().then_some(output.stdout))
    };
    if extract("Label", "raw", "string")?.as_deref() != Some(label.as_bytes()) {
        return Ok(false);
    }
    let program_key = if Command::new("/usr/bin/plutil").args(["-type", "Program", "--"])
        .arg(path).output()?.status.success()
    {
        "Program"
    } else {
        "ProgramArguments.0"
    };
    let Some(program) = extract(program_key, "raw", "string")? else { return Ok(false) };
    let program = Path::new(std::str::from_utf8(&program)?);
    if label == LOGIN_LABEL && program == Path::new("/bin/launchctl") {
        // The original login helper calls launchctl directly.
        let Some(args) = extract("ProgramArguments", "json", "array")? else { return Ok(false) };
        return Ok(serde_json::from_slice::<serde_json::Value>(&args)? == serde_json::json!([
            "/bin/launchctl", "bootstrap", install.domain, install.plist_path(),
        ]));
    }
    Ok(program.strip_prefix(&install.root).is_ok_and(|relative| {
        !relative.as_os_str().is_empty() && relative.components().all(|c| matches!(c, Component::Normal(_)))
    }) && match program.canonicalize() {
        Ok(program) => install.root.canonicalize().is_ok_and(|root| program.starts_with(&root)),
        Err(err) => err.kind() == ErrorKind::NotFound,
    })
}

fn write_plist(path: &Path, content: &[u8], replace: bool) -> Fallible {
    let dir = path.parent().expect("LaunchAgent path has a parent");
    fs::create_dir_all(dir)?;
    let mut staged = tempfile::NamedTempFile::new_in(dir)?;
    staged.write_all(content)?;
    if replace {
        staged.persist(path)?;
    } else {
        staged.persist_noclobber(path)?;
    }
    Ok(())
}

fn publish_links(links: &[(PathBuf, &str)], current: &Path) -> Fallible {
    for (link, binary) in links {
        let target = current.join(binary);
        if fs::read_link(link).ok().as_ref() != Some(&target) {
            atomic_link(link, &target)?;
        }
    }
    Ok(())
}

fn installed_version(current: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new(current.join(CLI)).arg("--version").output()?;
    let version = success(output, "read installed CLI version")?;
    Ok(version.split_whitespace().last().ok_or("Installed CLI returned no version.")?.to_owned())
}

fn launch_info(job: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let output = Command::new("/bin/launchctl").args(["print", job]).output()?;
    match output.status.code() {
        // launchctl reports a missing domain or service with these exit codes.
        Some(112 | 113) => Ok(None),
        _ => success(output, &format!("inspect LaunchAgent {job}")).map(Some),
    }
}

fn success(out: Output, action: &str) -> Result<String, Box<dyn std::error::Error>> {
    if !out.status.success() {
        return Err(format!("Could not {action} ({}): {}", out.status, String::from_utf8_lossy(&out.stderr).trim()).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn job_pid(job: &str) -> Result<Option<i32>, Box<dyn std::error::Error>> {
    Ok(launch_info(job)?.and_then(|info| info.lines().find_map(|line| {
        line.trim().strip_prefix("pid = ")?.parse::<i32>().ok()
    })))
}

async fn wait_for_job(install: &Install, socket: &Path, expected: &str) -> Fallible {
    let job = install.job();
    let output = Command::new("/bin/launchctl").args(["kickstart", &job]).output()?;
    success(output, "start the standalone LaunchAgent")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        let pid = job_pid(&job)?;
        if let Ok(Ok(link)) = tokio::time::timeout(Duration::from_secs(1), daemon_link::dial(socket)).await {
            if pid.is_some() && pid == link.local_pid() && same_build(link.daemon_build(), expected) {
                return Ok(());
            }
        }
        tokio::time::sleep(POLL).await;
    }
    Err("The selected runner did not answer as the launchd service. Check the runner log and rerun the installer.".into())
}

struct Reservation {
    _waiting: File,
    _landing: File,
    link: Option<Link>,
    legacy: bool,
}

async fn reserve(
    runtime: &Path, socket: &Path, database: &Path, wait: bool,
) -> Result<Reservation, Box<dyn std::error::Error>> {
    let waiting = landing_update::announce_update(runtime)?;
    let mut reported = false;
    loop {
        if let Some(landing) = landing_update::try_update(runtime)? {
            if daemon_link::cutover_held_here(database) {
                return Err("Finish the runner's workflow engine cutover before changing the installed build.".into());
            }
            if farcooler_store::read_schema(database)?.is_some_and(|schema| schema.newer_than_here()) {
                return Err("The runner database is from a newer build.".into());
            }
            let link = match tokio::time::timeout(Duration::from_secs(5), daemon_link::dial(socket)).await? {
                Ok(link) => Some(link),
                Err(ClientError::Connect(err)) if matches!(err.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => None,
                Err(err) => return Err(err.into()),
            };
            let legacy = link.as_ref().is_some_and(|link| !daemon_link::has_landing_update_lock(link));
            if !legacy || farcooler_store::workflow_effects::try_legacy_landing_fence(database)? {
                return Ok(Reservation { _waiting: waiting, _landing: landing, link, legacy });
            }
        }
        if !wait {
            return Err("A landing is in progress. Run the update again with --wait.".into());
        }
        if !reported {
            eprintln!("Waiting for in-flight landing checks or pushes to finish.");
            reported = true;
        }
        tokio::time::sleep(POLL).await;
    }
}

async fn drain(link: Option<Link>, socket: &Path, database: &Path, legacy: bool) -> Fallible {
    if let Some(mut link) = link {
        tokio::time::timeout(Duration::from_secs(5), daemon_link::stop(&mut link)).await??;
    }
    wait_for_runtime_stop(socket).await?;
    if legacy {
        farcooler_store::workflow_effects::clear_legacy_landing_fence(database)?;
    }
    Ok(())
}

async fn wait_for_runtime_stop(socket: &Path) -> Fallible {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !landing_update::runner_stopped(socket)? {
        if tokio::time::Instant::now() >= deadline {
            return Err("The previous runner still owns the runtime directory.".into());
        }
        tokio::time::sleep(POLL).await;
    }
    Ok(())
}

#[cfg(test)]
#[path = "standalone_runner_tests.rs"]
mod tests;
