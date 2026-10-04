import Foundation

/// The `runner_id` each paired runner last said, by the phone's own handle
/// for it (`Host.id`, as a string), kept beside the runner list (ov-231).
///
/// A runner's id is learned only from its daemon build on connect, and the
/// pairing reply never carries one. Kept, a push naming a runner this phone
/// isn't connected to can still find it, and `PhoneDestination.sources`
/// seats it idle with this id. A separate map rather than a field on the
/// runner, so the runner list's hand-written decoding stays as it is.
public struct RunnerIds {
    private let defaults: UserDefaults
    private let key = "runnerIds"

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    /// Everything remembered, by host id. Empty when nothing is stored.
    public var all: [String: String] {
        defaults.dictionary(forKey: key) as? [String: String] ?? [:]
    }

    /// Remember `runnerId` for `host`, over any earlier one (a reinstalled
    /// runner says a new id). Nothing to remember from a runner too old to
    /// say, or one that says an empty id.
    public func remember(_ runnerId: String?, for host: String) {
        guard let runnerId, !runnerId.isEmpty, all[host] != runnerId else { return }
        var ids = all
        ids[host] = runnerId
        defaults.set(ids, forKey: key)
    }

    /// Whether a runner's remembered id outlives an edit: not when the edit
    /// re-points it or changes its user, which makes it another runner, so a
    /// push for the old one must not dial it.
    public static func survivesEdit(reachChanged: Bool, userChanged: Bool) -> Bool {
        !reachChanged && !userChanged
    }

    /// Forget a removed runner's id.
    public func forget(_ host: String) {
        var ids = all
        guard ids.removeValue(forKey: host) != nil else { return }
        defaults.set(ids, forKey: key)
    }
}
