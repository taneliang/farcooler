//! A task's spend as JSON (ov-195): `usage.task`'s answer in the shape the
//! apps decode, `farcooler_core::usage_words::TaskSpend`. The phones read it
//! through the `usage.task` route and the Mac through `farcooler task usage
//! --json`, so the two can't come to disagree about one field's name.

use farcooler_core::usage_words::{Spend, SpendRow, TaskSpend};
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

pub fn task_spend_json(u: &pb::TaskUsage) -> serde_json::Value {
    serde_json::to_value(task_spend(u)).unwrap_or_default()
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
        assert_eq!(task_spend_json(&wire), case["usage"]);
    }
}
