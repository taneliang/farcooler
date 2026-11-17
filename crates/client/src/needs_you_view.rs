//! Cross-runner aggregation only. No clock, transport, classification or actions.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use typeshare::typeshare;

#[derive(Deserialize)]
pub struct RunnerInput {
    pub runner_id: String,
    pub items: Vec<Value>,
}

#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeedsYouRowKey {
    pub runner_id: String,
    pub item_id: String,
}

#[typeshare]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NeedsYouAggregateRow {
    pub key: NeedsYouRowKey,
    /// The input payload, including unknown fields and action IDs, unchanged.
    pub item_json: String,
    pub group: String,
    pub counted: bool,
}

#[typeshare]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NeedsYouBucket {
    pub runner_id: String,
    /// Empty for runner totals; workspace or repository ID otherwise.
    pub id: String,
    pub count: u32,
}

#[typeshare]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NeedsYouAggregateView {
    pub schema: u32,
    pub rows: Vec<NeedsYouAggregateRow>,
    pub yours: Vec<NeedsYouRowKey>,
    pub others: Vec<NeedsYouRowKey>,
    pub split: bool,
    pub counted_total: u32,
    pub runner_counts: Vec<NeedsYouBucket>,
    pub workspace_counts: Vec<NeedsYouBucket>,
    pub unclaimed_counts: Vec<NeedsYouBucket>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvalidInput { Invalid, DuplicateRunner, DuplicateItem }

fn known(kind: &str) -> bool {
    matches!(kind, "ask" | "blocked" | "decision" | "review" | "pr_review" | "pr_thread" | "claude_login" | "fyi")
}

pub fn aggregate(runners: Vec<RunnerInput>) -> Result<NeedsYouAggregateView, InvalidInput> {
    let mut runner_counts = BTreeMap::new();
    let mut items = Vec::new();
    for runner in runners {
        if runner_counts.insert(runner.runner_id.clone(), 0).is_some() { return Err(InvalidInput::DuplicateRunner); }
        let mut ids = BTreeSet::new();
        for item in runner.items {
            let id = item["id"].as_str().ok_or(InvalidInput::Invalid)?.to_owned();
            let kind = item["kind"].as_str().ok_or(InvalidInput::Invalid)?.to_owned();
            let rank = item["rank"].as_u64().filter(|n| *n <= u32::MAX as u64).ok_or(InvalidInput::Invalid)?;
            if !ids.insert(id.clone()) { return Err(InvalidInput::DuplicateItem); }
            for field in ["workspace_id", "repository_id"] {
                if let Some(value) = item.get(field) {
                    if !value.is_null() && !value.is_string() { return Err(InvalidInput::Invalid); }
                }
            }
            if let Some(concern) = item.get("concern").filter(|v| !v.is_null()) {
                if concern["yours"].as_bool().is_none() { return Err(InvalidInput::Invalid); }
            }
            items.push((!known(&kind), rank, runner.runner_id.clone(), id, kind, item));
        }
    }
    items.sort_by(|a, b| (&a.0, &a.1, &a.2, &a.3).cmp(&(&b.0, &b.1, &b.2, &b.3)));
    let mut seen = BTreeSet::new();
    let mut view = NeedsYouAggregateView { schema: 1, rows: vec![], yours: vec![], others: vec![], split: false,
        counted_total: 0, runner_counts: vec![], workspace_counts: vec![], unclaimed_counts: vec![] };
    let mut workspaces = BTreeMap::new();
    let mut unclaimed = BTreeMap::new();
    for (_, _, runner, id, kind, item) in items {
        if kind == "pr_review" && !seen.insert(id.clone()) { continue; }
        let key = NeedsYouRowKey { runner_id: runner.clone(), item_id: id };
        let others = item["concern"]["yours"].as_bool() == Some(false);
        let counted = kind != "fyi";
        if others { view.others.push(key.clone()); } else { view.yours.push(key.clone()); }
        if counted {
            view.counted_total += 1;
            *runner_counts.get_mut(&runner).expect("input runner") += 1;
            if let Some(workspace) = item["workspace_id"].as_str() {
                *workspaces.entry((runner.clone(), workspace.to_owned())).or_insert(0) += 1;
            } else if let Some(repository) = item["repository_id"].as_str() {
                *unclaimed.entry((runner.clone(), repository.to_owned())).or_insert(0) += 1;
            }
        }
        view.rows.push(NeedsYouAggregateRow { key, item_json: item.to_string(), group: if others { "others" } else { "yours" }.into(), counted });
    }
    view.split = !view.others.is_empty();
    view.runner_counts = runner_counts.into_iter().map(|(runner_id, count)| NeedsYouBucket { runner_id, id: String::new(), count }).collect();
    let buckets = |map: BTreeMap<(String, String), u32>| map.into_iter().map(|((runner_id, id), count)| NeedsYouBucket { runner_id, id, count }).collect();
    view.workspace_counts = buckets(workspaces);
    view.unclaimed_counts = buckets(unclaimed);
    Ok(view)
}

