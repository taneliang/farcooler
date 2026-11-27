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

/// The runner's "Main is red" entry as a row: it comes in the plan's landings,
/// not in the workflow's groups, and no numbered card draws it.
fn main_red<'a>(plan: &'a input::Plan, context: &'a input::Context) -> impl Iterator<Item = LandingGroupView> + 'a {
    plan.landings.iter().filter(|l| l.title == farcooler_core::MAIN_IS_RED).map(move |l| {
        let since = l.phase_since.unwrap_or(l.state_since);
        let mut lines = vec![LandingPhase::CiFailed.word(context.style), elapsed(plan, since)];
        lines.extend(l.tickets.iter().map(|t| ticket(plan, context, &t.key)));
        LandingGroupView { id: l.id.clone(), title: l.title.clone(), lines }
    })
}

pub(super) fn groups(plan: &input::Plan, context: &input::Context, workflow: Option<&Presentation>) -> Vec<LandingGroupView> {
    main_red(plan, context).chain(workflow.into_iter().flat_map(|w| &w.landing_groups).map(|g| {
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
}
