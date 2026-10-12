//! `farcooler needs-you`: what's waiting on a person, as the runner ranks it.
//! Moved out of `main.rs` whole, to keep it inside its size ceiling.

use farcooler_protocol::v1::result;

use crate::{Fallible, connect_to, expect_value, req, tasks, truncate};

/// What a runner too old for Needs You is told.
pub(crate) const NO_NEEDS_YOU: &str =
    "this runner's Far Cooler is older than needs-you. update it and try again";

/// What a refused needs-you read is told, whatever the runner's reason.
pub(crate) const NEEDS_YOU_UNREAD: &str = "the runner couldn't say what needs you. try again";

/// `farcooler needs-you`.
pub(crate) async fn needs_you(runner: Option<&str>, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    println!("{}", needs_you_read(&mut link, json).await?);
    Ok(())
}

/// The list, read and printed as `needs_you_output` prints it.
pub(crate) async fn needs_you_read<L: tasks::DispatchLink>(
    link: &mut L,
    json: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    needs_you_served(&link.capabilities())?;
    let r = link.call(req("needs_you.list")).await.map_err(|e| match &e {
        farcooler_transport::ClientError::Daemon { code, .. }
            if farcooler_core::error::word_for(*code) == "capability-unsupported" =>
        {
            Box::new(tasks::Refused::new(NO_NEEDS_YOU.to_string(), Some(*code))) as Box<dyn std::error::Error>
        }
        // Its own sentence for any other refusal, keeping the code and the
        // `what`: the board's would call this a task that isn't there.
        farcooler_transport::ClientError::Daemon { code, what, .. } => {
            Box::new(tasks::Refused::naming(NEEDS_YOU_UNREAD.to_string(), *code, what.clone()))
        }
        // A dropped link or an unreadable answer: the board's sentences for
        // those name no task.
        _ => Box::new(tasks::refusal(e, NEEDS_YOU_UNREAD)),
    })?;
    let result::Value::NeedsYouList(list) = expect_value(r.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    Ok(needs_you_output(&list, json))
}

/// Refused here, before a round trip, on a runner that doesn't advertise
/// `needs_you`, with the runner's own code so `--json` still says
/// `capability-unsupported`.
fn needs_you_served(capabilities: &[String]) -> Result<(), tasks::Refused> {
    if capabilities.iter().any(|c| c == farcooler_protocol::capability::NEEDS_YOU) {
        return Ok(());
    }
    Err(tasks::Refused::new(
        NO_NEEDS_YOU.to_string(),
        Some(farcooler_protocol::v1::ErrorCode::CapabilityUnsupported as i32),
    ))
}

/// What `needs-you` prints: the client core's JSON under `--json`, the one
/// shape the phones read too, and otherwise one line per item, most urgent
/// first.
pub(crate) fn needs_you_output(list: &farcooler_protocol::v1::NeedsYouList, json: bool) -> String {
    if json {
        return farcooler_client::needs_you_json::needs_you_json(list).to_string();
    }
    let mut items: Vec<_> = list.items.iter().collect();
    if items.is_empty() {
        return "nothing needs you".to_string();
    }
    // The daemon sends rank order already. Sorted again, stably, so a line
    // never depends on that.
    items.sort_by_key(|i| i.rank);
    items
        .iter()
        .map(|i| {
            let who = i
                .task
                .as_ref()
                .map(|t| t.key.clone())
                .or_else(|| i.terminal.as_ref().map(|t| t.label.clone()))
                .unwrap_or_default();
            let place = [i.workspace_name.as_str(), who.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let detail = i.detail.as_deref().map(|d| format!("  {d}")).unwrap_or_default();
            format!(
                "{:8}  {:24}  {}{detail}",
                farcooler_client::needs_you_json::kind_word(i.kind),
                truncate(&place, 24),
                i.question
            )
            .trim_end()
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
