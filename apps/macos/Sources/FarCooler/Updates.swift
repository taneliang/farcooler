import Foundation
import Sparkle

/// Checking whether a newer build of THIS channel exists.
///
/// Every channel asks before installing, canary included. Far Cooler is a tool
/// people work inside, and an app that replaces itself unasked is a worse
/// failure than an update noticed a day late — so `SUAutomaticallyUpdate` is
/// false everywhere and this exists to surface the question rather than to
/// answer it.
///
/// A build with no feed does not start an updater at all. That is how `local`
/// declines: it is the working tree of whoever built it, and replacing it with
/// a build from CI is not an update.
@MainActor
final class Updates {
    static let shared = Updates()

    /// Nil when this build has no feed, which is the local channel and any
    /// bundle somebody assembled by hand.
    private let controller: SPUStandardUpdaterController?

    var isEnabled: Bool { controller != nil }

    private init() {
        let feed = Bundle.main.object(forInfoDictionaryKey: "SUFeedURL") as? String ?? ""
        guard !feed.isEmpty else {
            controller = nil
            return
        }
        controller = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: nil,
            userDriverDelegate: nil
        )
    }

    func checkForUpdates() {
        controller?.checkForUpdates(nil)
    }
}
