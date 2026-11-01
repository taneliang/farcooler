//! `farcooler landing` (ov-309, ov-872): landings by number, against the plan
//! tests' fake runner.

use super::*;

async fn landing_in(link: &mut Runner, args: &str, json: bool) -> Result<String, Failed> {
    let argv = format!("farcooler landing {args} --workspace main");
    let cmd = match crate::Cli::try_parse_from(argv.split_whitespace()).unwrap_or_else(|e| panic!("{argv}: {e}")).command {
        crate::Command::Landing(a) => a.cmd,
        _ => panic!("not landing"),
    };
    landing::run_on(link, &the_board(), cmd, "manager", json, NOW).await
}

async fn landing_say(link: &mut Runner, args: &str) -> Result<String, Failed> {
    landing_in(link, args, false).await
}

/// A plan where Landing 9 is still open with ov-1 on it, and Landing 10,
/// the collecting one, open behind it.
fn two_open() -> pb::Plan {
    let mut plan = the_plan();
    let nine = &mut plan.landings[0];
    nine.state = pb::LandingState::Open as i32;
    nine.pushed_sha = None;
    nine.phase = pb::LandingPhase::Integrating as i32;
    let ten = pb::Landing {
        id: tid(0x6002),
        number: 10,
        state: pb::LandingState::Open as i32,
        collecting: true,
        task_ids: vec![tid(0x1004)],
        ..Default::default()
    };
    plan.landings.push(ten);
    plan
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

/// `landing start` sends its tickets by id, route, base, title and lander,
/// behind `board_line`, and answers with the landing it made.
#[tokio::test]
async fn start_sends_everything_in_one_write() {
    let mut link = runner();
    let said = landing_say(&mut link, "start --ticket ov-1 --route pull_request --base origin/main --title Polish --lander a002").await.unwrap();
    assert_eq!(
        said,
        "Polish (Landing 10) · Open · by pull request · ov-1 · lander Lander sonnet (0000a002)\n  cut from origin/main · open for under a minute"
    );
    let r = last(&link);
    assert_eq!((r.method.as_str(), r.required_capabilities.clone()), ("landing.start", vec![capability::BOARD_LINE.to_string()]));
    let Some(request::Payload::LandingStart(p)) = &r.payload else { panic!() };
    assert_eq!((p.route, p.base.as_str(), p.title.as_str()), (pb::LandingRoute::PullRequest as i32, "origin/main", "Polish"));
    assert_eq!((p.task_ids.clone(), p.lander_id.clone(), p.workspace_id.clone()), (vec![tid(0x1001)], Some(tid(LANDER)), id_bytes(WORKSPACE)));
    landing_say(&mut link, "start").await.expect("a landing of none takes up the collecting one");
    let Some(request::Payload::LandingStart(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.route, pb::LandingRoute::Unspecified as i32, "the runner's default is push");
}

/// Only a ticket ready to land goes on a landing: one in review goes on with
/// `--force`, moved to landing first, and any other is refused before
/// anything is sent.
#[tokio::test]
async fn only_a_ready_ticket_lands_and_force_moves_one_in_review() {
    let mut plan = the_plan();
    plan.lines[0].stage = pb::TicketStage::Reviewing as i32;
    let mut link = Runner { plan: Some(plan.clone()), ..runner() };
    let err = landing_say(&mut link, "start --ticket ov-2").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-2 is in review. Move it to landing when its review is clean, or add --force to put it on a landing now.");
    let err = landing_say(&mut link, "start --ticket ov-3").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-3 is landed already, so it can't go on a landing.");
    assert!(link.sent.iter().all(|r| r.method != "landing.start" && r.method != "line.set"), "refused before the wire");

    landing_say(&mut link, "start --ticket ov-2 --ticket ov-1 --force").await.unwrap();
    let writes: Vec<&str> = link.sent.iter().map(|r| r.method.as_str()).filter(|m| *m != "plan.get" && *m != "task.list").collect();
    assert_eq!(writes, ["line.set", "landing.start"], "moved to landing, then started");
    let moved = link.sent.iter().find(|r| r.method == "line.set").unwrap();
    let Some(request::Payload::LineSet(p)) = &moved.payload else { panic!() };
    assert_eq!((p.task_id.clone(), p.stage), (tid(0x1002), Some(pb::TicketStage::Landing as i32)));

    plan.lines[0].stage = pb::TicketStage::Waiting as i32;
    let mut link = Runner { plan: Some(plan), ..runner() };
    let err = landing_say(&mut link, "start --ticket ov-2 --force").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-2 is waiting. A landing takes only tickets that are ready to land, so it waits for the next one.");
    assert!(crate::Cli::try_parse_from(["farcooler", "landing", "start", "--force"]).is_err(), "--force needs a ticket");
}

/// `landing set` records a SHA, moves the landing and its tickets, and is
/// refused for a settled landing before anything is sent.
#[tokio::test]
async fn set_records_a_sha_and_moves_the_landing() {
    let mut link = runner();
    let said = landing_say(&mut link, "set 9 --sha abc1234").await.unwrap();
    assert!(said.starts_with("Mac interface polish (Landing 9) · Checking on CI · abc1234"), "{said}");
    let Some(request::Payload::LandingSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.landing_id.clone(), p.sha.as_deref(), p.state), (tid(0x6001), Some("abc1234"), pb::LandingState::Unspecified as i32));
    landing_say(&mut link, "set 9 --state landed --remove ov-1 --lander a002").await.unwrap();
    let Some(request::Payload::LandingSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.state, p.remove_task_ids.clone(), p.lander_id.clone()), (pb::LandingState::Landed as i32, vec![tid(0x1001)], Some(tid(LANDER))));

    let err = landing_say(&mut link, "set 9 --add ov-2").await.unwrap_err();
    assert!(err.to_string().starts_with("ov-2 is waiting. A landing takes only tickets"), "{err}");
    let mut plan = the_plan();
    plan.landings[0].state = pb::LandingState::Landed as i32;
    let mut landed = Runner { plan: Some(plan), ..runner() };
    let err = landing_say(&mut landed, "set 9 --title x").await.unwrap_err();
    assert_eq!(err.to_string(), "That landing has landed or been dropped, so it takes no more moves, SHAs or tickets.");
    assert!(landed.sent.iter().all(|r| r.method != "landing.set"));
}

