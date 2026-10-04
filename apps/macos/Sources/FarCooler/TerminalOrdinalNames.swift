import AgentKit

extension Worktree {
    /// How `terminal` is named where terminals are listed side by side (the
    /// sidebar, the jump bar and ⌘K): its label, with an ordinal from the
    /// second one on, the way Terminal.app tells two windows apart. Two
    /// `shell`s in one worktree read "shell" and "shell 2", never two
    /// identical lines.
    ///
    /// The first keeps its bare label, so a worktree with one terminal never
    /// shows a number. Both halves come from `ordinals()`, so every place
    /// agrees on which terminal is the second. (`Terminal.displayName` is the
    /// older form, which numbers every duplicate, the first as "shell 1".)
    func name(of terminal: Terminal) -> String {
        guard let ordinal = ordinals()[terminal.id], ordinal > 1 else { return terminal.label }
        return "\(terminal.label) \(ordinal)"
    }
}
