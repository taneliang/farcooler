import Foundation

/// Whether a runner's admins require members to sign in to each agent with
/// their own account (ov-652), as `member.list`'s `own_logins` says it.
///
/// Off by default: a member's pane of that agent may run on the runner's own
/// login, which is its owner's account, quota and history. A member signed in
/// to Claude still uses their own. The words are here so the Mac and the
/// iPhone say the same thing.
public struct MemberOwnLogins: Equatable, Sendable, Decodable {
    /// An agent with a switch.
    public enum Agent: String, CaseIterable, Identifiable, Sendable {
        case claude, codex, cursor

        public var id: String { rawValue }

        /// Its name as people read it.
        public var title: String {
            switch self {
            case .claude: return "Claude"
            case .codex: return "Codex"
            case .cursor: return "Cursor"
            }
        }
    }

    public var claude: Bool
    public var codex: Bool
    public var cursor: Bool
    /// Whether this device may change them: the runner's admins only.
    public var mayChange: Bool

    public init(claude: Bool = false, codex: Bool = false, cursor: Bool = false, mayChange: Bool = false) {
        self.claude = claude
        self.codex = codex
        self.cursor = cursor
        self.mayChange = mayChange
    }

    /// Whether members must use their own account for `agent`.
    public func requiresOwn(_ agent: Agent) -> Bool {
        switch agent {
        case .claude: return claude
        case .codex: return codex
        case .cursor: return cursor
        }
    }

    /// These, with `agent`'s switch set to `on`.
    public func setting(_ agent: Agent, _ on: Bool) -> MemberOwnLogins {
        var copy = self
        switch agent {
        case .claude: copy.claude = on
        case .codex: copy.codex = on
        case .cursor: copy.cursor = on
        }
        return copy
    }

    enum CodingKeys: String, CodingKey {
        case claude, codex, cursor
        case mayChange = "may_change"
    }

    // MARK: - Words

    public static let sectionTitle = "Member Logins"

    /// The switch's own label.
    public static func toggleTitle(_ agent: Agent) -> String { "Require Own \(agent.title) Account" }

    /// The line under a switch: what it means now.
    public static func stateLine(_ agent: Agent, on: Bool) -> String {
        switch (agent, on) {
        case (.claude, true): return "Each member signs in to Claude here with their own account."
        case (.claude, false): return "Members who haven’t signed in use this runner’s Claude login."
        case (_, true): return "Members can’t use \(agent.title) here until they can connect their own account."
        case (_, false): return "Members use this runner’s \(agent.title) login."
        }
    }

    /// What the section's footer says. `owner`: this device is the runner
    /// owner's, whose account is the one shared.
    public static func footer(owner: Bool, mayChange: Bool) -> String {
        let whose = owner ? "your account" : "the runner owner’s account"
        var text = "A member on this runner’s login uses \(whose), its quota and its history, "
            + "including Claude’s memory, Codex sessions and Cursor chats. "
            + "A change applies the next time each pane starts."
        if !mayChange { text += " Only this runner’s admins can change these." }
        return text
    }
}
