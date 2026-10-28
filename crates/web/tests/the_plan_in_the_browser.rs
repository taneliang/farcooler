//! What the browser's plan reads and edits, against a fake runner (ov-775).
//!
//! The plan is `plan.get` in the phones' shape (`plan_json`, which the client
//! crate holds to `test/fixtures/plan.json` that the Mac decodes), so the
//! browser reads the same plan the Mac does; here the runner is given a plan
//! on the wire and the browser's `Runner::plan` must hand back what it holds.
//!
//! - Acceptance 1: break `Runner::plan` or `workspaces` (drop a theme, read
//!   another board) and `the_browser_reads_the_board_the_runner_holds` goes red.
//! - Acceptance 2: break `Runner::set_theme_next` (another theme, another
//!   text, an actor that is not a person) and the edit test goes red.
//! - Acceptance 3: a device the runner refuses reads that refusal's code and
//!   `what` for every edit, which the page words.

mod support;

use std::sync::Arc;

use farcooler_protocol::v1 as pb;
use farcooler_web::ssh;
use farcooler_web::wire::Runner;
use support::{BROWSER_KEY, Ed25519InMemory, HOST_KEY, Seen, WORKSPACE, key, line};
use uuid::Uuid;

const THEME: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0b01);
const RULING: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0c01);

fn bytes(id: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(id.as_bytes())
}

fn the_plan() -> pb::Plan {
    pb::Plan {
        now_ms: 1_800_000_000_000,
        themes: vec![pb::BoardThemeView {
            theme: Some(pb::BoardTheme {
                id: bytes(THEME),
                name: "Visual language".into(),
                outcome: "Every Mac surface reads as one app.".into(),
                next: "Frosted window plane".into(),
                state: pb::BoardThemeState::Active as i32,
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    }
}

async fn connected() -> (Runner, Arc<Seen>) {
    let (browser_side, runner_side) = tokio::io::duplex(1 << 16);
    let seen = Arc::new(Seen::default());
    *seen.plan.lock().await = Some(the_plan());
    support::serve(runner_side, HOST_KEY, Some(key(BROWSER_KEY).public_key().clone()), seen.clone());
    let mut signer = Ed25519InMemory(key(BROWSER_KEY));
    let conn = ssh::connect(browser_side, &support::fingerprint(HOST_KEY), "owner", &line(BROWSER_KEY), &mut signer).await.expect("connects");
    (Runner::start(conn).await.expect("starts"), seen)
}

#[tokio::test]
async fn the_browser_reads_the_board_the_runner_holds() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, _) = connected().await;
            let boards = runner.workspaces().await.expect("workspace.list");
            assert_eq!(boards[0]["id"], WORKSPACE.to_string());
            assert_eq!(boards[0]["is_main"], true);
            let plan = runner.plan(WORKSPACE).await.expect("plan.get");
            assert_eq!(plan["themes"][0]["name"], "Visual language");
            assert_eq!(plan["themes"][0]["next"], "Frosted window plane");
            assert_eq!(plan["themes"][0]["id"], THEME.to_string());
        })
        .await;
}

#[tokio::test]
async fn an_edited_next_line_reaches_the_runner_as_a_persons_and_is_read_back() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connected().await;
            runner.set_theme_next(THEME, "Glass sidebar").await.expect("the runner takes it");
            assert_eq!(*seen.nexts.lock().await, vec![(THEME.as_bytes().to_vec(), Some("Glass sidebar".to_string()), "user".to_string())]);
            assert_eq!(runner.plan(WORKSPACE).await.unwrap()["themes"][0]["next"], "Glass sidebar");
        })
        .await;
}

#[tokio::test]
async fn keeping_goes_to_the_runner_as_a_persons() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connected().await;
            runner.keep_ruling(RULING).await.expect("ruling.set");
            assert_eq!(
                *seen.rulings_set.lock().await,
                vec![(RULING.as_bytes().to_vec(), pb::BoardRulingState::Confirmed as i32, "user".to_string())]
            );
            runner.keep_all_rulings(WORKSPACE).await.expect("ruling.keep_all");
            assert_eq!(*seen.kept_all.lock().await, vec![WORKSPACE.as_bytes().to_vec()]);
        })
        .await;
}

/// ov-775 acceptance 3: the runner's no comes back with its own code and
/// `what`, for each edit, and nothing changed.
#[tokio::test]
async fn an_edit_the_runner_refuses_comes_back_as_its_refusal() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connected().await;
            *seen.refuse_plan.lock().await = Some((pb::ErrorCode::NotYours, "not_yours"));
            let refused = runner.set_theme_next(THEME, "Mine").await.expect_err("refused");
            assert_eq!(refused.word(), "refused");
            assert_eq!(refused.refusal(), Some(("not-yours", "not_yours")));
            assert_eq!(runner.plan(WORKSPACE).await.unwrap()["themes"][0]["next"], "Frosted window plane");
            assert_eq!(runner.keep_ruling(RULING).await.unwrap_err().refusal(), Some(("not-yours", "not_yours")));
            assert_eq!(runner.keep_all_rulings(WORKSPACE).await.unwrap_err().refusal(), Some(("not-yours", "not_yours")));
        })
        .await;
}
