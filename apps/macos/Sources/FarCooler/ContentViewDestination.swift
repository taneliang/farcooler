import AgentKit
import Foundation

// Where a window goes back to, and where a clicked notification lands (ov-182,
// ov-183): the window's half of `DestinationOpener`. In its own file because
// `ContentView` is over its line budget.

extension ContentView {
    /// Go back to `restoring` (`WindowRestore`): resolved as the runners come
    /// up, with what the fleet doesn't hold read from them, and opened where
    /// the window was or, when that's gone, at its nearest level that isn't.
    /// A window somebody has already moved is left alone.
    var windowRestore: WindowRestore {
        WindowRestore(
            restoring: $restoring, interrupted: { selection != nil }, world: { MacDestination.world(of: store) },
            read: { await DestinationReads.read($1, from: store.clients[$0], fleet: store.fleet) },
            land: { land($0, arrival: .restore) }, landed: { restoreFocus() })
    }

    /// Open a resolved destination here: a relaunch sets the selection and leaves
    /// the keyboard alone; a click goes where the navigator would and brings the
    /// window forward (`MacDestination.landing`).
    func land(_ destination: Destination, arrival: DestinationResolver.Arrival) {
        let click = arrival == .notification
        let landing = MacDestination.landing(destination, click: click, in: store.fleet)
        if let task = landing.openTask {
            openTask(task.id, host: task.host, workspace: task.workspace)
        } else if let next = landing.selection {
            if click { navigate(to: next, key: landing.pane) } else { selection = next }
            if !click, let pane = landing.pane { keyPane = pane }
        }
        if let id = landing.taskID {
            if let tab = landing.tab { taskTabs.choose(tab, for: id) }
            if let agent = landing.agent { chosenAgents[id] = agent }
        }
        if click { windowBox.window?.makeKeyAndOrderFront(nil) }
    }

    /// Where this window is, as `lastDestination` keeps it.
    var keptPlace: String? {
        MacDestination.destination(selection, tabs: taskTabs, agents: chosenAgents, keyPane: keyPane)?.encoded
    }

    /// Open what a notification was clicked for, in this window if it's the
    /// one to (`DestinationOpener.claim`): a task through `openTask`, the
    /// palette's way and the navigator's selection, a pane where going to it
    /// lands, whatever kind of notification it was (ov-183).
    func openNoticedTask() async {
        guard let open = noticeOpener.pending else { return }
        await noticeOpener.drive(
            open, window: windowID, isKey: { windowBox.window?.isKeyWindow == true },
            world: { MacDestination.world(of: store) },
            read: { await DestinationReads.read($1, from: store.clients[$0], fleet: store.fleet) },
            land: { land($0, arrival: .notification) })
    }
}
