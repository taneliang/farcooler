//! The plan column, case by case, as AgentKit's `PlanSectionsColumnTests` and
//! the web's `plan-column.test.ts` held it before it moved here (ov-1016).

use serde_json::{Value, json};

use super::column::{Column, CountKind, FactKind, PipeStage};

const NOW: i64 = 1_800_000_000_000;
const MINUTE: i64 = 60_000;

fn line(task: &str, stage: &str) -> Value {
    json!({ "task": task, "key": format!("ov-{}", &task[1..]), "title": "", "status": "In Progress", "stage": stage,
        "reason": format!("Why {task}"), "branch": format!("branch-{task}"), "stage_since": NOW - 10 * MINUTE,
        "stale": false, "fix_rounds": 0, "work": [] })
}

fn with(mut v: Value, extra: Value) -> Value {
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    v
}

fn agent(id: &str, role: &str, tickets: &[&str], doing: &str, started: i64) -> Value {
    json!({ "id": id, "short": id, "role": role, "harness": "claude", "model": "sonnet", "title": "",
        "terminal": null, "tickets": tickets.iter().map(|t| json!({ "task": t, "key": format!("ov-{}", &t[1..]) })).collect::<Vec<_>>(),
        "doing_now": doing, "working": true, "started_at": NOW - started * MINUTE, "last_at": NOW,
        "spend": { "input_tokens": 0 } })
}

fn theme(name: &str, cards: &[&str], ordinal: i64) -> Value {
    json!({ "id": format!("theme-{name}"), "name": name, "outcome": "", "next": "", "owner_ask": "", "state": "active",
        "ordinal": ordinal, "cards": cards.iter().map(|t| json!({ "task": t, "key": format!("ov-{}", &t[1..]) })).collect::<Vec<_>>(),
        "counts": { "backlog": 0, "done": 0 } })
}

fn landing(number: u64, state: &str, tickets: &[&str], phase: &str, since: i64) -> Value {
    json!({ "id": format!("landing-{number}"), "number": number, "state": state, "state_since": since, "phase": phase,
        "phase_since": since, "title": format!("Landing {number}"), "summary": "",
        "tickets": tickets.iter().map(|t| json!({ "task": t, "key": format!("ov-{}", &t[1..]) })).collect::<Vec<_>>() })
}

fn column(plan: Value, context: Value) -> Column {
    let mut plan = plan;
    plan["now_ms"] = json!(NOW);
    let request = json!({ "schema": super::SCHEMA, "plan": plan, "context": context });
    let answer: Value = serde_json::from_str(&super::json(&request.to_string())).unwrap();
    assert!(answer["error"].is_null(), "{answer}");
    serde_json::from_value(answer["view"]["column"].clone()).unwrap()
}

fn facts(c: &Column) -> Vec<String> {
    c.facts.iter().map(|f| f.text.clone()).collect()
}

#[test]
fn the_footer_counts_waiting_not_started_and_done_each_ticket_once() {
    let c = column(
        json!({
            "lines": [line("t1", "waiting"), line("t1", "waiting"), line("t2", "waiting"), line("t3", "waiting"), line("t4", "waiting")],
            "board_counts": { "backlog": 20, "todo": 5, "done": 796 },
        }),
        json!({ "statuses": { "t1": "todo", "t2": "todo", "t3": "backlog", "t4": "cancelled" } }),
    );
    assert_eq!(c.waiting.iter().map(|w| w.task.as_str()).collect::<Vec<_>>(), ["t1", "t2", "t3"]);
    let words: Vec<(CountKind, &str)> = c.footer.iter().map(|f| (f.kind, f.words.as_str())).collect();
    assert_eq!(
        words,
        [(CountKind::Waiting, "3 tickets waiting for a builder"), (CountKind::NotStarted, "22 not started"), (CountKind::Done, "796 done")]
    );
}

