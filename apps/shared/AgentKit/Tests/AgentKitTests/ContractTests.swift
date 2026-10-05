import Foundation
import Testing

@testable import AgentKit

// The JSON this package shares with the relay, against the fixtures in
// `test/fixtures/contracts/` (ov-121). The relay's suite posts the
// registrations written here and writes the push and card payloads decoded
// here, so a key renamed on either side fails one suite or the other. See the
// README beside the fixtures for who writes and who reads each one.

enum Contracts {
    static var root: URL {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/contracts")
    }

    /// The fixtures in one directory, by name without `.json`.
    static func names(_ dir: String) throws -> [String] {
        try FileManager.default.contentsOfDirectory(atPath: root.appendingPathComponent(dir).path)
            .filter { $0.hasSuffix(".json") }
            .map { String($0.dropLast(".json".count)) }
            .sorted()
    }

    static func data(_ path: String) throws -> Data {
        try Data(contentsOf: root.appendingPathComponent(path))
    }

    /// A fixture as `JSONSerialization` reads it, which is how a push's
    /// `userInfo` arrives: strings, numbers, arrays and dictionaries.
    static func object(_ path: String) throws -> [String: Any] {
        try #require(try JSONSerialization.jsonObject(with: data(path)) as? [String: Any])
    }

    /// The same value, round-tripped through JSON, so a Swift `Bool` and a
    /// decoded `true` compare as one.
    static func json(_ value: [String: Any]) throws -> NSDictionary {
        let data = try JSONSerialization.data(withJSONObject: value)
        return try #require(try JSONSerialization.jsonObject(with: data) as? NSDictionary)
    }

    struct RewritingUnderCI: Error {}

    /// `FARCOOLER_WRITE_CONTRACTS=1` rewrites a producer's fixture instead of
    /// comparing. Only for a deliberate change, reviewed in the diff, and
    /// refused under CI, where a producer that rewrote its fixture would pass
    /// by definition.
    static func write(_ value: [String: Any], to path: String) throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["FARCOOLER_WRITE_CONTRACTS"] != nil else { return }
        guard environment["CI"] != "true" else { throw RewritingUnderCI() }
        var data = try JSONSerialization.data(
            withJSONObject: value, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
        data.append(0x0a)
        try data.write(to: root.appendingPathComponent(path))
    }
}

/// The task the daemon's notify fixtures name, and the runner it's on.
private let runner = "3a342465-0508-031f-1852-5550524f0f01"
private let noticeId = "t:\(runner):ov-90"

struct RegistrationContractTests {
    /// What the fixtures say for `version`. `AppVersion.reported` differs per
    /// build, so it's checked for being this build's and compared as this.
    static let version = "0.2.0 (canary) · 412"

    /// What `registerDevice` sends from each app, with the values it sends
    /// in a real registration.
    static func produced(_ name: String) -> [String: Any]? {
        switch name {
        case "ios": Account.registration(
            pushToken: "7c3f1a9e2b8d4c6f0e1a3b5c7d9e2f4a6b8c0d2e4f6a8b0c2d4e6f8a0b2c4d6e",
            platform: "apns", label: "iPhone", environment: "production",
            liveActivityStartToken: "80b9d3f7a2c64e1b9f0d8c7a6b5e4d3c2b1a09f8e7d6c5b4a3928170f6e5d4c3b2a1",
            notifyOnDone: true, notifyEvents: ["decision", "review", "blocked"],
            pulseToken: "5d41402abc4b2a76b9719d911017c592ae2f6b0c8e3d1f7a9b4c6e8d0f2a4b6c")
        case "macos": Account.registration(
            pushToken: "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90",
            platform: "apns", label: "Studio", environment: "development",
            liveActivityStartToken: nil, notifyOnDone: false,
            notifyEvents: ["decision", "review", "blocked", "done"], pulseToken: nil)
        default: nil
        }
    }

    @Test("Each Apple registration fixture is what registerDevice sends", arguments: ["ios", "macos"])
    func eachRegistrationIsWhatRegisterDeviceSends(name: String) throws {
        var payload = try #require(Self.produced(name))
        #expect(payload["version"] as? String == AppVersion.reported)
        payload["version"] = Self.version
        let path = "registration/\(name).json"
        try Contracts.write(payload, to: path)
        let fixture = try Contracts.json(Contracts.object(path))
        #expect(try Contracts.json(payload) == fixture, "\(path) is not what Account.registration sends")
    }
}

