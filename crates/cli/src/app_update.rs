//! `farcooler app`: this channel's Mac app, asked from the command line for
//! its version or to install its newest build (ov-302).
//!
//! The CLI downloads nothing. It asks the running app over a Unix socket,
//! `app.sock` in this channel's runtime directory, and the app hands the work
//! to Sparkle, so the check that decides whether a build is trusted stays
//! Sparkle's (`apps/macos/Sources/FarCooler/AppControl.swift`).
//!
//! Local only, by construction: the socket is a file in a 0700 directory, the
//! app answers only a peer with its own user ID, this side checks the app's,
//! and `--runner` is refused. Nothing carries the request over a network or
//! ssh.
//!
//! The conversation is JSON, one object per line. The CLI sends one request:
//!
//! - `{"op":"about"}`: the app's own facts, answered at once;
//! - `{"op":"version"}`: those, plus the newest build the feed offers;
//! - `{"op":"update","relaunch":true}`: check now and install what's newer.
//!
//! The app answers with events, each with an `event` word: `about`,
//! `version`, `upToDate`, `installing` (it's about to quit and relaunch),
//! `pending` (it installs on the next quit), `refused` (with a `code`), and
//! progress words a reader skips.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Subcommand;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use farcooler_protocol::{CHANNEL, Channel};

#[derive(Subcommand)]
pub(crate) enum AppCmd {
    /// Install the newest build of this channel's Mac app now.
    ///
    /// The running app checks its update feed, and Sparkle downloads,
    /// verifies and installs the build, then relaunches the app with its
    /// windows where they were. Prints the version before and after, or that
    /// it's already up to date. Only from this Mac, as the user running the
    /// app.
    Update {
        /// Install when the app next quits, rather than relaunching it now.
        #[arg(long)]
        no_relaunch: bool,
        /// Open the app first if it isn't open.
        #[arg(long)]
        launch: bool,
    },
    /// The installed app's version, build and channel, and the newest build
    /// its update feed offers.
    Version {
        /// Open the app first if it isn't open.
        #[arg(long)]
        launch: bool,
    },
}

/// How long a whole update may take, download included: the canary dmg is
/// about 66 MB, and a slow connection is not a failure.
const UPDATE_WAIT: Duration = Duration::from_secs(15 * 60);
/// How long the app has to come back after quitting to install.
const RELAUNCH_WAIT: Duration = Duration::from_secs(3 * 60);
/// How long `version` waits for the feed. The app gives up at 30 seconds.
const VERSION_WAIT: Duration = Duration::from_secs(45);
/// How long `--launch` waits for a launched app to start answering.
const LAUNCH_WAIT: Duration = Duration::from_secs(60);

pub(crate) async fn app(runner: Option<&str>, cmd: AppCmd, json: bool) -> crate::Fallible {
    if runner.is_some() {
        return Err("app updates the Mac app on this Mac only; drop --runner".into());
    }
    if !cfg!(target_os = "macos") {
        return Err(format!("there's no {} on this computer: the Mac app runs only on macOS", app_name(CHANNEL)).into());
    }
    let socket = socket_path()?;
    let names = Names::of(CHANNEL);
    match cmd {
        AppCmd::Update { no_relaunch, launch } => {
            let conn = reach(&socket, &names, launch).await?;
            let report = update(conn, &socket, &names, !no_relaunch).await?;
            println!("{}", if json { report.json().to_string() } else { report.said(&names) });
        }
        AppCmd::Version { launch } => {
            let conn = reach(&socket, &names, launch).await?;
            let version = version(conn, &names).await?;
            println!("{}", if json { version.json().to_string() } else { version.said(&names) });
        }
    }
    Ok(())
}

/// Where this channel's app listens: beside its daemon's socket, so
/// `FARCOOLER_HOME` moves both together.
fn socket_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(farcooler_daemon::paths::runtime_dir()?.join("app.sock"))
}

/// What a channel's app is called and how to open it. The same mapping as
/// `scripts/version.sh app-name` and `app-suffix`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Names {
    pub(crate) app: String,
    pub(crate) bundle_id: String,
    pub(crate) cli: &'static str,
}

