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
