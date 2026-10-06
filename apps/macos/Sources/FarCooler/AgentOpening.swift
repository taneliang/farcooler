import Foundation

/// Which view a detected coding agent's pane opens in on this Mac (ov-361,
/// ruling R-27).
///
/// The runner never decides: every pane it creates is a terminal
/// (`PaneMode::Terminal`, see `a_new_terminal_starts_in_terminal_pane_mode` in
/// crates/store), and the pane's mode is a record the runner keeps, shared by
/// every client, so a pane someone switched to chat stays in chat. The one
/// thing that ever moves a new pane out of the terminal on its own is this
/// Mac's "Open coding agents as a chat" preference, and until the CLI-backed
/// chat ships (ov-359) it is off for everyone who hasn't turned it on.
enum AgentOpening {
    /// What a Mac with no stored choice does: open the terminal.
    static let preferChatDefault = false

    /// Whether to switch `terminal` to its chat now. Only a pane that is still
    /// a terminal, can be a chat, and hasn't been offered it before: a pane
    /// already in chat stays there, and one switched back is left alone.
    static func opensAsChat(
        preferChat: Bool, canSwitch: Bool, isAgentPane: Bool, alreadyOffered: Bool
    ) -> Bool {
        preferChat && canSwitch && !isAgentPane && !alreadyOffered
    }
}
