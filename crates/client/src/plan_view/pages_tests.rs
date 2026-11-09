//! The pages' and tracks' words (ov-1038): the cases Android's `PlanTrackTest` and AgentKit's
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

fn pages_of(themes: Vec<Value>, lines: Vec<Value>) -> Pages {
    let request = json!({ "schema": SCHEMA, "plan": { "now_ms": NOW, "themes": themes, "lines": lines } });
    let answer: Value = serde_json::from_str(&json(&request.to_string())).unwrap();
    assert!(answer["error"].is_null(), "{answer}");
    serde_json::from_value::<PlanView>(answer["view"].clone()).unwrap().pages
}

fn track(theme: Value, lines: Vec<Value>) -> ThemeTrack {
    pages_of(vec![theme], lines).tracks.remove(0)
}

fn words(theme: Value, lines: Vec<Value>) -> String {
    track(theme, lines).words
}

#[test]
fn moving_names_one_ticket_with_its_state_and_counts_several() {
    let one = |name, stage, extra| words(theme(json!({})), vec![ticket(name, stage, extra)]);
    assert_eq!(one("skill-323", "building", json!({})), "skill-323 is building");
    assert_eq!(one("a", "reviewing", json!({})), "a is in review");
    let fixing = json!({ "fix_rounds": 1, "work": [{ "role": "fix", "started_at": NOW }] });
    assert_eq!(one("fix-gestures", "building", fixing), "fix-gestures is fixing, round 1");
    assert_eq!(one("a", "building", json!({ "work": [{ "role": "fix", "started_at": NOW }] })), "a is fixing");
    assert_eq!(one("a", "landing", json!({})), "a is landing");
    assert_eq!(words(theme(json!({})), vec![ticket("a", "building", json!({})), ticket("b", "reviewing", json!({}))]), "2 tickets moving");
}

#[test]
fn a_lone_stale_ticket_reads_stuck_and_among_moving_ones_it_is_named_after_them() {
    let stale = |since| json!({ "stale": true, "stage_since": since });
    assert_eq!(words(theme(json!({})), vec![ticket("s", "building", stale(NOW - 70 * 60_000))]), "s: no move in an hour");
    assert_eq!(words(theme(json!({})), vec![ticket("s", "building", stale(NOW - 3 * HOUR))]), "s: no move in 3 h");
    assert_eq!(
        words(theme(json!({})), vec![ticket("fine", "building", json!({})), ticket("slow", "reviewing", stale(NOW - 4 * HOUR))]),
        "2 tickets moving · slow: no move in 4 h"
    );
}

#[test]
fn queued_says_the_best_rank_and_a_working_ticket_beats_it() {
    let q = |rank: Option<u64>| ticket("a", "waiting", json!({ "plan_rank": rank }));
    assert_eq!(words(theme(json!({})), vec![q(Some(1))]), "Queued, 1st up");
    assert_eq!(words(theme(json!({})), vec![q(Some(3)), q(Some(2))]), "Queued, 2nd up");
    assert_eq!(words(theme(json!({})), vec![q(None)]), "Queued");
    assert_eq!(words(theme(json!({})), vec![q(Some(1)), ticket("b", "building", json!({}))]), "b is building");
}

#[test]
fn quiet_after_a_day_with_open_cards_and_no_ticket_idle_before_it() {
    let story = |at: i64| theme(json!({ "story_at": at }));
    assert_eq!(words(story(NOW - 3 * DAY - HOUR), vec![]), "No ticket · quiet for 3 days");
    assert_eq!(words(story(NOW - 25 * HOUR), vec![]), "No ticket · quiet for 1 day");
    assert_eq!(track(story(NOW - DAY), vec![]).kind, TrackKind::Quiet);
    assert_eq!(words(story(NOW - 23 * HOUR), vec![]), "No ticket yet");
    assert_eq!(words(story(0), vec![]), "No ticket yet");
}

#[test]
fn the_runners_last_moved_at_keeps_a_theme_awake_and_a_dropped_ticket_does_not() {
    let old = NOW - 5 * DAY;
    let kind = |theme: Value, lines: Vec<Value>| track(theme, lines).kind;
    assert_eq!(kind(theme(json!({ "story_at": old })), vec![]), TrackKind::Quiet);
    assert_eq!(kind(theme(json!({ "story_at": old, "last_moved_at": NOW - HOUR })), vec![]), TrackKind::Idle);
    let at = |stage| ticket("a", stage, json!({ "stage_since": NOW - HOUR }));
    assert_eq!(kind(theme(json!({ "story_at": old })), vec![at("dropped")]), TrackKind::Quiet);
    assert_eq!(kind(theme(json!({ "story_at": old })), vec![at("landed")]), TrackKind::Idle);
}

#[test]
fn all_done_paused_and_done() {
    assert_eq!(words(theme(json!({ "counts": { "done": 2 } })), vec![]), "Every card is done");
    assert_eq!(words(theme(json!({ "counts": {} })), vec![]), "No ticket yet");
    assert_eq!(words(theme(json!({ "state": "paused" })), vec![ticket("a", "building", json!({}))]), "Paused");
    assert_eq!(words(theme(json!({ "state": "done" })), vec![ticket("a", "building", json!({}))]), "Done");
}

