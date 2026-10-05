import Foundation
import Sparkle

/// One `farcooler app update`, seen through Sparkle's update session
/// (ov-302).
///
/// Sparkle asks its user driver what to do at each step: an update was
/// found, it is downloaded and ready. A person answers those in alerts; for
/// the command line this answers them instead, and tells the CLI what
/// happened. Sparkle still fetches the feed, downloads the build and checks
/// its EdDSA signature against the key baked into this bundle, so a build
/// this app would refuse from the menu it refuses here too.
///
/// It sends the CLI lines of `crates/cli/src/app_update.rs`'s conversation:
/// `checking`, `downloading` and `extracting` as Sparkle goes, then one of
/// `upToDate`, `installing` (Sparkle is about to quit and relaunch the app),
/// `pending` (it installs when the app next quits) or `refused`.
@MainActor
final class UpdateErrand {
    /// Relaunch once installed, rather than waiting for the next quit.
    let relaunch: Bool
    /// This app's facts, from before the update.
    let from: [String: Any]
    private let send: ([String: Any]) -> Void
    private let done: () -> Void
    /// The build Sparkle found, once it found one.
    private(set) var offer: [String: Any]?
    /// The CLI has had its last line.
    private(set) var finished = false

    init(
        relaunch: Bool, from: [String: Any], send: @escaping ([String: Any]) -> Void,
        done: @escaping () -> Void
    ) {
        self.relaunch = relaunch
        self.from = from
        self.send = send
        self.done = done
    }

    /// A build in the feed, as the CLI reads it.
    static func offer(version: String, build: String, notes: URL?) -> [String: Any] {
        ["version": version, "build": build, "notes": notes?.absoluteString ?? ""]
    }

    func checking() { progress("checking") }
    func downloading() { progress("downloading") }
    func extracting() { progress("extracting") }

    /// Sparkle found `offer`, at `stage`. Install it unless it can't be.
    func found(_ offer: [String: Any], informationOnly: Bool, stage: SPUUserUpdateStage) -> SPUUserUpdateChoice {
        self.offer = offer
        if informationOnly {
            refuse("information-only")
            return .dismiss
        }
        // Already installing in the background: it goes in at the next quit
        // whatever happens, and `install` quits and relaunches now.
        if stage == .installing {
            return readyToInstall()
        }
        return .install
    }

    /// Downloaded, verified and ready: relaunch into it now, or leave it to
    /// install when the app quits.
    func readyToInstall() -> SPUUserUpdateChoice {
        finish(["event": relaunch ? "installing" : "pending", "from": from, "to": offer ?? [:]])
        return relaunch ? .install : .dismiss
    }

    /// Nothing newer, or nothing this Mac can run.
    func notFound(_ error: NSError) {
        let reason = (error.userInfo[SPUNoUpdateFoundReasonKey] as? NSNumber)?.int32Value
        if reason == SPUNoUpdateFoundReason.systemIsTooOld.rawValue {
            refuse("system-too-old")
        } else {
            finish(["event": "upToDate", "app": from])
        }
    }

    /// Sparkle stopped with `error`.
    func failed(_ error: NSError) {
        refuse(Self.code(error))
    }

    /// The session ended. Anything still unsaid is a failure to install.
    func ended() {
        refuse("install-failed")
    }

    /// The CLI's word for a Sparkle error. The sentence is the CLI's.
    static func code(_ error: NSError) -> String {
        guard error.domain == SUSparkleErrorDomain else { return "install-failed" }
        switch Int(error.code) {
        case Int(SUError.appcastParseError.rawValue), Int(SUError.appcastError.rawValue),
            Int(SUError.invalidFeedURLError.rawValue), Int(SUError.insecureFeedURLError.rawValue):
            return "check-failed"
        case Int(SUError.downloadError.rawValue), Int(SUError.temporaryDirectoryError.rawValue):
            return "download-failed"
        case Int(SUError.signatureError.rawValue), Int(SUError.validationError.rawValue),
            Int(SUError.insufficientSigningError.rawValue), Int(SUError.noPublicDSAFoundError.rawValue):
            return "signature"
        default:
            return "install-failed"
        }
    }

    private func progress(_ word: String) {
        guard !finished else { return }
        send(["event": word])
    }

    /// Refused before or during the session, with the CLI's word for why.
    func refuse(_ code: String) {
        finish(["event": "refused", "code": code])
    }

