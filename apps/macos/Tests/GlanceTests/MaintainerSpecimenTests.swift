import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Roles on the Mac (ov-784), drawn offscreen in both appearances at the
/// narrowest and the usual width, with a long workspace and runner name:
/// Show Charter for someone who may edit it, reading and editing, a refused
/// save, and the screen a removed member's Mac shows. Written where
/// `FARCOOLER_GLANCE_OUT` says.
@MainActor
struct MaintainerSpecimenTests {
    private static let workspace = WorkspaceSummary(
        id: "w", name: "Customer Onboarding and Billing Experiments", taskPrefix: "cob", isMain: false, ordinal: 1,
        repository: "r", lead: "alexandra-montgomery")
    private static let charter = (1...12).map {
        "## Rule \($0)\nThe lead decides what always goes to the owner; everything else is the orchestrator’s call.\n"
    }.joined(separator: "\n")

    private static func write(_ view: some View, _ name: String, width: CGFloat, height: CGFloat, dark: Bool) async throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let host = NSHostingView(
            rootView: view.frame(width: width, height: height)
                .background(dark ? Color(white: 0.16) : Color(white: 0.96))
                .environment(\.colorScheme, dark ? .dark : .light))
        host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        host.frame = CGRect(origin: .zero, size: CGSize(width: width, height: height))
        host.layoutSubtreeIfNeeded()
        try? await Task.sleep(for: .milliseconds(300))
        host.layoutSubtreeIfNeeded()
        let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: rep)
        let png = try #require(rep.representation(using: .png, properties: [:]))
        try png.write(to: directory.appendingPathComponent("\(name)-\(Int(width))-\(dark ? "dark" : "light").png"))
    }

    @Test("Write the editable charter sheets")
    func charterSheets() async throws {
        let save: (String, String) async -> CharterReading = { _, _ in .failed("") }
        for (width, height) in [(320.0, 360.0), (560.0, 480.0)] {
            for dark in [false, true] {
                try await Self.write(
                    CharterSheet(workspace: Self.workspace, load: { .text(Self.charter, sha256: "ab") }, save: save),
                    "charter-editable", width: width, height: height, dark: dark)
                try await Self.write(
                    CharterSheet(
                        workspace: Self.workspace, load: { .text(Self.charter, sha256: "ab") }, save: save,
                        editing: Self.charter + "\n## Who decides\nAlexandra Montgomery-Fitzgerald, the lead.\n"),
                    "charter-editing", width: width, height: height, dark: dark)
                try await Self.write(
                    CharterSheet(workspace: Self.workspace, load: { .missing }, save: save),
                    "charter-none-editable", width: width, height: 260, dark: dark)
            }
        }
    }

    @Test("Write the removed screen")
    func removedScreen() async throws {
        for width in [480.0, 720.0] {
            for dark in [false, true] {
                try await Self.write(
                    FleetPlaceholder(
                        phase: .removed("platform-builds-eu-west-staging.example-corporation.internal"),
                        onShowNeedsYou: {},
                        welcome: WelcomeSetup(
                            model: WelcomeModel(home: "/nowhere", scan: { [] }), step: .ready([.claude]),
                            agents: HarnessAvailability(agentsFound: ["claude"]), register: { _ in nil },
                            onCheckAgain: {}, onAnotherRunner: nil),
                        onNewWorktree: {}, onTryAgain: {}),
                    "removed", width: width, height: 420, dark: dark)
            }
        }
    }

    /// A runner that refuses this Mac's key after letting it in (ov-818).
    @Test("Write the no-access screen")
    func noAccessScreen() async throws {
        for width in [480.0, 720.0] {
            for dark in [false, true] {
                try await Self.write(
                    FleetPlaceholder(
                        phase: .noAccess("platform-builds-eu-west-staging.example-corporation.internal"),
                        onShowNeedsYou: {},
                        welcome: WelcomeSetup(
                            model: WelcomeModel(home: "/nowhere", scan: { [] }), step: .ready([.claude]),
                            agents: HarnessAvailability(agentsFound: ["claude"]), register: { _ in nil },
                            onCheckAgain: {}, onAnotherRunner: nil),
                        onNewWorktree: {}, onTryAgain: {}),
                    "no-access", width: width, height: 460, dark: dark)
            }
        }
    }
}
