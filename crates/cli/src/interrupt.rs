//! `terminal interrupt` and `terminal send-now` (ov-368): the runner presses
//! one Esc, or claude's ctrl+x ctrl+s, in a working claude's terminal pane,
//! past its typing gate, and answers once claude took it; or it refuses,
//! pressing nothing, in this CLI's words for the runner's (`said_about`).

use farcooler_protocol::v1::{self as pb, request};

use super::{TerminalCmd, id_bytes, req, short, tasks, terminal_by_record, with};

/// `terminal interrupt` or `terminal send-now`, as parsed.
pub(crate) async fn press(runner: Option<&str>, cmd: TerminalCmd) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        TerminalCmd::Interrupt { terminal } => run(runner, &terminal, "terminal.interrupt").await,
        TerminalCmd::SendNow { terminal } => run(runner, &terminal, "terminal.send_now").await,
        _ => unreachable!("only interrupt and send-now are pressed"),
    }
}

/// Ask the runner to press `method`'s key (`terminal.interrupt` or
/// `terminal.send_now`) in `terminal`.
async fn run(runner: Option<&str>, terminal: &str, method: &'static str) -> Result<(), Box<dyn std::error::Error>> {
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    link.call(with(req(method), request::Payload::AgentCancel(pb::AgentCancel { terminal_id: id_bytes(id) })))
        .await
        .map_err(refused)?;
    let done = if method == "terminal.interrupt" { "stopped" } else { "sent the queue now in" };
    println!("{done} {}", short(id));
    Ok(())
}

/// This CLI's line for a refusal the runner named.
pub(crate) fn said_about(what: &str) -> Option<&'static str> {
    Some(match what {
        "idle" => "claude isn't working on a turn, so there's nothing to stop or send",
        "prompt" => "claude is showing a question or a dialog, or may be about to. answer it in its pane",
        "draft" => "there's a draft in claude's box, which send-now would send too. send or clear it first",
        "typing" => "someone typed in the pane in the last two seconds. try again in a moment",
        "too_soon" => "a key was pressed there in the last second and a half. try again in a moment",
        "sending" => "a message is still being typed there. try again in a moment",
        "nothing_queued" => "nothing is waiting in claude's queue",
        "not_an_agent" => "claude isn't running in that pane",
        "not_running" => "that terminal isn't running",
        "unsupported" => "only claude can be stopped from here",
        "unfamiliar" => "the pane's screen isn't one Far Cooler recognizes, so nothing was pressed",
        "unconfirmable" => "Far Cooler can't find claude's session or hasn't heard its hooks, so nothing was pressed",
        "settling" => "claude is starting a step that may ask something. try again in a moment",
        "unconfirmed" => "the key was pressed, but claude never said it took it. it may have; check its pane before pressing again",
        _ => return None,
    })
}

fn refused(e: farcooler_transport::ClientError) -> Box<dyn std::error::Error> {
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e
        && let Some(said) = said_about(what)
    {
        return Box::new(tasks::Refused::naming(said.to_string(), *code, what.clone()));
    }
    Box::new(e)
}

#[cfg(test)]
mod tests {
    /// Every word the runner refuses a key with has this CLI's line
    /// (`answer_wake::interrupt`'s docs, "The gate").
    #[test]
    fn every_refusal_word_has_a_line() {
        for word in [
            "idle", "prompt", "draft", "typing", "too_soon", "sending", "nothing_queued", "not_an_agent", "not_running",
            "unsupported", "unfamiliar", "unconfirmable", "unconfirmed", "settling",
        ] {
            assert!(super::said_about(word).is_some(), "{word}");
        }
        assert_eq!(super::said_about("paste_left"), None, "not a key's word");
    }
}