/// `landing move` puts a ticket ready to land on the next open landing in
/// one write, and takes it off when there's none; a landing past open keeps
/// its tickets.
#[tokio::test]
async fn move_sends_a_ticket_to_the_next_landing() {
    let mut link = Runner { plan: Some(two_open()), ..runner() };
    assert_eq!(landing_say(&mut link, "move ov-1").await.unwrap(), "Moved ov-1 from Mac interface polish (Landing 9) to Landing 10.");
    let Some(request::Payload::LandingSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.landing_id.clone(), p.add_task_ids.clone(), p.remove_task_ids.len()), (tid(0x6002), vec![tid(0x1001)], 0));
    let json: Value = serde_json::from_str(&landing_in(&mut link, "move ov-1 --to 10", true).await.unwrap()).unwrap();
    assert_eq!(json, json!({ "ticket": "ov-1", "from": 9, "to": 10 }));

    let mut alone = two_open();
    alone.landings.pop();
    let mut link = Runner { plan: Some(alone), ..runner() };
    let said = landing_say(&mut link, "move ov-1").await.unwrap();
    assert_eq!(
        said,
        "Took ov-1 off Mac interface polish (Landing 9). It stays off until you put it on a landing with `farcooler landing start --ticket ov-1`, or it goes back through review."
    );
    let Some(request::Payload::LandingSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.landing_id.clone(), p.remove_task_ids.clone()), (tid(0x6001), vec![tid(0x1001)]));

    let mut link = runner();
    let err = landing_say(&mut link, "move ov-1").await.unwrap_err();
    assert_eq!(err.to_string(), "Mac interface polish (Landing 9) has already gone to gating or CI, so a ticket can't come off it now.");
    let err = landing_say(&mut link, "move ov-2").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-2 isn't on a landing.");
    assert!(link.sent.iter().all(|r| r.method != "landing.set"));
}

/// A refusal reads as a landing's sentence, and a runner without the line is
/// told before anything is sent.
#[tokio::test]
async fn refusals_read_as_landing_sentences() {
    let mut link = Runner { plan: Some(two_open()), ..runner() };
    link.refuse = Some("landing_moving");
    let err = landing_say(&mut link, "move ov-1").await.unwrap_err();
    assert_eq!(err.to_string(), "That landing has already gone to gating or CI, so a ticket can't come off it. Wait for it to land, or drop it.");
    link.refuse = Some("sha");
    let err = landing_say(&mut link, "set 9 --sha zz").await.unwrap_err();
    assert!(err.to_string().starts_with("Give the SHA it pushed with --sha"), "{err}");

    let mut old = runner();
    old.capabilities.retain(|c| c != capability::BOARD_LINE);
    assert_eq!(landing_say(&mut old, "list").await.unwrap_err().to_string(), NEEDS_LINE);
    assert!(old.sent.is_empty());
    for route in ["push", "pull_request", "pull-request", "ask"] {
        assert!(crate::Cli::try_parse_from(["farcooler", "landing", "start", "--route", route]).is_ok(), "{route}");
    }
}
