import Foundation

/// What a failed read says about how soon to read again: the failures that
/// name a fix no retry makes, and so wait on the slow cadence
/// (`DaemonClient.scheduleRetry()`).
extension DaemonClient {
    /// Whether a failure names a runner that needs installing rather than
    /// one that is merely unreachable right now.
    ///
    /// `crates/cli/src/remote.rs`'s `explain` produces this exact phrase when
    /// ssh connects fine but nothing answers `farcoolerd --stdio` on the far
    /// side, which from here is indistinguishable from "not installed" — and
    /// is the CLI's own signal for it, not a guess. Local-only: the Mac
    /// bundles and starts its own daemon, so a local failure is never "go
    /// install this".
    func looksNotInstalled(_ message: String) -> Bool {
        !target.isEmpty && message.localizedCaseInsensitiveContains("is far cooler installed")
    }

    /// Whether a failure names two sides speaking different protocol
    /// versions.
    ///
    /// Same source as `looksNotInstalled`: `explain`'s other branch, for
    /// `ClientError::VersionMismatch`. No amount of retrying updates the
    /// older side, so this gets the same slow cadence as `notInstalled`
    /// rather than the fast exponential one — see `scheduleRetry()`.
    func looksVersionMismatch(_ message: String) -> Bool {
        !target.isEmpty && message.localizedCaseInsensitiveContains("update the older side")
    }

    /// Whether `state` is checked again on the slow cadence: a runner that
    /// needs installing, or that removed this Mac's member or refuses its
    /// key (ov-784, ov-818). Retrying fixes none of them, and a refused key
    /// retried fast is a refused login on the runner every thirty seconds.
    nonisolated static func waitsLong(_ state: HostState) -> Bool {
        switch state {
        case .notInstalled, .removed, .noAccess: return true
        case .connecting, .connected, .reconnecting, .unreachable: return false
        }
    }
}
