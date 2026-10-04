import Foundation
import Testing

@testable import Far_Cooler

/// ⌘W on an agent that is mid-turn asks first (ov-161).
///
/// It used to stop and remove the selected terminal at once, with the agent's
/// conversation in it and no undo, while both phones asked through
/// `ShellClose`. The Mac asks when an agent is working or waiting on you, and
/// closes a shell, an idle agent or an exited pane directly.
struct CloseTerminalGuardTests {
    private static let now = Date(timeIntervalSince1970: 1_000_000)

    private static func terminal(
        preset: String = "claude", state: String = "running", activity: String? = "working"
    ) throws -> Terminal {
        // Started four minutes before `now`, in the daemon's milliseconds.
        let json = """
            {"id":"t-1","short":"t1","title":"claude","preset":"\(preset)","state":"\(state)",
            \(activity.map { "\"activity\":\"\($0)\"," } ?? "")
            "activitySince":\((now.timeIntervalSince1970 - 240) * 1000),
            "turnStartedAt":\((now.timeIntervalSince1970 - 240) * 1000),"epoch":0}
            """
        return try JSONDecoder().decode(Terminal.self, from: Data(json.utf8))
    }

    @Test func aWorkingAgentAsksAndSaysWhatItCosts() throws {
        let terminal = try Self.terminal(activity: "working")
        let question = try #require(CloseTerminalGuard.question(for: terminal, at: Self.now))
        #expect(question.title == "Close “\(terminal.label)”?")
        #expect(question.message.contains("working for 4m"), Comment(rawValue: question.message))
        #expect(question.message.contains("There’s no undo."), Comment(rawValue: question.message))
        // The button names what happens, in title case.
        #expect(CloseTerminalGuard.confirm == "Stop Agent and Close")
    }

    @Test func anAgentWaitingOnYouAsksToo() throws {
        let question = try #require(
            CloseTerminalGuard.question(for: Self.terminal(activity: "blocked"), at: Self.now))
        #expect(question.message.contains("waiting on you for 4m"), Comment(rawValue: question.message))
    }

    @Test("What closes at once", arguments: [
        ("idle agent", "running", "idle"),
        ("finished agent", "running", "done"),
        ("plain shell", "running", ""),
        ("exited pane", "exited", "working"),
        ("lost pane", "lost", "working"),
    ])
    func closesDirectly(_ what: String, state: String, activity: String) throws {
        let terminal = try Self.terminal(
            preset: activity.isEmpty ? "shell" : "claude", state: state,
            activity: activity.isEmpty ? nil : activity)
        #expect(CloseTerminalGuard.question(for: terminal, at: Self.now) == nil, "\(what) asked")
    }
}
