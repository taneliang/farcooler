//! Exclusive keyboard access on the established control connection.

use super::{Pending, Session, SessionError, Uuid, request, result, wrong};

impl Session {
    /// Start input with the runner generation from `terminal_writer`. A
    /// queued request keeps the generation it was created with.
    pub fn start_terminal_input(
        &self,
        method: &str,
        terminal: Uuid,
        payload: request::Payload,
        generation: Option<u64>,
    ) -> Result<Pending, SessionError> {
        let mut req = farcooler_transport::request(method);
        req.target_resource_id = Some(bytes::Bytes::copy_from_slice(terminal.as_bytes()));
        req.payload = Some(payload);
        req.expected_lease_generation = generation;
        self.start_request(req)
    }

    pub async fn terminal_writer(
        &self,
        terminal: Uuid,
        action: farcooler_protocol::v1::terminal_writer_request::Action,
        generation: Option<u64>,
    ) -> Result<farcooler_protocol::v1::TerminalWriterState, SessionError> {
        use farcooler_protocol::v1::TerminalWriterRequest;
        let mut req = farcooler_transport::request("terminal.writer");
        req.target_resource_id = Some(bytes::Bytes::copy_from_slice(terminal.as_bytes()));
        req.payload = Some(request::Payload::TerminalWriter(TerminalWriterRequest {
            action: action as i32,
        }));
        req.expected_lease_generation = generation;
        match self.start_request(req)?.value().await? {
            result::Value::TerminalWriter(state) => Ok(state),
            other => Err(wrong("terminal_writer", &other)),
        }
    }
}
