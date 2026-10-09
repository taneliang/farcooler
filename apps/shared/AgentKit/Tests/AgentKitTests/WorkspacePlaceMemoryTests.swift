import Foundation
import Testing

@testable import AgentKit

/// Opening a workspace again goes back to where it was left (ov-442): the
/// screens pushed over it on the phone, and the iPad's canvas.
struct WorkspacePlaceMemoryTests {
    static let billing = PhoneWorkspace(runner: "r1", workspace: "billing")
    static let shop = PhoneWorkspace(runner: "r1", workspace: "shop")

    private static func defaults() -> UserDefaults { UserDefaults(suiteName: "place-\(UUID().uuidString)")! }

    @Test func aWorkspaceOpensWithTheTaskThatWasOpenInIt() {
        let defaults = Self.defaults()
        WorkspacePlaceMemory.keep(
            [.workspace(Self.billing), .task(Self.billing, task: "t1")], in: defaults)
        WorkspacePlaceMemory.keep([.workspace(Self.shop)], in: defaults)
        // Going back to Needs You is leaving, not clearing.
        WorkspacePlaceMemory.keep([], in: defaults)
        #expect(
            WorkspacePlaceMemory.opening(Self.billing, in: defaults)
                == [.workspace(Self.billing), .task(Self.billing, task: "t1")])
        #expect(WorkspacePlaceMemory.opening(Self.shop, in: defaults) == [.workspace(Self.shop)])
    }

    @Test func closingTheTaskIsRememberedToo() {
        let defaults = Self.defaults()
        WorkspacePlaceMemory.keep([.workspace(Self.billing), .task(Self.billing, task: "t1")], in: defaults)
        WorkspacePlaceMemory.keep([.workspace(Self.billing)], in: defaults)
        #expect(WorkspacePlaceMemory.opening(Self.billing, in: defaults) == [.workspace(Self.billing)])
    }

    @Test func aWorktreeCoverAndAnotherWorkspacesScreensAreNotKept() {
        let defaults = Self.defaults()
        WorkspacePlaceMemory.keep(
            [
                .workspace(Self.billing), .task(Self.billing, task: "t1"),
                .worktree(runner: "r1", worktree: "w", landing: .resume),
            ], in: defaults)
        #expect(
            WorkspacePlaceMemory.opening(Self.billing, in: defaults)
                == [.workspace(Self.billing), .task(Self.billing, task: "t1")])
        WorkspacePlaceMemory.keep([.workspace(Self.shop), .task(Self.billing, task: "t1")], in: defaults)
        #expect(WorkspacePlaceMemory.opening(Self.shop, in: defaults) == [.workspace(Self.shop)])
    }

    @Test func aTaskThatIsGoneEndsTheStackAtItsWorkspace() {
        let defaults = Self.defaults()
        WorkspacePlaceMemory.keep([.workspace(Self.billing), .task(Self.billing, task: "t1")], in: defaults)
        #expect(
            WorkspacePlaceMemory.opening(Self.billing, in: defaults) { route in route != .task(Self.billing, task: "t1") }
                == [.workspace(Self.billing)])
    }

    @Test func theCanvasIsKeptPerWorkspace() {
        let defaults = Self.defaults()
        #expect(WorkspacePlaceMemory.canvas(for: Self.billing, in: defaults) == .plan)
        WorkspacePlaceMemory.keep(.task("t1"), for: Self.billing, in: defaults)
        WorkspacePlaceMemory.keep(.page(.theme("th")), for: Self.shop, in: defaults)
        #expect(WorkspacePlaceMemory.canvas(for: Self.billing, in: defaults) == .task("t1"))
        #expect(WorkspacePlaceMemory.canvas(for: Self.shop, in: defaults) == .page(.theme("th")))
    }
}
