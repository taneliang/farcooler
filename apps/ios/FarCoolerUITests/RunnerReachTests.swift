import XCTest

/// A runner saved before `Runner.reach` existed still loads.
///
/// `Runner` used to be an address, a port, a user and a fingerprint, which is
/// why a ceremony could grant a phone a tunneled runner and the phone had
/// nowhere to put it. It carries a ``Reach`` now — an address OR a token — and
/// the entry on every phone that already has one is written in the old shape.
///
/// **`RunnerStore` decodes the whole list in one call.** A decoder that threw on
/// the old shape would not skip one row, it would leave `hosts` empty: every
/// runner anybody had ever added, gone, silently, on the first launch after an
/// update, with the app dropping to onboarding as though the phone were new.
/// That is the failure this exists to catch, and nothing else in this suite can
/// catch it — every other test either seeds no runners or supplies one through
/// `-farcoolerDemoHost`, which is parsed rather than decoded.
///
/// Needs no runner and no daemon, deliberately. The address it seeds is in
/// 10.255.255.0/24 and answers nothing, so what is asserted is the screen the
/// app puts up BEFORE any connection resolves — which is the screen that names
/// the runner it decoded. A test that needed a live daemon would skip itself
/// green on a machine where the demo host is down, and a suite that cannot fail
/// is worse than no suite.
final class RunnerReachTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    /// The exact shape every install on disk holds today.
    ///
    /// Transcribed from `RunnerStore.save` as it was before `reach` existed —
    /// `id`, `label`, `address`, `port`, `user`, `fingerprint` — and NOT written
    /// by encoding a `Runner`, because encoding one with today's code would
    /// produce today's shape and the test would prove nothing about yesterday's.
    ///
    /// `<hex>` is the old property-list spelling of `Data`, which is the type
    /// the `hosts` key holds, and `UserDefaults` reads every `-key value` pair
    /// on the command line as the ARGUMENT domain: above the persisted one and
    /// gone the moment the process is. Nothing is written to this simulator's
    /// disk, so no run can leave a fixture behind for the next one.
    func testARunnerSavedBeforeReachExistedStillLoads() throws {
        let id = UUID().uuidString
        let json = """
            [{"id":"\(id)","label":"Attic","address":"10.255.255.1","port":22,\
            "user":"me","fingerprint":"accept-any"}]
            """
        let hex = Data(json.utf8).map { String(format: "%02x", $0) }.joined()

        let app = XCUIApplication()
        app.launchArguments += ["-hosts", "<\(hex)>", "-hosts.last", id]
        app.launch()

        // `FleetView.connecting` says "Connecting to <name>…" the moment a
        // runner is selected, and `Runner.named` is the address for a direct
        // one — so this asserts both halves at once: the entry decoded, and it
        // decoded as `.direct` with its address intact rather than as some
        // default that would leave the sentence blank.
        //
        // A decoder that threw would leave `hosts` empty and put onboarding on
        // screen instead, where this text does not exist at all.
        let connecting = app.staticTexts["Connecting to 10.255.255.1…"]
        XCTAssertTrue(
            connecting.waitForExistence(timeout: 30),
            "the app never named the seeded runner — a runner saved in the old shape was lost")
    }
}
