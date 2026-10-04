//! The refusal a removal that needs the typed name returns.

/// The daemon's own `ConfirmationRequired`, as a `ClientError::Daemon`, so
/// `--json` prints `code: confirmation-required` (`error_code_lines`) and the
/// Mac asks for the name instead of calling the removal a failure.
pub(crate) fn required() -> Box<dyn std::error::Error> {
    Box::new(farcooler_transport::ClientError::Daemon {
        code: farcooler_protocol::v1::ErrorCode::ConfirmationRequired as i32,
        retryable: false,
        message: "exact typed confirmation required".into(),
        what: String::new(),
    })
}
