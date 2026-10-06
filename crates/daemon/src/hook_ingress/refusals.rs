//! The accept loop's refusals: which are about one connection, and how
//! often a refusal that won't go away is said (`HookIngress::listen`).

/// How often a refusal that will not go away is worth repeating.
///
/// The condition this exists for — a descriptor shortage — lasts as long as
/// whatever caused it, and at `ACCEPT_RETRY_PAUSE` the loop meets it about
/// seven times a second. A line each would put thousands of them an hour into
/// the log this module just went to some trouble to make worth reading.
pub(super) const REFUSAL_REPORT_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// Says the first refusal, then says it again rarely, then says how many there
/// were.
///
/// Separated from the loop so the rule can be tested against a clock the test
/// owns; provoking a real `EMFILE` would mean exhausting the descriptor table
/// of the whole test binary.
#[derive(Default)]
pub(super) struct Refusals {
    since_report: u64,
    total: u64,
    last_report: Option<std::time::Instant>,
}

impl Refusals {
    /// One more refusal. `Some(n)` when it is worth a line, `n` being how many
    /// have happened since the last one.
    ///
    /// The FIRST is always worth a line: the whole point is that a runner
    /// going deaf says so at the moment it happens, not thirty seconds later.
    pub(super) fn refused(&mut self, now: std::time::Instant) -> Option<u64> {
        self.since_report += 1;
        self.total += 1;
        let due = self.last_report.is_none_or(|t| now.duration_since(t) >= REFUSAL_REPORT_EVERY);
        if !due {
            return None;
        }
        self.last_report = Some(now);
        Some(std::mem::take(&mut self.since_report))
    }

    /// The socket is taking connections again. `Some(n)` when it had stopped,
    /// so the recovery names what the quiet was hiding.
    pub(super) fn recovered(&mut self) -> Option<u64> {
        let total = std::mem::take(&mut self.total);
        self.since_report = 0;
        self.last_report = None;
        (total > 0).then_some(total)
    }
}

/// Whether an `accept` failure is about this connection rather than the socket.
///
/// A descriptor shortage is the one that matters and it is the one Rust does
/// not name: `EMFILE` and `ENFILE` both arrive as `ErrorKind::Uncategorized`,
/// so they are matched by errno or not at all.
pub(super) fn transient(e: &std::io::Error) -> bool {
    if matches!(
        e.kind(),
        std::io::ErrorKind::Interrupted | std::io::ErrorKind::ConnectionAborted
    ) {
        return true;
    }
    matches!(
        e.raw_os_error(),
        Some(libc::EMFILE) | Some(libc::ENFILE) | Some(libc::ENOBUFS) | Some(libc::ENOMEM)
    )
}
