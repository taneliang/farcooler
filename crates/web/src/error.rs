//! Why something failed, as one stable word.
//!
//! The page owns every sentence a person reads, exactly as the phones do: what
//! crosses into JavaScript is [`WebError::word`], never a Rust error string.
//! Where a word already exists for the same failure on a phone it is that word
//! (`farcooler_client::ssh::SshError::word`), so the two vocabularies can be
//! read side by side.

/// A failure of the browser's client.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WebError {
    /// The runner answered with a host key that is not the one the enrollment
    /// reply pinned. Either the runner was reinstalled or something is in the
    /// middle, and the client will not connect to find out which.
    #[error("the host key is {actual}, and the reply pinned {expected}")]
    HostKeyChanged { expected: String, actual: String },
    /// There is no pin to check against. A runner entry without a host key is
    /// refused rather than trusted on first use: accepting whatever answers is
    /// what makes an interception invisible.
    #[error("no host key is pinned for this runner")]
    HostKeyUnknown,
    /// The runner's sshd did not accept this browser's key.
    #[error("the runner rejected this browser's key")]
    KeyRejected,
    /// The SSH connection failed before or during the handshake.
    #[error("the SSH handshake failed: {0}")]
    Handshake(String),
    /// The browser's key could not sign, or is not a key this client signs with.
    #[error("this browser's key could not sign: {0}")]
    Signer(String),
    /// The passkey prompt was dismissed, or the browser would not show one
    /// without a click just before it. Nothing is wrong with the key.
    #[error("the passkey did not sign")]
    PasskeyDeclined,
    /// The runner's daemon could not be started on a channel.
    #[error("the runner's daemon could not be started: {0}")]
    Exec(String),
    /// The daemon said something this client cannot read.
    #[error("the runner's answer could not be read: {0}")]
    Protocol(String),
    /// The runner is too old for something this client needs. Carries the
    /// capability's name, for the log.
    #[error("the runner does not offer {0}")]
    Capability(String),
    /// The daemon refused a call, with its own error code's name.
    #[error("the runner refused: {0}")]
    Refused(String),
    /// The connection ended.
    #[error("the connection closed")]
    Closed,
    /// Signing in waited on the runner past its allowance with nothing
    /// heard: a tunnel that went quiet after it was dialed (ov-834).
    #[error("the runner did not answer while signing in")]
    NoAnswer,
}

impl WebError {
    /// The word the page maps to a sentence.
    ///
    /// Stable in the way a proto field name is: the page matches on these.
    pub fn word(&self) -> &'static str {
        match self {
            WebError::HostKeyChanged { .. } => "host_key_changed",
            WebError::HostKeyUnknown => "host_key_unknown",
            WebError::KeyRejected => "key_rejected",
            WebError::Handshake(_) => "handshake_failed",
            WebError::Signer(_) => "bad_key",
            WebError::PasskeyDeclined => "passkey_declined",
            WebError::Exec(_) => "exec_failed",
            WebError::Protocol(_) => "protocol",
            WebError::Capability(_) => "runner_too_old",
            WebError::Refused(_) => "refused",
            WebError::Closed => "closed",
            WebError::NoAnswer => "no_answer",
        }
    }
}
