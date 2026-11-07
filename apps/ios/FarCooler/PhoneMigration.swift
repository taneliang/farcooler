import Foundation

/// What this app kept under names it has since changed, moved once at launch
/// (ov-55 4A.4). Idempotent: a key already moved is left alone.
enum PhoneMigration {
    /// The describe-a-task composer was "Quick Task", then "New Worktree…".
    /// It is gone (ov-951): a task is the orchestrator's to start. What it
    /// kept under these names is removed once, rather than left on the phone.
    static let retiredDrafts = [
        "quicktask.draft", "quicktask.project", "quicktask.agent", "quicktask.model",
        "newworktree.draft", "newworktree.project", "newworktree.agent", "newworktree.model",
    ]

    static func run(_ defaults: UserDefaults = .standard) {
        for key in retiredDrafts { defaults.removeObject(forKey: key) }
        // The crossing note and the cache of other runners' worktrees retired
        // with the overview; nothing reads either, so an old one would only
        // linger.
        defaults.removeObject(forKey: "shell.crossingTo")
        defaults.removeObject(forKey: "runnerDirectories")
    }
}
