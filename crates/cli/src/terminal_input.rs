//! The Mac's persistent stdin pipe uses the same runner lease as a browser.
use farcooler_protocol::v1::{
    ErrorCode, TerminalWrite, TerminalWriterRequest, TerminalWriterState, event, request, result,
    terminal_writer_request::Action,
};
use farcooler_transport::ClientError;
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

use crate::{Fallible, daemon_link::Link, req_for, terminal_by_record, with};

pub(crate) const WRITER_INSTANCE_ENV: &str = "FARCOOLER_WRITER_INSTANCE";

/// Commands spawned by one Mac app share a writer identity, including the
/// separate process that sends geometry or pastes a file.
pub(crate) fn writer_instance() -> bytes::Bytes {
    std::env::var(WRITER_INSTANCE_ENV)
        .ok()
        .and_then(|id| Uuid::parse_str(&id).ok())
        .map(|id| bytes::Bytes::copy_from_slice(id.as_bytes()))
        .unwrap_or_default()
}

pub(crate) fn decode_hex(hex: &str) -> Result<Vec<u8>, &'static str> {
    if !hex.is_ascii() || !hex.len().is_multiple_of(2) {
        return Err("Expected pairs of hex digits.");
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| "Expected pairs of hex digits.")
        })
        .collect()
}

pub(crate) async fn send(
    link: &mut Link,
    id: Uuid,
    bytes: Vec<u8>,
    generation: Option<u64>,
) -> Fallible {
    let mut req = with(
        req_for("terminal.write", id),
        request::Payload::TerminalWrite(TerminalWrite {
            payload: bytes.into(),
        }),
    );
    req.expected_lease_generation = generation;
    link.call(req).await?;
    Ok(())
}

async fn writer(
    link: &mut Link,
    id: Uuid,
    action: Action,
) -> Result<TerminalWriterState, Box<dyn std::error::Error>> {
    let req = with(
        req_for("terminal.writer", id),
        request::Payload::TerminalWriter(TerminalWriterRequest {
            action: action as i32,
        }),
    );
    match link.call(req).await?.value {
        Some(result::Value::TerminalWriter(state)) => Ok(state),
        _ => Err(crate::daemon_link::UNREADABLE.into()),
    }
}

fn notice(reason: &str) {
    let message = farcooler_core::error::sentence(reason);
    let action = message.map(|_| farcooler_client::work_view::TAKE_OVER_TERMINAL);
    println!(
        "{}",
        serde_json::json!({ "message": message, "action": action })
    );
}

pub(crate) async fn input(runner: Option<&str>, terminal: &str) -> Fallible {
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    // An old runner refuses the new method. Once a generation is granted,
    // all fenced writes go over that same connection.
    let mut supports_lease = true;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut generation = None;
    loop {
        let line = tokio::select! {
            line = lines.next_line() => line?,
            news = link.next_event(), if supports_lease => {
                if let Some(event::Payload::TerminalWriterLost(lost)) = news?.payload
                    && lost.terminal_id.as_ref() == id.as_bytes()
                    && generation == Some(lost.generation)
                {
                    generation = None;
                    notice("writer_taken_over");
                }
                continue;
            }
        };
        let Some(line) = line else { return Ok(()) };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let takeover = line == "take-over";
        if supports_lease && (takeover || generation.is_none()) {
            let action = if takeover {
                Action::TakeOver
            } else {
                Action::Claim
            };
            match writer(&mut link, id, action).await {
                Ok(state) => {
                    generation = state.generation;
                    notice(&state.reason);
                    if generation.is_none() {
                        continue;
                    }
                }
                Err(err)
                    if matches!(err.downcast_ref::<ClientError>(),
                    Some(ClientError::Daemon { code, .. }) if *code == ErrorCode::CapabilityUnsupported as i32) =>
                {
                    supports_lease = false;
                }
                Err(err) => return Err(err),
            }
        }
        let Ok(bytes) = decode_hex(line) else {
            continue;
        };
        if let Err(err) = send(&mut link, id, bytes, generation).await {
            if let Some(ClientError::Daemon { what, .. }) = err.downcast_ref::<ClientError>()
                && matches!(what.as_str(), "writer_busy" | "writer_taken_over")
            {
                generation = None;
                notice(what);
                continue;
            }
            return Err(err);
        }
    }
}
