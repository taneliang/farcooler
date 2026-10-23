import AgentKit
import Foundation
import Testing
import UserNotifications

@testable import Far_Cooler

/// A member's Mac banners only what is theirs (ov-717): the runner's audience
/// rule (`notice_audience.rs`) applied to the pane banners the runner can't
/// filter, and the own-write guard (ov-712) tested where it is wired.
@MainActor
@Suite(.serialized)
struct MemberBannerTests {
    private static func pane(_ id: String, login: String?) throws -> Terminal {
        let owned = login.map { #","claudeLogin":{"handle":"\#($0)","name":"","owner":\#($0.isEmpty)}"# } ?? ""
        return try JSONDecoder().decode(
            Terminal.self,
            from: Data(
                """
                {"id":"\(id)","short":"s","title":"claude","preset":"claude","state":"running",
                "activity":"blocked","epoch":0\(owned)}
                """.utf8))
    }

    private static let worktree = try! JSONDecoder().decode(
        Worktree.self,
        from: Data(#"{"id":"w","short":"w","task":"t","branch":"b","worktree":"/tmp/w","state":"active","terminals":[]}"#.utf8))

    @Test("The rule: the owner hears every pane, a member their own, a released device none")
    func theRule() throws {
        let alices = try Self.pane("a", login: "alice").claudeLogin
        let owners = try Self.pane("o", login: "").claudeLogin
        #expect(alices?.handle == "alice" && owners?.owner == true, "the fixture reads as a login")
        #expect(Notifier.hearsPane(login: alices, viewer: ""))
        #expect(Notifier.hearsPane(login: owners, viewer: ""))
        #expect(Notifier.hearsPane(login: nil, viewer: ""))
        #expect(Notifier.hearsPane(login: owners, viewer: nil), "not yet told who this Mac is: shown")
        #expect(Notifier.hearsPane(login: alices, viewer: "alice"))
        #expect(!Notifier.hearsPane(login: alices, viewer: "bob"))
        #expect(!Notifier.hearsPane(login: owners, viewer: "bob"), "the owner's agents aren't a member's")
        #expect(!Notifier.hearsPane(login: nil, viewer: "bob"), "a pane the runner didn't name isn't provably theirs")
        #expect(!Notifier.hearsPane(login: alices, viewer: RunnerPeople.released))
    }

    /// Through `report`, with a stand-in for the notification center: Alice's
    /// Mac banners her own blocked agent and not Bob's or the owner's; the
    /// owner's Mac banners all three.
    @Test("report banners a member's own panes alone, and the owner's Mac all of them")
    func reportFollowsTheRule() throws {
        let notifier = Notifier.shared
        let (oldDeliver, oldStanding) = (notifier.deliver, notifier.standingIn)
        var posted: [String] = []
        notifier.deliver = { posted.append($0.identifier) }
        notifier.standingIn = true
        let wasOn = Preferences.shared.notifyOnAttention
        Preferences.shared.notifyOnAttention = true
        defer {
            notifier.deliver = oldDeliver
            notifier.standingIn = oldStanding
            Preferences.shared.notifyOnAttention = wasOn
        }
        func banners(_ viewer: String, round: String) throws -> [String] {
            posted = []
            for (name, login) in [("alice", "alice"), ("bob", "bob"), ("owner", "")] {
                let pane = try Self.pane("\(round)-\(name)", login: login)
                notifier.report(terminal: pane, place: "lane", in: Self.worktree, runner: nil, viewer: viewer)
            }
            return posted.map { $0.replacingOccurrences(of: "\(round)-", with: "") }.sorted()
        }
        let alicesMac = try banners("alice", round: "r1")
        let ownersMac = try banners("", round: "r2")
        #expect(alicesMac.count == 1 && alicesMac[0].contains("alice"), "\(alicesMac)")
        #expect(ownersMac.count == 3, "\(ownersMac)")
    }

    /// `post` skips what the Mac's person wrote (`isOwnWrite`) and posts the
    /// rest, with the viewer the runner told this Mac it is.
    @Test("post leaves a member's own write unposted, and posts another's")
    func postFollowsTheViewer() async throws {
        let notifier = Notifier.shared
        let (oldDeliver, oldStanding) = (notifier.deliver, notifier.standingIn)
        var posted: [String] = []
        notifier.deliver = { posted.append($0.content.body) }
        notifier.standingIn = true
        let wasOn = Preferences.shared.notifyOnAttention
        Preferences.shared.notifyOnAttention = true
        let key = "talk.you.alices-runner"
        UserDefaults.standard.set("alice", forKey: key)
        defer {
            notifier.deliver = oldDeliver
            notifier.standingIn = oldStanding
            Preferences.shared.notifyOnAttention = wasOn
            UserDefaults.standard.removeObject(forKey: key)
        }
        let client = DaemonClient(target: "alices-runner")
        #expect(client.talk.viewer == "alice")
        func notice(_ body: String, author: String) -> NoticeEvent {
            NoticeEvent(
                noticeId: "t:r-1:ov-90", event: "review", level: "active", title: "ov-90", body: body,
                task: "ov-90", runner: "r-1", options: [], author: author)
        }
        notifier.post(notice: notice("her own", author: "alice"), from: client)
        notifier.post(notice: notice("Bob's", author: "bob"), from: client)
        notifier.post(notice: notice("nobody's", author: ""), from: client)
        await Task {}.value
        #expect(posted == ["Bob's", "nobody's"], "\(posted)")
    }

    @Test("A member's Mac posts the task notices the relay doesn't push it, and leaves the rest to the push")
    func aMembersRegisteredMacStillHearsTheRest() {
        func posts(_ viewer: String?, _ event: String, paired: Bool = true, registered: Bool = true) -> Bool {
            Notifier.postsLocally(pushPaired: paired, registered: registered, viewer: viewer, event: event)
        }
        for event in ["done", "new"] {
            #expect(posts("alice", event), "the relay never pushes a member \(event)")
            #expect(!posts("", event), "the owner's push brings it")
            #expect(!posts(nil, event))
        }
        for event in ["decision", "blocked", "review"] {
            #expect(!posts("alice", event), "her push brings \(event)")
            #expect(posts("alice", event, registered: false), "unregistered, it's posted here")
            #expect(posts("alice", event, paired: false))
        }
    }
}
