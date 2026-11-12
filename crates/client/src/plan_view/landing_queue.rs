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
pub(super) fn groups(plan: &input::Plan, context: &input::Context, workflow: Option<&Presentation>) -> Vec<LandingGroupView> {
    workflow.into_iter().flat_map(|w| &w.landing_groups).map(|g| {
        let seconds = plan.now_ms.saturating_sub(g.since).max(0) / 1000;
        let elapsed = if g.since <= 0 { "Time unavailable".to_owned() }
            else if seconds < 60 { "Less than a minute in this step".to_owned() }
            else if seconds < 3600 { format!("{} min in this step", seconds / 60) }
            else { format!("{} hr {} min in this step", seconds / 3600, seconds % 3600 / 60) };
        let mut lines = vec![g.step.clone(), elapsed];
        for key in &g.members {
            let line = plan.lines.iter().find(|l| &l.key == key);
            let title = line.map(|l| l.title.as_str()).filter(|t| !t.is_empty())
                .or_else(|| plan.cards.iter().find(|c| &c.key == key).map(|c| c.title.as_str()).filter(|t| !t.is_empty()))
                .or_else(|| line.and_then(|l| context.titles.get(&l.task)).map(String::as_str));
            lines.push(title.map_or_else(|| key.clone(), |t| format!("{key} · {t}")));
        }
        if g.split { lines.push("Split after a gate failed".into()); }
        if let Some(f) = &g.failure {
            lines.push(format!("Failed job: {}", f.job));
            lines.push(f.first_line.clone());
        }
        LandingGroupView { id: g.id.clone(), title: g.label.clone(), lines }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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
