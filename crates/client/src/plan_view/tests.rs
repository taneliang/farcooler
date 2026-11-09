use super::*;
use serde_json::{Value, json};

fn cases() -> Vec<Value> {
    let text = include_str!("../../../../test/fixtures/plan-sections-cases.json");
    let fixture: Value = serde_json::from_str(text).unwrap();
    fixture["cases"].as_array().unwrap().clone()
}

fn view_of(request: Value) -> PlanView {
    let answer: Value = serde_json::from_str(&json(&request.to_string())).unwrap();
    assert!(answer["error"].is_null(), "{answer}");
    serde_json::from_value(answer["view"].clone()).unwrap()
}

fn request(case: &Value, style: &str) -> Value {
    json!({
        "schema": SCHEMA,
        "plan": case["plan"],
        "context": { "needs_you": case["needs_you"], "statuses": case["statuses"], "style": style },
    })
}

/// The section lines of `plan-sections-cases.json`'s outline: everything
/// before the glance, which (with the sidebar trees) is still drawn natively.
fn outline(v: &PlanView) -> Vec<String> {
    let mut out = vec![format!(
        "shown: {}",
        v.sections.iter().map(|s| serde_json::to_value(s.kind).unwrap().as_str().unwrap().replace('_', "")).collect::<Vec<_>>().join(" ")
    )];
    let shown = |k: SectionKind| v.sections.iter().find(|s| s.kind == k);
    let empty = |k: SectionKind| shown(k).and_then(|s| s.empty.clone());
    let ticket = |out: &mut Vec<String>, label: &str, c: &TicketCard| {
        let mut line = format!("  {label} {} | {}", c.key, c.status);
        if let Some(a) = &c.agent {
            line += &format!(" | agent: {}", a.who);
        }
        if c.waits_on_owner {
            line += " | owner";
        }
        if let Some(l) = &c.landed_landing {
            line += &format!(" | names landed {l}");
        }
        out.push(line);
    };
    if v.needs_you > 0 {
        out.push(format!("needs-you {}", v.needs_you));
    }
    out.push("landing".into());
    for l in &v.landings {
        out.push(format!("  landing {} | {} | {}", l.label, l.subtitle.as_deref().unwrap_or("-"), l.phase_sentence));
        out.push(format!("    ladder: {} | {}", l.ladder.compact, l.explanation));
        for t in &l.tickets {
            out.push(format!("    ticket {} | {}{}", t.key, t.note.as_deref().unwrap_or("-"), if t.waits_on_owner { " | owner" } else { "" }));
        }
    }
    if let Some(e) = empty(SectionKind::Landing) {
        out.push(format!("  empty: {e}"));
    }
    if !v.in_review.is_empty() {
        out.push("inreview".into());
        v.in_review.iter().for_each(|c| ticket(&mut out, "ticket", c));
    }
    out.push("building".into());
    v.building.iter().for_each(|c| ticket(&mut out, "ticket", c));
    // The outline says Building's empty words even where the section isn't
    // drawn (nothing is building and nothing is next).
    if let Some(e) = empty(SectionKind::Building).or_else(|| {
        (shown(SectionKind::Building).is_none() && v.building.is_empty()).then(|| "Nothing is building.".to_string())
    }) {
        out.push(format!("  empty: {e}"));
    }
    out.push("waiting".into());
    for g in &v.waiting {
        let label = serde_json::to_value(g.kind).unwrap().as_str().unwrap().replace('_', "");
        g.cards.iter().for_each(|c| ticket(&mut out, &label, c));
    }
    if let Some(e) = empty(SectionKind::Waiting) {
        out.push(format!("  empty: {e}"));
    }
    if let Some(s) = shown(SectionKind::OnLandedLanding) {
        out.push(format!("onlandedlanding (folded: {})", s.folded));
        v.on_landed_landing.iter().for_each(|c| ticket(&mut out, "ticket", c));
    }
    if shown(SectionKind::Agents).is_some() {
        out.push("agents".into());
        out.push(format!("  header: {}", v.agents.header));
        for a in &v.agents.rows {
            out.push(format!(
                "  {} {} | {} | {} | {} | {}",
                if a.working { "working" } else { "idle" },
                a.who,
                a.works_for,
                a.state,
                a.tokens.as_deref().unwrap_or("-"),
                a.cost.as_deref().unwrap_or("-")
            ));
        }
    }
    out.push("themes".into());
    for t in &v.themes.tiles {
        out.push(format!(
            "  tile {} | {} | {} | ask: {}",
            t.name,
            t.progress,
            if t.moving { "moving" } else { "still" },
            t.owner_ask.as_deref().unwrap_or("-")
        ));
        for c in &t.cards {
            let glyph = serde_json::to_value(c.glyph).unwrap().as_str().unwrap().replace('_', "");
            out.push(format!("    card {glyph} {}", c.key));
        }
    }
    if let (Some(title), Some(words)) = (&v.themes.finished_title, &v.themes.finished_words) {
        out.push(format!("  {title} | {words}: {}", v.themes.finished.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")));
    }
    out.into_iter().map(|l| l.to_lowercase().trim_end().to_string()).collect()
}

#[test]
fn sections_match_the_shared_cases() {
    let all = cases();
    assert!(all.len() >= 10);
    for case in &all {
        let expect: Vec<String> = case["expect"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().to_string())
            .take_while(|l| !l.starts_with("glance now"))
            .collect();
        assert!(!expect.is_empty(), "{} has an outline", case["name"]);
        for style in ["apple", "material", "web"] {
            let got = outline(&view_of(request(case, style)));
            assert_eq!(got, expect, "{} in {style}", case["name"]);
        }
    }
}

#[test]
fn titles_follow_the_platform_case_but_never_a_login() {
    let case = cases().into_iter().find(|c| c["name"] == "board-no-phases").unwrap();
    let apple = view_of(request(&case, "apple"));
    let material = view_of(request(&case, "material"));
    let title = |v: &PlanView, k: SectionKind| v.sections.iter().find(|s| s.kind == k).unwrap().title.clone();
    assert_eq!(title(&apple, SectionKind::InReview), "In Review");
    assert_eq!(title(&material, SectionKind::InReview), "In review");
    assert_eq!(title(&apple, SectionKind::NeedsYou), "Needs You");
    assert_eq!(title(&material, SectionKind::NeedsYou), "Needs you");
    assert_eq!(apple.in_review[0].status, "In Review");
    assert_eq!(material.in_review[0].status, "In review");
    assert_eq!(apple.landings[0].phase_word, "Waiting for Tickets");
    assert_eq!(material.landings[0].phase_word, "Waiting for tickets");

    let mut plan = case["plan"].clone();
    let line = plan["lines"].as_array_mut().unwrap().iter_mut().find(|l| l["key"] == "ov-311").unwrap();
    line["pr_stage"] = json!({ "kind": "waiting_on_reviewer", "reviewers": ["alice", "bob"] });
    let mut with_stage = case.clone();
    with_stage["plan"] = plan;
    let apple = view_of(request(&with_stage, "apple"));
    let card = apple.in_review.iter().find(|c| c.key == "ov-311").unwrap();
    assert_eq!(card.status, "Waiting on alice and bob");
    let note = card.notes.iter().find(|n| n.kind == NoteKind::Stage).unwrap();
    assert_eq!(note.text, "Waiting on alice and bob");
    assert!(!note.attention);
}

#[test]
fn title_and_sentence_case() {
    use words::{sentence_case, title_case};
    assert_eq!(title_case("waiting for the next landing"), "Waiting for the Next Landing");
    assert_eq!(title_case("waiting for main's checks to pass"), "Waiting for Main's Checks to Pass");
    assert_eq!(title_case("checking on CI"), "Checking on CI");
    assert_eq!(title_case("waiting on you"), "Waiting on You");
    assert_eq!(title_case("approved, checks running"), "Approved, Checks Running");
    assert_eq!(sentence_case("Awaiting Approval"), "Awaiting approval");
    assert_eq!(sentence_case("Check CI"), "Check CI");
    assert_eq!(sentence_case("Open Pull Request"), "Open pull request");
}

/// A workspace's workflow presentation with one run for `item`, as `workflow.snapshot` answers it.
fn presentation(item: &str, approval: bool) -> crate::workflow_presentation::Presentation {
    use farcooler_workflow::{Command, Definition, Grant, Input, Progression, State, step};
    let mut definition = Definition::coding();
    if approval {
        definition.stages[0].progression = Progression::AwaitApproval { principal: "owner".into() };
    }
    let state = step(
        &State::empty("workspace".into(), 1),
        &Input {
            schema: 1,
            id: "event-0".into(),
            workspace: "workspace".into(),
            expected_revision: 0,
            epoch: 1,
            source: "fixture".into(),
            source_sequence: 1,
            now_ms: 1,
            grant: Grant::Controller,
            command: Command::Enqueue {
                item: item.into(),
                definition,
                policy_pin: "policy".into(),
                resource: "resource".into(),
                dependencies: Default::default(),
                initial_candidate: None,
            },
            busy_resources: Default::default(),
        },
    )
    .unwrap()
    .state;
    let auth = crate::workflow_presentation::Capabilities {
        workspace: "workspace".into(),
        epoch: 1,
        inspect: true,
        controller: true,
        decision_principal: Some("owner".into()),
    };
    crate::workflow_presentation::project(&state, &auth).unwrap()
}

#[test]
fn a_ticket_card_carries_its_engine_run() {
    let case = cases().into_iter().find(|c| c["name"] == "board-with-phases").unwrap();
    for (approval, gated) in [(false, false), (true, true)] {
        let mut req = request(&case, "apple");
        req["workflow"] = serde_json::to_value(presentation("OV-310", approval)).unwrap();
        let view = view_of(req);
        let card = view.building.iter().find(|c| c.key == "ov-310").unwrap();
        let run = card.run.as_ref().expect("the run on ov-310's card");
        assert_eq!(run.step.as_deref(), Some("Prepare Workspace"));
        assert_eq!(run.gate.is_some(), gated);
        assert_eq!(run.attention, gated);
        let note = card.notes.iter().find(|n| n.kind == NoteKind::Run).unwrap();
        if gated {
            assert_eq!(run.phase_label, "Awaiting Approval");
            assert_eq!(note.text, "Prepare Workspace · Approval Required");
            assert_eq!(run.gate.as_ref().unwrap().text, "Waiting for approval from owner.");
        } else {
            assert_eq!(run.phase_label, "Running");
            assert_eq!(note.text, "Prepare Workspace · Running");
        }
        // Its agent stays the plan's: the builder working it.
        assert_eq!(card.agent.as_ref().unwrap().who, "Builder · sonnet");
        // No other card claims the run.
        assert!(view.building.iter().chain(&view.in_review).filter(|c| c.run.is_some()).count() == 1);

        let material = {
            let mut req = request(&case, "material");
            req["workflow"] = serde_json::to_value(presentation("ov-310", approval)).unwrap();
            view_of(req)
        };
        let run = material.building.iter().find(|c| c.key == "ov-310").unwrap().run.clone().unwrap();
        assert_eq!(run.step.as_deref(), Some("Prepare workspace"));
    }
}

#[test]
fn a_bad_request_says_invalid() {
    for bad in ["", "{}", "{\"schema\": 9, \"plan\": {}}", "{\"schema\": 1, \"plan\": 3}"] {
        let answer: Value = serde_json::from_str(&json(bad)).unwrap();
        assert_eq!(answer["error"], "invalid", "{bad}");
        assert!(answer["view"].is_null());
    }
}

#[cfg(feature = "native")]
#[test]
fn the_c_bridge_answers_the_same_bytes() {
    let case = &cases()[0];
    let input = request(case, "apple").to_string();
    let output = json(&input);
    unsafe {
        let request = std::ffi::CString::new(input).unwrap();
        let bridge = crate::ffi::farcooler_client_plan_view;
        let needed = bridge(request.as_ptr(), std::ptr::null_mut(), 0);
        let mut bytes = vec![0; needed];
        assert_eq!(bridge(request.as_ptr(), bytes.as_mut_ptr(), bytes.len()), needed);
        assert_eq!(bytes, output.as_bytes());
    }
}

/// The view the apps decode, pinned: `board-with-phases` with an engine run
/// on ov-310, in each case. The Swift, Kotlin and TypeScript renderers' own
/// tests read these files, so a field renamed here fails there too. Rewrite
/// with UPDATE_PLAN_VIEW_GOLDEN=1.
#[test]
fn the_view_golden_is_current() {
    let case = cases().into_iter().find(|c| c["name"] == "board-with-phases").unwrap();
    for style in ["apple", "material"] {
        let mut req = request(&case, style);
        req["workflow"] = serde_json::to_value(presentation("ov-310", true)).unwrap();
        let answer: Value = serde_json::from_str(&json(&req.to_string())).unwrap();
        let actual = serde_json::to_string_pretty(&json!({ "request": req, "answer": answer })).unwrap() + "\n";
        let path = format!("{}/../../test/fixtures/plan-view-{style}.json", env!("CARGO_MANIFEST_DIR"));
        if std::env::var_os("UPDATE_PLAN_VIEW_GOLDEN").is_some() {
            std::fs::write(&path, &actual).unwrap();
        }
        assert_eq!(actual, std::fs::read_to_string(&path).unwrap_or_default(), "{path} is stale: UPDATE_PLAN_VIEW_GOLDEN=1");
    }
}

/// A ticket on a landing carries what the Apple apps draw beside its title:
/// its mark, its warnings (stale, over budget) as notes, its branch and what a
/// screen reader says; and an agent names its raw model for the screen reader.
#[test]
fn a_landing_ticket_carries_its_mark_and_warnings() {
    let hour = 3_600_000;
    let now = 1_800_000_000_000i64;
    let line = |task: &str, stage: &str, extra: Value| {
        let mut l = json!({ "task": task, "key": task, "title": format!("Title of {task}"), "stage": stage,
            "branch": format!("lane-{task}"), "stage_since": now - 30 * hour, "landing": 7 });
        l.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        l
    };
    let plan = json!({
        "now_ms": now,
        "lines": [
            line("ov-1", "building", json!({ "stale": true, "budget_tokens": 10, "spend": { "input_tokens": 20 } })),
            line("ov-2", "landing", json!({})),
            line("ov-3", "building", json!({ "work": [{ "agent": "a", "role": "fix" }], "fix_rounds": 2 })),
            line("ov-4", "needs_you", json!({ "stale": true })),
        ],
        "landings": [{ "id": "l7", "number": 7, "state": "open", "tickets": [] }],
        "agents": [{ "id": "pane:l", "role": "lander", "model": "claude-opus-4-1", "working": true, "tickets": [] }],
    });
    let mut plan = plan;
    plan["landings"][0]["lander"] = json!("pane:l");
    let view = view_of(json!({ "schema": SCHEMA, "plan": plan, "context": { "style": "apple" } }));
    let card = &view.landings[0];
    assert_eq!(card.lander.as_ref().unwrap().model, "claude-opus-4-1");
    let ticket = |key: &str| card.tickets.iter().find(|t| t.key == key).unwrap();

    let stale = ticket("ov-1");
    assert_eq!(stale.mark, Mark::Building);
    assert!(stale.mark_attention);
    assert_eq!(stale.branch, "lane-ov-1");
    let notes: Vec<(NoteKind, &str)> = stale.notes.iter().map(|n| (n.kind, n.text.as_str())).collect();
    assert_eq!(notes, [(NoteKind::Warning, "No move in 30 h"), (NoteKind::OverBudget, "Over budget: 20 of 10 tokens")]);
    assert_eq!(stale.accessibility, "Title of ov-1, No move in 30 h, Over budget. 20 of 10 tokens used.");

    // The line under its title: its key, and how long it has stood, unless a
    // warning says so; a landed one leads with its commit.
    assert_eq!(stale.second, "ov-1");
    let ready = ticket("ov-2");
    assert_eq!((ready.mark, ready.mark_attention, ready.notes.len()), (Mark::Landing, false, 0));
    assert_eq!(ready.second, "ov-2 · 1 d");
    assert_eq!(ticket("ov-3").mark, Mark::Fixing);

    // Waiting on the owner says "Needs you" once, as its note: never stale too.
    let owner = ticket("ov-4");
    assert!(owner.notes.is_empty());
    assert_eq!(owner.second, "ov-4");
    assert_eq!(owner.accessibility, "Title of ov-4, Needs you");
}

/// Android draws a working ticket's agent as a line of its own, and its second
/// line says only the stage and key; the Apple apps and the web say the agent
/// and the time in stage in the second line.
#[test]
fn androids_second_line_leaves_the_agent_to_its_own_line() {
    let case = cases().into_iter().find(|c| c["name"] == "board-with-phases").unwrap();
    let card = |style: &str| view_of(request(&case, style)).building.into_iter().find(|c| c.key == "ov-310").unwrap();
    let (apple, material) = (card("apple"), card("material"));
    assert_eq!(apple.second, "Building · ov-310 · Builder · sonnet · 380K · 1 h");
    assert_eq!(material.second, "Building · ov-310");
    assert_eq!(material.agent.unwrap().line, "Builder · sonnet · 380K");
}
