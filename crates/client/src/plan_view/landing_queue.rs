//! The landing queue's words, ordering, titles and elapsed times belong to the shared presenter.
use super::*;

#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LandingGroupView {
    pub id: String,
    pub title: String,
    /// Complete lines, in reading order; renderers only wrap and lay them out.
    pub lines: Vec<String>,
}
fn elapsed(plan: &input::Plan, since: i64) -> String {
    let seconds = plan.now_ms.saturating_sub(since).max(0) / 1000;
    if since <= 0 { "Time unavailable".to_owned() }
    else if seconds < 60 { "Less than a minute in this step".to_owned() }
    else if seconds < 3600 { format!("{} min in this step", seconds / 60) }
    else { format!("{} hr {} min in this step", seconds / 3600, seconds % 3600 / 60) }
}

fn ticket(plan: &input::Plan, context: &input::Context, key: &str) -> String {
    let line = plan.lines.iter().find(|l| l.key == key);
    let title = line.map(|l| l.title.as_str()).filter(|t| !t.is_empty())
        .or_else(|| plan.cards.iter().find(|c| c.key == key).map(|c| c.title.as_str()).filter(|t| !t.is_empty()))
        .or_else(|| line.and_then(|l| context.titles.get(&l.task)).map(String::as_str));
    title.map_or_else(|| key.to_owned(), |t| format!("{key} · {t}"))
}

/// The runner's own number-0 entries as rows, which no numbered card draws.
/// "Main is red" isn't one of the workflow's groups, so it always gets one. The
/// engine's groups get theirs here only without a workflow, which is what a
/// read-only runner, or one with no workflow service, sends (ov-1520).
fn runner_rows<'a>(plan: &'a input::Plan, context: &'a input::Context, workflow: bool) -> impl Iterator<Item = LandingGroupView> + 'a {
    plan.landings.iter().filter_map(move |l| {
        let main_red = l.title == farcooler_core::MAIN_IS_RED;
        let live = l.number == 0 && l.state != "landed" && l.state != "dropped";
        if !main_red && (workflow || !live) {
            return None;
        }
        let phase = if main_red { LandingPhase::CiFailed } else { LandingPhase::of(l.phase.as_deref().unwrap_or_default()) };
        let since = l.phase_since.unwrap_or(l.state_since);
        let mut lines = vec![phase.word(context.style), elapsed(plan, since)];
        lines.extend(l.tickets.iter().map(|t| ticket(plan, context, &t.key)));
        Some(LandingGroupView { id: l.id.clone(), title: l.title.clone(), lines })
    })
}

pub(super) fn groups(plan: &input::Plan, context: &input::Context, workflow: Option<&Presentation>) -> Vec<LandingGroupView> {
    runner_rows(plan, context, workflow.is_some()).chain(workflow.into_iter().flat_map(|w| &w.landing_groups).map(|g| {
        let mut lines = vec![g.step.clone(), elapsed(plan, g.since)];
        if let Some(wait) = &g.heavy_lock_wait { lines.push(wait.clone()); }
        lines.extend(g.members.iter().map(|key| ticket(plan, context, key)));
        if g.split { lines.push("Split after a gate failed".into()); }
        if let Some(f) = &g.failure {
            lines.push(format!("Failed job: {}", f.job));
            lines.push(f.first_line.clone());
        }
        LandingGroupView { id: g.id.clone(), title: g.label.clone(), lines }
    })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn landing_row_shows_the_heavy_lock_holder() {
        let mut fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../test/fixtures/landing-queue.json")).unwrap();
        let message = "Waiting for the machine-wide heavy-job lock. Holder PID 42 (/builder) is paused.";
        fixture["request"]["plan"]["workflow"]["landingGroups"][0]["heavyLockWait"] = message.into();
        let answer: serde_json::Value = serde_json::from_str(&super::super::json(&fixture["request"].to_string())).unwrap();
        assert!(answer["view"]["landingGroups"][0]["lines"].as_array().unwrap().contains(&message.into()), "{answer}");
    }
    #[test]
    fn queue_golden_covers_checking_waiting_split_failure_and_long_titles() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../test/fixtures/landing-queue.json")).unwrap();
        for style in ["apple", "material"] {
            let mut request = fixture["request"].clone();
            request["context"] = serde_json::json!({ "style": style });
            let answer: serde_json::Value = serde_json::from_str(&super::super::json(&request.to_string())).unwrap();
            assert!(answer["error"].is_null(), "{answer}");
            assert_eq!(answer["view"]["landingGroups"], fixture["expected"]);
            let landing = answer["view"]["sections"].as_array().unwrap().iter().find(|s| s["kind"] == "landing").unwrap();
            assert_eq!(answer["view"]["landingGroupHeading"], landing["title"]);
            assert_eq!(landing["count"], 4);
            assert!(landing["empty"].is_null(), "groups replace the empty section");
        }
    }
    #[test]
    fn no_workflow_and_future_clocks_do_not_invent_time_or_groups() {
        let plan = input::Plan::default();
        assert!(groups(&plan, &input::Context::default(), None).is_empty());
        let mut fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../test/fixtures/landing-queue.json")).unwrap();
        fixture["request"]["plan"]["now_ms"] = 1.into();
        let answer: serde_json::Value = serde_json::from_str(&super::super::json(&fixture["request"].to_string())).unwrap();
        assert_eq!(answer["view"]["landingGroups"][0]["lines"][1], "Less than a minute in this step");
    }
    /// A read-only runner, or one with no workflow service, sends the plan
    /// without a workflow: its number-0 entries are all there is to draw (ov-1520).
    #[test]
    fn an_engine_landing_still_has_a_row_without_the_workflow() {
        let mut fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../test/fixtures/landing-queue.json")).unwrap();
        let mut request = fixture["request"].take();
        request["context"] = serde_json::json!({ "style": "apple" });
        request["plan"]["landings"] = serde_json::json!([{ "id": "engine", "number": 0, "title": "Landing", "state": "gating",
            "phase": "gating_locally", "phase_since": 1, "tickets": [] }]);
        // With the workflow, its groups are the rows and the entry adds none.
        let with: serde_json::Value = serde_json::from_str(&super::super::json(&request.to_string())).unwrap();
        assert!(!with["view"]["landingGroups"].as_array().unwrap().iter().any(|g| g["id"] == "engine"), "{with}");
        request["plan"].as_object_mut().unwrap().remove("workflow");
        let without: serde_json::Value = serde_json::from_str(&super::super::json(&request.to_string())).unwrap();
        let rows = without["view"]["landingGroups"].as_array().expect("a row");
        assert_eq!(rows.len(), 1, "{without}");
        assert_eq!((&rows[0]["title"], &rows[0]["lines"][0]), (&"Landing".into(), &"Checking Here".into()), "{without}");
        let section = without["view"]["sections"].as_array().unwrap().iter().find(|s| s["kind"] == "landing").unwrap();
        assert_eq!(section["count"], 1);
        assert_eq!(without["view"]["landingGroupHeading"], "Landing");
    }
}
