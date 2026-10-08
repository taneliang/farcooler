//! `terminal bring-draft` (ov-369): Bring Here from the command line. The
//! draft a person left in a terminal-mode claude's own box, read with
//! nothing typed; with `--expected`, the text a read printed, cleared too,
//! only while the box still reads exactly that. Refused, typing nothing, in
//! this CLI's words for the runner's (`said_about`).

use std::io::Read;

use farcooler_protocol::v1::{self as pb, request};

use super::{id_bytes, req, short, tasks, terminal_by_record, with};

/// Read `terminal`'s box, and with `expected` (`-` for stdin) clear it.
/// Prints the draft as is, or under `json`, `{"text":…,"cleared":…}`.
pub(crate) async fn run(runner: Option<&str>, terminal: &str, expected: Option<String>, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let expected = match expected.as_deref() {
        Some("-") => {
            let mut read = String::new();
            std::io::stdin().read_to_string(&mut read)?;
            read.strip_suffix('\n').map(str::to_string).unwrap_or(read)
        }
        _ => expected.unwrap_or_default(),
    };
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    let payload = request::Payload::BringDraft(pb::BringDraft { terminal_id: id_bytes(id), expected });
    let answer = link.call(with(req("terminal.bring_draft"), payload)).await.map_err(refused)?;
    let Some(pb::result::Value::BroughtDraft(brought)) = answer.value else {
        return Err("the runner answered something other than a draft".into());
    };
    if json {
        println!("{}", serde_json::json!({ "text": brought.text, "cleared": brought.cleared }));
    } else if brought.text.is_empty() {
        eprintln!("claude's box in {} is empty", short(id));
    } else {
        println!("{}", brought.text);
        if brought.cleared {
            eprintln!("cleared claude's box in {}", short(id));
        }
    }
    Ok(())
}

/// This CLI's line for a refusal the runner named.
pub(crate) fn said_about(what: &str) -> Option<&'static str> {
    Some(match what {
        "pasted" => "claude's box holds a collapsed paste or an image, which its screen doesn't show. use the terminal",
        "too_tall" => "claude's box is too tall to read whole. use the terminal",
        "cursor" => "the cursor in claude's box isn't at the end. move it there, or use the terminal",
        "changed" => "claude's box no longer holds what was read, so it was left as it is",
        "typing" => "someone typed in the pane in the last three seconds. try again in a moment",
        "sending" => "a message is still being typed there. try again in a moment",
        "prompt" => "claude is showing a question or a dialog. answer it in its pane",
        "unfamiliar" => "the pane's screen isn't one Far Cooler recognizes, so nothing was read",
        "unsupported" => "only claude's box can be brought here",
        "not_an_agent" => "claude isn't running in that pane",
        "not_running" => "that terminal isn't running",
        "partly" => "the box was partly cleared and couldn't be put back. press ctrl+y in the pane to bring it back",
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
    /// Every word the runner refuses Bring Here with has this CLI's line
    /// (`answer_wake::bring`'s docs, "The gate").
    #[test]
    fn every_refusal_word_has_a_line() {
        for word in [
            "pasted", "too_tall", "cursor", "changed", "typing", "sending", "prompt", "unfamiliar", "unsupported",
            "not_an_agent", "not_running", "partly",
        ] {
            assert!(super::said_about(word).is_some(), "{word}");
        }
        assert_eq!(super::said_about("paste_left"), None, "not Bring Here's word");
    }
}
