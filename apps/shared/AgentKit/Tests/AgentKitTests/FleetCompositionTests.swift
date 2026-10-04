import Foundation
import Testing

@testable import AgentKit

// The shared fleet table (ov-166): `test/fixtures/fleet-composition.json`,
// which the relay's `composeFleet` is held to as well
// (`services/relay/test/fleet-composition.test.ts`). The lock screen card is
// the relay's count of a fleet and the widgets and the watch are this one's,
// so the two must rank and count one fleet one way. See the table's `_about`.

/// The table, in the shape it's written in.
private struct Table: Decodable, Sendable {
    struct Runner: Decodable, Sendable {
        var id: String
        var needsYou: Int?
        /// Worktrees to review, as the runner's inbox said; nil when it sent none.
        var reviews: Int?
        var quiet: Bool
    }

    struct Agent: Decodable, Sendable {
        var terminal: String
        var runner: String
        var status: String
        var sinceS: Int
        var sinceMs: Int?
        var heardS: Int
        var rank: UInt32
    }

    struct Expect: Decodable, Sendable {
        var order: [String]
        var blocked: Int
        var working: Int
        var needsYou: Int?
        var header: Int
        /// What the app's widgets say is waiting on review: the sum of what the
        /// runners counted, nil when none did (ov-181). Absent in a case where none did.
        var reviewsWaiting: Int?
    }

    struct Case: Decodable, Sendable, CustomTestStringConvertible {
        var name: String
        var runners: [Runner]
        var agents: [Agent]
        var expect: Expect

        var testDescription: String { name }
    }

    var now: Int64
    var cases: [Case]

    static func load() throws -> Table {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(
            contentsOf: root.appendingPathComponent("test/fixtures/fleet-composition.json"))
        return try JSONDecoder().decode(Table.self, from: data)
    }
}

/// The table's cases, or none when it won't load — which the first test
/// below refuses, so a table that failed to load can't pass as empty.
private let table = try? Table.load()

/// The fleet as the app assembles it from the table: one snapshot per runner,
/// as its poll records it, a quiet runner's link lost, and each counting
/// runner's Needs You list as long as its count.
private func published(_ fleet: Table.Case, now: Date) -> FleetSnapshot {
    var publication = FleetPublication()
    for runner in fleet.runners {
        let agents = fleet.agents.filter { $0.runner == runner.id }.map { agent in
            FleetSnapshot.Agent(
                id: agent.terminal, label: agent.terminal, machine: runner.id,
                status: agent.status, glyph: "", headline: agent.terminal, line: "", feed: [],
                rank: agent.rank, turnFailed: false,
                activityChangedAt: now.addingTimeInterval(
                    -(agent.sinceMs.map { TimeInterval($0) / 1000 } ?? TimeInterval(agent.sinceS))),
                observedAt: now.addingTimeInterval(-TimeInterval(agent.heardS)),
                runner: runner.id)
        }
        publication.record(
            runner: runner.id,
            snapshot: FleetSnapshot(
                agents: agents, capturedAt: now, complete: true, reviewsWaiting: runner.reviews),
            named: runner.id)
    }
    publication.keeping(
        runners: Set(fleet.runners.map(\.id)),
        answering: Set(fleet.runners.filter { !$0.quiet }.map(\.id)))

    // The store hands over the lists only once some runner has sent one.
    var lists: [String: [NeedsYouItem]] = [:]
    for runner in fleet.runners {
        guard let count = runner.needsYou else { continue }
        lists[runner.id] = (0..<count).map { index in
            NeedsYouItem(
                id: "decision:\(runner.id)-\(index)", kind: .decision, rank: UInt32(index),
                since: nil, question: "")
        }
    }
    if fleet.runners.contains(where: { $0.needsYou != nil }) {
        publication.record(needsYou: lists)
    }
    return publication.merged(at: now)
}

@Test("The fleet table loads, with cases")
func theFleetTableLoads() throws {
    let table = try Table.load()
    #expect(table.cases.count > 5)
}

@Test("The app ranks and counts the fleet as the table says", arguments: table?.cases ?? [])
private func theAppRanksAndCountsTheFleetAsTheTableSays(_ fleet: Table.Case) throws {
    let now = Date(timeIntervalSince1970: TimeInterval(try #require(table).now) / 1000)
    let snapshot = published(fleet, now: now)

    #expect(snapshot.ranked.map(\.id) == fleet.expect.order)
    #expect(snapshot.agents.filter { $0.status == "blocked" }.count == fleet.expect.blocked)
    #expect(snapshot.working(at: now) == fleet.expect.working)
    #expect(snapshot.needsYou?.count == fleet.expect.needsYou)
    #expect(snapshot.needingYou == fleet.expect.header)
    // Worktrees, as each runner counted them. The relay's card holds the same
    // number for the same fleet, in `fleet-composition.test.ts`.
    #expect(snapshot.reviewsWaiting == fleet.expect.reviewsWaiting)

    // The one thing a small widget or a complication says: the header's count
    // when anything needs you, else the worktrees to review (ov-181), else
    // what's working, else nothing. The table has no failed turns.
    let glance: FleetSnapshot.Glance? =
        fleet.expect.header > 0 ? .blocked(fleet.expect.header)
        : (fleet.expect.reviewsWaiting ?? 0) > 0 ? .review(fleet.expect.reviewsWaiting ?? 0)
        : fleet.expect.working > 0 ? .working(fleet.expect.working) : nil
    #expect(snapshot.glance(at: now) == glance)
}
