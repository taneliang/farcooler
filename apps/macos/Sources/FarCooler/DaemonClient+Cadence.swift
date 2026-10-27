import AgentKit
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

    /// Whether the next check of `state` waits on the slow cadence for a
    /// reason `waitsLong(_:)` can't see (ov-821): a refusal the probe has
    /// twice failed to prove, which asking again every thirty seconds would
    /// only cost the runner's address a failed login for.
    func retryWaitsLong(_ state: HostState) -> Bool {
        if Self.waitsLong(state) { return true }
        guard case .unreachable(let reason) = state else { return false }
        return !target.isEmpty && keyRefusals.cannotBeProven && KeyRefusals.isKeyRefusal(reason)
    }

    /// How long the next check of `state` waits when it isn't on the backoff:
    /// the slow cadence for a state retrying fixes none of, a version
    /// mismatch, or a refusal the probe has twice failed to prove (ov-821).
    func slowWait(for state: HostState) -> Double? {
        if retryWaitsLong(state) { return slowRetrySeconds }
        if case .unreachable(let reason) = state, looksVersionMismatch(reason) { return slowRetrySeconds }
        return nil
    }

    /// What follows a scheduled check's read: the events stream, unless the
    /// runner just refused this Mac, when the stream would be one more
    /// failed login for the same answer (ov-821). The next check is armed
    /// here instead; the stream starts once a read gets in.
    func afterRecheck() {
        if awaitingRecheck { scheduleRetry() } else { startEvents() }
    }
}
