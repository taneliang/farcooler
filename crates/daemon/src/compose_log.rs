//! One record for every `terminal.compose` the runner receives (ov-1499), so a
//! report of a message with no response can be checked against this side. It
//! never holds the message or its attachments.

use farcooler_protocol::v1::{Response, response, result};

pub(crate) struct ComposeLog {
    started: std::time::Instant,
    client: String,
    device: String,
    recorded: bool,
}

impl ComposeLog {
    pub(crate) fn begin(client: Option<&str>, device: Option<&str>) -> Self {
        Self {
            started: std::time::Instant::now(),
            client: client.unwrap_or("unknown").to_string(),
            device: device.unwrap_or("local").to_string(),
            recorded: false,
        }
    }

    /// The compose was answered: `sent`, `queued`, or why it was refused.
    pub(crate) fn answered(mut self, response: &Response) {
        let outcome = match &response.outcome {
            Some(response::Outcome::Result(value)) => match &value.value {
                Some(result::Value::TerminalTold(told)) if told.queued => "queued",
                Some(result::Value::TerminalTold(_)) => "sent",
                _ => "unexpected_result",
            },
            Some(response::Outcome::Error(error)) if !error.what.is_empty() => &error.what,
            Some(response::Outcome::Error(error)) => farcooler_core::error::word_for(error.code),
            None => "missing_response",
        };
        self.record(outcome);
    }

    fn record(&mut self, outcome: &str) {
        self.recorded = true;
        tracing::info!(outcome, elapsed_ms = self.started.elapsed().as_millis() as u64,
            client = ?self.client, device = %self.device, "terminal.compose");
    }
}

/// A connection that has ended drops what outlasts its drain deadline.
impl Drop for ComposeLog {
    fn drop(&mut self) {
        if !self.recorded {
            self.record("abandoned");
        }
    }
}
