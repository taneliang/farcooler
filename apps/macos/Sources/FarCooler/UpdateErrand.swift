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
    /// install when the app quits. `offer` is the build, when this errand
    /// took the session over at this point and never saw it found.
    func readyToInstall(offer: [String: Any]? = nil) -> SPUUserUpdateChoice {
        if let offer { self.offer = offer }
        finish(["event": relaunch ? "installing" : "pending", "from": from, "to": self.offer ?? [:]])
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
        refuse(Self.code(error), detail: Self.chain(error).map { "\($0.domain) \($0.code)" }.joined(separator: " < "))
    }

    /// The session ended. Anything still unsaid is a failure to install.
    func ended() {
        refuse("install-failed")
    }

    /// The CLI's word for a Sparkle error. The sentence is the CLI's.
    ///
    /// A signature that doesn't check out arrives wrapped: Sparkle reports
    /// "The update is improperly signed" as an installation error, with the
    /// validation failure underneath (seen against a scratch appcast). So the
    /// whole chain is read, and a signature failure anywhere in it wins.
    static func code(_ error: NSError) -> String {
        let words = chain(error).map(word)
        return words.first { $0 == "signature" } ?? words[0]
    }

    /// `error`, then what it wraps, outermost first.
    private static func chain(_ error: NSError) -> [NSError] {
        var chain: [NSError] = [error]
        while let under = chain.last?.userInfo[NSUnderlyingErrorKey] as? NSError, chain.count < 8 {
            chain.append(under)
        }
        return chain
    }

    private static func word(_ error: NSError) -> String {
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
    /// `detail` is Sparkle's error chain, for a log: the CLI says its own
    /// sentence.
    func refuse(_ code: String, detail: String? = nil) {
        var event: [String: Any] = ["event": "refused", "code": code]
        if let detail { event["detail"] = detail }
        finish(event)
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
///
/// An errand can also take over a session the alerts are showing. Sparkle
/// checks on its own schedule, so when a newer build exists the app is often
/// already asking about it — "A new version is available" or "Install and
/// Relaunch" — and that session is the update the command line wants. The
/// question it is waiting on is answered by the errand, and the alert closes.
@MainActor
final class UpdateUserDriver: NSObject, SPUUserDriver {
    private let standard: SPUStandardUserDriver
    /// The command line's update, for as long as Sparkle's session runs.
    var errand: UpdateErrand?

    /// A question the alerts are showing, which an errand can answer instead.
    private enum Waiting {
        case found(offer: [String: Any], informationOnly: Bool, stage: SPUUserUpdateStage, reply: Once)
        case ready(offer: [String: Any]?, reply: Once)
    }

    /// Sparkle takes one answer per question; whoever answers second, the
    /// person or the errand, is ignored.
    final class Once {
        private var reply: ((SPUUserUpdateChoice) -> Void)?
        init(_ reply: @escaping (SPUUserUpdateChoice) -> Void) { self.reply = reply }
        func callAsFunction(_ choice: SPUUserUpdateChoice) {
            let reply = self.reply
            self.reply = nil
            reply?(choice)
        }
    }

    private var waiting: Waiting?
    /// The build the alerts last offered, while their session runs.
    private var shown: [String: Any]?

    /// The build an alert is asking about, for `farcooler app version`.
    var offerShown: [String: Any]? { shown }

    init(standard: SPUStandardUserDriver) {
        self.standard = standard
    }

    /// Answer the question the alerts are waiting on with `errand`, and let
    /// it see the rest of the session through. False when no alert waits.
    func adopt(_ errand: UpdateErrand) -> Bool {
        guard let waiting else { return false }
        self.waiting = nil
        self.errand = errand
        standard.dismissUpdateInstallation()
        switch waiting {
        case .found(let offer, let informationOnly, let stage, let reply):
            reply(errand.found(offer, informationOnly: informationOnly, stage: stage))
        case .ready(let offer, let reply):
            reply(errand.readyToInstall(offer: offer))
        }
        return true
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
        let offer = UpdateErrand.offer(
            version: appcastItem.displayVersionString, build: appcastItem.versionString,
            notes: appcastItem.releaseNotesURL)
        let informationOnly = appcastItem.isInformationOnlyUpdate
        if let errand { return reply(errand.found(offer, informationOnly: informationOnly, stage: state.stage)) }
        standard.showUpdateFound(
            with: appcastItem, state: state,
            reply: alertAsks(offer, informationOnly: informationOnly, stage: state.stage, reply: reply))
    }

    /// The alerts ask whether to install `offer`: remember the question, so
    /// an errand can answer it, and return the reply the alert answers with.
    func alertAsks(
        _ offer: [String: Any], informationOnly: Bool, stage: SPUUserUpdateStage,
        reply: @escaping (SPUUserUpdateChoice) -> Void
    ) -> (SPUUserUpdateChoice) -> Void {
        shown = offer
        let once = Once(reply)
        waiting = .found(offer: offer, informationOnly: informationOnly, stage: stage, reply: once)
        return { [weak self] choice in
            self?.waiting = nil
            once(choice)
        }
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
        if let errand { return reply(errand.readyToInstall()) }
        let once = Once(reply)
        waiting = .ready(offer: shown, reply: once)
        standard.showReady { [weak self] choice in
            self?.waiting = nil
            once(choice)
        }
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
        waiting = nil
        shown = nil
        guard let errand else { return standard.dismissUpdateInstallation() }
        errand.ended()
        self.errand = nil
    }
}