    private func finish(_ event: [String: Any]) {
        guard !finished else { return }
        finished = true
        send(event)
        done()
    }
}

/// Sparkle's user driver: the standard alerts, except while the command line
/// has an errand running, when the errand answers instead.
@MainActor
final class UpdateUserDriver: NSObject, SPUUserDriver {
    private let standard: SPUStandardUserDriver
    /// The command line's update, for as long as Sparkle's session runs.
    var errand: UpdateErrand?

    init(standard: SPUStandardUserDriver) {
        self.standard = standard
    }

    func show(_ request: SPUUpdatePermissionRequest, reply: @escaping (SUUpdatePermissionResponse) -> Void) {
        standard.show(request, reply: reply)
    }

    func showUserInitiatedUpdateCheck(cancellation: @escaping () -> Void) {
        guard let errand else { return standard.showUserInitiatedUpdateCheck(cancellation: cancellation) }
        errand.checking()
    }

    func showUpdateFound(
        with appcastItem: SUAppcastItem, state: SPUUserUpdateState,
        reply: @escaping (SPUUserUpdateChoice) -> Void
    ) {
        guard let errand else { return standard.showUpdateFound(with: appcastItem, state: state, reply: reply) }
        let offer = UpdateErrand.offer(
            version: appcastItem.displayVersionString, build: appcastItem.versionString,
            notes: appcastItem.releaseNotesURL)
        reply(errand.found(offer, informationOnly: appcastItem.isInformationOnlyUpdate, stage: state.stage))
    }

    func showUpdateReleaseNotes(with downloadData: SPUDownloadData) {
        guard errand == nil else { return }
        standard.showUpdateReleaseNotes(with: downloadData)
    }

    func showUpdateReleaseNotesFailedToDownloadWithError(_ error: any Error) {
        guard errand == nil else { return }
        standard.showUpdateReleaseNotesFailedToDownloadWithError(error)
    }

    func showUpdateNotFoundWithError(_ error: any Error, acknowledgement: @escaping () -> Void) {
        guard let errand else { return standard.showUpdateNotFoundWithError(error, acknowledgement: acknowledgement) }
        errand.notFound(error as NSError)
        acknowledgement()
    }

    func showUpdaterError(_ error: any Error, acknowledgement: @escaping () -> Void) {
        guard let errand else { return standard.showUpdaterError(error, acknowledgement: acknowledgement) }
        errand.failed(error as NSError)
        acknowledgement()
    }

    func showDownloadInitiated(cancellation: @escaping () -> Void) {
        guard let errand else { return standard.showDownloadInitiated(cancellation: cancellation) }
        errand.downloading()
    }

    func showDownloadDidReceiveExpectedContentLength(_ expectedContentLength: UInt64) {
        guard errand == nil else { return }
        standard.showDownloadDidReceiveExpectedContentLength(expectedContentLength)
    }

    func showDownloadDidReceiveData(ofLength length: UInt64) {
        guard errand == nil else { return }
        standard.showDownloadDidReceiveData(ofLength: length)
    }

    func showDownloadDidStartExtractingUpdate() {
        guard let errand else { return standard.showDownloadDidStartExtractingUpdate() }
        errand.extracting()
    }

    func showExtractionReceivedProgress(_ progress: Double) {
        guard errand == nil else { return }
        standard.showExtractionReceivedProgress(progress)
    }

    func showReady(toInstallAndRelaunch reply: @escaping (SPUUserUpdateChoice) -> Void) {
        guard let errand else { return standard.showReady(toInstallAndRelaunch: reply) }
        reply(errand.readyToInstall())
    }

    func showInstallingUpdate(
        withApplicationTerminated applicationTerminated: Bool, retryTerminatingApplication: @escaping () -> Void
    ) {
        guard errand == nil else { return }
        standard.showInstallingUpdate(
            withApplicationTerminated: applicationTerminated,
            retryTerminatingApplication: retryTerminatingApplication)
    }

    func showUpdateInstalledAndRelaunched(_ relaunched: Bool, acknowledgement: @escaping () -> Void) {
        guard errand == nil else { return acknowledgement() }
        standard.showUpdateInstalledAndRelaunched(relaunched, acknowledgement: acknowledgement)
    }

    func showUpdateInFocus() {
        guard errand == nil else { return }
        standard.showUpdateInFocus()
    }

    func dismissUpdateInstallation() {
        guard let errand else { return standard.dismissUpdateInstallation() }
        errand.ended()
        self.errand = nil
    }
}
