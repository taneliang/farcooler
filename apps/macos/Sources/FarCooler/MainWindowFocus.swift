import SwiftUI

/// What the main window says about itself to the menu bar, while it is the key
/// window.
///
/// A menu item's key equivalent reaches whatever window is key. Close Terminal
/// posts a notification the main window's `ContentView` acts on, so with the
/// Settings window key, ⌘W stopped and removed the selected terminal behind it
/// and left Settings open. The menu bar cannot tell which window is key on its
/// own; this is how it learns. `ContentView` publishes it as a focused SCENE
/// value, so it is nil whenever any other window (Settings, About) is key, and
/// the items that act on the main window are disabled then. A disabled item's
/// chord goes on to the next match, which for ⌘W is File ▸ Close.
struct MainWindowFocus: Equatable {
    /// The ⌘N task panel or the ⌘P palette is open over the window.
    var overlayOpen: Bool
    /// A task is open in the main area, with its tabs (ov-98).
    var taskOpen = false
    /// A workspace's navigator is on screen: ⌘F filters its tasks, and Board
    /// ▸ Mark All as Read reads its Unread (ov-104).
    var hasNavigator = false

    // What the rest of the menu bar can act on, so an item that can't is
    // dimmed rather than left to do nothing (ov-211). HIG, The menu bar: "If a
    // menu bar item isn't actionable, disable the action instead of hiding it
    // from the menu."

    /// The sidebar, the workspace's navigator, is out: View ▸ Hide Sidebar,
    /// else Show Sidebar (ov-178).
    var sidebarShown = true
    /// A worktree is on screen for ⌘T and Open in Editor to act on.
    var hasWorktree = false
    /// The terminals on screen, which ⌃⌘1 through ⌃⌘9 pick from.
    var terminals = 0
    /// ⌘] and ⌘[ go somewhere: another terminal, or the one terminal when
    /// the keyboard isn't in it.
    var stepsTerminals = false
    /// Something is waiting on you, for ⌃⌘N to open.
    var hasAttention = false
    /// Back (⌃⌘←) has somewhere to go.
    var goesBack = false
    /// Focus (⌃⌘↩) has something to put at full size, and whether it has.
    var focuses = false
    var focused = false
    /// A workspace is selected, for ⌥⌘1 through ⌥⌘3.
    var inWorkspace = false
    /// ⌃⌘↓ and ⌃⌘↑ each have a worktree to go to.
    var nextWorktree = false
    var previousWorktree = false
    /// The workspaces ⌘1 through ⌘9 can go to.
    var workspaces = 0
    /// A repository on a runner with workspaces, for New Workspace.
    var makesWorkspaces = false
    /// The Layout menu's worktree, or nil when none is on screen.
    var layout: LayoutMenuFocus?

    /// What ⌘F says it does: the navigator's filter in a workspace (or a
    /// loose worktree beside one), else Go to Anything's find.
    static func findTitle(_ focus: MainWindowFocus?) -> String {
        focus?.hasNavigator == true ? "Filter Tasks" : "Find Workspace, Task, or Agent…"
    }

    /// Board ▸ Mark All as Read acts only on a navigator in the key main
    /// window, nothing over it.
    static func marksRead(_ focus: MainWindowFocus?) -> Bool {
        navigates(focus) && focus?.hasNavigator == true
    }

    /// A task's next or previous tab (⌃⌘] and ⌃⌘[) acts only with a task
    /// open in the key main window, nothing over it.
    static func stepsTaskTabs(_ focus: MainWindowFocus?) -> Bool {
        navigates(focus) && focus?.taskOpen == true
    }

    /// Close Terminal (⌘W) acts only when the main window is key.
    static func closesTerminal(_ focus: MainWindowFocus?) -> Bool {
        focus != nil
    }

    /// Next Needing Attention (⌃⌘N) acts only when the main window is key: with
    /// Settings or About key, it opened an item in the window behind.
    /// And only with something waiting: with nothing, it did nothing (ov-211).
    static func stepsToAttention(_ focus: MainWindowFocus?) -> Bool {
        focus?.hasAttention == true
    }

    /// Going somewhere from the keyboard (⌘0, ⌘1–⌘9, ⌃⌘↑ and ⌃⌘↓, ov-86)
    /// acts only when the main window is key and nothing is open over it:
    /// with the palette up, or Settings key, it moved the window behind.
    static func navigates(_ focus: MainWindowFocus?) -> Bool {
        guard let focus else { return false }
        return !focus.overlayOpen
    }

    /// Zoom Pane (⇧⌘↩) acts only when the main window is key and nothing is
    /// open over it. The ⌘N panel and the ⌘P palette each have their own
    /// meaning for ⇧⌘↩ (a newline, and submit), and an enabled menu item would
    /// take the chord before either field saw it, zooming a pane nobody can
    /// see behind the overlay.
    static func zoomsPane(_ focus: MainWindowFocus?) -> Bool {
        guard let focus else { return false }
        return !focus.overlayOpen && focus.layout?.zooms == true
    }

    /// A Layout menu item acts only with a worktree's layout on screen in
    /// the key main window, nothing over it, and only when `can` says the
    /// layout has what the item needs (ov-211).
    static func lays(_ can: KeyPath<LayoutMenuFocus, Bool>, _ focus: MainWindowFocus?) -> Bool {
        guard navigates(focus), let layout = focus?.layout else { return false }
        return layout[keyPath: can]
    }

    /// A menu item that reads the main window (⌘N, ⌘P, ⌘R, ⌘/) acts only
    /// while it's key: with Settings or About key, nothing hears it.
    static func isKey(_ focus: MainWindowFocus?) -> Bool {
        focus != nil
    }

    /// A Workspace or Terminal menu item that goes somewhere acts only in
    /// the key main window, nothing over it, and when `can` says there's
    /// somewhere to go (ov-211).
    static func goes(_ can: KeyPath<MainWindowFocus, Bool>, _ focus: MainWindowFocus?) -> Bool {
        guard navigates(focus), let focus else { return false }
        return focus[keyPath: can]
    }

    /// ⌃⌘`n`: only for a terminal that's there.
    static func picksTerminal(_ n: Int, _ focus: MainWindowFocus?) -> Bool {
        navigates(focus) && n <= (focus?.terminals ?? 0)
    }

    /// ⌘`n`: only for a workspace that's there.
    static func picksWorkspace(_ n: Int, _ focus: MainWindowFocus?) -> Bool {
        navigates(focus) && n <= (focus?.workspaces ?? 0)
    }

    /// View ▸ Show Sidebar or Hide Sidebar, by what it would do. HIG, The menu
    /// bar: "Ensure that each show/hide item title reflects the current state
    /// of the corresponding view."
    static func sidebarTitle(_ focus: MainWindowFocus?) -> String {
        focus?.sidebarShown == false ? "Show Sidebar" : "Hide Sidebar"
    }

    /// View ▸ Show or Hide Sidebar (⌘B) acts only in the key main window,
    /// and only where a navigator is drawn to show or hide: not on Needs
    /// You, a loose worktree with no board, or nothing chosen, where it
    /// flipped a state nothing showed (ov-178 review).
    static func togglesSidebar(_ focus: MainWindowFocus?) -> Bool {
        isKey(focus) && focus?.hasNavigator == true
    }
}

extension FocusedValues {
    @Entry var mainWindow: MainWindowFocus?
}