struct ActivityRegistrationContractTests {
    /// What `registerActivityToken` sends for a card's token, for a card a
    /// person swiped away, and for one that ended any other way
    /// (`LiveActivities.watch` in the iOS app).
    static func produced(_ name: String) -> [String: Any]? {
        let terminal = "term-01999a8f2c4e"
        return switch name {
        case "running": Account.activityRegistration(
            terminal: terminal,
            updateToken: "80f1c2d3e4b5a6978869504132a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7",
            environment: "production", dismissed: false)
        case "dismissed": Account.activityRegistration(
            terminal: terminal, updateToken: nil, environment: "production", dismissed: true)
        case "ended": Account.activityRegistration(
            terminal: terminal, updateToken: nil, environment: "production", dismissed: false)
        default: nil
        }
    }

    @Test("Each activity fixture is what registerActivityToken sends", arguments: ["running", "dismissed", "ended"])
    func eachActivityIsWhatRegisterActivityTokenSends(name: String) throws {
        let payload = try #require(Self.produced(name))
        let path = "activity/\(name).json"
        try Contracts.write(payload, to: path)
        let fixture = try Contracts.json(Contracts.object(path))
        #expect(try Contracts.json(payload) == fixture, "\(path) is not what Account.activityRegistration sends")
    }

    @Test("Every activity fixture has a producer")
    func everyFixtureHasAProducer() throws {
        #expect(try Contracts.names("activity") == ["dismissed", "ended", "running"])
    }
}

struct PulseContractTests {
    @Test("The relay's pulse answer decodes to the runner that beat")
    func thePulseDecodes() throws {
        let pulses = try #require(RunnerPulse.decode(Contracts.data("pulse/response.json")))
        #expect(pulses.count == 1)
        let pulse = try #require(pulses.first)
        #expect(pulse.label == "Studio")
        #expect(pulse.name == "Studio")
        #expect(pulse.heardAgo == 90_000)
        #expect(pulse.beatEvery == 300)
        // The relay's per-account key over the beat's runner id, which a
        // phone makes too (ov-71).
        #expect(pulse.runner == RunnerPulse.key(account: "user_1", runner: runner))
        #expect(!pulse.isQuiet)
    }
}

struct PushContractTests {
    /// Where each alert the relay sends an Apple device opens (its `Destination`'s
    /// place; ov-183 replaced `PushTap` with it), and the task
    /// notice it carries. Every fixture must have a line here.
    static let expected: [String: (tap: Destination.Place, notice: TaskNotice?)] = [
        "agent-blocked": (.terminal("term-01999a8f2c4e"), nil),
        "agent-done-failed": (.terminal("term-01999a90aa10"), nil),
        // A runner older than ov-94: no event and no notice id, so it opens
        // the task's card and is no task notice.
        "decision-old-runner": (.task(workspace: nil, task: .init(key: "ov-90")), nil),
        "task-decision": (
            .task(workspace: nil, task: .init(key: "ov-90")),
            TaskNotice(key: "ov-90", runner: runner, event: .decision, noticeId: noticeId, options: ["pdfkit", "pdf.js"])
        ),
        "task-review": (
            .task(workspace: nil, task: .init(key: "ov-90")),
            TaskNotice(key: "ov-90", runner: runner, event: .review, noticeId: noticeId, options: [])
        ),
    ]

    @Test("Every APNs fixture has an expectation, so none is read by nothing")
    func everyFixtureIsCovered() throws {
        #expect(try Contracts.names("push/apns") == Self.expected.keys.sorted())
    }

    @Test("Each APNs payload the relay sends opens what it's about", arguments: Self.expected.keys.sorted())
    func eachPayloadOpensWhatItIsAbout(name: String) throws {
        let userInfo = try Contracts.object("push/apns/\(name).json")
        let aps = try #require(userInfo["aps"] as? [String: Any])
        let thread = try #require(aps["thread-id"] as? String)
        let want = try #require(Self.expected[name])
        #expect(Destination(userInfo: userInfo, thread: thread)?.place == want.tap)
        #expect(TaskNotice(userInfo: userInfo) == want.notice)
        #expect(AgentPush(userInfo: userInfo) == Self.agents[name] ?? nil)
    }

