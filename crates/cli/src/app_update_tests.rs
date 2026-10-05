//! `farcooler app`: the commit a version names, the sentences, and the
//! conversation with an app, played by a fake on a scratch socket.

use tokio::net::UnixListener;

use super::*;

fn canary() -> Names {
    Names::of(Channel::Canary)
}

fn about(build: &str, commit: &str, pid: i64) -> serde_json::Value {
    serde_json::json!({
        "version": "0.1.0", "build": build, "channel": "canary",
        "display": format!("0.1.0 (canary {commit})"), "pid": pid,
        "path": "/Applications/Far Cooler Canary.app",
    })
}

fn latest(build: &str) -> serde_json::Value {
    serde_json::json!({
        "version": "0.1.0", "build": build,
        "notes": "https://github.com/taneliang/farcooler/commit/8476e3bd5d9cb12ee68a7ad68ba4ff626950137a",
    })
}

/// A fake app on a scratch socket. Each connection reads one request and
/// gets the next script in `scripts`, a line at a time, then the fake
/// hangs up — as the real app does when it quits to install.
struct FakeApp {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    requests: std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
}

impl FakeApp {
    fn start(scripts: Vec<Vec<serde_json::Value>>) -> FakeApp {
        // Under /tmp: the default temp directory is too long for a socket.
        let dir = tempfile::Builder::new().prefix("fc-app").tempdir_in("/tmp").unwrap();
        let socket = dir.path().join("app.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = requests.clone();
        tokio::spawn(async move {
            for script in scripts {
                let Ok((stream, _)) = listener.accept().await else { return };
                let mut lines = BufReader::new(stream).lines();
                let Ok(Some(request)) = lines.next_line().await else { continue };
                seen.lock().unwrap().push(serde_json::from_str(&request).unwrap());
                let stream = lines.get_mut().get_mut();
                for line in script {
                    stream.write_all(format!("{line}\n").as_bytes()).await.unwrap();
                }
            }
        });
        FakeApp { _dir: dir, socket, requests }
    }

    fn requests(&self) -> Vec<serde_json::Value> {
        self.requests.lock().unwrap().clone()
    }
}

async fn run_update(app: &FakeApp, relaunch: bool) -> Result<UpdateReport, String> {
    let conn = Conn::open(&app.socket).await.unwrap();
    update(conn, &app.socket, &canary(), relaunch).await.map_err(|e| e.to_string())
}

#[test]
fn a_canary_display_names_its_commit_and_a_stable_one_none() {
    assert_eq!(commit_of_display("0.1.0 (canary 8476e3b)").as_deref(), Some("8476e3b"));
    assert_eq!(commit_of_display("0.1.0 (local a1b2c3d)").as_deref(), Some("a1b2c3d"));
    assert_eq!(commit_of_display("0.2.0"), None);
    assert_eq!(commit_of_display("0.2.0 (preview 3)"), None);
}

#[test]
fn the_feeds_notes_link_names_the_commit_shortened() {
    let notes = "https://github.com/taneliang/farcooler/commit/8476e3bd5d9cb12ee68a7ad68ba4ff626950137a";
    assert_eq!(commit_of_notes(notes).as_deref(), Some("8476e3b"));
    assert_eq!(commit_of_notes("https://github.com/o/r/releases/tag/v0.2.0"), None);
}

#[test]
fn each_channel_opens_its_own_app() {
    assert_eq!(
        Names::of(Channel::Canary),
        Names { app: "Far Cooler Canary".into(), bundle_id: "com.farcooler.FarCooler.canary".into(), cli: "farcooler-canary" }
    );
    assert_eq!(Names::of(Channel::Stable).bundle_id, "com.farcooler.FarCooler");
    assert_eq!(Names::of(Channel::Stable).app, "Far Cooler");
}

#[test]
fn an_app_that_isnt_open_and_one_too_old_to_answer_are_told_apart() {
    let closed = not_answering(&canary(), false);
    assert_eq!(closed, "Far Cooler Canary isn't open. Open it and try again, or run `farcooler-canary app update --launch`");
    let old = not_answering(&canary(), true);
    assert!(old.contains("Check for Updates…"), "{old}");
}

#[tokio::test]
async fn an_app_on_the_newest_build_says_so() {
    let app = FakeApp::start(vec![vec![
        serde_json::json!({ "event": "checking" }),
        serde_json::json!({ "event": "upToDate", "app": about("2329", "8476e3b", 10) }),
    ]]);
    let report = run_update(&app, true).await.unwrap();
    assert_eq!(app.requests(), vec![serde_json::json!({ "op": "update", "relaunch": true })]);
    assert_eq!(
        report.said(&canary()),
        "Already up to date: Far Cooler Canary 0.1.0 (build 2329, commit 8476e3b)."
    );
    assert_eq!(report.json()["result"], "upToDate");
}

#[tokio::test]
async fn an_update_reports_the_build_before_and_the_relaunched_one_after() {
    let app = FakeApp::start(vec![
        vec![
            serde_json::json!({ "event": "downloading" }),
            serde_json::json!({ "event": "installing", "from": about("2328", "1a2b3c4", 10), "to": latest("2329") }),
        ],
        // The relaunched app, a new process at the new build.
        vec![serde_json::json!({ "event": "about", "app": about("2329", "8476e3b", 11) })],
    ]);
    let report = run_update(&app, true).await.unwrap();
    assert_eq!(app.requests()[1], serde_json::json!({ "op": "about" }));
    assert_eq!(
        report.said(&canary()),
        "Updated Far Cooler Canary from 0.1.0 (build 2328, commit 1a2b3c4) to 0.1.0 (build 2329, commit 8476e3b)."
    );
    let json = report.json();
    assert_eq!(json["from"]["build"], "2328");
    assert_eq!(json["to"]["build"], "2329");
    assert_eq!(json["to"]["commit"], "8476e3b");
}

#[tokio::test]
async fn a_relaunch_at_the_same_build_is_a_failure_not_an_update() {
    let app = FakeApp::start(vec![
        vec![serde_json::json!({ "event": "installing", "from": about("2328", "1a2b3c4", 10), "to": latest("2329") })],
        vec![serde_json::json!({ "event": "about", "app": about("2328", "1a2b3c4", 11) })],
    ]);
    let said = run_update(&app, true).await.unwrap_err();
    assert_eq!(said, "Far Cooler Canary reopened still at build 2328: the update didn't install");
}

#[tokio::test]
async fn without_a_relaunch_the_update_waits_for_the_next_quit() {
    let app = FakeApp::start(vec![vec![serde_json::json!({
        "event": "pending", "from": about("2328", "1a2b3c4", 10), "to": latest("2329"),
    })]]);
    let report = run_update(&app, false).await.unwrap();
    assert_eq!(app.requests(), vec![serde_json::json!({ "op": "update", "relaunch": false })]);
    assert_eq!(
        report.said(&canary()),
        "Far Cooler Canary 0.1.0 (build 2329, commit 8476e3b) is downloaded and installs when you quit \
         Far Cooler Canary. This Mac runs 0.1.0 (build 2328, commit 1a2b3c4) until then."
    );
}

#[tokio::test]
async fn a_refusal_is_said_in_this_commands_words_not_sparkles() {
    let app = FakeApp::start(vec![vec![serde_json::json!({
        "event": "refused", "code": "signature", "detail": "The update is improperly signed.",
    })]]);
    let said = run_update(&app, true).await.unwrap_err();
    assert_eq!(said, "the update's signature didn't check out, so Far Cooler Canary didn't install it");
}

#[tokio::test]
async fn an_app_that_hangs_up_without_answering_stopped_answering() {
    let app = FakeApp::start(vec![vec![serde_json::json!({ "event": "checking" })]]);
    let said = run_update(&app, true).await.unwrap_err();
    assert_eq!(said, "Far Cooler Canary stopped answering before the update finished");
}

#[tokio::test]
async fn version_says_when_an_update_is_waiting() {
    let app = FakeApp::start(vec![vec![serde_json::json!({
        "event": "version", "app": about("2328", "1a2b3c4", 10), "latest": latest("2329"),
    })]]);
    let report = version(Conn::open(&app.socket).await.unwrap(), &canary()).await.unwrap();
    assert!(report.update_waiting());
    assert_eq!(
        report.said(&canary()),
        "Far Cooler Canary 0.1.0 (build 2328, commit 1a2b3c4)\nChannel: canary\n\
         Latest: 0.1.0 (build 2329, commit 8476e3b). An update is waiting: run `farcooler-canary app update`."
    );
    let json = report.json();
    assert_eq!(json["updateWaiting"], true);
    assert_eq!(json["latest"]["commit"], "8476e3b");
    assert_eq!(json["installed"]["commit"], "1a2b3c4");
}

#[tokio::test]
async fn version_on_the_newest_build_has_nothing_waiting() {
    let app = FakeApp::start(vec![vec![serde_json::json!({
        "event": "version", "app": about("2329", "8476e3b", 10), "latest": latest("2329"),
    })]]);
    let report = version(Conn::open(&app.socket).await.unwrap(), &canary()).await.unwrap();
    assert!(!report.update_waiting());
    assert!(report.said(&canary()).ends_with("This is the newest build."));
}

#[tokio::test]
async fn version_of_a_build_without_updates_says_why_latest_is_unknown() {
    let app = FakeApp::start(vec![vec![serde_json::json!({
        "event": "version", "app": about("2329", "8476e3b", 10), "unknown": "updates-off",
    })]]);
    let report = version(Conn::open(&app.socket).await.unwrap(), &canary()).await.unwrap();
    assert!(!report.update_waiting());
    assert!(report.said(&canary()).ends_with("doesn't check for updates: it was built from a working tree."));
    assert_eq!(report.json()["latestUnknown"], "updates-off");
}
