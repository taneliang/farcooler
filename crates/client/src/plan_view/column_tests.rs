//! The plan column, case by case, as AgentKit's `PlanSectionsColumnTests` and
//! the web's `plan-column.test.ts` held it before it moved here (ov-1016).

use serde_json::{Value, json};

use super::column::{Column, CountKind, FactKind, PipeStage, Role};

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
fn each_agent_is_one_card_in_its_tickets_stage_with_each_tickets_own_initiative() {
    let mut web = theme("Web client", &["t1"], 1);
    web["outcome"] = json!("You use Far Cooler from any browser");
    let c = column(
        json!({
            "themes": [web, theme("Mobile experience", &["t2", "t3"], 2)],
            "lines": [line("t1", "building"), line("t2", "building"), line("t3", "reviewing")],
            "agents": [
                agent("orchestrator", "orchestrator", &[], "", 300),
                with(agent("b", "builder", &["t1", "t2"], "Reading the chat list's scroll code", 30), json!({ "terminal": "term-b" })),
                agent("r", "reviewer", &["t3"], "", 8),
            ],
            "cards": [
                { "task": "t1", "key": "ov-1", "title": "A paired browser starts after an update", "status": "In Progress" },
                { "task": "t2", "key": "ov-2", "title": "Chat scrolling stutters", "status": "In Progress" },
            ],
        }),
        json!({}),
    );
    assert_eq!(c.pipeline.iter().map(|g| g.stage).collect::<Vec<_>>(), [PipeStage::Building, PipeStage::Reviewing]);
    let builder = &c.pipeline[0].agents[0];
    assert_eq!((builder.name.as_str(), builder.role, builder.terminal.as_deref()), ("Builder", Role::Builder, Some("term-b")));
    assert_eq!((builder.doing.as_str(), builder.time.as_deref()), ("Reading the chat list's scroll code", Some("30 min")));
    assert_eq!(builder.tickets.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["A paired browser starts after an update", "Chat scrolling stutters"]);
    assert_eq!(builder.tickets.iter().map(|t| t.initiative.as_deref()).collect::<Vec<_>>(), [Some("Web client"), Some("Mobile experience")]);
    let reviewer = &c.pipeline[1].agents[0];
    assert_eq!(reviewer.tickets[0].title, "ov-3", "no title anywhere: the key");
    assert_eq!(reviewer.doing, "Working", "it said nothing, so it is working");
    assert_eq!(c.agent_words, "2 agents", "the orchestrator isn't in the pipeline");
}

#[test]
fn an_agent_with_no_words_says_its_title_else_working_or_idle_and_several_are_numbered() {
    let c = column(
        json!({
            "lines": [line("t1", "building"), line("t2", "building")],
            "agents": [
                with(agent("t", "builder", &["t1"], "", 9), json!({ "title": "Design ov-1" })),
                with(agent("i", "builder", &["t2"], "", 40), json!({ "working": false })),
            ],
        }),
        json!({}),
    );
    let names: Vec<String> = c.pipeline[0].agents.iter().map(|a| format!("{}={}={}", a.id, a.name, a.doing)).collect();
    assert_eq!(names, ["i=Builder 1=Idle", "t=Builder 2=Design ov-1"], "start order: i started first");
}

#[test]
fn a_stale_ticket_puts_its_agent_in_stuck_saying_why_and_the_facts_count_it() {
    let c = column(
        json!({
            "lines": [with(line("t1", "building"), json!({ "stale": true, "stage_since": NOW - 150 * MINUTE })), line("t2", "building")],
            "agents": [agent("s", "builder", &["t1"], "Running the tests", 47), agent("ok", "builder", &["t2"], "", 3)],
        }),
        json!({ "needs_you": 2 }),
    );
    let stuck = &c.pipeline[0];
    assert_eq!(stuck.stage, PipeStage::Stuck);
    assert_eq!((stuck.agents[0].doing.as_str(), stuck.agents[0].doing_is_trouble), ("No move in 2 h", true));
    assert_eq!(facts(&c), ["2 need you", "1 stuck, 47 min"]);
    assert!(c.facts.iter().all(|f| f.attention));
}

#[test]
fn tickets_nobody_is_on_are_said_under_their_stage() {
    let lines: Vec<Value> = ["t1", "t2", "t3"].iter().map(|t| line(t, "reviewing")).chain([line("t4", "building")]).collect();
    let c = column(json!({ "lines": lines, "agents": [agent("r", "reviewer", &["t1"], "", 8)] }), json!({}));
    let reviewing = c.pipeline.iter().find(|g| g.stage == PipeStage::Reviewing).unwrap();
    assert_eq!((reviewing.count, reviewing.unattended.len()), (3, 2));
    assert_eq!(reviewing.unattended_words.as_deref(), Some("2 more waiting for review"));
    assert_eq!(reviewing.agents.last().unwrap().more, 2);
    let building = c.pipeline.iter().find(|g| g.stage == PipeStage::Building).unwrap();
    assert!(building.agents.is_empty());
    assert_eq!(building.unattended_words.as_deref(), Some("1 ticket with no builder running"));
}

