import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Every Mac empty state with icon rows (ov-205, ov-266), drawn offscreen in
/// both appearances so they can be looked at. Not an assertion beyond "it
/// drew": a rendering, written where `FARCOOLER_GLANCE_OUT` says.
@MainActor
struct EmptyStateSpecimenTests {
    private struct Specimen {
        let name: String
        let view: AnyView
    }

    private static func specimens() -> [Specimen] {
        func placeholder(_ phase: FleetPlaceholder.Phase) -> AnyView {
            AnyView(
                FleetPlaceholder(
                    phase: phase, onOpenMain: {}, onNewWorkspace: {}, onAddRepository: {},
                    onNewWorktree: {}, onTryAgain: {}))
        }
        return [
            Specimen(name: "no-workspace-selected", view: placeholder(.chooseWorkspace)),
            Specimen(name: "no-repositories", view: placeholder(.noRepositories)),
            Specimen(name: "no-worktrees", view: placeholder(.noWorktrees)),
            Specimen(
                name: "nothing-needs-you",
                view: AnyView(
                    NeedsYouView(
                        items: [], olderRunners: [], canAct: { _ in false }, onOpen: { _ in },
                        onAnswerAsk: { _, _ in nil }, onDecide: { _, _ in false }))),
            Specimen(
                name: "no-orchestrator",
                view: AnyView(
                    ContentUnavailableView {
                        Label("No Orchestrator", systemImage: "person.crop.circle.badge.questionmark")
                    } description: {
                        EmptyStateRows(copy: ConversationColumn.emptyCopy)
                    })),
            Specimen(name: "board-blank", view: AnyView(BoardBlankState())),
        ]
    }

    @Test("Write the empty state sheets")
    func writeSheets() throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for specimen in Self.specimens() {
            for dark in [false, true] {
                let view = specimen.view
                    .frame(width: 520, height: 360)
                    .background(dark ? Color(white: 0.12) : Color.white)
                let host = NSHostingView(rootView: view)
                host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                host.frame = CGRect(origin: .zero, size: host.fittingSize)
                host.layoutSubtreeIfNeeded()
                let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                host.cacheDisplay(in: host.bounds, to: rep)
                let png = try #require(rep.representation(using: .png, properties: [:]))
                try png.write(
                    to: directory.appendingPathComponent("empty-\(specimen.name)-\(dark ? "dark" : "light").png"))
            }
        }
    }
}
