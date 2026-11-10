//! A task's spend as JSON (ov-195): `usage.task`'s answer in the shape the
//! apps decode, `farcooler_core::usage_words::TaskSpend`. The phones read it
//! through the `usage.task` route and the Mac through `farcooler task usage
//! --json`, so the two can't come to disagree about one field's name.
//!
//! Beside the numbers it carries `said`, every line a Usage section draws in
//! `usage_words`' words (ov-1048, R-112), so the apps lay the lines out and
//! write none of them. Numbers are en_US, as the plan's are.

use farcooler_core::usage_words::{NOTHING_YET, Spend, SpendRow, TaskSpend};
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use crate::plan_view::words;
use farcooler_protocol::v1 as pb;
use uuid::Uuid;

pub fn spend(t: &pb::UsageTotals) -> Spend {
    Spend {
        turns: t.turns,
        turns_partial: t.turns_partial,
        turns_not_reported: t.turns_not_reported,
        subagent_runs: t.subagent_runs,
        active_ms: t.active_ms,
        input_tokens: t.input_tokens,
        output_tokens: t.output_tokens,
        cache_read_tokens: t.cache_read_tokens,
        cache_write_tokens: t.cache_write_tokens,
        cost_reported_micros: t.cost_reported_micros,
        cost_estimated_micros: t.cost_estimated_micros,
        unpriced_tokens: t.unpriced_tokens,
        price_tables: t.price_tables.clone(),
    }
}

pub fn task_spend(u: &pb::TaskUsage) -> TaskSpend {
    TaskSpend {
        task: Uuid::from_slice(&u.task_id).map(|id| id.to_string()).unwrap_or_default(),
        price_table: u.price_table.clone(),
        totals: u.totals.as_ref().map(spend).unwrap_or_default(),
        by_harness_model: u
            .by_harness_model
            .iter()
            .map(|g| SpendRow {
                harness: g.harness.clone().unwrap_or_default(),
                model: g.model.clone().unwrap_or_default(),
                totals: g.totals.as_ref().map(spend).unwrap_or_default(),
            })
            .collect(),
    }
}

/// A task's spend and the lines it says: what the apps and `task usage --json` read.
#[derive(Serialize)]
pub struct Shown {
    #[serde(flatten)]
    pub spend: TaskSpend,
    pub said: UsageSaid,
}

/// What a Usage section says. With nothing recorded it says `nothing` alone.
#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSaid {
    /// "No agent usage recorded yet." with nothing recorded; otherwise none.
    pub nothing: Option<String>,
    pub tokens: String,
    pub token_detail: Option<String>,
    pub cost: String,
    /// The cost has a dollar figure, so the API-equivalent note goes under it.
    pub priced: bool,
    pub time: Option<String>,
    /// Most tokens first, then by title.
    pub rows: Vec<UsageRowSaid>,
}

/// One harness and model's line in the breakdown.
#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRowSaid {
    /// Unique within a task: the runner keys rows by harness and model.
    pub id: String,
    /// "claude · Claude Opus 5", or the harness alone for an unnamed model.
    pub title: String,
    /// The recorded model id, for assistive technology.
    pub model: String,
    pub detail: String,
}

pub fn said(spend: &TaskSpend) -> UsageSaid {
    let t = &spend.totals;
    UsageSaid {
        nothing: t.is_empty().then(|| NOTHING_YET.to_string()),
        tokens: t.tokens_line(),
        token_detail: t.token_detail(),
        cost: t.cost_line(),
        priced: t.is_priced(),
        time: t.time_line(),
        rows: spend
            .rows()
            .into_iter()
            .map(|r| UsageRowSaid {
                id: format!("{}\u{1F}{}", r.harness, r.model),
                title: if r.model.is_empty() { r.harness.clone() } else { format!("{} · {}", r.harness, words::model(&r.model)) },
                model: r.model.clone(),
                detail: r.detail(),
            })
            .collect(),
    }
}

pub fn shown(u: &pb::TaskUsage) -> Shown {
    let spend = task_spend(u);
    Shown { said: said(&spend), spend }
}