#[test]
fn over_budget_is_the_one_amber_state_and_wins_over_a_moving_ticket() {
    let building = || vec![ticket("a", "building", json!({}))];
    let over = track(theme(json!({ "budget_tokens": 100, "spend": { "input_tokens": 600 } })), building());
    assert!(over.attention);
    assert_eq!(over.words, "Over budget: 600 of 100 tokens");
    assert_eq!(over.spoken, "Over budget. 600 of 100 tokens used.");
    assert!(!track(theme(json!({ "budget_tokens": 1000, "spend": { "input_tokens": 600 } })), building()).attention);
    assert!(!track(theme(json!({ "state": "paused" })), vec![]).attention);
}

#[test]
fn spoken_swaps_the_middle_dot_for_a_comma() {
    let t = track(theme(json!({ "story_at": NOW - 3 * DAY })), vec![]);
    assert_eq!((t.words.as_str(), t.spoken.as_str()), ("No ticket · quiet for 3 days", "No ticket, quiet for 3 days"));
}

#[test]
fn the_summary_counts_asks_moving_themes_and_quiet_ones_leaving_out_zeros() {
    let t = |id: &str, state: &str, ask: &str, card: &str| {
        json!({ "id": id, "name": id, "state": state, "owner_ask": ask, "story_at": NOW - 5 * DAY, "cards": [{ "task": card, "key": card }], "counts": { "todo": 1 } })
    };
    let themes = vec![t("A", "active", "?", "c0"), t("B", "active", "", "c1"), t("C", "active", "?", "c2"), t("D", "paused", "", "c3")];
    let moving = json!({ "task": "c0", "key": "c0", "title": "w", "stage": "building", "stage_since": NOW - 600_000 });
    let pages = pages_of(themes, vec![moving]);
    assert_eq!(pages.track_summary.as_deref(), Some("2 waiting on you · 1 moving · 2 quiet"));
    assert_eq!(pages_of(vec![theme(json!({ "counts": { "done": 2 } }))], vec![]).track_summary, None);
}

#[test]
fn a_ticket_page_says_its_status_its_stage_and_its_stale_warning() {
    let line = ticket("a", "reviewing", json!({
        "stale": true, "stage_since": NOW - 3 * HOUR,
        "pr_stage": { "kind": "approved_checks_failing", "pr_number": 31, "unresolved_threads": 2 },
    }));
    let pages = pages_of(vec![], vec![line, ticket("b", "building", json!({}))]);
    let a = &pages.tickets[0];
    assert_eq!(a.stale.as_deref(), Some("No move in 3 h"));
    assert_eq!((a.status.as_str(), a.status_beside_stage.as_str()), ("Approved, Checks Failing", "Approved"));
    let stage = a.stage.as_ref().unwrap();
    assert_eq!((stage.words.as_str(), stage.row.as_str()), ("Checks Failing", "Checks Failing"));
    assert_eq!((stage.detail.as_deref(), stage.attention), (Some("PR 31 · 2 threads open"), true));
    assert_eq!((pages.tickets[1].status.as_str(), pages.tickets[1].status_beside_stage.as_str()), ("Building", "Building"));
    assert!(pages.tickets[1].stage.is_none() && pages.tickets[1].stale.is_none());
}

/// The runner's own stage messages (`test/fixtures/plan-stages.json`), worded for Android: every
/// kind says what the pages used to say natively.
#[test]
fn every_pull_request_stage_kind_has_its_words_on_a_ticket_page() {
    let fixture: Value = serde_json::from_str(include_str!("../../../../test/fixtures/plan-stages.json")).unwrap();
    let plan = fixture.get("plan").unwrap_or(&fixture).clone();
    let request = json!({ "schema": SCHEMA, "plan": plan, "context": { "style": "material" } });
    let answer: Value = serde_json::from_str(&json(&request.to_string())).unwrap();
    let pages = serde_json::from_value::<PlanView>(answer["view"].clone()).unwrap().pages;
    let words: Vec<Option<&str>> = pages.tickets.iter().take(14).map(|p| p.stage.as_ref().map(|s| s.words.as_str())).collect();
    assert_eq!(
        words,
        [
            None, Some("Agent review"), Some("Fixing"), Some("Waiting on alice and 2 others"),
            Some("Waiting on maximilian-wolfgang-hohenzollern-schwarzenberg"), Some("Changes requested"),
            Some("Approved, checks running"), Some("Checks failing"), Some("Merge conflicts"), Some("Approved"),
            Some("Queued, position 2"), Some("Merged"), Some("Closed"), Some("PR state unknown"),
        ]
    );
    let status = |n: usize| (pages.tickets[n - 1].status.as_str(), pages.tickets[n - 1].status_beside_stage.as_str());
    assert_eq!(status(1), ("In review", "In review"));
    assert_eq!(status(6), ("Changes requested", "In review"));
    assert_eq!(status(8), ("Approved, checks failing", "Approved"));
    assert_eq!(status(11), ("Queued, position 2", "In review"));
    assert_eq!(status(14), ("In review · PR state unknown", "In review"));
    assert_eq!(status(15), ("In review", "In review"));
    let row = |n: usize| pages.tickets[n - 1].stage.as_ref().map(|s| (s.row.as_str(), s.attention));
    assert_eq!(row(8), Some(("Checks failing", true)));
    assert_eq!(row(4), Some(("Waiting on alice and 2 others", false)));
    assert_eq!(row(7), Some(("Approved, checks running", false)));
    assert_eq!(row(15), None);
}
