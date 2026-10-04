import Foundation
import Testing

@testable import AgentKit

/// A push naming a paired runner the phone isn't connected to (ov-231).
struct PhoneDestinationIdleRunnerTests {
    static let push = ["terminal": "x", "runner": "RUNNER-B"]

    static func resolve(_ sources: [PhoneDestination.Source]) throws -> DestinationResolver.Resolution {
        let destination = try #require(Destination(userInfo: push, thread: ""))
        return DestinationResolver.resolve(
            destination, arrival: .notification, in: PhoneDestination.world(sources, last: nil), elapsed: 1,
            deadline: 60)
    }

    static func sources(known: [String: String], connected: [PhoneDestination.Source] = [PhoneDestinationTests.ready()])
        -> [PhoneDestination.Source]
    {
        PhoneDestination.sources(
            connected: connected, paired: [PhoneDestinationTests.runner, "host-b"], known: known, everyRunner: false,
            selected: PhoneDestinationTests.runner)
    }

    @Test("A push for a paired, idle runner whose id was remembered connects it")
    func connectsRememberedRunner() throws {
        let sources = Self.sources(known: ["host-b": "runner-b"])
        let idle = try #require(sources.first { $0.host == "host-b" })
        #expect(idle.runnerId == "runner-b" && idle.idle)
        #expect(try Self.resolve(sources) == .connect(host: "host-b"))
    }

    @Test("A push for a runner never connected here is not matched, and nothing is dialed")
    func unknownRunnerStaysAbsent() throws {
        let sources = Self.sources(known: [:])
        #expect(try Self.resolve(sources) != .connect(host: "host-b"))
        #expect(sources.first { $0.host == "host-b" }?.idle == true)
    }

    @Test("A connected runner's live id beats a stale remembered one")
    func liveIdBeatsStored() throws {
        let live = PhoneDestinationTests.ready(host: "host-b", runnerId: "runner-b")
        let sources = Self.sources(known: ["host-b": "stale"], connected: [PhoneDestinationTests.ready(), live])
        #expect(sources.first { $0.host == "host-b" }?.runnerId == "runner-b")
    }

    @Test("Runner ids are remembered, overwritten and forgotten")
    func store() throws {
        let suite = "runner-ids-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let ids = RunnerIds(defaults: defaults)
        #expect(ids.all.isEmpty)
        ids.remember("one", for: "h")
        ids.remember(nil, for: "h")
        ids.remember("", for: "h")
        #expect(RunnerIds(defaults: defaults).all == ["h": "one"])
        ids.remember("two", for: "h")
        #expect(ids.all == ["h": "two"])
        ids.forget("h")
        #expect(ids.all.isEmpty)
    }
}
