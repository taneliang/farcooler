import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): a card's record and Decided For You, light,
/// on the owner's Mac and on a member's (ov-707). A member's entries are
/// signed with their name, the owner's read "Owner" to a member, and a
/// member is offered no Keep or Keep All. Long real names, so a byline that
/// doesn't fit shows.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct MemberBylineCaptures {
    static let record = """
        {"task": {}, "notes": [
          {"id": "n1", "kind": "answer", "actor": "member:bob", "at": 1760000000000,
           "body": "pdfkit. It's already in the receipts export, so nothing new to vet."},
          {"id": "n2", "kind": "question", "actor": "manager", "at": 1759999000000,
           "body": "Which PDF library should invoices use?", "extra": {"options": ["pdfkit", "printpdf"]}},
          {"id": "n3", "kind": "comment", "actor": "member:priya", "at": 1759990000000,
           "body": "The totals row is off by a cent on multi-currency invoices."},
          {"id": "n4", "kind": "comment", "actor": "user", "at": 1759985000000,
           "body": "Hold the export until the rounding is fixed."},
          {"id": "n5", "kind": "status_change", "actor": "member:bob", "at": 1759980000000, "body": "moved from todo to in_progress"},
          {"id": "n6", "kind": "created", "actor": "member:bob", "at": 1759970000000, "body": "Export: invoices as PDF"}
        ]}
        """

    static let members = #"[{"handle":"bob","name":"Bartholomew Okonkwo-Lindqvist"},{"handle":"priya","name":"Priyanka Venkataraman-Holloway"}]"#

    /// The runner's people, with this Mac as `you`.
    static func people(you: String) throws -> RunnerPeople {
        try #require(RunnerPeople(json: Data(#"{"members":\#(members),"claude_account":"","you":"\#(you)"}"#.utf8)))
    }

    static func shoot<V: View>(_ name: String, width: CGFloat, height: CGFloat, _ view: V) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let window = NSWindow(
            contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(
            rootView: view.frame(width: width, height: height, alignment: .topLeading).background(WorkspaceStyle.paper))
        window.appearance = NSAppearance(named: .aqua)
        TestWindow.show(window)
        try await Capture.keep(
            to: URL(fileURLWithPath: out).appendingPathComponent("\(name)-light.png"), expecting: name, until: { true }
        ) { window.contentView?.lookBitmap(scale: 2)?.representation(using: .png, properties: [:]) }
        window.close()
    }

    @Test(arguments: [("owner", ""), ("member", "bob")])
    func cardRecord(viewer: String, you: String) async throws {
        let detail = try TaskDetailModel.decode(Data(Self.record.utf8))
        let row = TaskRow(
            id: "t1", key: "ov-707", title: "Export: invoices as PDF", status: .inProgress,
            statusSince: Date(timeIntervalSinceNow: -3 * 86_400), intent: "Customers download their invoices as PDF.",
            createdAt: Date(timeIntervalSinceNow: -5 * 86_400), updatedAt: Date(timeIntervalSinceNow: -3_600))
        try await Self.shoot(
            "member-record-\(viewer)", width: 640, height: 900,
            TaskCard(
                row: row, detail: detail, question: nil, canAnswer: true, people: try Self.people(you: you),
                onAnswer: { _ in true }
            ).padding(TaskTypography.inset))
    }

    @Test(arguments: [("owner", ""), ("member", "bob")])
    func rulingRows(viewer: String, you: String) async throws {
        // A second open ruling, so the owner is offered Keep All.
        let calls = PlanViewTests.Calls()
        var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        var list = try #require(object["rulings"] as? [[String: Any]])
        var more = list[0]
        more["id"] = "00000000-0000-0000-0000-000000005003"
        more["number"] = 3
        more["short"] = "R-3"
        more["decision"] = "Invoices export through pdfkit, the library the receipts export already uses."
        list.insert(more, at: 0)
        object["rulings"] = list
        calls.plan = try JSONSerialization.data(withJSONObject: object)
        let target = "capture-\(viewer)-\(UUID().uuidString)"
        defer { UserDefaults.standard.removeObject(forKey: "talk.you.\(target)") }
        let store = try await PlanViewTests.store(
            plan: true, defaults: PlanViewTests.defaults(), calls: calls, target: target, you: you)
        await store.client.talk.load(store.client, force: true)
        await store.plan.reload()
        let actions = PlanRulingActions(canKeep: store.plan.canMarkRulings, canAsk: true, alwaysShown: true)
        try await Self.shoot(
            "member-rulings-\(viewer)", width: 560, height: 520,
            PlanRulingsSection(plan: store.plan, defaults: PlanViewTests.defaults(), copy: { _ in })
                .environment(\.planRulingActions, actions)
                .padding(TaskTypography.inset))
    }
}
