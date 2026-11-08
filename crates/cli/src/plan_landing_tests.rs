//! `farcooler landing` (ov-309, ov-872): landings by number, against the plan
//! tests' fake runner.

use super::*;

async fn landing_in(link: &mut Runner, args: &str, json: bool) -> Result<String, Failed> {
    let argv = format!("farcooler landing {args} --workspace main");
    let cmd = match crate::Cli::try_parse_from(argv.split_whitespace()).unwrap_or_else(|e| panic!("{argv}: {e}")).command {
        crate::Command::Landing(a) => a.cmd,
        _ => panic!("not landing"),
    };
    landing::run_on(link, &the_board(), cmd, json, NOW).await
}

async fn landing_say(link: &mut Runner, args: &str) -> Result<String, Failed> {
    landing_in(link, args, false).await
}

/// `landing list` says each landing's line, base, how long it's been where it
/// is, what it carries, and the jobs that failed with their run.
#[tokio::test]
async fn the_list_names_each_landing_and_its_failed_jobs() {
    let mut link = runner();
    let text = landing_say(&mut link, "list").await.unwrap();
    let expected = [
        "Mac interface polish (Landing 9) · Red · c85bf83d · CI Failed · 1 of 3 jobs failed · ov-1 · lander Lander sonnet (0000a002) · 120K tokens · $4.00 API-equivalent",
        "  cut from origin/main · red for 20m",
        "  Carries: Title of ov-1",
        "  Failed: CI / Swift (shared + macOS)",
        "  https://github.com/taneliang/farcooler/actions/runs/37275435256 · read 1m ago",
    ];
    assert_eq!(text.lines().collect::<Vec<_>>(), expected, "{text}");
    let json: Value = serde_json::from_str(&landing_in(&mut link, "list", true).await.unwrap()).unwrap();
    let plan = the_plan();
    assert_eq!(json["landings"][0], farcooler_client::plan_json::landing_json(&plan, &plan.landings[0]));
    let mut empty = Runner { plan: Some(pb::Plan { landings: vec![], ..the_plan() }), ..runner() };
    assert!(landing_say(&mut empty, "list").await.unwrap().starts_with("No landings yet."));
}

/// A read that failed says "CI unknown" once, and one that stopped working
/// says how old it is.
#[test]
fn a_ci_read_says_unknown_once_and_how_old_it_is() {
    let mut plan = the_plan();
    plan.ci[0].status = pb::BoardCiStatus::Unknown as i32;
    plan.ci[0].jobs.clear();
    assert!(landing::landing_line(&plan, &plan.landings[0]).contains("c85bf83d · CI unknown · ov-1"));
    let mut plan = the_plan();
    plan.ci[0].fetched_at = NOW - 3 * HOUR;
    assert!(landing::landing_line(&plan, &plan.landings[0]).contains("1 of 3 jobs failed · as of 3h ago · ov-1"));
}

/// `landing show` takes the number however it's written and reads the
/// landing's own timeline.
#[tokio::test]
async fn show_takes_the_number_and_reads_the_timeline() {
    for number in ["9", "#9", "Landing-9", "landing9"] {
        let mut link = runner();
        link.events = vec![plan_event(NOW - HOUR, "state", "Pushed.", json!({ "to": "pushed" }))];
        let text = landing_say(&mut link, &format!("show {number}")).await.unwrap_or_else(|e| panic!("{number}: {e}"));
        assert!(text.ends_with("\n\nTimeline\n  1h ago  Pushed."), "{number}: {text}");
        let Some(request::Payload::PlanEvents(p)) = &last(&link).payload else { panic!() };
        assert_eq!(p.subject, Some(pb::plan_events_request::Subject::LandingId(tid(0x6001))));
    }
    let mut link = runner();
    assert_eq!(landing_say(&mut link, "show 12").await.unwrap_err().to_string(), "No landing here is number 12. `farcooler landing list` shows them.");
    assert_eq!(landing_say(&mut link, "show soon").await.unwrap_err().to_string(), "Name a landing by its number, like 12. \"soon\" isn't one.");
    let json: Value = serde_json::from_str(&landing_in(&mut link, "show 9", true).await.unwrap()).unwrap();
    assert_eq!((json["number"].as_u64(), json["events"].as_array().map(Vec::len)), (Some(9), Some(0)));
}

/// The engine lands (ov-960): `landing start`, `set` and `move` are gone, and
/// a runner without the line is told before anything is sent.
#[tokio::test]
async fn landings_are_only_read_here() {
    for args in ["start ov-1", "start --record-only --ticket ov-1", "set 9 --sha abc1234", "move ov-1"] {
        let argv = format!("farcooler landing {args} --workspace main");
        assert!(crate::Cli::try_parse_from(argv.split_whitespace()).is_err(), "{args} still parses");
    }
    let mut old = runner();
    old.capabilities.retain(|c| c != capability::BOARD_LINE);
    assert_eq!(landing_say(&mut old, "list").await.unwrap_err().to_string(), NEEDS_LINE);
    assert!(old.sent.is_empty());
}
