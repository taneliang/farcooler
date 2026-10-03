import Foundation
import Testing

@testable import AgentKit

/// `test/fixtures/needs-you-shown.json`, the case table Kotlin's
/// `NeedsYouShownTest` reads too: what Needs You shows across runners, and the
/// caveat under "Nothing needs you". The case it exists for is a connected
/// runner whose list read failed, which Android once dropped (ov-151).
private struct ShownTable: Decodable {
    struct Runner: Decodable {
        var runner: String
        var answering: Bool
        var needs_you: NeedsYouList?
        var fleet: Fleet?
    }
    struct Case: Decodable {
        var name: String
        var runners: [Runner]
        var shown: [String]
        var derived: [String]
        var caveat: String?
    }
    var cases: [Case]

    static func load() throws -> ShownTable {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(
            contentsOf: root.appendingPathComponent("test/fixtures/needs-you-shown.json"))
        return try JSONDecoder().decode(ShownTable.self, from: data)
    }
}

@Test("Every case in the shared Needs You table holds")
func everyCaseInTheSharedNeedsYouTableHolds() throws {
    let table = try ShownTable.load()
    #expect(table.cases.count >= 5)
    for c in table.cases {
        // As `FleetStore.publish` builds them: a read list, else the fleet.
        var lists: [String: [NeedsYouItem]] = [:]
        var unread: [String: [NeedsYou.OlderPane]] = [:]
        for runner in c.runners {
            if let list = runner.needs_you {
                lists[runner.runner] = list.items
            } else if let fleet = runner.fleet {
                unread[runner.runner] = fleet.olderPanes()
            }
        }
        let shown = PhoneInbox.shown(lists: lists, unread: unread)
        #expect(shown.map(\.itemID) == c.shown, "\(c.name)")
        #expect(shown.filter(\.isDerived).map(\.itemID) == c.derived, "\(c.name)")
        // As `NeedsYouScreen.unanswered` names them.
        let unanswered = c.runners.filter { $0.needs_you == nil || !$0.answering }.map(\.runner)
        #expect(PhoneInbox.caveat(unanswered: unanswered) == c.caveat, "\(c.name)")
    }
}
