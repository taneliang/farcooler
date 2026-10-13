import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The real title bar with one status button's plate held down, drawn into a
/// bitmap (ov-511): for the geometry test and the captures. No input is sent;
/// `TitleStatusForcing` stands in for the pointer.
@MainActor
enum TitleStatusButtonKit {
    typealias Harness = TitleBarHarness

    /// The status buttons' identifiers, left to right.
    static let ids = ["title-status-activity", "title-status-need-you", "title-status-running", "title-status-in-review"]

    /// A store with `running` tasks in progress and `review` in review.
    static func store(running: Int, review: Int) async -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let now = Int64(Date().timeIntervalSince1970 * 1000) - 60_000
        client.commandRunnerForTesting = { args in
            guard args.starts(with: ["task", "list"]) else { return (Data(), nil) }
            let rows =
                (0..<running).map { ("r\($0)", "ov-\($0 + 1)", "in_progress") }
                + (0..<review).map { ("v\($0)", "ov-\($0 + 1000)", "in_review") }
            let tasks = rows.map { id, key, status in
                #"{"id":"\#(id)","key":"\#(key)","title":"A task","status":"\#(status)","status_since":\#(now),"created_at":\#(now),"updated_at":\#(now)}"#
            }
            return (Data(#"{"tasks":[\#(tasks.joined(separator: ","))]}"#.utf8), nil)
        }
        let store = TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
        await store.readIfNeverRead()
        return store
    }

    struct Shot {
        let rep: NSBitmapImageRep
        /// The status area's frame, in the bitmap's points from the left.
        let status: CGRect
    }

    /// The title bar at `width` with `count` of each counter, in `appearance`,
    /// with `forced` held; closed again before it returns.
    static func shot(
        width: CGFloat, count: Int, appearance: NSAppearance.Name, forced: (id: String, plate: TitleStatusPlate)?
    ) async throws -> Shot {
        TitleStatusForcing.plate = forced
        defer { TitleStatusForcing.plate = nil }
        let store = await store(running: count, review: count)
        let words = Harness.Words()
        words.needYou = count
        let window = try await Harness.window(
            Harness.Root(words: words, board: store, content: Color.clear), width: width, height: 120, beforeShowing: {})
        window.appearance = NSAppearance(named: appearance)
        try await Harness.settle(window)
        defer { window.close() }
        let view = try #require(window.contentView?.superview)
        let rep = try #require(view.lookBitmap(scale: 2))
        let status = try #require(Harness.status(in: window)).frame
        return Shot(rep: rep, status: status)
    }
}
