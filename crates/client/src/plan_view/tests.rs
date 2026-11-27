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
        v.sections
            .iter()
            .filter(|s| s.kind != SectionKind::LandedToday)
            .map(|s| serde_json::to_value(s.kind).unwrap().as_str().unwrap().replace('_', ""))
            .collect::<Vec<_>>()
            .join(" ")
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
        for style in ["apple", "material"] {
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
    assert_eq!(apple.in_review.iter().find(|c| c.key == "ov-311").unwrap().status, "In Review");
    assert_eq!(material.in_review.iter().find(|c| c.key == "ov-311").unwrap().status, "In review");
    assert!(apple.landings.is_empty());
    assert!(material.landings.is_empty());

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

/// `plan.get`'s run for `item` (ov-1040): the first stage, running, or waiting on `owner`'s approval.
fn with_run(req: &mut Value, item: &str, approval: bool) {
    let run = if approval {
        json!({ "run": "run-1", "phase": "WaitingApproval", "stage_kind": "\"Provision\"", "gate": "approval", "gate_principal": "owner" })
    } else {
        json!({ "run": "run-1", "phase": "Running", "stage_kind": "\"Provision\"", "gate": "", "gate_principal": "" })
    };
    let line = req["plan"]["lines"].as_array_mut().unwrap().iter_mut().find(|l| l["key"] == item).unwrap();
    line["run"] = run;
}

#[test]
fn a_ticket_card_carries_its_engine_run() {
    let case = cases().into_iter().find(|c| c["name"] == "board-with-phases").unwrap();
    for (approval, gated) in [(false, false), (true, true)] {
        let mut req = request(&case, "apple");
        with_run(&mut req, "ov-310", approval);
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
            with_run(&mut req, "ov-310", approval);
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
        with_run(&mut req, "ov-310", true);
        let answer: Value = serde_json::from_str(&json(&req.to_string())).unwrap();
        let actual = serde_json::to_string_pretty(&json!({ "request": req, "answer": answer })).unwrap() + "\n";
        let path = format!("{}/../../test/fixtures/plan-view-{style}.json", env!("CARGO_MANIFEST_DIR"));
        if std::env::var_os("UPDATE_PLAN_VIEW_GOLDEN").is_some() {
            std::fs::write(&path, &actual).unwrap();
        }
        assert_eq!(actual, std::fs::read_to_string(&path).unwrap_or_default(), "{path} is stale: UPDATE_PLAN_VIEW_GOLDEN=1");
    }
}

#[test]
fn old_landing_rows_do_not_affect_the_plan() {
    let plan = json!({ "lines": [{ "task": "t1", "key": "ov-1", "stage": "landing", "landing": 7 }],
        "landings": [{ "id": "l7", "number": 7, "state": "gating", "phase": "waiting_for_ci", "tickets": [{ "task": "t1" }] }] });
    let view = view_of(json!({ "schema": SCHEMA, "plan": plan, "context": {} }));
    assert!(view.landings.is_empty());
    assert!(!view.column.facts.iter().any(|f| f.kind == column::FactKind::NextLanding));
    assert!(!view.column.pipeline[0].unattended[0].detail.as_deref().unwrap().contains("Landing:"));
}

/// An engine group is a number-0 entry beside the old rows. It draws as a
/// group view only, never as a "Landing 0" card.
#[test]
fn an_engine_group_entry_draws_no_landing_card() {
    let plan = json!({ "lines": [{ "task": "t1", "key": "ov-1", "stage": "landing" }],
        "landings": [{ "id": "g", "number": 0, "state": "gating", "phase": "waiting_for_ci", "title": "Landing", "tickets": [{ "task": "t1" }] }] });
    let view = view_of(json!({ "schema": SCHEMA, "plan": plan, "context": {} }));
    assert!(view.landings.is_empty());
    assert_eq!(view.in_review.len(), 1, "the ticket stays on the Plan");
}

/// The engine's "Main is red" warning arrives as a number-0 entry, on the
/// culprit's group or on a row of its own with no tickets. With no numbered
/// cards to carry it, it draws as a group row, ahead of the engine's groups.
#[test]
fn main_is_red_still_shows_without_a_landing_card() {
    for tickets in [json!([]), json!([{ "task": "t1", "key": "ov-1" }])] {
        let carried = tickets.as_array().unwrap().len();
        let plan = json!({ "now_ms": 1_800_000_000_000i64,
            "lines": [{ "task": "t1", "key": "ov-1", "title": "Broke it", "stage": "landing" }],
            "landings": [{ "id": "red", "number": 0, "state": "red", "phase": "ci_failed",
                "phase_since": 1_800_000_000_000i64 - 120_000, "title": "Main is red", "tickets": tickets }] });
        let view = view_of(json!({ "schema": SCHEMA, "plan": plan, "context": { "style": "apple" } }));
        assert!(view.landings.is_empty());
        let groups = view.landing_groups;
        assert_eq!(groups.len(), 1);
        assert_eq!((groups[0].id.as_str(), groups[0].title.as_str()), ("red", "Main is red"));
        let mut lines = vec!["CI Failed", "2 min in this step"];
        if carried == 1 {
            lines.push("ov-1 · Broke it");
        }
        assert_eq!(groups[0].lines, lines);
        assert!(view.landing_group_heading.is_some());
        let landing = view.sections.iter().find(|s| s.kind == SectionKind::Landing).unwrap();
        assert_eq!((landing.count, landing.empty.as_deref()), (1, None));
    }
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

/// Landed Today is the tickets that landed on the viewer's day, newest first,
/// whose day ends where the context's offset from UTC says.
#[test]
fn landed_today_is_the_viewers_day_newest_first() {
    let hour = 3_600_000;
    // 22:00 UTC on a day.
    let now = 1_800_000_000_000i64 - 1_800_000_000_000i64 % (24 * hour) + 22 * hour;
    let line = |task: &str, ago: i64| {
        json!({ "task": task, "key": task, "title": task, "stage": "landed", "stage_since": now - ago * hour, "landed_sha": "ac840108f" })
    };
    let plan = json!({ "now_ms": now, "lines": [line("ov-1", 5), line("ov-2", 1), line("ov-3", 23), line("ov-4", 30)] });
    let keys = |offset: i64| {
        let view = view_of(json!({ "schema": SCHEMA, "plan": plan, "context": { "utc_offset": offset } }));
        let folded = view.sections.iter().find(|s| s.kind == SectionKind::LandedToday).map(|s| (s.count, s.folded));
        (view.landed_today.iter().map(|c| c.key.clone()).collect::<Vec<_>>(), folded)
    };
    assert_eq!(keys(0), (vec!["ov-2".into(), "ov-1".into()], Some((2, true))));
    // At +6 h it is 04:00 the next morning: ov-1 landed at 23:00 the day before.
    assert_eq!(keys(6 * 3600).0, ["ov-2"]);
    assert_eq!(keys(-8 * 3600).0, ["ov-2", "ov-1"]);
    let view = view_of(json!({ "schema": SCHEMA, "plan": plan }));
    assert_eq!(view.landed_today[0].status, "Landed · ac840108");
    // Android draws no status of its own: the sha rides in the second line.
    let material = view_of(json!({ "schema": SCHEMA, "plan": plan, "context": { "style": "material" } }));
    assert_eq!(material.landed_today[0].second, "Landed · ac840108 · ov-2");
}

/// The phrases the shell's close question, the Agents card and a card's answer
/// row share with the plan (ov-1041), pinned per style. The Swift, Kotlin and
/// TypeScript tests read `test/fixtures/plan-words.json`. Rewrite with
/// UPDATE_PLAN_VIEW_GOLDEN=1.
#[test]
fn the_shared_words_are_pinned() {
    let mut out = serde_json::Map::new();
    for style in ["apple", "material"] {
        let req = json!({ "schema": 1, "words": { "style": style } });
        let answer: Value = serde_json::from_str(&json(&req.to_string())).unwrap();
        assert_eq!(answer["view"], Value::Null);
        out.insert(style.into(), answer);
    }
    let actual = serde_json::to_string_pretty(&Value::Object(out)).unwrap() + "\n";
    let path = format!("{}/../../test/fixtures/plan-words.json", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("UPDATE_PLAN_VIEW_GOLDEN").is_some() {
        std::fs::write(&path, &actual).unwrap();
    }
    assert_eq!(actual, std::fs::read_to_string(&path).unwrap_or_default(), "{path} is stale: UPDATE_PLAN_VIEW_GOLDEN=1");
    let bad: Value = serde_json::from_str(&json(r#"{"schema":2,"words":{"style":"apple"}}"#)).unwrap();
    assert_eq!(bad["error"], "invalid");
}

#[test]
fn sol_62_model_words_for_all_plan_styles() {
    for style in ["apple", "material"] {
        let plan = json!({ "now_ms": 1_800_000_000_000i64, "lines": [], "landings": [],
            "agents": [{ "id": "pane:1", "role": "builder", "model": "gpt-6.2-sol", "working": true, "tickets": [] }] });
        let view = view_of(json!({ "schema": SCHEMA, "plan": plan, "context": { "style": style } }));
        let rendered = serde_json::to_string(&view).unwrap();
        assert!(rendered.contains("GPT-6.2 Sol"), "{style}: {rendered}");
    }
}

/// ov-1356: the engine's Main model choices must stay priced and named in the UI.
#[test]
fn main_codex_models_are_accepted_priced_and_displayed() {
    for text in [include_str!("../../../../.claude/agent/reports/ov-1071/workflow-main.json"),
        include_str!("../../../../reports/ov-1341/workflow-main-draft.json")] {
        let bundle: Value = serde_json::from_str(text).unwrap();
        let definition: farcooler_workflow::Definition = serde_json::from_value(bundle["definition"].clone()).unwrap();
        definition.validate().unwrap();
        for (step, models) in &definition.models {
            assert!(models.iter().any(|m| m == "codex:gpt-6.2-sol"), "{step} lacks 6.2 Sol");
            for model in models {
                let picks = [(step.clone(), model.clone())].into();
                assert_eq!(&definition.picked_models(&picks).unwrap()[step], model);
                // Claude's choices are moving harness aliases; Codex's are exact pricing/display IDs.
                if let Some(raw) = model.strip_prefix("codex:") {
                    assert!(farcooler_core::usage::rates(raw).is_some(), "{model} has no price");
                    assert_ne!(words::model(raw), raw, "{model} has no display name");
                }
            }
        }
    }
}
