import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The old Fleet sidebar is gone (ov-178), and what only it offered has a
/// home elsewhere: the fleet's first-run and failure states in the detail,
/// a workspace's Show Charter and Wake the Agent When You Answer in the
/// orchestrator's header.
@MainActor
struct FleetSidebarRetiredTests {
    // MARK: - The detail says what the sidebar said before the fleet had anything

    /// Each was the sidebar's, and nothing else drew it: a first launch
    /// whose daemon wouldn't answer read "No Workspace Selected".
    @Test("The detail says loading, couldn't read, no repositories, no worktrees, then choose")
    func thePlaceholderFollowsTheFleet() {
        typealias P = FleetPlaceholder
        #expect(P.phase(hasWorktrees: false, localLoaded: false, localError: nil, hasRepositories: false) == .loading)
        #expect(
            P.phase(hasWorktrees: false, localLoaded: false, localError: "error: no daemon", hasRepositories: false)
                == .failed("error: no daemon"))
        #expect(P.phase(hasWorktrees: false, localLoaded: true, localError: nil, hasRepositories: false) == .noRepositories)
        #expect(P.phase(hasWorktrees: false, localLoaded: true, localError: nil, hasRepositories: true) == .noWorktrees)
        // A read that came back wins over an older failure.
        #expect(P.phase(hasWorktrees: false, localLoaded: true, localError: "stale", hasRepositories: true) == .noWorktrees)
        // Any runner's worktree is past all of it, this Mac's trouble or not.
        #expect(P.phase(hasWorktrees: true, localLoaded: false, localError: "down", hasRepositories: true) == .chooseWorkspace)
    }

    // MARK: - The orchestrator's header holds the workspace's own items

    /// Show Charter and Wake the Agent When You Answer were on the old
    /// sidebar's workspace row, and Wake was nowhere else. With no
    /// orchestrator, the header's menu was not drawn at all.
    @Test("With no orchestrator, the header still offers Show Charter and Wake the Agent When You Answer")
    func theHeaderOffersTheWorkspacesItemsWithoutASeat() {
        let charter = CharterAccess.unavailable("No charter yet.")
        #expect(
            ConversationHeader.menu(hasSeat: false, charter: charter, wakeOnAnswer: false)
                == [.showCharter, .wakeOnAnswer])
        #expect(
            ConversationHeader.menu(hasSeat: true, charter: charter, wakeOnAnswer: true)
                == [.replaceOrchestrator, .showCharter, .wakeOnAnswer])
        // A runner that can't wake anyone gets no switch, and starting one
        // is the column's placeholder's, never the header's.
        #expect(ConversationHeader.menu(hasSeat: false, charter: charter, wakeOnAnswer: nil) == [.showCharter])
        #expect(ConversationHeader.menu(hasSeat: false, charter: nil, wakeOnAnswer: nil).isEmpty)
    }

    // MARK: - One sidebar, the navigator

    /// A fresh suite, and its name to remove it by.
    private static func defaults() throws -> UserDefaults {
        let name = "ov178-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: name))
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    /// The old sidebar's "hidden" was everyone's who never opened it, and
    /// "shown" was a choice about a list that's gone: neither says
    /// anything about the navigator, which starts out either way.
    @Test("A stored choice for the old sidebar falls back to the navigator shown, and is removed")
    func anOldSidebarChoiceFallsBack() throws {
        for old in ["hidden", "shown"] {
            let defaults = try Self.defaults()
            defaults.set(old, forKey: "window.sidebar")
            defaults.set("\u{1}ws", forKey: "sidebar.openWorktrees")
            defaults.set("\u{1}overnight", forKey: "sidebar.collapsedProjects")
            defaults.set(
                ["0.000000, 0.000000, 320.000000, 1130.000000, YES, NO"],
                forKey: "NSSplitView Subview Frames X-1-AppWindow-1, SidebarNavigationSplitView")
            #expect(!NavigatorVisibility.hiddenAtLaunch(in: defaults), "the old sidebar \(old) hid the navigator")
            for key in NavigatorVisibility.retiredKeys + ["NSSplitView Subview Frames X-1-AppWindow-1, SidebarNavigationSplitView"] {
                #expect(defaults.object(forKey: key) == nil, "\(key) is still there")
            }
        }
    }

    /// The navigator's own choice is kept, beside the old one's removal.
    @Test("The navigator opens as it was left")
    func theNavigatorOpensAsItWasLeft() throws {
        let defaults = try Self.defaults()
        #expect(!NavigatorVisibility.hiddenAtLaunch(in: defaults))
        defaults.set(NavigatorVisibility.stored(true), forKey: NavigatorVisibility.key)
        defaults.set("shown", forKey: "window.sidebar")
        #expect(NavigatorVisibility.hiddenAtLaunch(in: defaults))
        defaults.set(NavigatorVisibility.stored(false), forKey: NavigatorVisibility.key)
        #expect(!NavigatorVisibility.hiddenAtLaunch(in: defaults))
        #expect(NavigatorVisibility.key != "window.sidebar")
    }

    // MARK: - Nothing of the old sidebar is left to reach

    /// Every source file of the app, by name.
    private static func sources() throws -> [(name: String, text: String)] {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/FarCooler")
        let files = FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil)?
            .compactMap { $0 as? URL }.filter { $0.pathExtension == "swift" } ?? []
        return try files.map { (name: $0.lastPathComponent, text: try String(contentsOf: $0, encoding: .utf8)) }
    }

    /// Deleted, not hidden (ov-178's third acceptance line): no view, row
    /// model, drag or remembered state of the old sidebar is declared
    /// anywhere, and the window has no split view to draw one in. Its
    /// preference keys are named only where they're removed.
    @Test("No old-sidebar type, split view or preference is left in the app")
    func nothingOfTheOldSidebarIsLeft() throws {
        let declarations = [
            "struct WorkspaceRow", "struct ProjectHeader", "struct NeedsYouRow:", "struct TerminalRow",
            "struct HiddenWorktrees", "struct UnclaimedWorktrees", "struct NoWorktreesRow", "struct HostDot",
            "struct SidebarSearchRow", "struct SidebarTitleRow", "struct SidebarGroupSection", "struct SidebarEntry",
            "struct SidebarRow", "struct SidebarMenuButton", "struct WorkspaceHeaderActions", "struct DaemonUpdateBar",
            "final class WorktreeDrag", "enum SidebarDefault", "enum SidebarMetrics", "enum WorkspacesTip",
            "func sidebarRows", "func reorderWorktrees", "NavigationSplitView(", "toggleSidebar(_:)",
        ]
        let keys = ["\"window.sidebar\"", "\"sidebar.openWorktrees\"", "\"sidebar.collapsedProjects\""]
        let files = try Self.sources()
        #expect(files.count > 50, "found \(files.count) source files")
        for (name, text) in files {
            for declaration in declarations where text.contains(declaration) {
                Issue.record("\(name) still has \(declaration)")
            }
            for key in keys where text.contains(key) && name != "NavigatorVisibility.swift" {
                Issue.record("\(name) still reads \(key)")
            }
        }
    }
}