    /// What the notification service extension folds into the widget from
    /// each: an agent's pane, status, name and how its turn ended, or nothing
    /// for a task notice.
    static let agents: [String: AgentPush?] = [
        "agent-blocked": AgentPush(terminal: "term-01999a8f2c4e", status: "blocked", label: "claude", failed: false),
        "agent-done-failed": AgentPush(terminal: "term-01999a90aa10", status: "done", label: "codex", failed: true),
        "decision-old-runner": nil,
        "task-decision": nil,
        "task-review": nil,
    ]
}

struct LiveActivityContractTests {
    @Test("Every card fixture decodes its headline, counts and rows")
    func everyCardDecodes() throws {
        let names = try Contracts.names("live-activity")
        #expect(names == ["agent-blocked", "agent-blocked-quiet"])
        for name in names {
            let body = try Contracts.object("live-activity/\(name).json")
            let aps = try #require(body["aps"] as? [String: Any])
            #expect(aps["attributes-type"] as? String == "AgentActivityAttributes")
            #expect((aps["attributes"] as? [String: Any])?["version"] as? Int == 3)
            let state = try JSONSerialization.data(withJSONObject: try #require(aps["content-state"]))
            let card = try JSONDecoder().decode(AgentCardState.self, from: state)
            #expect(card.status == "blocked")
            #expect(card.machine == "Studio")
            #expect(card.knowsFleet)
            #expect(card.needsYou == 2)
            #expect(card.rows.map(\.terminal) == [card.terminal])
        }
    }

    @Test("A blocked agent's card carries its ask, its clock and its trace")
    func aBlockedCardCarriesEverything() throws {
        let body = try Contracts.object("live-activity/agent-blocked.json")
        let aps = try #require(body["aps"] as? [String: Any])
        let state = try JSONSerialization.data(withJSONObject: try #require(aps["content-state"]))
        let card = try JSONDecoder().decode(AgentCardState.self, from: state)
        #expect(card.terminal == "term-01999a8f2c4e")
        #expect(card.label == "claude")
        #expect(card.workspace == "Billing")
        #expect(card.detail == "auth-refactor — Do you want to run git push --force-with-lease?")
        #expect(card.startedAt == Date(timeIntervalSince1970: 1_791_018_420))
        #expect([card.blocked, card.review, card.working, card.more] as [Int] == [1, 0, 0, 0])
        #expect([card.insertions, card.deletions, card.commits] as [Int?] == [142, 37, 1])
        #expect(card.ask?.id == "hook-ask-0199a8f3-1b2c-7d4e-8f50-6a7b8c9d0e1f")
        #expect(card.ask?.tool == "Bash")
        #expect(card.ask?.until == Date(timeIntervalSince1970: 1_791_020_340))

        let row = try #require(card.rows.first)
        #expect(row.label == "claude")
        #expect(row.workspace == "Billing")
        #expect([row.insertions, row.deletions, row.commits] as [Int?] == [142, 37, 1])
        #expect(row.startedAt == Date(timeIntervalSince1970: 1_791_018_420))
        #expect(row.updatedAt == Date(timeIntervalSince1970: 1_791_019_800))
        #expect(row.trace?.count == 66, "the daemon's 66 trace bytes, base64")
        #expect(row.traceAnchor == 5_970_066)
    }

    @Test("A running card's update and end carry the card the app decodes")
    func aRunningCardUpdatesAndEnds() throws {
        #expect(try Contracts.names("live-activity/running") == ["end", "plan", "update"])

        let update = try #require(try Contracts.object("live-activity/running/update.json")["aps"] as? [String: Any])
        #expect(update["event"] as? String == "update")
        #expect(update["attributes"] == nil, "APNs refuses an update that repeats the attributes")
        let state = try JSONSerialization.data(withJSONObject: try #require(update["content-state"]))
        let card = try JSONDecoder().decode(AgentCardState.self, from: state)
        #expect(card.status == "working")
        #expect(card.terminal == "term-01999a8f2c4e")
        #expect(card.detail == "3/7 · Designing test matrix")
        #expect(card.rows.first?.traceAnchor == 5_970_066)

        let end = try #require(try Contracts.object("live-activity/running/end.json")["aps"] as? [String: Any])
        #expect(end["event"] as? String == "end")
        #expect(end["dismissal-date"] as? Int == 1_791_019_800, "taken down now: the retire's `immediate`")
        let over = try JSONSerialization.data(withJSONObject: try #require(end["content-state"]))
        #expect(try JSONDecoder().decode(AgentCardState.self, from: over).status == "done")
    }
}
