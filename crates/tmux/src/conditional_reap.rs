//! Input and receipt validation for the watcher conditional-reap contract.
//!
//! These helpers do not establish a server or pane incarnation. Callers must
//! hold a durable reservation and fail closed when that binding is unavailable.

use uuid::Uuid;

/// Parse only the generated final marker from one `display-message -p`
/// response. The arbitrary command prefix is neither returned nor logged.
pub fn opening_terminal_id(output: &str) -> Option<Uuid> {
    let command = output.strip_suffix('\n')?;
    let start = command.len().checked_sub(36)?;
    let (prefix, suffix) = (command.get(..start)?, command.get(start..)?);
    if !prefix.ends_with(" #farcooler-opening:") { return None }
    let id = Uuid::parse_str(suffix).ok()?;
    (id.hyphenated().to_string() == suffix).then_some(id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapReceipt { Killed, Skipped }

/// The marker must be the sole stdout line, with a successful client exit.
/// No or ambiguous acknowledgement leaves the reservation unresolved.
pub fn receipt(exit_success: bool, stdout: &str, nonce: Uuid) -> Option<ReapReceipt> {
    if !exit_success { return None }
    if stdout == format!("fc-reap-{nonce}:killed\n") { Some(ReapReceipt::Killed) }
    else if stdout == format!("fc-reap-{nonce}:skipped\n") { Some(ReapReceipt::Skipped) }
    else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_requires_exact_canonical_suffix_and_one_newline() {
        let id = Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").unwrap();
        assert_eq!(opening_terminal_id(&format!("echo x #farcooler-opening:{id}\n")), Some(id));
        assert_eq!(opening_terminal_id(&format!("#farcooler-opening:{id}\n")), None);
        assert_eq!(opening_terminal_id(&format!("x #farcooler-opening:{id} extra\n")), None);
        assert_eq!(opening_terminal_id(&format!("x #farcooler-opening:{}\n", id.to_string().to_uppercase())), None);
        assert_eq!(opening_terminal_id(&format!("x #farcooler-opening:{id}\n\n")), None);
        assert_eq!(opening_terminal_id(&format!("x #farcooler-opening:{id}")), None);
    }

    #[test]
    fn receipt_requires_one_exact_nonce_and_success() {
        let nonce = Uuid::now_v7();
        let killed = format!("fc-reap-{nonce}:killed\n");
        let skipped = format!("fc-reap-{nonce}:skipped\n");
        assert_eq!(receipt(true, &killed, nonce), Some(ReapReceipt::Killed));
        assert_eq!(receipt(true, &skipped, nonce), Some(ReapReceipt::Skipped));
        assert_eq!(receipt(false, &killed, nonce), None);
        assert_eq!(receipt(true, &(killed.clone() + &killed), nonce), None);
        assert_eq!(receipt(true, &killed, Uuid::now_v7()), None);
        assert_eq!(receipt(true, "", nonce), None);
    }
}