#[test]
fn initiatives_in_motion_or_asking_are_listed_the_rest_are_one_quiet_line() {
    let t = |name: &str, cards: &[&str], ordinal: i64, done: u64, total: u64, next: &str, ask: &str, state: &str| {
        with(theme(name, cards, ordinal), json!({ "counts": { "backlog": total - done, "done": done }, "next": next, "owner_ask": ask, "state": state }))
    };
    let c = column(
        json!({
            "themes": [
                t("Web client", &["t1"], 1, 20, 29, "a shared computer signs in", "", "active"),
                t("Mobile", &["t9"], 2, 16, 24, "", "the Play service account", "active"),
                t("Mac look and feel", &["t8"], 3, 33, 34, "", "", "active"),
                t("Release", &[], 4, 4, 4, "", "", "active"),
                t("Old", &[], 5, 1, 2, "", "", "done"),
            ],
            "lines": [line("t1", "building")],
        }),
        json!({}),
    );
    assert_eq!(c.initiatives.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Web client", "Mobile"]);
    assert_eq!((c.initiatives[0].next.as_deref(), c.initiatives[0].asks_owner), (Some("Next: a shared computer signs in"), false));
    assert_eq!((c.initiatives[1].next.as_deref(), c.initiatives[1].asks_owner), (Some("Waiting on you: the Play service account"), true));
    assert_eq!(c.initiatives[0].progress, "20 of 29");
    assert_eq!(c.quiet_line.as_deref(), Some("Nothing moving: Mac look and feel, 33 of 34"));
}

#[test]
fn an_empty_plan_has_an_empty_column() {
    let c = column(json!({ "lines": [] }), json!({}));
    assert!(c.pipeline.is_empty() && c.initiatives.is_empty() && c.quiet_initiatives.is_empty());
    assert!(c.footer.is_empty() && c.facts.is_empty());
}

#[test]
fn a_live_landing_with_no_ticket_aboard_is_not_the_next_landing() {
    let c = column(
        json!({
            "lines": [line("t1", "building")],
            "landings": [landing(9, "gating", &[], "gating_locally", NOW - 5 * MINUTE)],
        }),
        json!({}),
    );
    assert!(!c.facts.iter().any(|f| f.kind == FactKind::NextLanding), "{:?}", facts(&c));
}


#[test]
fn ov895_stage_age_never_replaces_reported_agent_activity() {
    let c = column(json!({
        "lines": [with(line("t1", "landing"), json!({ "stale": true, "stage_since": NOW - 180 * MINUTE }))],
        "agents": [agent("l", "lander", &["t1"], "Checking source CI", 15)],
    }), json!({}));
    let a = &c.roster[0];
    assert_eq!(a.doing, "Checking source CI");
    assert!(!a.doing_is_trouble && a.working);
    assert_eq!(c.pipeline[0].stage, PipeStage::Landing);
    assert_eq!(c.pipeline[0].assigned[0].since, NOW - 180 * MINUTE);
    assert!(c.pipeline[0].assigned[0].detail.as_ref().unwrap().contains("In stage for 3 h"));
    assert!(c.facts.is_empty());
}

#[test]
fn ov895_section_unassigned_membership_is_never_owned_by_an_agent() {
    let c = column(json!({
        "lines": [line("t1", "building"), line("t2", "building")],
        "agents": [agent("b", "builder", &["t1"], "Compiling", 5)],
    }), json!({}));
    assert!(c.roster.iter().all(|a| a.more == 0));
    let g = &c.pipeline[0];
    assert!(g.agents.is_empty());
    assert_eq!(g.assigned[0].task, "t1");
    assert_eq!(g.unattended[0].task, "t2");
    assert_eq!(g.preview[0].task, "t2");
    assert_eq!(g.count as usize, g.assigned.len() + g.unattended.len());
}

#[test]
fn ov895_canonical_stages_and_unique_roster_count_different_units() {
    let c = column(json!({
        "lines": [line("t1", "building"), line("t2", "reviewing"), line("t3", "needs_you"),
            line("t2", "reviewing"), line("t4", "reviewing"), line("t5", "landing")],
        "agents": [agent("b", "builder", &["t1", "t2", "t3"], "Checking tests", 30),
            agent("r", "reviewer", &[], "Reading guidance", 8),
            agent("l", "lander", &["t5"], "Publishing", 4),
            agent("b", "builder", &["t1", "t2", "t3"], "Checking tests", 30),
            agent("orch", "orchestrator", &[], "", 100)],
    }), json!({}));
    assert_eq!(c.agent_words, "3 agents");
    assert_eq!(c.roster_title, "Agents");
    assert_eq!(c.roster.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["b", "r", "l"]);
    assert!(c.roster[1].tickets.is_empty());
    assert_eq!(c.pipeline.iter().map(|g| (g.stage, g.count)).collect::<Vec<_>>(),
        [(PipeStage::Stuck, 1), (PipeStage::Building, 1), (PipeStage::Reviewing, 2), (PipeStage::Landing, 1)]);
    assert_eq!(c.pipeline[0].title, "Needs You");
    assert!(c.pipeline[0].assigned[0].detail.as_ref().unwrap().contains("Why t3"));
    assert_eq!(c.pipeline[2].assigned[0].task, "t2");
    assert_eq!(c.pipeline[2].unattended[0].task, "t4");
    assert_eq!(c.roster[0].doing, "Checking tests", "one activity, not an execution claim for each assignment");
    let ids: Vec<_> = c.pipeline.iter().flat_map(|g| g.assigned.iter().chain(&g.unattended)).map(|t| &t.task).collect();
    assert_eq!(ids.len(), 5);
    assert_eq!(ids.into_iter().collect::<std::collections::BTreeSet<_>>().len(), 5);
}

#[test]
fn ov895_absent_assignees_are_retained_and_ended_assignments_do_not_count() {
    let c = column(json!({ "lines": [
        with(line("t1", "reviewing"), json!({ "work": [{ "agent": "missing", "role": "review", "ended_at": null }] })),
        with(line("t2", "reviewing"), json!({ "work": [{ "agent": "old", "role": "review", "ended_at": NOW - MINUTE }, {}] })),
        with(line("t3", "reviewing"), json!({ "work": [{ "agent": "r", "role": "review" }] })),
    ], "agents": [agent("r", "reviewer", &[], "Reading", 8)] }), json!({}));
    let g = &c.pipeline[0];
    assert_eq!(g.count, 3);
    assert_eq!(g.assigned.len(), 2);
    assert!(g.assigned[0].detail.as_ref().unwrap().contains("missing (not listed)"));
    assert_eq!(g.unattended[0].task, "t2");
    assert_eq!(c.roster[0].tickets[0].task, "t3");
}

#[test]
fn ov895_activity_fallback_does_not_invent_idle_or_blocked() {
    let c = column(json!({ "agents": [
        with(agent("a", "builder", &[], "", 20), json!({ "working": false, "title": "Design ticket" })),
        agent("b", "builder", &[], "", 10),
        with(agent("c", "reviewer", &[], "Waiting on a decision", 5), json!({ "working": false })),
    ] }), json!({}));
    assert!(c.pipeline.is_empty());
    assert_eq!(c.roster.iter().map(|a| a.doing.as_str()).collect::<Vec<_>>(),
        ["No activity reported", "Runner reports working", "Waiting on a decision"]);
    assert_eq!(c.roster[0].name, "Builder 1");
    assert_eq!(c.roster[1].name, "Builder 2");
}

#[test]
fn ov895_eligibility_and_preview_partition_are_explicit() {
    let mut lines: Vec<_> = (1..=5).map(|n| line(&format!("t{n}"), "reviewing")).collect();
    lines.extend([line("t6", "waiting"), line("t7", "landed"), line("t8", "dropped"),
        line("t9", "future_stage"), line("t10", "building"), line("t11", "reviewing")]);
    let c = column(json!({ "lines": lines, "agents": [agent("r", "reviewer", &[], "Reading", 8)] }),
        json!({ "statuses": { "t10": "cancelled", "t11": "done" } }));
    assert_eq!(c.pipeline.len(), 1);
    assert_eq!(c.pipeline[0].count, 5);
    assert_eq!(c.pipeline[0].preview.len(), 3);
    assert_eq!(c.pipeline[0].hidden, 2);
    assert_eq!(c.waiting.len(), 1);
}

#[test]
fn ov895_shared_assignment_does_not_duplicate_a_ticket_and_keeps_its_initiative() {
    let c = column(json!({ "themes": [theme("Mobile", &["t1"], 1)],
        "lines": [line("t1", "reviewing")], "agents": [
            agent("b", "builder", &["t1", "t1"], "Fixing", 10),
            agent("r", "reviewer", &["t1"], "Reviewing", 5)],
    }), json!({}));
    assert_eq!(c.pipeline[0].count, 1);
    assert_eq!(c.roster.len(), 2);
    let t = &c.pipeline[0].assigned[0];
    assert_eq!(t.initiative.as_deref(), Some("Mobile"));
    assert_eq!(t.initiative_id.as_deref(), Some("theme-Mobile"));
    assert!(t.detail.as_ref().unwrap().starts_with("Assigned: Builder, Reviewer"));
    assert!(c.roster.iter().all(|a| a.tickets.len() == 1));
}

#[test]
fn ov895_landing_facts_and_agent_identity_survive_the_projection() {
    let c = column(json!({
        "lines": [with(line("t1", "landing"), json!({ "landing": 9 })), line("t2", "building")],
        "landings": [landing(9, "gating", &["t1", "t2"], "waiting_for_ci", NOW - 14 * MINUTE)],
        "agents": [with(agent("l", "lander", &["t1"], "Watching source checks", 20),
            json!({ "terminal": "term-l", "title": "Land the mobile fixes" }))],
        "board_counts": { "done": 8, "in_progress": 2 },
    }), json!({}));
    assert_eq!(facts(&c), ["Next landing: 1 ticket, 14 min", "8 of 10 done"]);
    assert_eq!(c.roster[0].terminal.as_deref(), Some("term-l"));
    assert_eq!(c.roster[0].title, "Land the mobile fixes");
    assert_eq!(c.roster[0].doing, "Watching source checks");
    assert!(c.pipeline[1].assigned[0].detail.as_ref().unwrap().contains("Landing: checking on CI"));
    let red = column(json!({
        "lines": [with(line("t1", "landing"), json!({ "landing": 9 }))],
        "landings": [landing(9, "red", &["t1"], "ci_failed", NOW - 5 * MINUTE)],
    }), json!({}));
    assert!(!red.facts.iter().any(|f| f.kind == FactKind::NextLanding));
    assert_eq!(red.pipeline[0].stage, PipeStage::Landing);
    assert!(red.pipeline[0].unattended[0].detail.as_ref().unwrap().contains("CI failed"));
}

/// The actual Rust answer drives the focused DOM and screenshot check on every UI edge.
#[test]
fn ov895_ui_fixture_is_current() {
    let plan = json!({ "now_ms": NOW, "themes": [], "lines": [
        with(line("t1", "building"), json!({ "title": "Builder's current implementation" })),
        with(line("t2", "reviewing"), json!({ "title": "Builder's earlier ticket awaiting review" })),
        with(line("t3", "reviewing"), json!({ "title": "Review ticket with no listed assignee" })),
        with(line("t4", "landing"), json!({ "title": "Source checks on an older landing", "stale": true,
            "stage_since": NOW - 180 * MINUTE, "landing": 9 })),
        with(line("t5", "needs_you"), json!({ "title": "Schema choice", "reason": "Waiting on your schema decision" })),
        with(line("t6", "reviewing"), json!({ "title": "Assignment retained after agent left the roster",
            "work": [{ "agent": "previous-reviewer", "role": "review", "ended_at": null }] })),
    ], "agents": [
        with(agent("b", "builder", &["t1", "t2"], "Running implementation tests", 30), json!({ "title": "Mobile pipeline fixes" })),
        with(agent("r", "reviewer", &[], "", 8), json!({ "working": false, "title": "Review available" })),
        agent("l", "lander", &["t4"], "Checking source CI", 15),
    ], "landings": [landing(9, "gating", &["t4"], "waiting_for_ci", NOW - 14 * MINUTE)] });
    let request = json!({ "schema": super::SCHEMA, "plan": plan, "context": { "needs_you": 1 } });
    let answer: Value = serde_json::from_str(&super::json(&request.to_string())).unwrap();
    let c: Column = serde_json::from_value(answer["view"]["column"].clone()).unwrap();
    assert_eq!(c.agent_words, "3 agents");
    assert_eq!(c.pipeline.iter().map(|g| g.count).sum::<u64>(), 6);
    assert!(facts(&c).contains(&"1 ticket needs you".into()));
    let actual = serde_json::to_string_pretty(&json!({ "request": request, "answer": answer })).unwrap() + "\n";
    let path = format!("{}/../../test/fixtures/plan-pipeline.json", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("UPDATE_PLAN_VIEW_GOLDEN").is_some() { std::fs::write(&path, &actual).unwrap(); }
    assert_eq!(actual, std::fs::read_to_string(&path).unwrap_or_default(), "regenerate with UPDATE_PLAN_VIEW_GOLDEN=1");
}

#[test]
fn ov895_next_landing_and_pipeline_share_status_eligibility() {
    let on = |t: &str| with(line(t, "landing"), json!({ "landing": 9 }));
    let c = column(json!({
        "lines": [on("t1"), on("t2"), on("t3")],
        "landings": [landing(9, "gating", &["t1", "t2", "t3"], "waiting_for_ci", NOW - MINUTE)],
    }), json!({ "statuses": { "t2": "done", "t3": "cancelled" } }));
    assert_eq!(c.pipeline[0].count, 1);
    assert_eq!(facts(&c), ["Next landing: 1 ticket, 1 min"]);
}
