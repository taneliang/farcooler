import AppKit
import SwiftUI

/// Collapsing the sidebar.
///
/// `toggleSidebar:` is the platform's own action — what the View menu item and the
/// toolbar button both send — and it produces the native animated slide. Driving
/// SwiftUI's `columnVisibility` binding instead does the wrong thing twice: no
/// animation, and the window jumps.
///
/// That is the whole implementation, and it is worth saying why, because three
/// plausible-looking fixes went in here first and every one of them was treating a
/// symptom:
///
///   - `NSSplitViewItem.collapseBehavior` would be the correct knob, except
///     `NavigationSplitView` on macOS 26 is not backed by an
///     `NSSplitViewController` at all — walking the window's view-controller tree
///     finds none. SwiftUI answers `toggleSidebar:` from its own hosting layer.
///   - Pinning the window's `minSize`/`maxSize` across the toggle. Those constrain
///     only interactive resizing; `setFrame` ignores them. It never prevented the
///     resize, and holding a frame mid-animation could wedge the sidebar half-open.
///   - Serialising toggles so two could not overlap. Overlap was never the cause.
///
/// The window moved, and the reveal jerked, because the column widths did not add
/// up: the root view declared a 900pt minimum while the window sat at 900pt wide,
/// so revealing the sidebar could not take its width out of the detail and took it
/// out of the window instead — an AppKit window resize running against the
/// sidebar's own animation. Two disagreeing `navigationSplitViewColumnWidth`
/// declarations made which width it settled on nondeterministic.
///
/// Fixed where it was caused, in `OvernightApp` and `ContentView`, which leaves
/// nothing here to work around.
@MainActor
enum Sidebar {
    static func toggle() {
        NSApp.sendAction(#selector(NSSplitViewController.toggleSidebar(_:)), to: nil, from: nil)
    }
}
