import AgentKit
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
/// The one exception is somebody asking from the command line, which is an
/// answer: `farcooler app update` (ov-302) runs an `UpdateErrand` through the
/// same updater, and `UpdateUserDriver` lets the errand answer Sparkle's
/// questions instead of an alert.
///
/// A build with no feed does not start an updater at all. That is how `local`
/// declines: it is the working tree of whoever built it, and replacing it with
/// a build from CI is not an update.
@MainActor
final class Updates: NSObject, SPUUpdaterDelegate {
    static let shared = Updates()

    /// Nil when this build has no feed, which is the local channel and any
    /// bundle somebody assembled by hand, or when Sparkle wouldn't start.
    private var updater: SPUUpdater?
    private var driver: UpdateUserDriver?
    /// `latest`'s callers, waiting on one feed read.
    private var probes: [(_ latest: [String: Any]?, _ unknown: String?) -> Void] = []
    /// Which feed read is running, so a timer left from an earlier one can't
    /// answer for it.
    private var probe = 0

    var isEnabled: Bool { updater != nil }

    private override init() {
        super.init()
        let feed = Bundle.main.object(forInfoDictionaryKey: "SUFeedURL") as? String ?? ""
        guard !feed.isEmpty else { return }
        let driver = UpdateUserDriver(standard: SPUStandardUserDriver(hostBundle: .main, delegate: nil))
        let updater = SPUUpdater(hostBundle: .main, applicationBundle: .main, userDriver: driver, delegate: self)
        do {
            try updater.start()
        } catch {
            NSLog("Far Cooler: the updater didn't start: \(error)")
            return
        }
        self.driver = driver
        self.updater = updater
    }

    func checkForUpdates() {
        updater?.checkForUpdates()
    }

    /// Run the command line's update: check now, and let `errand` answer.
    func run(_ errand: UpdateErrand) {
        guard let updater, let driver else { return errand.refuse("updates-off") }
        // An errand that finished is done with once its session is; Sparkle
        // doesn't promise to say so (`dismissUpdateInstallation`) every time.
        let free = driver.errand.map(\.finished) ?? true
        guard free, updater.canCheckForUpdates, !updater.sessionInProgress else {
            return errand.refuse("busy")
        }
        driver.errand = errand
        updater.checkForUpdates()
    }

    /// The newest build the feed offers, or the refusal code for why it
    /// isn't known. Sparkle reads the feed; nothing is shown.
    func latest(_ answer: @escaping (_ latest: [String: Any]?, _ unknown: String?) -> Void) {
        guard let updater else { return answer(nil, "updates-off") }
        guard !updater.sessionInProgress else { return answer(nil, "busy") }
        probes.append(answer)
        guard probes.count == 1 else { return }
        probe += 1
        let this = probe
        updater.checkForUpdateInformation()
        // Sparkle answers a probe with one of the delegate calls below; one
        // that never comes is given up on rather than waited for.
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(30))
            guard let self, self.probe == this else { return }
            self.answerProbes(nil, "check-failed")
        }
    }

    private func answerProbes(_ latest: [String: Any]?, _ unknown: String?) {
        probe += 1
        let waiting = probes
        probes = []
        for answer in waiting { answer(latest, unknown) }
    }

    private static func offer(_ item: SUAppcastItem) -> [String: Any] {
        UpdateErrand.offer(version: item.displayVersionString, build: item.versionString, notes: item.releaseNotesURL)
    }

    // MARK: SPUUpdaterDelegate, for `latest`

    nonisolated func updater(_ updater: SPUUpdater, didFindValidUpdate item: SUAppcastItem) {
        MainActor.assumeIsolated { answerProbes(Self.offer(item), nil) }
    }

    nonisolated func updaterDidNotFindUpdate(_ updater: SPUUpdater, error: any Error) {
        let info = (error as NSError).userInfo
        let latest = info[SPULatestAppcastItemFoundKey] as? SUAppcastItem
        let reason = (info[SPUNoUpdateFoundReasonKey] as? NSNumber)?.int32Value
        MainActor.assumeIsolated {
            if let latest {
                answerProbes(Self.offer(latest), nil)
            } else if reason == SPUNoUpdateFoundReason.onLatestVersion.rawValue {
                // The newest is this build, though Sparkle didn't hand it over.
                answerProbes(
                    UpdateErrand.offer(version: AppVersion.marketing, build: AppVersion.build, notes: nil), nil)
            } else {
                answerProbes(nil, "check-failed")
            }
        }
    }

    nonisolated func updater(
        _ updater: SPUUpdater, didFinishUpdateCycleFor updateCheck: SPUUpdateCheck, error: (any Error)?
    ) {
        MainActor.assumeIsolated {
            if updateCheck == .updateInformation { answerProbes(nil, "check-failed") }
        }
    }
}