#[test]
fn a_ticket_is_on_one_agent_the_lander_first_then_the_one_whose_job_fits() {
    let c = column(
        json!({
            "lines": [with(line("t1", "landing"), json!({ "landing": 9 })), line("t2", "building")],
            "landings": [landing(9, "gating", &["t1"], "gating_locally", NOW - MINUTE)],
            "agents": [
                agent("b1", "builder", &["t1"], "", 9), agent("i", "lander", &["t1"], "", 20),
                agent("b2", "builder", &["t2"], "", 8), agent("b3", "builder", &["t2"], "", 7),
            ],
        }),
        json!({}),
    );
    let tickets = |id: &str| -> Vec<String> {
        c.pipeline.iter().flat_map(|g| &g.agents).find(|a| a.id == id).unwrap().tickets.iter().map(|t| t.task.clone()).collect()
    };
    assert_eq!(tickets("i"), ["t1"]);
    assert_eq!(tickets("b2"), ["t2"]);
    assert!(tickets("b3").is_empty() && tickets("b1").is_empty());
    assert_eq!(c.pipeline.iter().map(|g| g.count).sum::<u64>(), 2);
}

#[test]
fn a_live_landings_lander_and_the_next_landing_on_the_facts_line() {
    let on = |t: &str, stage: &str| with(line(t, stage), json!({ "landing": 9 }));
    let c = column(
        json!({
            "lines": [on("t1", "landing"), on("t2", "landing"), on("t3", "landing"), on("t4", "building")],
            "landings": [landing(9, "gating", &["t1", "t2", "t3", "t4"], "gating_locally", NOW - 14 * MINUTE)],
            "agents": [agent("i", "lander", &["t1", "t2", "t3"], "Checking here", 20)],
            "board_counts": { "backlog": 20, "todo": 2, "in_progress": 5, "in_review": 3, "done": 282, "cancelled": 4 },
        }),
        json!({}),
    );
    let lander = &c.pipeline.iter().find(|g| g.stage == PipeStage::Landing).unwrap().agents[0];
    assert_eq!((lander.role, lander.doing.as_str()), (Role::Lander, "Checking here"));
    assert_eq!(lander.tickets.iter().map(|t| t.task.as_str()).collect::<Vec<_>>(), ["t1", "t2", "t3"]);
    assert_eq!(c.pipeline.iter().find(|g| g.stage == PipeStage::Building).unwrap().unattended[0].task, "t4");
    assert_eq!(facts(&c), ["Next landing: 3 tickets, 14 min", "282 of 312 done"]);
}

#[test]
fn a_landing_whose_ci_failed_puts_its_lander_in_stuck_and_is_not_the_next_landing() {
    let c = column(
        json!({
            "lines": [with(line("t1", "landing"), json!({ "landing": 9 }))],
            "landings": [landing(9, "red", &["t1"], "ci_failed", NOW - 5 * MINUTE)],
            "agents": [with(agent("i", "lander", &["t1"], "", 20), json!({ "title": "Landing six" }))],
        }),
        json!({}),
    );
    let lander = &c.pipeline[0].agents[0];
    assert_eq!(c.pipeline[0].stage, PipeStage::Stuck);
    assert_eq!((lander.doing.as_str(), lander.doing_is_trouble), ("CI failed", true));
    assert!(!c.facts.iter().any(|f| f.kind == FactKind::NextLanding));
}

#[test]
fn a_stage_with_no_agent_previews_three_and_with_agents_previews_none() {
    let lines: Vec<Value> = (1..=5).map(|n| line(&format!("t{n}"), "reviewing")).collect();
    let bare = column(json!({ "lines": lines }), json!({}));
    assert_eq!(bare.pipeline[0].preview.iter().map(|t| t.task.as_str()).collect::<Vec<_>>(), ["t1", "t2", "t3"]);
    assert_eq!(bare.pipeline[0].hidden, 2);
    let staffed = column(json!({ "lines": lines, "agents": [agent("r", "reviewer", &["t1"], "", 8)] }), json!({}));
    assert!(staffed.pipeline[0].preview.is_empty());
    assert_eq!(staffed.pipeline[0].hidden, 4);
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
fn a_ticket_that_needs_the_owner_puts_its_agent_in_stuck() {
    let c = column(
        json!({
            "lines": [line("t1", "needs_you")],
            "agents": [agent("b", "builder", &["t1"], "Asking about the schema", 5)],
        }),
        json!({}),
    );
    assert_eq!(c.pipeline.len(), 1);
    assert_eq!(c.pipeline[0].stage, PipeStage::Stuck);
    assert_eq!(c.pipeline[0].agents[0].doing, "Asking about the schema");
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
