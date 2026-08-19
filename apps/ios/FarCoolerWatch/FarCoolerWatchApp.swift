import SwiftUI

/// The watchOS app's entry point.
///
/// One screen and one client, both handed over here rather than reached for
/// wherever they are needed. `FleetListView` takes its client as a parameter and
/// knows nothing about `WatchLinkClient.shared`, which is what lets a preview
/// hand it a fixed fleet with no paired phone in the room — see
/// `PreviewFleetClient`.
///
/// It has a body at all — rather than none — because a watchOS target with no
/// `App` conformance still builds. It produces a bundle with no entry point,
/// which installs onto a watch and then does nothing when it is tapped: a
/// failure with no build error, no crash log, and nothing to grep for.
@main
struct FarCoolerWatchApp: App {
    var body: some Scene {
        WindowGroup {
            FleetListView(client: WatchLinkClient.shared)
                // The link is started here rather than by whichever screen
                // happens to be first. A `WCSession` that is never activated
                // receives no application context at all, so the fleet would
                // simply never arrive — and there is no error for that, only an
                // app that is permanently empty. Started once, from the app,
                // because the app is the thing that outlives its screens.
                .task { WatchLinkClient.shared.start() }
        }
    }
}
