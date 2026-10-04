//! The terminal requests the CLI builds: one place for what each one names.
//!
//! Moved out of `main.rs` (ov-234), which was at its size ceiling. A request is
//! built apart from being sent so a test can read what it names.

use farcooler_protocol::v1::request;

use crate::{Fallible, req_for, terminal_by_record, with};

/// `terminal.create`, with each optional field's capability named when it is
/// sent: a daemon too old to know the field then refuses the request, rather
/// than dropping it in silence (opening the agent on an empty composer, or a
/// pane that knows no task). See `capability::LAUNCH_PROMPT` and
/// `capability::TERMINAL_TASK`.
pub(crate) fn terminal_create_request(
    worktree: uuid::Uuid,
    title: String,
    preset: String,
    tile: bool,
    prompt: Option<String>,
    task: Option<String>,
) -> farcooler_protocol::v1::Request {
    let prompt = prompt.filter(|p| !p.trim().is_empty());
    let task = task.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    let mut req = with(
        req_for("terminal.create", worktree),
        request::Payload::TerminalCreate(farcooler_protocol::v1::TerminalCreate {
            title,
            command_preset: preset,
            join_active_group: tile,
            prompt: prompt.clone(),
            task_key: task.clone(),
        }),
    );
    if prompt.is_some() {
        req.required_capabilities.push(farcooler_protocol::capability::LAUNCH_PROMPT.to_string());
    }
    if task.is_some() {
        req.required_capabilities.push(farcooler_protocol::capability::TERMINAL_TASK.to_string());
    }
    req
}

/// `terminal.rename`, naming the capability it needs: a runner without it
/// refuses the request (`CAPABILITY_UNSUPPORTED`) instead of dropping the call.
/// An empty name clears the name.
pub(crate) fn terminal_rename_request(terminal: uuid::Uuid, name: &str) -> farcooler_protocol::v1::Request {
    let mut req = with(
        req_for("terminal.rename", terminal),
        request::Payload::TerminalRename(farcooler_protocol::v1::TerminalRename { name: name.to_string() }),
    );
    req.required_capabilities.push(farcooler_protocol::capability::TERMINAL_NAMES.to_string());
    req
}

/// `farcooler terminal rename <terminal> <name>`.
pub(crate) async fn rename(runner: Option<&str>, terminal: &str, name: &str, json: bool) -> Fallible {
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    let r = link.call(terminal_rename_request(id, name)).await?;
    let farcooler_protocol::v1::result::Value::Terminal(t) = crate::expect_value(r.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if json {
        println!("{}", serde_json::json!({ "id": crate::uuid_of(&t.id).to_string(), "title": t.title }));
    } else {
        println!("renamed {}  {}", crate::short_bytes(&t.id), t.title);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The request names the terminal as its target, carries the name as given,
    /// and asks for the capability, so an older runner can't swallow it.
    #[test]
    fn a_rename_names_its_terminal_its_text_and_its_capability() {
        let id = uuid::Uuid::from_u128(9);
        let req = terminal_rename_request(id, "gcp proxy");
        assert_eq!(req.method, "terminal.rename");
        assert_eq!(req.required_capabilities, [farcooler_protocol::capability::TERMINAL_NAMES]);
        let Some(request::Payload::TerminalRename(p)) = req.payload else { panic!("payload") };
        assert_eq!(p.name, "gcp proxy");
    }
}
