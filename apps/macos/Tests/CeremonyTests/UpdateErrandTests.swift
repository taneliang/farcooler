import Foundation
import Sparkle
import Testing

@testable import Far_Cooler

/// `farcooler app update`'s answers to Sparkle (ov-302): what the errand
/// replies at each of Sparkle's questions, and what it tells the CLI.
@MainActor
struct UpdateErrandTests {
    private final class Heard {
        var lines: [[String: Any]] = []
        var done = 0
        var events: [String] { lines.compactMap { $0["event"] as? String } }
    }

    private static let from: [String: Any] = ["version": "0.1.0", "build": "2328", "pid": 10]
    private static let offer = UpdateErrand.offer(
        version: "0.1.0", build: "2329",
        notes: URL(string: "https://github.com/taneliang/farcooler/commit/8476e3bd5d9cb12ee68a7ad68ba4ff626950137a"))

    private static func errand(relaunch: Bool, _ heard: Heard) -> UpdateErrand {
        UpdateErrand(relaunch: relaunch, from: from) { heard.lines.append($0) } done: { heard.done += 1 }
    }

    @Test func aFoundUpdateIsDownloadedThenInstalledWithARelaunch() {
        let heard = Heard()
        let errand = Self.errand(relaunch: true, heard)
        errand.checking()
        #expect(errand.found(Self.offer, informationOnly: false, stage: .notDownloaded) == .install)
        errand.downloading()
        #expect(heard.done == 0)
        #expect(errand.readyToInstall() == .install)
        #expect(heard.events == ["checking", "downloading", "installing"])
        #expect((heard.lines.last?["to"] as? [String: Any])?["build"] as? String == "2329")
        #expect((heard.lines.last?["from"] as? [String: Any])?["build"] as? String == "2328")
        #expect(heard.done == 1)
    }

    @Test func withoutARelaunchTheReadyUpdateWaitsForTheNextQuit() {
        let heard = Heard()
        let errand = Self.errand(relaunch: false, heard)
        #expect(errand.found(Self.offer, informationOnly: false, stage: .notDownloaded) == .install)
        #expect(errand.readyToInstall() == .dismiss)
        #expect(heard.events == ["pending"])
    }

    @Test func anUpdateAlreadyInstallingIsRelaunchedIntoOrLeftForTheQuit() {
        let now = Heard()
        #expect(Self.errand(relaunch: true, now).found(Self.offer, informationOnly: false, stage: .installing) == .install)
        #expect(now.events == ["installing"])

        let later = Heard()
        #expect(
            Self.errand(relaunch: false, later).found(Self.offer, informationOnly: false, stage: .installing) == .dismiss)
        #expect(later.events == ["pending"])
    }

    @Test func anInformationOnlyUpdateIsNotInstalled() {
        let heard = Heard()
        #expect(Self.errand(relaunch: true, heard).found(Self.offer, informationOnly: true, stage: .notDownloaded) == .dismiss)
        #expect(heard.events == ["refused"])
        #expect(heard.lines.last?["code"] as? String == "information-only")
    }

    @Test func nothingNewerIsUpToDateUnlessThisMacIsTooOld() {
        let latest = Heard()
        let onLatest = NSError(
            domain: SUSparkleErrorDomain, code: Int(SUError.noUpdateError.rawValue),
            userInfo: [SPUNoUpdateFoundReasonKey: NSNumber(value: SPUNoUpdateFoundReason.onLatestVersion.rawValue)])
        Self.errand(relaunch: true, latest).notFound(onLatest)
        #expect(latest.events == ["upToDate"])

        let old = Heard()
        let tooOld = NSError(
            domain: SUSparkleErrorDomain, code: Int(SUError.noUpdateError.rawValue),
            userInfo: [SPUNoUpdateFoundReasonKey: NSNumber(value: SPUNoUpdateFoundReason.systemIsTooOld.rawValue)])
        Self.errand(relaunch: true, old).notFound(tooOld)
        #expect(old.lines.last?["code"] as? String == "system-too-old")
    }

    @Test func sparklesErrorsBecomeTheCLIsWords() {
        func code(_ error: SUError) -> String {
            UpdateErrand.code(NSError(domain: SUSparkleErrorDomain, code: Int(error.rawValue)))
        }
        #expect(code(.signatureError) == "signature")
        #expect(code(.validationError) == "signature")
        #expect(code(.downloadError) == "download-failed")
        #expect(code(.appcastError) == "check-failed")
        #expect(code(.installationError) == "install-failed")
        #expect(UpdateErrand.code(NSError(domain: NSURLErrorDomain, code: -1009)) == "install-failed")
    }

    @Test func theCLIHearsOneEndingOnly() {
        let heard = Heard()
        let errand = Self.errand(relaunch: true, heard)
        errand.failed(NSError(domain: SUSparkleErrorDomain, code: Int(SUError.downloadError.rawValue)))
        errand.ended()
        errand.downloading()
        #expect(heard.events == ["refused"])
        #expect(heard.done == 1)
    }

    /// Through the user driver Sparkle actually calls: the errand answers,
    /// and lets go when the session ends.
    @Test func theDriverLetsTheErrandAnswerSparkle() {
        let heard = Heard()
        let driver = UpdateUserDriver(standard: SPUStandardUserDriver(hostBundle: .main, delegate: nil))
        driver.errand = Self.errand(relaunch: true, heard)

        var replied: SPUUserUpdateChoice?
        driver.showReady { replied = $0 }
        #expect(replied == .install)

        var acknowledged = false
        driver.showUpdaterError(NSError(domain: SUSparkleErrorDomain, code: 0)) { acknowledged = true }
        #expect(acknowledged)

        driver.dismissUpdateInstallation()
        #expect(driver.errand == nil)
        #expect(heard.events == ["installing"])
    }
}
