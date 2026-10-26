import AgentKit
import Foundation

/// Where a runner's connection stands.
///
/// `notInstalled` is separate from `unreachable` because it is not a failure
/// worth retrying at full speed. It is a runner that needs `host install`,
/// and retrying it every second forever produces noise instead of the one
/// sentence that would fix it. It still gets checked again — every few
/// minutes, see `DaemonClient.scheduleRetry()` — because "never" is the
/// opposite failure: installing it later would otherwise go unnoticed until
/// someone restarts the app.
enum HostState: Equatable {
    case connecting
    case connected
    case reconnecting(attempt: Int)
    case unreachable(reason: String)
    case notInstalled
    /// This Mac's member was removed from the runner (ov-784): it refuses
    /// every read but `member list`, which says so. Not fixed by retrying
    /// at full speed, but checked again slowly, so being added back is seen.
    case removed

    var isUsable: Bool { self == .connected }

    /// What to tell someone whose action was refused, or nil to let it proceed.
    ///
    /// Only `.unreachable`, `.notInstalled` and `.removed` refuse. Those are the two states
    /// where a command was already tried against this runner and failed, so
    /// trying another is asking the same question twice. `.connecting` and
    /// `.reconnecting` mean "we do not know yet", not "no" — refusing on those
    /// would turn an ordinary transient stream drop on an otherwise-reachable
    /// runner into a read-only window for up to a 30s backoff, when the
    /// command itself would simply have succeeded, or failed on its own and
    /// bounded by `ConnectTimeout`, exactly as it would on a runner this
    /// client had never seen go down at all. A false refusal here is worse
    /// than the bounded wait a real attempt risks.
    var refusal: String? {
        switch self {
        case .connected, .connecting, .reconnecting: return nil
        case .unreachable(let why): return why
        case .notInstalled: return "Far Cooler isn’t installed on this runner."
        case .removed: return RunnerPeople.removedBody
        }
    }
}
