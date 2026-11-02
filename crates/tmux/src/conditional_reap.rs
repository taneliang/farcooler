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

/// Validated snapshot inputs are supplementary checks, not an incarnation
/// binding. Do not execute this candidate for an unsupported identity.
#[derive(Debug)]
pub struct Snapshot {
    pub pane: u32,
    pub pane_pid: u32,
    pub window: u32,
    pub session: u32,
    pub server_pid: u32,
    pub server_started: u64,
    pub daemon: Uuid,
    pub schema: u32,
    pub terminal: Uuid,
    pub worktree: Option<Uuid>,
}

/// Construct the one-server command, without running it. A caller still needs
/// an independently supported incarnation binding and a durable reservation.
pub fn conditional_argv(snapshot: &Snapshot, nonce: Uuid) -> Option<Vec<String>> {
    if snapshot.pane_pid == 0 || snapshot.server_pid == 0 || snapshot.server_started == 0 || snapshot.schema == 0 {
        return None;
    }
    fn eq(field: &str, value: &str) -> String { format!("#{{==:#{{{field}}},{value}}}") }
    let pane = format!("%{}", snapshot.pane);
    let mut terms = vec![
        eq("pane_id", &pane), eq("pane_pid", &snapshot.pane_pid.to_string()),
        eq("window_id", &format!("@{}", snapshot.window)), eq("session_id", &format!("${}", snapshot.session)),
        eq("pid", &snapshot.server_pid.to_string()), eq("start_time", &snapshot.server_started.to_string()),
        eq("session_name", "farcooler"), eq("@farcooler_daemon_id", &snapshot.daemon.to_string()),
        eq("@farcooler_schema_version", &snapshot.schema.to_string()),
    ];
    if let Some(worktree) = snapshot.worktree {
        terms.push(eq("@farcooler_terminal_id", &snapshot.terminal.to_string()));
        terms.push(eq("@farcooler_workspace_id", &worktree.to_string()));
    } else {
        terms.push(eq("@farcooler_terminal_id", ""));
        terms.push(format!("#{{m:* ##farcooler-opening:{},#{{pane_start_command}}}}", snapshot.terminal));
    }
    let predicate = terms.into_iter().reduce(|a, b| format!("#{{&&:{a},{b}}}"))?;
    Some(vec!["if-shell".into(), "-F".into(), "-t".into(), pane.clone(), predicate,
        format!("kill-pane -t {pane} ; display-message -p 'fc-reap-{nonce}:killed'"),
        format!("display-message -p 'fc-reap-{nonce}:skipped'")])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_has_server_side_predicate_and_nonce_branches() {
        let nonce = Uuid::now_v7();
        let mut snapshot = Snapshot { pane: 4, pane_pid: 11, window: 2, session: 1,
            server_pid: 12, server_started: 13, daemon: Uuid::now_v7(), schema: 1,
            terminal: Uuid::now_v7(), worktree: None };
        let argv = conditional_argv(&snapshot, nonce).unwrap();
        assert_eq!(&argv[..4], &["if-shell", "-F", "-t", "%4"]);
        assert!(argv[4].contains("#{==:#{pane_pid},11}"));
        assert!(argv[4].contains("#{==:#{@farcooler_terminal_id},}"));
        assert!(argv[4].contains(&format!("##farcooler-opening:{}", snapshot.terminal)));
        assert_eq!(argv[5], format!("kill-pane -t %4 ; display-message -p 'fc-reap-{nonce}:killed'"));
        assert_eq!(argv[6], format!("display-message -p 'fc-reap-{nonce}:skipped'"));
        snapshot.pane_pid = 0;
        assert!(conditional_argv(&snapshot, nonce).is_none());
    }

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
