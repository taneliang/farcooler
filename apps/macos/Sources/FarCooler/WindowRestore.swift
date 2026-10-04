import AgentKit
import SwiftUI

/// A window going back to where it was (ov-182, ov-262): `restoring`, run by
/// a task tied to it, until it opens or something else takes its place.
///
/// SwiftUI may cancel the task and start it again while the window's state
/// lives on: every window of a relaunch had its first run cancelled as it
/// came up, with the same open still waiting. A cancelled run is not an
/// answer, so it leaves `restoring` for the run that follows. Clearing it
/// there, as the window once did, left the run that followed nothing to
/// open, and every window on "No Workspace Selected".
struct WindowRestore: ViewModifier {
    @Binding var restoring: DestinationOpen?
    /// Somebody moved first: a restore yields to it.
    let interrupted: () -> Bool
    let world: () -> DestinationResolver.World
    let read: (_ host: String, _ request: DestinationReads.Request) async -> Data?
    let land: (Destination) -> Void
    /// The open is done with: opened, or given up on.
    var landed: () -> Void = {}

    func body(content: Content) -> some View {
        content.task(id: restoring?.id) { await run() }
    }

    func run() async {
        guard let open = restoring else { return }
        let outcome = await DestinationOpener.run(
            open, isCurrent: { restoring?.id == open.id }, interrupted: interrupted, world: world, read: read,
            land: land)
        if Self.finished(outcome, cancelled: Task.isCancelled), restoring?.id == open.id {
            restoring = nil
            landed()
        }
    }

    /// Whether a run that ended `outcome` is done with its open: not when
    /// it was cancelled, which leaves the open to the run that follows.
    static func finished(_ outcome: DestinationOpener.Outcome, cancelled: Bool) -> Bool {
        !(cancelled && outcome == .cancelled)
    }
}
