import Foundation

/// The tab somebody chose in each worktree of one runner, kept across a
/// launch (ov-233): "when the app is restarted, it should try to go back to
/// wherever the user was", down to the pane.
///
/// Generic over the value, because `PaneFocus` lives in the iOS target (it
/// reads the phone's inbox) and AgentKit cannot see it; a test stands in an
/// enum of the same shape. `UserDefaults` and not `@SceneStorage`: the old
/// scene copy went with the navigation it belonged to, which is what
/// `Connection.lastFocus` records.
///
/// One key per runner, because worktree ids are minted per runner.
public enum FocusMemory<Value: Codable & Sendable> {
    public static func key(runner: String) -> String { "phone.paneFocus.v1.\(runner)" }

    /// What was kept for `runner`, or nothing: a value this build can't read
    /// (a newer build's, or damage) is no memory rather than a crash.
    public static func load(runner: String, from defaults: UserDefaults = .standard) -> [String: Value] {
        guard let data = defaults.data(forKey: key(runner: runner)),
            let kept = try? JSONDecoder().decode([String: Value].self, from: data)
        else { return [:] }
        return kept
    }

    public static func save(
        _ memory: [String: Value], runner: String, to defaults: UserDefaults = .standard
    ) {
        if memory.isEmpty {
            defaults.removeObject(forKey: key(runner: runner))
        } else {
            defaults.set(try? JSONEncoder().encode(memory), forKey: key(runner: runner))
        }
    }

    /// `memory` without the worktrees `keeping` doesn't name: gone from a
    /// loaded fleet, so a choice there names nothing.
    public static func pruned(_ memory: [String: Value], keeping worktrees: Set<String>) -> [String: Value] {
        memory.filter { worktrees.contains($0.key) }
    }
}

/// One runner's chosen tabs as a `Connection` holds them: loaded when the
/// connection starts, saved on every choice, pruned to a loaded fleet (ov-233).
///
/// A value, so a test reaches the save, the load and the prune without a
/// runner. `Connection` holds one and `FocusLedgerWiringTests` pins that its
/// `start` adopts and its `rememberFocus` remembers.
public struct FocusLedger<Value: Codable & Sendable>: Sendable {
    public private(set) var memory: [String: Value] = [:]
    private var runner: String?
    private let defaults: UserDefaultsBox

    /// `UserDefaults` isn't `Sendable`; this carries one.
    private struct UserDefaultsBox: @unchecked Sendable { let defaults: UserDefaults }

    public init(defaults: UserDefaults = .standard) {
        self.defaults = UserDefaultsBox(defaults: defaults)
    }

    /// The runner this is for is known: read what was kept for it. A choice made
    /// before this wins over the kept one for its worktree, and the merge is
    /// written back, so an early choice doesn't overwrite the rest.
    public mutating func adopt(runner: String) {
        self.runner = runner
        let before = memory
        memory = FocusMemory<Value>.load(runner: runner, from: defaults.defaults)
            .merging(before) { _, new in new }
        if !before.isEmpty { FocusMemory<Value>.save(memory, runner: runner, to: defaults.defaults) }
    }

    /// Somebody chose `value` in `worktree`. Written at once when the runner is known.
    public mutating func remember(_ value: Value, in worktree: String) {
        memory[worktree] = value
        if let runner { FocusMemory<Value>.save(memory, runner: runner, to: defaults.defaults) }
    }

    /// Forget choices in worktrees a loaded fleet no longer has. Not on an empty
    /// one: that is a runner that hasn't answered, and would forget them all.
    public mutating func prune(keeping worktrees: Set<String>) {
        guard !worktrees.isEmpty else { return }
        let kept = FocusMemory<Value>.pruned(memory, keeping: worktrees)
        guard kept.count != memory.count else { return }
        memory = kept
        if let runner { FocusMemory<Value>.save(memory, runner: runner, to: defaults.defaults) }
    }
}
