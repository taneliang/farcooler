import AgentKit
import Combine
import SwiftUI

/// The window's side of its record (ov-248, ov-233): taking the one an earlier
/// run left, keeping it as the window changes, and letting it go on close.
/// Here rather than in `ContentView.swift`, which is at its size ceiling.
extension ContentView {
    /// A window opening: take a record, put back what it kept, and open the
    /// windows the launch had more of. The place itself is put back by
    /// `WindowRestore`, as the runners come up.
    func adoptSession() {
        guard kept == nil else { return }
        let adoption = WindowSessions.shared.adopt(holding: kept?.id)
        let record = adoption.session
        windowID = record.id
        kept = record
        if record.savedAt > Date(timeIntervalSince1970: 0) {
            jumpBar.history = Self.history(of: record)
            jumpBar.restoredTitles = Self.titles(of: record)
            navigatorHidden = record.layout.navigatorHidden
            navigatorSplit = record.layout.split
        }
        for _ in 0..<adoption.open { openWindow(id: FarCoolerApp.mainWindowID) }
        if adoption.open > 0 {
            // The windows opened after this one are in front of it, and it
            // holds the one that was last in use.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { windowBox.window?.makeKeyAndOrderFront(nil) }
        }
        WindowFrame.removeStale()
        if let window = windowBox.window {
            WindowFrame.apply(record, to: window)
            captureFrame(of: window)
        } else {
            windowBox.attached = { window in
                WindowFrame.apply(record, to: window)
                captureFrame(of: window)
            }
        }
    }

    /// The window's frame and full-screen state, into its record.
    func captureFrame(of window: NSWindow) {
        let full = window.styleMask.contains(.fullScreen)
        kept?.fullScreen = full
        if !full { kept?.frame = window.frameDescriptor }
    }

    /// The window moved, was resized, or entered or left full screen: its
    /// record follows. A full-screen frame isn't kept; leaving full screen
    /// goes back to the one before.
    func keepFrame(_ note: Notification) {
        guard let window = windowBox.window, note.object as? NSWindow === window, kept != nil else { return }
        captureFrame(of: window)
    }

    /// What `sessionState` is made from, cheap to compare: the record is built
    /// only when one of these changes, not on every redraw.
    struct SessionInputs: Equatable {
        var selection: Selection?
        var tabs: TaskTabMemory
        var agents: [String: String]
        var keyPane: PaneRef?
        var history: NavigationHistory
        var focus: Bool
        var hidden: Bool
        var split: String
        var kept: WindowSession?
        var restoring: Bool
    }

    var sessionInputs: SessionInputs {
        SessionInputs(
            selection: selection, tabs: taskTabs, agents: chosenAgents, keyPane: keyPane, history: jumpBar.history,
            focus: focusColumn, hidden: navigatorHidden, split: navigatorSplit, kept: kept, restoring: restoring != nil)
    }

    /// The window as its record says it is now, or nil while it has nowhere to
    /// be yet: a window still going back to its place would otherwise write
    /// over what it's going back to.
    var sessionState: WindowSession? {
        guard var record = kept, selection != nil, restoring == nil else { return nil }
        record.place = MacDestination.destination(selection, tabs: taskTabs, agents: chosenAgents, keyPane: keyPane)
        let names = historyNames
        record.back = Self.entries(jumpBar.history.back, names: names)
        record.forward = Self.entries(jumpBar.history.forward.reversed(), names: names)
        record.layout = WindowSession.Layout(focus: focusColumn, navigatorHidden: navigatorHidden, split: navigatorSplit)
        return record
    }

    /// What says the window's frame changed (`keepFrame`).
    var windowGeometry: some Publisher<Notification, Never> {
        Publishers.MergeMany(WindowFrame.changes.map { NotificationCenter.default.publisher(for: $0) })
    }

    /// The window closed: its record goes, unless it's the last window or the
    /// app is quitting (`WindowSessions.closed`).
    func closeSession() { WindowSessions.shared.closed(windowID) }

    /// Focus comes back once the place does: choosing a place clears it.
    func restoreFocus() {
        if kept?.layout.focus == true, selection?.focus != nil { focusColumn = true }
    }

    // MARK: - History, kept

    static func entries(_ stops: [NavigationHistory.Stop], names: HistoryMenu.Names) -> [WindowSession.Entry] {
        stops.compactMap { stop in
            MacDestination.destination(stop.place).map {
                WindowSession.Entry(
                    place: $0, trail: stop.trail.flatMap { MacDestination.destination($0) },
                    title: HistoryMenu.title(of: stop.place, names: names).map(HistoryMenu.shortened))
            }
        }
    }

    /// The history `record` kept, as stops: a place that won't read back (a
    /// terminal, which no window keeps as a place) is left out.
    static func history(of record: WindowSession) -> NavigationHistory {
        func stops(_ entries: [WindowSession.Entry]) -> [NavigationHistory.Stop] {
            entries.compactMap { entry in
                MacDestination.place(entry.place).map {
                    NavigationHistory.Stop($0, trail: entry.trail.flatMap { MacDestination.place($0) })
                }
            }
        }
        return NavigationHistory(back: stops(record.back), forward: stops(record.forward.reversed()))
    }

    static func titles(of record: WindowSession) -> [Selection: String] {
        var titles: [Selection: String] = [:]
        for entry in record.back + record.forward {
            if let title = entry.title, let place = MacDestination.place(entry.place) {
                titles[WorkspaceSelection.place(place)] = title
            }
        }
        return titles
    }
}
