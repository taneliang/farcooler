import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Decided For You on the Mac (ov-304): the canvas's section, read from the
/// CLI's real `plan --json` shape (`test/fixtures/plan.json`) through the
/// store's own read. Standing first, then settled; Copy Reference is the only
/// action; a runner without `board_rulings` shows nothing.
@MainActor
@Suite(.serialized)
struct PlanRulingsViewTests {
    struct Hosted: View {
        @ObservedObject var plan: PlanStore
        let seen: NavigatorFilterTests.Seen
        let copy: @MainActor (String) -> Void

        var body: some View {
            PlanRulingsSection(plan: plan, defaults: PlanViewTests.defaults(), copy: copy)
            .frame(width: 520, height: 600, alignment: .topLeading)
            .environment(\.gridProbing, true)
            .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                GeometryReader { proxy in
                    let _ = seen.views = Dictionary(
                        probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                    Color.clear
                }
            }
        }
    }

    final class Drawn {
        let seen = NavigatorFilterTests.Seen()
        var copied: [String] = []
        var host: NSHostingView<Hosted>!
        var window: NavigatorFilterTests.KeyWindow!

        @MainActor func settle() async {
            for _ in 0..<15 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
    }

    /// The section as the canvas draws it, from a store that read the
    /// fixture through its stubbed CLI, on a runner that keeps rulings or not.
    static func draw(rulings: Bool) async throws -> Drawn {
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults())
        if !rulings {
            store.client.daemonBuild = DaemonBuild(
                version: "test", matches: true, platform: "macos",
                capabilities: Set(Capability.allCases.map(\.rawValue).filter { $0 != "board_rulings" }))
        }
        await store.plan.reload()
        let drawn = Drawn()
        drawn.host = NSHostingView(rootView: Hosted(plan: store.plan, seen: drawn.seen) { drawn.copied.append($0) })
        drawn.window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 520, height: 600), styleMask: [.borderless],
            backing: .buffered, defer: false)
        drawn.window.isReleasedWhenClosed = false
        drawn.window.contentView = drawn.host
        drawn.window.makeKeyAndOrderFront(nil)
        await drawn.settle()
        return drawn
    }

    @Test("Standing rulings come first, each with its id and copy control; settled ones follow")
    func standingFirst() async throws {
        let drawn = try await Self.draw(rulings: true)
        defer { drawn.window.close() }
        let views = drawn.seen.views
        let standing = try #require(views["plan-ruling-R-2"], "\(views.keys.sorted())")
        let settled = try #require(views["plan-ruling-R-1"])
        #expect(views["plan-rulings"] != nil)
        #expect(standing.minY < settled.minY, "standing above settled")
        #expect(views["plan-ruling-R-2-copy"] != nil, "a standing ruling offers Copy Reference")
        #expect(standing.height > settled.height, "the standing one shows why and what reversing costs")
    }

    @Test("A runner without board_rulings shows no Decided For You, whatever the plan carries")
    func oldRunnerShowsNothing() async throws {
        let drawn = try await Self.draw(rulings: false)
        defer { drawn.window.close() }
        #expect(!drawn.seen.views.keys.contains { $0.hasPrefix("plan-ruling") }, "\(drawn.seen.views.keys.sorted())")
    }

    @Test("Copy Reference copies what the owner tells the orchestrator, and edits nothing")
    func copyReference() async throws {
        let drawn = try await Self.draw(rulings: true)
        defer { drawn.window.close() }
        let frame = try #require(drawn.seen.views["plan-ruling-R-2-copy"])
        let at = NSPoint(x: frame.midX, y: 600 - frame.midY)
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            drawn.window.sendEvent(
                NSEvent.mouseEvent(
                    with: type, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: drawn.window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!)
        }
        await drawn.settle()
        #expect(drawn.copied == ["ruling R-2: The inbox is amber."])
    }
}