impl Names {
    pub(crate) fn of(channel: Channel) -> Names {
        let bundle_id = match channel {
            Channel::Stable => "com.farcooler.FarCooler".to_string(),
            other => format!("com.farcooler.FarCooler.{}", other.as_str()),
        };
        Names { app: app_name(channel), bundle_id, cli: channel.cli_binary_name() }
    }
}

fn app_name(channel: Channel) -> String {
    match channel {
        Channel::Stable => "Far Cooler".into(),
        Channel::Preview => "Far Cooler Preview".into(),
        Channel::Canary => "Far Cooler Canary".into(),
        Channel::Local => "Far Cooler Local".into(),
    }
}

/// The app's own facts, as it reports them.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct About {
    pub(crate) version: String,
    pub(crate) build: String,
    #[serde(default)]
    pub(crate) channel: String,
    /// `0.1.0 (canary 8476e3b)`: where the commit is, for a canary or local build.
    #[serde(default)]
    pub(crate) display: String,
    #[serde(default)]
    pub(crate) pid: i64,
    #[serde(default)]
    pub(crate) path: String,
}

impl About {
    fn commit(&self) -> Option<String> {
        commit_of_display(&self.display)
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "version": self.version, "build": self.build, "channel": self.channel,
            "commit": self.commit(), "path": self.path, "pid": self.pid,
        })
    }

    fn said(&self) -> String {
        build_said(&self.version, &self.build, self.commit().as_deref())
    }
}

/// The newest build the feed names.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct Latest {
    pub(crate) version: String,
    pub(crate) build: String,
    /// The release notes link, which canary points at the commit it built.
    #[serde(default)]
    pub(crate) notes: String,
}

impl Latest {
    fn commit(&self) -> Option<String> {
        commit_of_notes(&self.notes)
    }

    /// The whole commit, under `--json`: the feed names all of it, and a
    /// script matches it against CI's.
    fn json(&self) -> serde_json::Value {
        let commit = full_commit_of_notes(&self.notes);
        serde_json::json!({ "version": self.version, "build": self.build, "commit": commit })
    }

    fn said(&self) -> String {
        build_said(&self.version, &self.build, self.commit().as_deref())
    }
}

/// `0.1.0 (build 2329, commit 8476e3b)`.
fn build_said(version: &str, build: &str, commit: Option<&str>) -> String {
    match commit {
        Some(commit) => format!("{version} (build {build}, commit {commit})"),
        None => format!("{version} (build {build})"),
    }
}

/// The commit a display version names: `0.1.0 (canary 8476e3b)` → `8476e3b`.
/// Stable and preview name none.
pub(crate) fn commit_of_display(display: &str) -> Option<String> {
    let inside = display.rsplit_once('(')?.1.strip_suffix(')')?;
    let word = inside.split_whitespace().last()?;
    is_commit(word).then(|| word.to_string())
}

/// The commit a release notes link names:
/// `https://github.com/o/r/commit/8476e3bd…` → `8476e3b`, shortened as git does.
pub(crate) fn commit_of_notes(notes: &str) -> Option<String> {
    Some(full_commit_of_notes(notes)?.chars().take(7).collect())
}

fn full_commit_of_notes(notes: &str) -> Option<String> {
    let full = notes.split("/commit/").nth(1)?.split(['/', '?', '#']).next()?;
    is_commit(full).then(|| full.to_string())
}

fn is_commit(word: &str) -> bool {
    (7..=40).contains(&word.len()) && word.chars().all(|c| c.is_ascii_hexdigit())
}

/// One event from the app.
#[derive(Debug, Deserialize)]
struct Event {
    event: String,
    #[serde(default)]
    app: Option<About>,
    #[serde(default)]
    from: Option<About>,
    #[serde(default)]
    to: Option<Latest>,
    #[serde(default)]
    latest: Option<Latest>,
    /// Why `latest` is missing from a `version` answer, as a refusal code.
    #[serde(default)]
    unknown: Option<String>,
    #[serde(default)]
    code: Option<String>,
    /// Sparkle's own account of a refusal, for `FARCOOLER_LOG=info`.
    #[serde(default)]
    detail: Option<String>,
}

