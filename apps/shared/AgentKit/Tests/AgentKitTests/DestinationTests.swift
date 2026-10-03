import Foundation
import Testing

@testable import AgentKit

/// `test/fixtures/destinations.json`, which Android's `DestinationTest`
/// replays too, so the two can't drift apart on a byte of the encoding, a
/// spelling of an old notification, or a rule of the resolver.
private enum Fixture {
    nonisolated(unsafe) static let root: [String: Any] = {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try! Data(contentsOf: root.appendingPathComponent("test/fixtures/destinations.json"))
        return try! JSONSerialization.jsonObject(with: data) as! [String: Any]
    }()

    static func cases(_ section: String) -> [[String: Any]] {
        root[section] as? [[String: Any]] ?? []
    }

    /// The encoding of the `encodings` case named `name`.
    static func encoding(_ name: String) -> String {
        cases("encodings").first(where: { $0["case"] as? String == name })?["json"] as? String ?? "(no such case)"
    }
}

struct DestinationFixtureTests {
    @Test func encodingsRoundTripByteForByte() {
        let cases = Fixture.cases("encodings")
        #expect(cases.count >= 10)
        for each in cases {
            let json = each["json"] as! String
            let value = Destination(encoded: json)
            #expect(value != nil, "\(each["case"]!)")
            #expect(value?.encoded == json, "\(each["case"]!)")
        }
    }

    @Test func invalidEncodingsAreNoPlace() {
        let cases = Fixture.cases("invalid")
        #expect(cases.count >= 8)
        for each in cases {
            #expect(Destination(encoded: each["json"] as! String) == nil, "\(each["case"]!)")
        }
    }

    @Test func lenientEncodingsDropWhatTheyDontKnow() {
        let cases = Fixture.cases("lenient")
        #expect(cases.count >= 8)
        for each in cases {
            #expect(Destination(encoded: each["json"] as! String)?.encoded == each["reads"] as? String, "\(each["case"]!)")
        }
    }

    @Test func everyNotificationSpellingStillReads() {
        let cases = Fixture.cases("payloads")
        #expect(cases.count >= 15)
        for each in cases {
            let info = each["userInfo"] as! [String: Any]
            let parsed = Destination(userInfo: info, thread: each["thread"] as? String ?? "")
            #expect(parsed?.encoded == each["destination"] as? String, "\(each["case"]!)")
        }
    }

    @Test func everyLinkStillReads() {
        let cases = Fixture.cases("urls")
        #expect(cases.count >= 8)
        for each in cases {
            let url = URL(string: each["url"] as! String)!
            #expect(Destination(url: url)?.encoded == each["destination"] as? String, "\(each["case"]!)")
        }
    }

    @Test func theResolverAgreesWithEveryCase() throws {
        let cases = Fixture.cases("resolve")
        #expect(cases.count >= 50)
        for each in cases {
            let name = each["case"] as! String
            let destination = try #require(Destination(encoded: each["destination"] as! String), "\(name)")
            let worldData = try JSONSerialization.data(withJSONObject: each["world"]!)
            let world = try JSONDecoder().decode(DestinationResolver.World.self, from: worldData)
            let arrival = try #require(DestinationResolver.Arrival(rawValue: each["arrival"] as! String))
            let got = DestinationResolver.resolve(
                destination, arrival: arrival, in: world,
                elapsed: (each["elapsed"] as! NSNumber).doubleValue,
                deadline: (each["deadline"] as! NSNumber).doubleValue,
                interrupted: each["interrupted"] as? Bool ?? false)
            #expect(got == expected(each["expect"] as! [String: Any]), "\(name)")
        }
    }

    private func expected(_ object: [String: Any]) -> DestinationResolver.Resolution {
        if object["wait"] as? Bool == true { return .wait }
        if let host = object["connect"] as? String { return .connect(host: host) }
        if let open = object["open"] as? String {
            return .open(Destination(encoded: open)!, fellBack: object["fellBack"] as? Bool ?? false)
        }
        return .stay((object["stay"] as? String).flatMap(DestinationResolver.Note.init(rawValue:)))
    }
}

struct DestinationTests {
    // MARK: - The typed values write the fixture's bytes

    @Test func typedValuesWriteTheFixturesBytes() {
        let pairs: [(String, Destination)] = [
            ("needs you", .needsYou),
            ("a workspace on this Mac", Destination(runner: .init(host: ""), place: .workspace("ws-a"))),
            (
                "a task, restored, with its tab, pane and agent",
                Destination(
                    runner: .init(host: "h1"), place: .task(workspace: "ws-a", task: .init(id: "T1")), tab: .changes,
                    pane: "t-1", agent: "t-2")
            ),
            (
                "a task from a notice, by key and repository, with its question",
                Destination(
                    runner: .init(id: "r1"), place: .task(workspace: nil, task: .init(key: "bil-7", repository: "repo-1")),
                    question: true)
            ),
            ("a loose worktree", Destination(runner: .init(host: "h1"), place: .worktree("wt-loose", workspace: nil))),
            ("a slash and an accent are written as they are", Destination(runner: .init(host: "ssh://box"), place: .workspace("team/é"))),
        ]
        for (name, value) in pairs {
            #expect(value.encoded == Fixture.encoding(name), "\(name)")
        }
    }

