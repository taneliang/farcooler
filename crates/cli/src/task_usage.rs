//! `farcooler task usage`: what agents spent on one task (ov-195).
//!
//! `--json` prints `farcooler_core::usage_words::TaskSpend`, the shape the
//! Mac's Usage section decodes; otherwise the same words that section says,
//! from the same `usage_words`.

use std::error::Error;

use farcooler_client::usage_json::task_spend;
use farcooler_core::usage_words::{NOTHING_YET, TaskSpend};
use farcooler_protocol::v1::{self as pb, request, result};

use crate::tasks::{DispatchLink, Refused, find_task, refusal};
use crate::{expect_value, req, with};

/// What a runner too old to record spend is told.
const NO_USAGE: &str = "this runner's Far Cooler is older than usage reports. update it and try again";

/// One task's spend, as `--json` or in words.
pub(crate) async fn task_usage<L: DispatchLink>(
    link: &mut L,
    repo: Option<&str>,
    key: &str,
    json: bool,
) -> Result<String, Box<dyn Error>> {
    if !link.capabilities().iter().any(|c| c == farcooler_protocol::capability::AGENT_USAGE) {
        return Err(Box::new(Refused::new(
            NO_USAGE.to_string(),
            Some(pb::ErrorCode::CapabilityUnsupported as i32),
        )));
    }
    let task = find_task(link, repo, key).await?;
    let ask = pb::TaskUsageRequest { task_id: task.id.clone() };
    let r = link
        .call(with(req("usage.task"), request::Payload::UsageTask(ask)))
        .await
        .map_err(|e| Box::new(refusal(e, "the runner couldn't read this task's usage. try again")) as Box<dyn Error>)?;
    let result::Value::TaskUsage(usage) = expect_value(r.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    let spend = task_spend(&usage);
    if json {
        return Ok(serde_json::to_string(&spend)?);
    }
    Ok(render(&task.key, &task.title, &spend))
}

/// The words: totals, then each harness and model.
pub(crate) fn render(key: &str, title: &str, spend: &TaskSpend) -> String {
    let mut out = vec![format!("{key}  {title}")];
    let t = &spend.totals;
    if t.is_empty() {
        out.push(NOTHING_YET.to_string());
        return out.join("\n");
    }
    out.push(match t.token_detail() {
        Some(detail) => format!("{} ({detail})", t.tokens_line()),
        None => t.tokens_line(),
    });
    out.push(t.cost_line());
    out.extend(t.time_line());
    let rows = spend.rows();
    if !rows.is_empty() {
        out.push(String::new());
        out.push("By harness and model".to_string());
        let width = rows.iter().map(|r| r.title().chars().count()).max().unwrap_or(0);
        for row in rows {
            out.push(format!("  {:width$}  {}", row.title(), row.detail()));
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(n: usize) -> TaskSpend {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/task-usage.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        serde_json::from_value(fixture["cases"][n]["usage"].clone()).unwrap()
    }

    #[test]
    fn a_task_with_nothing_says_so() {
        assert_eq!(render("ov-1", "Docs", &case(0)), "ov-1  Docs\nNo agent usage recorded yet.");
    }

    #[test]
    fn a_task_reads_its_totals_then_each_harness_and_model() {
        assert_eq!(
            render("ov-1", "Docs", &case(5)),
            "ov-1  Docs\n\
             2K tokens (1K input · 1K output · 0 cache)\n\
             $3.20 · API-equivalent, partly not reported\n\
             Agent time 41 h · 4 turns\n\
             \n\
             By harness and model\n  \
             codex · gpt-5.5         1.5K tokens · Cost not reported\n  \
             claude · claude-opus-5  500 tokens · $3.20"
        );
    }
}
