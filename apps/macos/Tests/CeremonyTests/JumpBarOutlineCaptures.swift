import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the jump bar over the sidebar's outline
/// (ov-512), for a card, a ticket and an agent pane, at a narrow and a wide
/// width in the four appearances. The production `DrillBreadcrumb` fed the
/// crumbs and menus `ContentView.treePath` and `JumpMenus.siblings` build over
/// the Plan scenario's outline (long real titles, as `SidebarCaptures`), in a
/// real titled window off every screen, taken with `screencapture -l`. Sends
/// no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct JumpBarOutlineCaptures {
    static func shoot(_ name: String, to target: OneTreeTarget, hint: String?, width: CGFloat) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let outline = try SidebarCaptures.outline()
        let place = ContentView.Selection.workspace(host: "h", workspace: "w", focus: nil)
        let top = WorkspaceNavigation.Crumb(
            title: "Far Cooler", target: .workspace(host: "h", workspace: "w", focus: nil))
        let path = try #require(
            ContentView.treePath(to: target, in: outline, hint: hint, top: top) { _ in place })
        for (variant, appearance) in RealWindowCaptures.variants {
            let frames = NativePolishTests.Frames()
            let bar = DrillBreadcrumb(
                crumbs: path.crumbs, onGo: { _ in }, onClose: nil,
                menus: JumpMenuSource(count: path.crumbs.count) { [] })
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: 56), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(
                rootView: NativePolishTests.Framed(
                    frames: frames,
                    content: bar.padding(.horizontal, 12).frame(width: width, height: 56, alignment: .leading)
                        .background(Color(nsColor: .windowBackgroundColor))))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            // A path of two or more crumbs has a separator between them, and a
            // probe only reports a view once it has been laid out.
            try await Capture.keep(
                window, to: URL(fileURLWithPath: out).appendingPathComponent("jumpbar-\(name)-\(Int(width))-\(variant).png"),
                expecting: "the jump bar's crumbs laid out",
                until: { frames.views.contains { $0.key.hasPrefix("jump-separator-") && !$0.key.hasSuffix("-baseline") && $0.value.width > 0 } })
            window.close()
        }
    }

    /// The first ticket with a pane under it, as the sidebar draws it.
    static func ticketWithPane() throws -> (ticket: OneTreeNode, pane: OneTreeNode) {
        let outline = try SidebarCaptures.outline()
        for group in outline.tickets {
            for ticket in group.children where ticket.kind == .ticket {
                if let pane = ticket.children.first(where: { $0.kind == .terminal && $0.target != nil }) { return (ticket, pane) }
            }
        }
        throw CaptureError.noPane
    }

    enum CaptureError: Error { case noPane }

    @Test(arguments: [420.0, 1000.0])
    func card(_ width: CGFloat) async throws {
        // ov-492: the long title.
        let id = PlanScenario.id(492, 1)
        try await Self.shoot("card", to: .task(id), hint: nil, width: width)
    }

    @Test(arguments: [420.0, 1000.0])
    func ticket(_ width: CGFloat) async throws {
        let (ticket, _) = try Self.ticketWithPane()
        try await Self.shoot("ticket", to: try #require(ticket.target), hint: ticket.id, width: width)
    }

    @Test(arguments: [420.0, 1000.0])
    func agentPane(_ width: CGFloat) async throws {
        let (_, pane) = try Self.ticketWithPane()
        try await Self.shoot("agent", to: try #require(pane.target), hint: pane.id, width: width)
    }
}
