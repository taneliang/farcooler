//! The ticket details' words and the Themes summary (ov-1038): the cases Android's `PlanTrackTest` and AgentKit's
//! `PlanThemeTrackTests` held, now in the one place that says them.

use serde_json::{Value, json};

use super::*;

const NOW: i64 = 1_800_000_000_000;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

fn theme(extra: Value) -> Value {
    let mut t = json!({
        "id": "t", "name": "T", "state": "active", "ordinal": 0,
        "cards": [{ "task": "c1", "key": "ov-1" }, { "task": "c2", "key": "ov-2" }],
        "counts": { "todo": 1 },
    });
    for (k, v) in extra.as_object().unwrap() {
        t[k] = v.clone();
    }
    t
}

fn ticket(name: &str, stage: &str, extra: Value) -> Value {
    let mut l = json!({ "task": "c1", "key": "ov-1", "title": name, "stage": stage, "stage_since": NOW - 10 * 60_000 });
    for (k, v) in extra.as_object().unwrap() {
        l[k] = v.clone();
    }
    l
}

fn details_of(themes: Vec<Value>, lines: Vec<Value>) -> TicketDetails {
    let request = json!({ "schema": SCHEMA, "plan": { "now_ms": NOW, "themes": themes, "lines": lines } });
    let answer: Value = serde_json::from_str(&json(&request.to_string())).unwrap();
    assert!(answer["error"].is_null(), "{answer}");
    serde_json::from_value::<PlanView>(answer["view"].clone()).unwrap().details
}

/// The details as the core builds them, with the themes' kinds (the wire carries only what they sum to).
fn built(plan: Value) -> TicketDetails {
    view(&serde_json::from_value(plan).unwrap(), &input::Context::default(), None).details
}

fn kind(theme: Value, lines: Vec<Value>) -> TrackKind {
    built(json!({ "now_ms": NOW, "themes": [theme], "lines": lines })).tracks[0]
}

#[test]
fn working_tickets_read_moving_and_a_lone_stale_one_reads_stuck() {
    let stale = |since| json!({ "stale": true, "stage_since": since });
    let k = |lines| kind(theme(json!({})), lines);
    assert_eq!(k(vec![ticket("a", "building", json!({}))]), TrackKind::Moving);
    assert_eq!(k(vec![ticket("a", "landing", json!({}))]), TrackKind::Moving);
    assert_eq!(k(vec![ticket("s", "building", stale(NOW - 3 * HOUR))]), TrackKind::Stuck);
    // Among moving tickets a stale one does not make the theme stuck.
    assert_eq!(k(vec![ticket("fine", "building", json!({})), ticket("slow", "reviewing", stale(NOW - 4 * HOUR))]), TrackKind::Moving);
}

#[test]
fn queued_reads_queued_and_a_working_ticket_beats_it() {
    let q = |rank: Option<u64>| ticket("a", "waiting", json!({ "plan_rank": rank }));
    assert_eq!(kind(theme(json!({})), vec![q(Some(1)), q(None)]), TrackKind::Queued);
    assert_eq!(kind(theme(json!({})), vec![q(Some(1)), ticket("b", "building", json!({}))]), TrackKind::Moving);
}

#[test]
fn quiet_after_a_day_with_open_cards_and_no_ticket_idle_before_it() {
    let story = |at: i64| theme(json!({ "story_at": at }));
    assert_eq!(kind(story(NOW - 3 * DAY - HOUR), vec![]), TrackKind::Quiet);
    assert_eq!(kind(story(NOW - DAY), vec![]), TrackKind::Quiet);
    assert_eq!(kind(story(NOW - 23 * HOUR), vec![]), TrackKind::Idle);
    assert_eq!(kind(story(0), vec![]), TrackKind::Idle);
}

#[test]
fn the_runners_last_moved_at_keeps_a_theme_awake_and_a_dropped_ticket_does_not() {
    let old = NOW - 5 * DAY;
    assert_eq!(kind(theme(json!({ "story_at": old })), vec![]), TrackKind::Quiet);
    assert_eq!(kind(theme(json!({ "story_at": old, "last_moved_at": NOW - HOUR })), vec![]), TrackKind::Idle);
    let at = |stage| ticket("a", stage, json!({ "stage_since": NOW - HOUR }));
    assert_eq!(kind(theme(json!({ "story_at": old })), vec![at("dropped")]), TrackKind::Quiet);
    assert_eq!(kind(theme(json!({ "story_at": old })), vec![at("landed")]), TrackKind::Idle);
}

