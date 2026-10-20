import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): a member's Claude login line in Needs You
/// (ov-587), as the member sees it and as the owner does, in light and dark.
/// The production row, drawn into a bitmap; sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct NeedsYouLoginCaptures {
    private static func line(_ question: String, actions: [NeedsYouAction]) -> NeedsYouItem {
        NeedsYouItem(
            id: "claude-login:alice", kind: .claudeLogin, rank: 100_000_000, since: Date().addingTimeInterval(-300),
            question: question, actions: actions, runner: "studio")
    }

    @Test(arguments: [("member", true), ("owner", false)])
    func capture(who: String, own: Bool) throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let item = own
            ? Self.line(
                "Your Claude login on Studio stopped working.",
                actions: [NeedsYouAction(id: "sign_in", title: "Sign In Again", destructive: false, primary: true)])
            : Self.line("Alice Chen’s Claude login stopped working", actions: [])
        for (variant, name) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
            let row = NeedsYouItemRow(
                item: item, canAct: true, onOpen: {}, onAnswerAsk: { _ in nil }, onDecide: { _ in true }
            )
            .padding(16)
            .frame(width: 560, height: 130, alignment: .topLeading)
            .background(WorkspaceStyle.canvas)
            let host = NSHostingView(rootView: row)
            host.appearance = NSAppearance(named: name)
            host.frame = CGRect(x: 0, y: 0, width: 560, height: 130)
            host.layoutSubtreeIfNeeded()
            let rep = try #require(host.lookBitmap(scale: 2))
            try #require(rep.representation(using: .png, properties: [:]))
                .write(to: URL(fileURLWithPath: out).appendingPathComponent("mac-needs-you-login-\(who)-\(variant).png"))
        }
    }
}
