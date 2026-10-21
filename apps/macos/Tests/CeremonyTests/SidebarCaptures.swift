import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the sidebar's two lists at a narrow and a
/// wide width in the four appearances (ov-503, phase 6), over the Plan
/// scenario's board: long real titles, a lane that combines two cards, one
/// that combines four, a train, and agents. The production `OneTreeNavigator`
/// over the outline `SidebarOutline` builds from that plan, in a real titled
/// window off every screen, taken with `screencapture -l`. Sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct SidebarCaptures {
    final class Drive: ObservableObject {
        @Published var selected: OneTreeTarget?
        @Published var fold = TreeFoldRequest()
    }

    static func outline() throws -> SidebarOutline {
        var raw = try #require(try JSONSerialization.jsonObject(with: PlanScenario.data(.busy)) as? [String: Any])
        // Card titles as the board writes them, long ones among them.
        let titles = [
            "ov-487": "Relay: the team directory, so each person's asks reach the orchestrator",
            "ov-492": "iOS: a draft is never overwritten by the daemon's echo while someone is typing in the composer",
            "ov-493": "iOS: the composer keeps its selection when the keyboard moves",
            "ov-503": "Plan: a Plan you can read at a glance",
        ]
        raw["cards"] = (raw["cards"] as? [[String: Any]] ?? []).map { card in
            var card = card
            if let key = card["key"] as? String, let title = titles[key] { card["title"] = title }
            return card
        }
        let plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: raw))
        func worktree(_ name: String) -> OneTreeWorktree {
            OneTreeWorktree(
                id: "wt-\(name)", name: name, path: "/repo/.claude/worktrees/\(name)", branch: name,
                terminals: [
                    OneTreeTerminal(id: "\(name)-agent", title: "claude", isAgent: true),
                    OneTreeTerminal(id: "\(name)-sh", title: "zsh", isAgent: false),
                ])
        }
        return SidebarOutline.build(
            OneTreeInput(
                tasks: plan.cards.compactMap(OneTreeTask.init(card:)), plan: plan,
                worktrees: ["team-dir", "ios-overwrite", "site-ship", "phones-plan"].map(worktree),
                mainCheckout: OneTreeWorktree(
                    id: "main", name: "overnight", isMainCheckout: true,
                    terminals: [OneTreeTerminal(id: "m1", title: "zsh", isAgent: false)]),
                needsYouCount: 2))
    }

    struct Hosted: View {
        @ObservedObject var drive: Drive
        let outline: SidebarOutline
        let key: String
        let frames: NativePolishTests.Frames

        var body: some View {
            NativePolishTests.Framed(frames: frames, content: navigator)
        }

        private var navigator: some View {
            OneTreeNavigator(
                sidebar: OneTreeSidebar(
                    tree: { _ in outline }, selected: drive.selected, hint: nil, key: key, onChoose: { _ in }, fold: drive.fold),
                filterText: "", keyed: true
            )
            .background(Color(nsColor: .windowBackgroundColor))
        }
    }

    static func shoot(_ name: String, select: OneTreeTarget, width: CGFloat, height: CGFloat) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let outline = try outline()
        for (variant, appearance) in RealWindowCaptures.variants {
            let drive = Drive()
            let frames = NativePolishTests.Frames()
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(
                rootView: Hosted(drive: drive, outline: outline, key: "capture|\(UUID().uuidString)", frames: frames)
                    .frame(width: width, height: height, alignment: .topLeading))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            func has(_ part: String) -> Bool { frames.views.keys.contains { $0.hasPrefix("tree-") && $0.contains(part) } }
            try await Capture.wait(expecting: "the sidebar drawn", until: { has("place:") })
            drive.fold.expand()
            // Open to its cards and its lanes' rows: a collapsed theme draws neither.
            try await Capture.wait(expecting: "the sidebar with every fold open", until: { has("/task:") && has("lane:") })
            drive.selected = select
            // The chosen row's relatives are lit quietly (`tree-related-`), and only once the choice has landed.
            try await Capture.keep(
                window, to: URL(fileURLWithPath: out).appendingPathComponent("\(name)-\(Int(width))-\(variant).png"),
                expecting: "the sidebar expanded, with the chosen row's relatives lit",
                reading: { frames.views.sorted { $0.key < $1.key }.map { "\($0.key) \($0.value)" }.joined(separator: "\n") },
                until: { frames.views.keys.contains { $0.hasPrefix("tree-related-") } })
            window.close()
        }
    }

    @Test(arguments: [240.0, 400.0])
    func card(_ width: CGFloat) async throws {
        // ov-492 is one of the two cards the ios-overwrite lane combines.
        try await Self.shoot("sidebar-card", select: .task(PlanScenario.id(492, 1)), width: width, height: 1500)
    }

    @Test(arguments: [240.0, 400.0])
    func lane(_ width: CGFloat) async throws {
        try await Self.shoot("sidebar-lane", select: .lane(PlanScenario.id(2)), width: width, height: 1500)
    }

    @Test
    func usualHeight() async throws {
        try await Self.shoot("sidebar-short", select: .lane(PlanScenario.id(2)), width: 320, height: 860)
    }
}