#[test]
fn all_done_paused_and_done() {
    assert_eq!(kind(theme(json!({ "counts": { "done": 2 } })), vec![]), TrackKind::AllDone);
    assert_eq!(kind(theme(json!({ "counts": {} })), vec![]), TrackKind::Idle);
    assert_eq!(kind(theme(json!({ "state": "paused" })), vec![ticket("a", "building", json!({}))]), TrackKind::Paused);
    assert_eq!(kind(theme(json!({ "state": "done" })), vec![ticket("a", "building", json!({}))]), TrackKind::Done);
}

#[test]
fn over_budget_wins_over_a_moving_ticket_only_once_spend_passes_it() {
    let building = || vec![ticket("a", "building", json!({}))];
    assert_eq!(kind(theme(json!({ "budget_tokens": 100, "spend": { "input_tokens": 600 } })), building()), TrackKind::OverBudget);
    assert_eq!(kind(theme(json!({ "budget_tokens": 1000, "spend": { "input_tokens": 600 } })), building()), TrackKind::Moving);
}

#[test]
fn the_summary_counts_asks_moving_themes_and_quiet_ones_leaving_out_zeros() {
    let t = |id: &str, state: &str, ask: &str, card: &str| {
        json!({ "id": id, "name": id, "state": state, "owner_ask": ask, "story_at": NOW - 5 * DAY, "cards": [{ "task": card, "key": card }], "counts": { "todo": 1 } })
    };
    let themes = vec![t("A", "active", "?", "c0"), t("B", "active", "", "c1"), t("C", "active", "?", "c2"), t("D", "paused", "", "c3")];
    let moving = json!({ "task": "c0", "key": "c0", "title": "w", "stage": "building", "stage_since": NOW - 600_000 });
    let details = details_of(themes, vec![moving]);
    assert_eq!(details.track_summary.as_deref(), Some("2 waiting on you · 1 moving · 2 quiet"));
    assert_eq!(details_of(vec![theme(json!({ "counts": { "done": 2 } }))], vec![]).track_summary, None);
}

#[test]
fn a_ticket_page_says_its_status_its_stage_and_its_stale_warning() {
    let line = ticket("a", "reviewing", json!({
        "stale": true, "stage_since": NOW - 3 * HOUR,
        "pr_stage": { "kind": "approved_checks_failing", "pr_number": 31, "unresolved_threads": 2 },
    }));
    let details = details_of(vec![], vec![line, ticket("b", "building", json!({}))]);
    let a = &details.tickets[0];
    assert_eq!(a.stale.as_deref(), Some("No move in 3 h"));
    assert_eq!((a.status.as_str(), a.status_beside_stage.as_str()), ("Approved, Checks Failing", "Approved"));
    let stage = a.stage.as_ref().unwrap();
    assert_eq!((stage.words.as_str(), stage.row.as_str()), ("Checks Failing", "Checks Failing"));
    assert_eq!((stage.detail.as_deref(), stage.attention), (Some("PR 31 · 2 threads open"), true));
    assert_eq!((details.tickets[1].status.as_str(), details.tickets[1].status_beside_stage.as_str()), ("Building", "Building"));
    assert!(details.tickets[1].stage.is_none() && details.tickets[1].stale.is_none());
}

