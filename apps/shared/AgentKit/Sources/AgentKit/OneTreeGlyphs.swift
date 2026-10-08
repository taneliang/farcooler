import Foundation

/// The one place each kind of thing in the workspace gets its SF Symbol
/// (ov-328). The navigator's rows (`OneTree`'s nodes) and the jump bar's
/// segments and menu rows all read it, so the sidebar and the bar can't come
/// to draw one thing two ways. Add a kind here, never as a literal at a call
/// site; `OneTreeGlyphTests` and `JumpBarGlyphTests` read both sides.
public enum OneTreeGlyph {
    /// The orchestrator's symbol: the title bar's mark at rest and the jump
    /// bar's Orchestrator row. The sidebar has no Orchestrator row now (R-14);
    /// the row it had drew a speech bubble where the title bar drew this
    /// person, two symbols for one thing.
    public static let orchestrator = "person.wave.2"
    public static let needsYou = "flag"
    public static let plan = "map"
    public static let page = "doc.text"
    /// A worktree, and a lane's own with no state: a branch.
    public static let worktree = "arrow.triangle.branch"
    public static let doneFold = "checkmark"
    public static let noTheme = "tray"
    public static let mainCheckout = "house"
    public static let looseWorktrees = "archivebox"
    public static let hidden = "eye.slash"
    public static let notOnThisVersion = "questionmark.square.dashed"
    public static let unreadableCard = "questionmark.circle"
    public static let subagent = "person.crop.circle.dashed"

    /// A theme's, by its state: a paused one and a finished one say so.
    public static func theme(state: String) -> String {
        switch state {
        case "paused": "pause.circle"
        case "done": "checkmark.circle"
        default: plan
        }
    }

    /// A card's, by its status: the shape before the color.
    public static func task(_ status: TaskStatus) -> String { TaskStatusGlyph.name(status) }

    /// A plan lane's, by its state.
    public static func lane(_ state: LaneState) -> String { OneTreeWords.glyph(state) }

    /// A terminal's: an agent's sparkles, a shell's prompt.
    public static func terminal(isAgent: Bool) -> String { isAgent ? "sparkles" : "terminal" }

    /// A web pane's (ov-435).
    public static let web = "globe"
}
