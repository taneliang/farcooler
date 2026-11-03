use super::{DomainError, Response, WireError, response};

/// A refusal in the shape every answer takes.
///
/// Shared with `Rpc::handle` so a response built outside the dispatcher cannot
/// be a differently shaped one — same code mapping, same redaction.
pub(super) fn error_response(request_id: bytes::Bytes, err: DomainError) -> Response {
    let (code, retryable) = err.wire();
    Response {
        request_id,
        outcome: Some(response::Outcome::Error(WireError {
            code: code as i32,
            retryable,
            // Redacted by construction: never a path, terminal byte, command,
            // or session id.
            message: err.redacted_message(),
            // WHICH argument, when the code alone does not say. A client
            // switches on this and never shows it; see `DomainError::what`.
            what: err.what().to_string(),
        })),
    }
}