pub fn task_spend_json(u: &pb::TaskUsage) -> serde_json::Value {
    serde_json::to_value(shown(u)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field the wire carries lands under the name the fixture, and so
    /// the apps, read it by.
    #[test]
    fn the_wire_reads_into_the_fixture_shape() {
        let id = Uuid::parse_str("8d3c1a52-0b8e-4c1e-9a3f-0f3b2a6c9e11").unwrap();
        let totals = pb::UsageTotals {
            turns: 12,
            active_ms: 11_430_000,
            input_tokens: 12_000,
            output_tokens: 3_400,
            cache_read_tokens: 1_100_000,
            cache_write_tokens: 40_000,
            cost_reported_micros: 3_200_000,
            ..Default::default()
        };
        let wire = pb::TaskUsage {
            task_id: bytes::Bytes::copy_from_slice(id.as_bytes()),
            totals: Some(totals.clone()),
            by_harness_model: vec![pb::UsageGroup {
                harness: Some("claude".into()),
                model: Some("claude-opus-5".into()),
                totals: Some(totals),
                ..Default::default()
            }],
            price_table: "2026-09-25".into(),
        };
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/task-usage.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let case = &fixture["cases"][1];
        assert_eq!(case["case"], "a claude chat, its cost reported");
        let mut json = task_spend_json(&wire);
        let said = json.as_object_mut().unwrap().remove("said").expect("the lines ride along");
        assert_eq!(json, case["usage"]);
        assert_eq!(said["cost"], case["cost"]);
    }

    /// The plan view's bridge answers a spend with what `usage.task` would
    /// carry, for the iPhone harness that fakes the route; another schema is refused.
    #[test]
    fn the_bridge_says_a_spend() {
        let spend = r#"{"task":"t","totals":{"turns":1,"input_tokens":900,"output_tokens":70,"cost_estimated_micros":420000}}"#;
        let answer: serde_json::Value =
            serde_json::from_str(&crate::plan_view::json(&format!(r#"{{"schema":1,"usage":{spend}}}"#))).unwrap();
        assert_eq!(answer["usage"]["said"]["cost"], "$0.42 · API-equivalent, estimated");
        assert_eq!(answer["usage"]["task"], "t");
        let refused: serde_json::Value =
            serde_json::from_str(&crate::plan_view::json(&format!(r#"{{"schema":2,"usage":{spend}}}"#))).unwrap();
        assert_eq!((refused["usage"].is_null(), refused["error"].as_str()), (true, Some("invalid")));
    }

    /// The answer the apps decode, pinned: every fixture case and the two
    /// spends the Mac's captures draw. AgentKit's and Android's tests and the
    /// Mac captures read this file, so they draw Rust's own bytes. Rewrite with
    /// UPDATE_USAGE_GOLDEN=1.
    #[test]
    fn the_shown_golden_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/task-usage.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut cases: Vec<(String, serde_json::Value)> = fixture["cases"].as_array().unwrap().iter()
            .map(|c| (c["case"].as_str().unwrap().to_string(), c["usage"].clone()))
            .collect();
        let claude = serde_json::json!({"harness": "claude", "model": "claude-opus-5", "totals": {"turns": 9,
            "active_ms": 7800000, "input_tokens": 41000, "output_tokens": 18400, "cache_read_tokens": 1020000,
            "cache_write_tokens": 62000, "cost_reported_micros": 2870000}});
        let totals = serde_json::json!({"turns": 12, "turns_partial": 1, "active_ms": 10200000, "input_tokens": 251000,
            "output_tokens": 27400, "cache_read_tokens": 1084000, "cache_write_tokens": 62000,
            "cost_reported_micros": 2870000, "unpriced_tokens": 283000});
        let mut partial = claude.clone();
        partial["totals"]["turns_partial"] = 1.into();
        let codex = serde_json::json!({"harness": "codex", "model": "gpt-5.5", "totals": {"turns": 3, "active_ms": 2400000,
            "input_tokens": 210000, "output_tokens": 9000, "cache_read_tokens": 64000, "unpriced_tokens": 283000}});
        cases.push(("specimen: claude in part, codex unpriced".into(),
            serde_json::json!({"task": "t", "price_table": "2026-09-25", "totals": totals, "by_harness_model": [partial, codex]})));
        cases.push(("specimen: claude alone".into(),
            serde_json::json!({"task": "t", "price_table": "2026-09-25", "totals": totals, "by_harness_model": [claude]})));
        let shown: Vec<serde_json::Value> = cases.into_iter().map(|(case, usage)| {
            let spend: TaskSpend = serde_json::from_value(usage).unwrap();
            serde_json::json!({"case": case, "shown": Shown { said: said(&spend), spend }})
        }).collect();
        let actual = serde_json::to_string_pretty(&shown).unwrap() + "\n";
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/task-usage-shown.json");
        if std::env::var_os("UPDATE_USAGE_GOLDEN").is_some() {
            std::fs::write(path, &actual).unwrap();
        }
        assert_eq!(actual, std::fs::read_to_string(path).unwrap_or_default(), "{path} is stale: UPDATE_USAGE_GOLDEN=1");
    }

    /// Every case says, in `said`, the lines the fixture pins; a row's title
    /// names its model as the plan does.
    #[test]
    fn every_case_says_the_fixture_lines() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/task-usage.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let spend: TaskSpend = serde_json::from_value(case["usage"].clone()).unwrap();
            let s = serde_json::to_value(said(&spend)).unwrap();
            let name = &case["case"];
            if case["empty"] == true {
                assert_eq!(s["nothing"], fixture["words"]["nothing_yet"], "{name}");
                continue;
            }
            assert_eq!(s["nothing"], serde_json::Value::Null, "{name}");
            for (key, wire) in [("tokens", "tokens"), ("token_detail", "tokenDetail"), ("cost", "cost"), ("time", "time")] {
                assert_eq!(s[wire], case[key], "{name}: {key}");
            }
            assert_eq!(s["priced"], case["cost"] != "Not reported", "{name}: priced");
            let rows = s["rows"].as_array().unwrap();
            assert_eq!(rows.len(), case["rows"].as_array().unwrap().len(), "{name}");
            for (got, want) in rows.iter().zip(case["rows"].as_array().unwrap()) {
                assert_eq!(got["detail"], want["detail"], "{name}");
                let raw = want["title"].as_str().unwrap();
                let pretty = raw.split_once(" · ").map_or(raw.to_string(), |(h, m)| format!("{h} · {}", words::model(m)));
                assert_eq!(got["title"], pretty, "{name}");
            }
        }
    }
}
