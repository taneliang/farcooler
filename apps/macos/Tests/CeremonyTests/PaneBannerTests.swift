import AgentKit
import Foundation
import Testing
import UserNotifications

@testable import Far_Cooler

/// One banner per pane, and a closed pane's banner goes with it (ov-163).
///
/// A banner was posted under `<terminal id>-<activity>`, so a pane that blocked
/// and then finished showed two banners while a comment said the later one
/// replaced the earlier, and the removal asked for `<terminal id>`, which was
/// never posted. The center replaces by identifier, so both are about what the
/// identifier is.
@MainActor
@Suite(.serialized)
struct PaneBannerTests {
    private static func terminal(_ id: String, activity: String = "blocked") throws -> Terminal {
        try JSONDecoder().decode(
            Terminal.self,
            from: Data(
                """
                {"id":"\(id)","short":"s","title":"claude","preset":"claude","state":"running",
                "activity":"\(activity)","epoch":0}
                """.utf8))
    }

    private static func ownBanner(_ terminal: Terminal) -> UNNotificationRequest {
        Notifier.ownBanner(
            terminal: terminal, words: (title: "claude needs you", body: "lane"),
            activity: terminal.agent, host: "", runnerId: nil)
    }

    @Test func blockedThenDoneIsOneIdentifierSoTheSecondReplacesTheFirst() throws {
        let blocked = Self.ownBanner(try Self.terminal("t-1", activity: "blocked"))
        let done = Self.ownBanner(try Self.terminal("t-1", activity: "done"))
        #expect(blocked.identifier == done.identifier, "two identifiers stack two banners")
        #expect(blocked.identifier != Self.ownBanner(try Self.terminal("t-2")).identifier)
        let failed = Notifier.failedExit(terminal: try Self.terminal("t-1"), place: "lane", host: "", runnerId: nil)
        #expect(failed.identifier == done.identifier, "a failed command replaces the pane's banner too")
    }

    @Test func forgettingAPaneRemovesTheBannerItPosted() throws {
        let posted = Self.ownBanner(try Self.terminal("t-1")).identifier
        var removed: [String] = []
        let original = Notifier.shared.removeDelivered
        Notifier.shared.removeDelivered = { removed += $0 }
        defer { Notifier.shared.removeDelivered = original }
        Notifier.shared.forget("t-1")
        #expect(removed.contains(posted), "forget asked for \(removed), but the banner is \(posted)")
    }

    @Test func aPaneThatLeftTheFleetHasItsBannersTakenDown() async throws {
        func fleet(_ ids: [String]) -> Data {
            let terminals = ids.map {
                #"{"id":"\#($0)","short":"\#($0)","title":"claude","preset":"claude","state":"running","epoch":0}"#
            }.joined(separator: ",")
            return Data(
                """
                {"runtime_healthy":true,"live_panes":\(ids.count),"worktrees":[{"id":"w-1","short":"w",
                "task":"lane","branch":"b","worktree":"/tmp/w","state":"active","terminals":[\(terminals)]}]}
                """.utf8)
        }
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        var ids = ["t-1", "t-2"]
        client.commandRunnerForTesting = { args in
            if args.prefix(2) == ["worktree", "list"] { return (fleet(ids), nil) }
            return (nil, "not in this test")
        }
        await client.refresh()

        var removed: [String] = []
        let original = Notifier.shared.removeDelivered
        Notifier.shared.removeDelivered = { removed += $0 }
        defer { Notifier.shared.removeDelivered = original }
        ids = ["t-1"]
        await client.refresh()

        #expect(removed.contains(PaneBanner.identifier(forPane: "t-2")), "\(removed)")
        #expect(!removed.contains("t-1"), "a pane still there keeps its banner")
    }

    /// With "Remove terminals when they exit" on, the pane is reaped within a
    /// second of a failed command, and its banner is all that is left of the
    /// failure. Reaping must not take it down.
    @Test func aFailedCommandsBannerOutlivesTheReapOfItsPane() async throws {
        let failed = try JSONDecoder().decode(
            Terminal.self,
            from: Data(
                """
                {"id":"t-fail","short":"s","title":"cargo","preset":"cargo","state":"exited",
                "exitCode":101,"epoch":0}
                """.utf8))
        #expect(failed.status == .failedRun)
        var removed: [String] = []
        let original = Notifier.shared.removeDelivered
        Notifier.shared.removeDelivered = { removed += $0 }
        defer { Notifier.shared.removeDelivered = original }

        Preferences.shared.notifyOnAttention = true
        let worktree = try JSONDecoder().decode(
            Worktree.self,
            from: Data(#"{"id":"w","short":"w","task":"t","branch":"b","worktree":"/tmp/w","state":"active","terminals":[]}"#.utf8))
        Notifier.shared.report(terminal: failed, place: "lane", in: worktree, runner: nil)
        Notifier.shared.forget("t-fail")  // what `reapIfExited` and the closed-pane loop call
        #expect(removed.isEmpty, "the failure banner was taken down: \(removed)")

        // A pane that never failed is still cleaned up.
        Notifier.shared.forget("t-other")
        #expect(removed.contains("t-other"))
    }
}
