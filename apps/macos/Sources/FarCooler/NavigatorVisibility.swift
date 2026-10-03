import Foundation

/// Whether a window's navigator is out (ov-178): the one sidebar the window
/// has, shown and hidden by ⌘B, View ▸ Toggle Sidebar and the title bar's
/// sidebar button.
///
/// The old Fleet sidebar kept its own choice under `window.sidebar`, and
/// its open workspaces and collapsed repositories beside it. None of that
/// says anything about the navigator: most people's old sidebar was hidden,
/// since a new window opened without it, and reading that as "hide the
/// navigator" would take the task list away from everyone who never chose
/// to. So the old keys are removed, unread, and the navigator starts out
/// until someone puts it away.
enum NavigatorVisibility {
    /// What's stored: "shown" or "hidden", written each time it's toggled.
    static let key = "window.navigator"

    /// What the retired sidebar left in this app's defaults: its own
    /// visibility, its open workspaces, its collapsed repositories.
    static let retiredKeys = ["window.sidebar", "sidebar.openWorktrees", "sidebar.collapsedProjects"]

    /// Whether a new window opens with its navigator put away: only when
    /// it was put away last, in the navigator's own key. The retired
    /// sidebar's keys are removed first, whatever they held.
    static func hiddenAtLaunch(in defaults: UserDefaults = .standard) -> Bool {
        forgetRetiredSidebar(in: defaults)
        return defaults.string(forKey: key) == stored(true)
    }

    /// The stored word for a state.
    static func stored(_ hidden: Bool) -> String { hidden ? "hidden" : "shown" }

    /// Remove the retired sidebar's keys, and AppKit's saved frames for the
    /// split view it was drawn in.
    static func forgetRetiredSidebar(in defaults: UserDefaults) {
        for key in retiredKeys { defaults.removeObject(forKey: key) }
        for key in defaults.dictionaryRepresentation().keys
        where key.hasPrefix("NSSplitView Subview Frames") && key.hasSuffix("SidebarNavigationSplitView") {
            defaults.removeObject(forKey: key)
        }
    }
}