/// Schema-versioned envelope carried over the existing native/wasm view-model bridge.
pub fn json(request: &str) -> String {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request { schema: u32, needs_you: Vec<RunnerInput> }
    let result = serde_json::from_str::<Request>(request).map_err(|_| InvalidInput::Invalid)
        .and_then(|r| if r.schema == 1 { aggregate(r.needs_you) } else { Err(InvalidInput::Invalid) });
    match result {
        Ok(view) => serde_json::json!({"schema":1,"needs_you":view}).to_string(),
        Err(error) => serde_json::json!({"schema":1,"error":error}).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view(runners: Value) -> NeedsYouAggregateView {
        aggregate(serde_json::from_value(runners).unwrap()).unwrap()
    }

    #[test]
    fn order_groups_and_pr_count_table() {
        let cases = [
            (json!([
                {"runner_id":"z","items":[{"id":"pr","kind":"pr_review","rank":3}, {"id":"same","kind":"ask","rank":1}]},
                {"runner_id":"a","items":[{"id":"future","kind":"new_kind","rank":0}, {"id":"pr","kind":"pr_review","rank":2},
                    {"id":"same","kind":"ask","rank":1,"also":["decision"]}, {"id":"note","kind":"fyi","rank":9,"concern":{"yours":false}}]}
            ]), vec![("a","same"), ("z","same"), ("a","pr"), ("a","note"), ("a","future")], 4, vec!["note"]),
            (json!([{"runner_id":"empty","items":[]}]), vec![], 0, vec![]),
            (json!([{"runner_id":"old","items":[{"id":"block","kind":"blocked","rank":199999999,"concern":{"yours":true}}]}]), vec![("old","block")], 1, vec![]),
            (json!([{"runner_id":"b","items":[{"id":"pr","kind":"pr_review","rank":5}]},
                {"runner_id":"a","items":[{"id":"pr","kind":"pr_review","rank":5}]}]), vec![("a","pr")], 1, vec![]),
        ];
        for (input, keys, count, others) in cases {
            let shown = view(input.clone());
            assert_eq!(shown.rows.iter().map(|r| (r.key.runner_id.as_str(), r.key.item_id.as_str())).collect::<Vec<_>>(), keys);
            assert_eq!(shown.counted_total, count);
            assert_eq!(shown.others.iter().map(|k| k.item_id.as_str()).collect::<Vec<_>>(), others);
            assert_eq!(shown.split, !others.is_empty());
            assert_eq!(shown.yours.len() + shown.others.len(), shown.rows.len());
            let mut reversed = input.as_array().unwrap().clone();
            reversed.reverse();
            for r in &mut reversed { r["items"].as_array_mut().unwrap().reverse(); }
            assert_eq!(serde_json::to_value(&shown).unwrap(), serde_json::to_value(view(Value::Array(reversed))).unwrap());
        }
    }

    #[test]
    fn runner_qualified_buckets_and_payload() {
        let item = json!({"id":"ask","kind":"ask","rank":1,"workspace_id":"w","repository_id":"r", "actions":[{"id":"byte-for-byte option"}],"future":{"raw":"yes"}});
        let shown = view(json!([
            {"runner_id":"a","items":[item, {"id":"unclaimed","kind":"review","rank":2,"repository_id":"r"}, {"id":"bare","kind":"blocked","rank":3}]},
            {"runner_id":"b","items":[item]}, {"runner_id":"empty","items":[]}
        ]));
        assert_eq!(shown.workspace_counts.len(), 2);
        assert_eq!(shown.workspace_counts.iter().map(|b| b.count).collect::<Vec<_>>(), [1, 1]);
        assert_eq!(shown.unclaimed_counts.len(), 1);
        assert_eq!(shown.unclaimed_counts[0].count, 1);
        assert_eq!(shown.runner_counts.iter().map(|b| b.count).collect::<Vec<_>>(), [3, 1, 0]);
        assert_eq!(serde_json::from_str::<Value>(&shown.rows[0].item_json).unwrap(), item);
    }

    #[test]
    fn explicit_invalid_input_table() {
        for item in [json!({"id":"a","kind":"ask"}), json!({"id":"a","kind":"ask","rank":-1}),
            json!({"id":"a","kind":"ask","rank":4294967296_u64}), json!({"id":"a","kind":"ask","rank":1.5}),
            json!({"id":"a","kind":"ask","rank":1,"concern":{}})] {
            let answer: Value = serde_json::from_str(&json(&json!({"schema":1,"needs_you":[{"runner_id":"a","items":[item]}]}).to_string())).unwrap();
            assert_eq!(answer["error"], "invalid");
        }
        for (runners, error) in [
            (json!([{"runner_id":"a","items":[]},{"runner_id":"a","items":[]}]), "duplicate_runner"),
            (json!([{"runner_id":"a","items":[{"id":"x","kind":"ask","rank":1},{"id":"x","kind":"ask","rank":2}]}]), "duplicate_item")
        ] {
            let answer: Value = serde_json::from_str(&json(&json!({"schema":1,"needs_you":runners}).to_string())).unwrap();
            assert_eq!(answer["error"], error);
        }
        assert!(json(r#"{"schema":2,"needs_you":[]}"#).contains("invalid"));
    }

    #[cfg(feature = "native")]
    #[test]
    fn shared_adapter_fixture_through_native_bridge() {
        let input = include_str!("../../../test/fixtures/needs-you-aggregate.json");
        let answer: Value = serde_json::from_str(&json(input)).unwrap();
        let shown: NeedsYouAggregateView = serde_json::from_value(answer["needs_you"].clone()).unwrap();
        assert_eq!(shown.rows.iter().map(|r| (r.key.runner_id.as_str(), r.key.item_id.as_str())).collect::<Vec<_>>(),
            [("z", "ask"), ("a", "pr"), ("a", "fyi"), ("a", "future")]);
        assert_eq!(shown.counted_total, 3);
        assert_eq!(shown.runner_counts.iter().map(|b| b.count).collect::<Vec<_>>(), [2, 0, 1]);
        let request = std::ffi::CString::new(input).unwrap();
        unsafe {
            let bridge = crate::ffi::farcooler_client_plan_view;
            let needed = bridge(request.as_ptr(), std::ptr::null_mut(), 0);
            let mut bytes = vec![0; needed];
            assert_eq!(bridge(request.as_ptr(), bytes.as_mut_ptr(), bytes.len()), needed);
            assert_eq!(bytes, json(input).as_bytes());
        }
    }

    #[test]
    fn existing_wire_fixture_preserves_every_payload() {
        let fixture: Value = serde_json::from_str(include_str!("../../../test/fixtures/needs-you.json")).unwrap();
        for runner in fixture["runners"].as_array().unwrap() {
            let items = runner["needs_you"]["items"].as_array().unwrap();
            let shown = view(json!([{"runner_id":runner["runner"],"items":items}]));
            for row in shown.rows {
                let original = items.iter().find(|i| i["id"] == row.key.item_id).unwrap();
                assert_eq!(&serde_json::from_str::<Value>(&row.item_json).unwrap(), original);
            }
        }
    }
}
