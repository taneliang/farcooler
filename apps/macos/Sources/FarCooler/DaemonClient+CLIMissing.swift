import Foundation

extension DaemonClient {
    /// What `.unreachable` says when there is no CLI to run. It stays
    /// `.unreachable` (not `.notInstalled`, whose slow recheck would notice an
    /// install minutes late); `isCLIMissing` tells it apart.
    static let cliMissingReason = "Far Cooler’s command-line tool isn’t installed."

    /// Whether this runner has no Far Cooler to run: the local CLI is missing,
    /// or a remote said so.
    var isCLIMissing: Bool {
        switch state {
        case .notInstalled: true
        case .unreachable(let reason): reason == Self.cliMissingReason
        default: false
        }
    }
}