    @Test func codableIsTheEncodingAsAString() throws {
        let value = Destination(runner: .init(host: "h1"), place: .orchestrator(workspace: "ws-a"), pane: "t-orch")
        let data = try JSONEncoder().encode([value])
        #expect(try JSONDecoder().decode([String].self, from: data) == [value.encoded])
        #expect(try JSONDecoder().decode([Destination].self, from: data) == [value])
        #expect(throws: DecodingError.self) { try JSONDecoder().decode([Destination].self, from: Data(#"["{\"v\":2}"]"#.utf8)) }
    }

    // MARK: - The ladder

    @Test func ancestorsStopAtTheWorkspace() {
        #expect(Destination.Place.task(workspace: "w", task: .init(id: "T")).ancestors == [.workspace("w")])
        #expect(Destination.Place.task(workspace: nil, task: .init(key: "k")).ancestors == [])
        #expect(Destination.Place.worktree("wt", workspace: "w").ancestors == [.workspace("w")])
        #expect(Destination.Place.orchestrator(workspace: "w").ancestors == [.workspace("w")])
        #expect(Destination.Place.terminal("t").ancestors == [])
        #expect(Destination.Place.workspace("w").ancestors == [])
    }

    // MARK: - Notifications and links

    @Test func aLocalPostReadsBackAndStillReadsAsATaskNotice() {
        let value = Destination(
            runner: .init(host: "", id: "r1"), place: .task(workspace: nil, task: .init(key: "bil-7", repository: "repo-1")),
            question: true)
        #expect(Destination(userInfo: value.userInfo, thread: "t:r1:bil-7") == value)
        let notice = TaskNotice(userInfo: value.userInfo)
        #expect(notice?.key == "bil-7")
        #expect(notice?.runner == "r1")
        // An older reader of the legacy keys still finds the pane.
        let pane = Destination(runner: .init(id: "r1"), place: .terminal("t-1"))
        #expect(pane.userInfo["terminal"] as? String == "t-1")
        #expect(PushTap(userInfo: pane.userInfo, thread: "t-1") == .terminal("t-1"))
    }

    @Test func linksRoundTrip() throws {
        let bare = Destination(runner: .init(id: "r1"), place: .terminal("t-1"))
        let bareURL = try #require(bare.url(scheme: "farcooler-canary"))
        // A bare pane keeps the form every build already opens.
        #expect(bareURL.absoluteString == "farcooler-canary://terminal/t-1?runner=r1")
        #expect(Destination(url: bareURL) == bare)
        let full = Destination(
            runner: .init(id: "r1"), place: .task(workspace: "ws-a", task: .init(id: "T1", key: "a&b=c+d")), tab: .agent)
        let fullURL = try #require(full.url(scheme: "farcooler"))
        #expect(fullURL.host() == "open")
        #expect(Destination(url: fullURL) == full)
    }

    @Test func theNoticeIdParsesAsTaskNoticeOpenDid() {
        #expect(Destination.parse(noticeId: "t:r1:bil-7")! == ("r1", "bil-7"))
        #expect(Destination.parse(noticeId: "t:r1:a:b")! == ("r1", "a:b"))
        #expect(Destination.parse(noticeId: "t:0123456789abcdef") == nil)
        #expect(Destination.parse(noticeId: "a:r1:bil-7") == nil)
        #expect(Destination.parse(noticeId: "t::bil-7") == nil)
    }

    // MARK: - The phone's saved stack

    @Test func thePhonesStackMigratesToItsDeepestScreen() {
        let place = PhoneWorkspace(runner: "host-1", workspace: "ws-a")
        #expect(Destination(phoneStack: []) == nil)
        #expect(
            Destination(phoneStack: [.workspace(place), .task(place, task: "T1")])
                == Destination(runner: .init(host: "host-1"), place: .task(workspace: "ws-a", task: .init(id: "T1"))))
        #expect(
            Destination(phoneStack: [.workspace(place), .history(place, status: "done")])
                == Destination(runner: .init(host: "host-1"), place: .history(workspace: "ws-a", status: "done")))
        #expect(
            Destination(phoneStack: [
                .workspace(place), .task(place, task: "T1"),
                .worktree(runner: "host-1", worktree: "wt-1", landing: .terminal("t-1")),
            ]) == Destination(runner: .init(host: "host-1"), place: .worktree("wt-1", workspace: "ws-a"), pane: "t-1"))
        // A worktree on another runner than the screens under it takes no workspace from them.
        #expect(
            Destination(phoneStack: [.workspace(place), .worktree(runner: "host-2", worktree: "wt-9", landing: .resume)])
                == Destination(runner: .init(host: "host-2"), place: .worktree("wt-9", workspace: nil)))
    }

    // MARK: - The deadlines are today's

    @Test func theDeadlinesAreTheOnesEachPlatformAlreadyKeeps() {
        #expect(DestinationResolver.Deadline.restore == PhoneLaunch.decideWithin)
        #expect(DestinationResolver.Deadline.notificationPhone == PhoneDecisionLink.followWithin)
        #expect(DestinationResolver.Deadline.notificationMac == 30)
    }
}
