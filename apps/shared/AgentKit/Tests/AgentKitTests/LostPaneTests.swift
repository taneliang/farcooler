import Foundation
import Testing

@testable import AgentKit

/// `test/fixtures/lost-pane.json`, which Android's `LostPaneTest` replays too:
/// every sentence whole, so changing one on this side and not the other fails
/// on the side left behind.
private enum Fixture {
    nonisolated(unsafe) static let root: [String: Any] = {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try! Data(contentsOf: root.appendingPathComponent("test/fixtures/lost-pane.json"))
        return try! JSONSerialization.jsonObject(with: data) as! [String: Any]
    }()

    static func list(_ key: String) -> [[String: Any]] { root[key] as? [[String: Any]] ?? [] }
}

/// The page a terminal with no running pane opens to (ov-191).
struct LostPaneTests {
    /// Each kind's title, why, offer and whole phone sentence, from either
    /// spelling of its state: the Mac's CLI writes `LOST`, the phone's core
    /// `lost`.
    @Test func everyKindSaysWhatTheFixtureSays() {
        let kinds = Fixture.list("kinds")
        #expect(kinds.count == 3)
        for each in kinds {
            let state = each["state"] as! String
            guard let kind = LostPane.Kind(state: state), LostPane.Kind(state: state.lowercased()) == kind else {
                Issue.record("\(state) isn't a kind")
                continue
            }
            #expect(LostPane.title(for: kind) == each["title"] as? String, "\(state)")
            #expect(LostPane.explanation(for: kind) == each["explanation"] as? String, "\(state)")
            #expect(LostPane.actions(for: kind).map(\.title) == each["actions"] as? [String], "\(state)")
            #expect(LostPane.message(for: kind, preset: "shell") == each["shell_message"] as? String, "\(state)")
        }
    }

    /// A pane that's running, starting, or can't be read right now gets the
    /// terminal, not this page.
    @Test func aLiveOrUnreadPaneIsNotThisPage() {
        let states = Fixture.root["not_this_page"] as? [String] ?? []
        #expect(!states.isEmpty)
        for state in states { #expect(LostPane.Kind(state: state) == nil, "\(state)") }
    }

    /// **Restart with and without a recorded command.** A shell says what
    /// was typed into it wasn't recorded; any other preset is named.
    @Test func restartNotesAreTheFixtures() {
        let notes = Fixture.list("restart_notes")
        #expect(notes.count >= 6)
        for each in notes {
            let preset = each["preset"] as! String
            #expect(LostPane.restartNote(preset: preset) == each["note"] as? String, "\(preset)")
        }
    }

    @Test func dismissAndFailuresAreTheFixtures() {
        #expect(LostPane.dismissNote == Fixture.root["dismiss_note"] as? String)
        let failures = Fixture.root["failures"] as? [String: String] ?? [:]
        for action in [LostPane.Action.restart, .dismiss] {
            #expect(action.failure == failures[action.title], "\(action.title)")
        }
    }

    /// The edge a not-live pane re-attaches on, after a Restart among
    /// others. Android's `NotLivePane.revives` replays the same table.
    @Test func revivalIsTheFixtures() {
        let cases = Fixture.list("revives")
        #expect(cases.count >= 10)
        for each in cases {
            let from = StateKind.parse(each["from"] as! String)
            let to = StateKind.parse(each["to"] as! String)
            #expect(NotLivePane.revives(from: from, to: to) == each["revives"] as? Bool, "\(each)")
        }
    }
}