/// A conversation with the app.
pub(crate) struct Conn {
    lines: tokio::io::Lines<BufReader<UnixStream>>,
}

impl Conn {
    /// Connect, and make sure the app at the other end runs as this user.
    pub(crate) async fn open(socket: &Path) -> std::io::Result<Conn> {
        let stream = UnixStream::connect(socket).await?;
        let peer = stream.peer_cred()?.uid();
        // SAFETY: getuid has no preconditions and cannot fail.
        if peer != unsafe { libc::getuid() } {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "another user"));
        }
        Ok(Conn { lines: BufReader::new(stream).lines() })
    }

    async fn send(&mut self, request: serde_json::Value) -> std::io::Result<()> {
        let stream = self.lines.get_mut().get_mut();
        stream.write_all(format!("{request}\n").as_bytes()).await?;
        stream.flush().await
    }

    /// The next event worth reading, or `None` when the app hung up.
    async fn next(&mut self) -> std::io::Result<Option<Event>> {
        while let Some(line) = self.lines.next_line().await? {
            if let Ok(event) = serde_json::from_str::<Event>(&line) {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }
}

/// The app, connected: launched first when asked to.
async fn reach(socket: &Path, names: &Names, launch: bool) -> Result<Conn, Box<dyn std::error::Error>> {
    match Conn::open(socket).await {
        Ok(conn) => return Ok(conn),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(format!("something other than {} is answering at {}", names.app, socket.display()).into());
        }
        Err(_) => {}
    }
    if !launch {
        return Err(not_answering(names, is_running(names).await).into());
    }
    let opened = tokio::process::Command::new("/usr/bin/open").args(["-g", "-b", &names.bundle_id]).status().await;
    if !matches!(opened, Ok(s) if s.success()) {
        return Err(format!("{} isn't installed on this Mac", names.app).into());
    }
    let deadline = tokio::time::Instant::now() + LAUNCH_WAIT;
    while tokio::time::Instant::now() < deadline {
        if let Ok(conn) = Conn::open(socket).await {
            return Ok(conn);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(not_answering(names, true).into())
}

/// Why nothing answered: the app isn't open, or it's a build from before
/// it listened.
pub(crate) fn not_answering(names: &Names, running: bool) -> String {
    if running {
        format!(
            "{app} is open but can't take this request yet: it's a build from before updates came to the \
             command line. Choose {app} > Check for Updates… once, and this works from then on",
            app = names.app
        )
    } else {
        format!("{} isn't open. Open it and try again, or run `{} app update --launch`", names.app, names.cli)
    }
}

/// Whether LaunchServices has this channel's app running.
async fn is_running(names: &Names) -> bool {
    let found = tokio::process::Command::new("/usr/bin/lsappinfo")
        .args(["find", &format!("bundleid={}", names.bundle_id)])
        .output()
        .await;
    matches!(found, Ok(out) if out.status.success() && !out.stdout.trim_ascii().is_empty())
}

/// What a refusal from the app says. The app sends a word; the sentences
/// live here, beside the rest of this command's.
pub(crate) fn refused_said(code: &str, names: &Names) -> String {
    let app = &names.app;
    match code {
        "updates-off" => format!("this build of {app} doesn't check for updates: it was built from a working tree"),
        "updates-broken" => format!(
            "{app}'s updater didn't start, so it can't update itself. Quit and reopen {app}, and if this keeps \
             happening, download {app} again"
        ),
        "busy" => format!("{app} is already checking for or installing an update. Try again in a minute"),
        "check-failed" => format!("{app} couldn't read its update feed. Check your connection and try again"),
        "download-failed" => "the update didn't download. Check your connection and try again".into(),
        "signature" => format!("the update's signature didn't check out, so {app} didn't install it"),
        "system-too-old" => format!("the newest build of {app} needs a newer version of macOS"),
        "information-only" => format!("this update can't be installed from here. Open {app} to read about it"),
        _ => format!("{app} couldn't install the update"),
    }
}

/// What `app update` did.
#[derive(Debug, PartialEq)]
pub(crate) enum UpdateReport {
    UpToDate(About),
    Updated { from: About, to: About, notes: Latest },
    Pending { from: About, to: Latest },
}

impl UpdateReport {
    pub(crate) fn said(&self, names: &Names) -> String {
        let app = &names.app;
        match self {
            UpdateReport::UpToDate(now) => format!("Already up to date: {app} {}.", now.said()),
            UpdateReport::Updated { from, to, notes } => {
                // The relaunched app's display names its commit; the feed's
                // link is the fallback for a build whose display doesn't.
                let commit = to.commit().or_else(|| notes.commit());
                format!(
                    "Updated {app} from {} to {}.",
                    from.said(),
                    build_said(&to.version, &to.build, commit.as_deref())
                )
            }
            UpdateReport::Pending { from, to } => format!(
                "{app} {} is downloaded and installs when you quit {app}. This Mac runs {} until then.",
                to.said(),
                from.said()
            ),
        }
    }

    pub(crate) fn json(&self) -> serde_json::Value {
        match self {
            UpdateReport::UpToDate(now) => serde_json::json!({ "result": "upToDate", "app": now.json() }),
            UpdateReport::Updated { from, to, notes } => {
                let mut to = to.json();
                if to["commit"].is_null() {
                    to["commit"] = notes.commit().into();
                }
                serde_json::json!({ "result": "updated", "from": from.json(), "to": to })
            }
            UpdateReport::Pending { from, to } => {
                serde_json::json!({ "result": "pending", "from": from.json(), "to": to.json() })
            }
        }
    }
}

/// Ask for an update and see it through: to the relaunched app, when it
/// relaunches.
pub(crate) async fn update(
    conn: Conn,
    socket: &Path,
    names: &Names,
    relaunch: bool,
) -> Result<UpdateReport, Box<dyn std::error::Error>> {
    update_waiting(conn, socket, names, relaunch, RELAUNCH_WAIT).await
}

/// `update`, with how long the relaunch may take named, for tests.
pub(crate) async fn update_waiting(
    mut conn: Conn,
    socket: &Path,
    names: &Names,
    relaunch: bool,
    relaunch_wait: Duration,
) -> Result<UpdateReport, Box<dyn std::error::Error>> {
    let stopped = || format!("{} stopped answering before the update finished", names.app);
    conn.send(serde_json::json!({ "op": "update", "relaunch": relaunch })).await.map_err(|_| stopped())?;
    let answered = tokio::time::timeout(UPDATE_WAIT, async {
        // `installing` doesn't end the conversation: Sparkle can still fail
        // before the app quits, and says so on the same line (ov-302 F2).
        // The app quitting to install is what hangs up.
        let mut installing = None;
        loop {
            let Some(event) = conn.next().await.map_err(|_| stopped())? else {
                return match installing {
                    Some((from, to)) => Ok(Answered::Relaunching { from, to }),
                    None => Err(stopped()),
                };
            };
            match (event.event.clone().as_str(), event) {
                ("upToDate", Event { app: Some(app), .. }) => return Ok(Answered::Done(UpdateReport::UpToDate(app))),
                ("pending", Event { from: Some(from), to: Some(to), .. }) => {
                    return Ok(Answered::Done(UpdateReport::Pending { from, to }));
                }
                ("installing", Event { from: Some(from), to: Some(to), .. }) => installing = Some((from, to)),
                ("refused", Event { code, detail, .. }) => {
                    tracing::info!(?code, ?detail, "the app refused to update");
                    let said = refused_said(code.as_deref().unwrap_or(""), names);
                    return Err(match &installing {
                        Some((from, _)) => format!(
                            "{said}. {app} is still on build {}, and an alert in {app} says why",
                            from.build,
                            app = names.app
                        ),
                        None => said,
                    });
                }
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| format!("{} took too long to update. Open it to see where it got to", names.app))??;
    let (from, to) = match answered {
        Answered::Done(report) => return Ok(report),
        Answered::Relaunching { from, to } => (from, to),
    };
    drop(conn);
    let now = relaunched(socket, names, &from, relaunch_wait).await?;
    if now.build == from.build {
        return Err(format!("{} reopened still at build {}: the update didn't install", names.app, now.build).into());
    }
    Ok(UpdateReport::Updated { from, to: now, notes: to })
}

/// How the app answered an update before any relaunch.
enum Answered {
    Done(UpdateReport),
    /// It's quitting to install `to`, and relaunching.
    Relaunching { from: About, to: Latest },
}

/// The app that came back after quitting to install: a different process
/// from `from`. The old one still answering, or a quitting one answering
/// late, is not the relaunch.
async fn relaunched(
    socket: &Path,
    names: &Names,
    from: &About,
    wait: Duration,
) -> Result<About, Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + wait;
    // Whether the last look found the old app still answering.
    let mut old_answered = false;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let Ok(mut conn) = Conn::open(socket).await else {
            old_answered = false;
            continue;
        };
        if conn.send(serde_json::json!({ "op": "about" })).await.is_err() {
            continue;
        }
        if let Ok(Ok(Some(Event { app: Some(app), .. }))) =
            tokio::time::timeout(Duration::from_secs(5), conn.next()).await
        {
            if app.pid != from.pid {
                return Ok(app);
            }
            old_answered = true;
        }
    }
    let app = &names.app;
    Err(if old_answered {
        format!("{app} didn't quit to install the update. Open {app} to see why")
    } else {
        format!("{app} quit to install the update and hasn't reopened. Open {app} to finish")
    }
    .into())
}

/// What `app version` found.
#[derive(Debug, PartialEq)]
pub(crate) struct VersionReport {
    pub(crate) app: About,
    pub(crate) latest: Option<Latest>,
    /// The refusal code saying why `latest` is missing.
    pub(crate) unknown: Option<String>,
}

impl VersionReport {
    /// Whether the feed offers a build newer than the one installed. Builds
    /// are commit counts, so newer is larger.
    pub(crate) fn update_waiting(&self) -> bool {
        let number = |b: &str| b.parse::<u64>().ok();
        match (&self.latest, number(&self.app.build)) {
            (Some(latest), Some(installed)) => number(&latest.build).is_some_and(|l| l > installed),
            _ => false,
        }
    }

    pub(crate) fn said(&self, names: &Names) -> String {
        let mut lines = vec![
            format!("{} {}", names.app, self.app.said()),
            format!("Channel: {}", self.app.channel),
        ];
        lines.push(match (&self.latest, &self.unknown) {
            (Some(latest), _) if self.update_waiting() => {
                format!("Latest: {}. An update is waiting: run `{} app update`.", latest.said(), names.cli)
            }
            (Some(latest), _) => format!("Latest: {}. This is the newest build.", latest.said()),
            (None, code) => format!("Latest: unknown, because {}.", refused_said(code.as_deref().unwrap_or(""), names)),
        });
        lines.join("\n")
    }

    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "installed": self.app.json(),
            "latest": self.latest.as_ref().map(Latest::json),
            "latestUnknown": self.unknown,
            "updateWaiting": self.update_waiting(),
        })
    }
}

pub(crate) async fn version(mut conn: Conn, names: &Names) -> Result<VersionReport, Box<dyn std::error::Error>> {
    let stopped = || format!("{} stopped answering", names.app);
    conn.send(serde_json::json!({ "op": "version" })).await.map_err(|_| stopped())?;
    let answer = tokio::time::timeout(VERSION_WAIT, async {
        loop {
            match conn.next().await {
                Ok(Some(event)) if event.event == "version" => return Some(event),
                Ok(Some(_)) => {}
                _ => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(stopped)?;
    let app = answer.app.ok_or_else(stopped)?;
    Ok(VersionReport { app, latest: answer.latest, unknown: answer.unknown })
}

#[cfg(test)]
#[path = "app_update_tests.rs"]
mod tests;