/// The runner's own stage messages (`test/fixtures/plan-stages.json`), worded for Android: every
/// kind says what the page used to say natively.
#[test]
fn every_pull_request_stage_kind_has_its_words_on_a_ticket_page() {
    let fixture: Value = serde_json::from_str(include_str!("../../../../test/fixtures/plan-stages.json")).unwrap();
    let plan = fixture.get("plan").unwrap_or(&fixture).clone();
    let request = json!({ "schema": SCHEMA, "plan": plan, "context": { "style": "material" } });
    let answer: Value = serde_json::from_str(&json(&request.to_string())).unwrap();
    let details = serde_json::from_value::<PlanView>(answer["view"].clone()).unwrap().details;
    let words: Vec<Option<&str>> = details.tickets.iter().take(14).map(|p| p.stage.as_ref().map(|s| s.words.as_str())).collect();
    assert_eq!(
        words,
        [
            None, Some("Agent review"), Some("Fixing"), Some("Waiting on alice and 2 others"),
            Some("Waiting on maximilian-wolfgang-hohenzollern-schwarzenberg"), Some("Changes requested"),
            Some("Approved, checks running"), Some("Checks failing"), Some("Merge conflicts"), Some("Approved"),
            Some("Queued, position 2"), Some("Merged"), Some("Closed"), Some("PR state unknown"),
        ]
    );
    let status = |n: usize| (details.tickets[n - 1].status.as_str(), details.tickets[n - 1].status_beside_stage.as_str());
    assert_eq!(status(1), ("In review", "In review"));
    assert_eq!(status(6), ("Changes requested", "In review"));
    assert_eq!(status(8), ("Approved, checks failing", "Approved"));
    assert_eq!(status(11), ("Queued, position 2", "In review"));
    assert_eq!(status(14), ("In review · PR state unknown", "In review"));
    assert_eq!(status(15), ("In review", "In review"));
    let row = |n: usize| details.tickets[n - 1].stage.as_ref().map(|s| (s.row.as_str(), s.attention));
    assert_eq!(row(8), Some(("Checks failing", true)));
    assert_eq!(row(4), Some(("Waiting on alice and 2 others", false)));
    assert_eq!(row(7), Some(("Approved, checks running", false)));
    assert_eq!(row(15), None);
    // The facts under the words: the pull request, and its threads only when some are open.
    let detail = |n: usize| details.tickets[n - 1].stage.as_ref().and_then(|s| s.detail.as_deref());
    assert_eq!(detail(4), Some("PR 31 · 2 threads open"));
    assert_eq!(detail(5), Some("PR 32"));
}

/// The cases every platform read before the words moved here (`test/fixtures/plan-track-cases.json`): a
/// plan, the first theme's track line and the Themes summary, so a phrase or a threshold can't change
/// on one platform alone.
#[test]
fn every_shared_track_case_reads_as_written() {
    let file: Value = serde_json::from_str(include_str!("../../../../test/fixtures/plan-track-cases.json")).unwrap();
    let cases = file["cases"].as_array().unwrap();
    assert!(cases.len() >= 30);
    for case in cases {
        let name = case["name"].as_str().unwrap_or("?");
        let details = built(case["plan"].clone());
        if let Some(want) = case["kind"].as_str() {
            let got = format!("{:?}", details.tracks[0]);
            let snake: String = got.chars().enumerate().fold(String::new(), |mut out, (i, c)| {
                if c.is_uppercase() && i > 0 {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
                out
            });
            assert_eq!(snake, want, "{name}");
        }
        if let Some(summary) = case.get("summary") {
            assert_eq!(details.track_summary.as_deref(), summary.as_str(), "{name}: the summary");
        }
    }
}

/// A task with a live line and a dropped one gets one detail, the live one: a first-match lookup in an app
/// then finds the detail its own `line(ofTask:)` would draw.
#[test]
fn a_task_with_a_live_line_and_a_dropped_one_has_only_the_live_page() {
    let dropped = ticket("old", "dropped", json!({ "stage_since": NOW - DAY }));
    let live = ticket("new", "building", json!({}));
    for lines in [vec![dropped.clone(), live.clone()], vec![live.clone(), dropped.clone()]] {
        let details = details_of(vec![], lines);
        assert_eq!(details.tickets.len(), 1);
        assert_eq!(details.tickets[0].status, "Building");
    }
    // Alone, a dropped line keeps its detail.
    assert_eq!(details_of(vec![], vec![dropped]).tickets.len(), 1);
}

/// The Mac and the iPhone's words for the same stage kinds (`Style::Apple` is what a request with no style
/// gets): title case, and the state word first.
#[test]
fn every_pull_request_stage_kind_has_its_apple_words_on_a_ticket_page() {
    let fixture: Value = serde_json::from_str(include_str!("../../../../test/fixtures/plan-stages.json")).unwrap();
    let plan = fixture.get("plan").unwrap_or(&fixture).clone();
    let details = built(plan);
    let words: Vec<Option<&str>> = details.tickets.iter().take(14).map(|p| p.stage.as_ref().map(|s| s.words.as_str())).collect();
    assert_eq!(
        words,
        [
            None, Some("Agent Review"), Some("Fixing"), Some("Waiting on alice and 2 others"),
            Some("Waiting on maximilian-wolfgang-hohenzollern-schwarzenberg"), Some("Changes Requested"),
            Some("Approved, Checks Running"), Some("Checks Failing"), Some("Merge Conflicts"), Some("Approved"),
            Some("Queued, Position 2"), Some("Merged"), Some("Closed"), Some("PR State Unknown"),
        ]
    );
}
