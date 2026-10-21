import Foundation

/// Which panes of one runner have really gone, from the fleets it has told
/// this app about (ov-622): a pane that goes takes its draft's photos and its
/// sent messages' thumbnails with it, and one fleet read that lacks a pane
/// (a read racing a restart, a runner still starting up) must not.
///
/// A pane counts as gone once it is missing from two reads at least `after`
/// seconds apart, with no read between that had it. Panes that went while the
/// app was closed are never seen here: the age limit on the files is their
/// backstop, and so it is for a runner that was removed or signed out of.
public struct MissingPanes: Sendable {
    public static let after: TimeInterval = 30

    private var known: Set<String>?
    private var missingSince: [String: Date] = [:]

    public init() {}

    /// A fleet read listed `now`. The panes confirmed gone by it.
    public mutating func read(_ now: Set<String>, at time: Date = Date()) -> Set<String> {
        guard let before = known else {
            known = now
            return []
        }
        var gone: Set<String> = []
        for pane in before.subtracting(now) {
            if let since = missingSince[pane] {
                if time.timeIntervalSince(since) >= Self.after { gone.insert(pane) }
            } else {
                missingSince[pane] = time
            }
        }
        for pane in now { missingSince[pane] = nil }
        for pane in gone { missingSince[pane] = nil }
        known = before.union(now).subtracting(gone)
        return gone
    }
}
