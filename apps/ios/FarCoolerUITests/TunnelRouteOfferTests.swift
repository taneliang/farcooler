import XCTest

/// The owner's route rule on the iPhone (ov-485): a runner granted through
/// Tailcat with an address beside it, whose tunnel fails, is asked about —
/// "Connecting through Tailcat failed. Connect directly to <address> instead?"
/// — and moves to the address only when somebody taps Connect Directly.
///
/// Needs no runner and no daemon. The Simulator links no tunnel and holds no
/// node key, so a tunneled runner fails before anything is dialed; the address
/// is in 10.255.255.0/24 and answers nothing. Everything is seeded in the
/// argument domain, and `-farcoolerEphemeral` keeps the tap from writing the
/// moved runner to this simulator's disk for the next test to launch into.
final class TunnelRouteOfferTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    private static let address = "10.255.255.1"

    private func launch(withAddressBeside: Bool) -> XCUIApplication {
        let id = UUID().uuidString
        let runners = """
            [{"id":"\(id)","label":"Attic","reach":{"kind":"tailcat","token":"tc-demo"},\
            "user":"me","fingerprint":"SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4"}]
            """
        let beside = """
            {"\(id)":{"kind":"direct","host":"\(Self.address)","port":22}}
            """
        let app = XCUIApplication()
        app.launchArguments += [
            "-hosts", "<\(Self.hex(runners))>", "-hosts.last", id, "-farcoolerEphemeral", "YES",
        ]
        if withAddressBeside { app.launchArguments += ["-runnerAlternatives", "<\(Self.hex(beside))>"] }
        app.launchDrawn()
        return app
    }

    /// The question is asked in the owner's words, nothing moves until it is
    /// answered, and Connect Directly moves the runner to its address.
    func testAFailedTunnelAsksBeforeConnectingDirectly() throws {
        let app = launch(withAddressBeside: true)

        let question = app.staticTexts["Connect directly to \(Self.address) instead?"]
        XCTAssertTrue(question.waitForExistence(timeout: 60), "no question: \(app.debugDescription)")
        XCTAssertTrue(
            app.staticTexts.containing(NSPredicate(format: "label BEGINSWITH 'Far Cooler will connect to it directly'")).firstMatch.exists,
            "doesn't say Connect Directly sticks")
        // Never automatic: still on Tailcat, still named by its label, and no
        // row has dialed the address.
        XCTAssertTrue(app.staticTexts["Attic"].exists)
        XCTAssertFalse(app.staticTexts[Self.address].exists, "moved to the address on its own")
        capture("ios-failure-prompt", expecting: .shows(question, "the question about connecting directly"))

        app.buttons["connect-directly"].tap()

        // On its address now: a direct runner is named by it, and the question
        // has nothing left to ask.
        XCTAssertTrue(
            app.staticTexts[Self.address].waitForExistence(timeout: 30),
            "Connect Directly didn't move the runner: \(app.debugDescription)")
        XCTAssertFalse(app.staticTexts["Connect directly to \(Self.address) instead?"].exists)

        // One route at a time, visible: the runner's editor names the one it
        // takes and offers the other.
        app.buttons["Edit…"].firstMatch.tap()
        let route = app.staticTexts["route-value"]
        XCTAssertTrue(route.waitForExistence(timeout: 30), "the editor doesn't name the route: \(app.debugDescription)")
        XCTAssertEqual(route.label, "Direct")
        let back = app.buttons["switch-route"]
        XCTAssertEqual(back.label, "Use Tailcat")
        capture("ios-route-row", expecting: .all(.reads(route, "Direct"), .reads(back, "Use Tailcat")))

        // And back: on Tailcat again, the tunnel fails again, and it asks again.
        back.tap()
        XCTAssertTrue(
            app.staticTexts["Connect directly to \(Self.address) instead?"].waitForExistence(timeout: 60),
            "Use Tailcat didn't put it back: \(app.debugDescription)")
        XCTAssertTrue(app.staticTexts["Attic"].exists)
    }

    /// A screenshot into `FARCOOLER_CAPTURE_OUT`, only when a person asked for
    /// one (`TEST_RUNNER_FARCOOLER_CAPTURE_OUT`); never in CI.
    private func capture(_ name: String, expecting state: CaptureState) {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"], !out.isEmpty else { return }
        guard requireState(state, for: name) else { return }
        let url = URL(fileURLWithPath: out).appendingPathComponent("\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    /// No address beside the tunnel, no question: there is nothing to offer.
    func testNoAddressBesideTheTunnelAsksNothing() throws {
        let app = launch(withAddressBeside: false)

        // The failure itself is on screen, so the question had its chance.
        let failed = app.staticTexts["This Device Has No Tailcat Key"]
        XCTAssertTrue(failed.waitForExistence(timeout: 60), "no failure: \(app.debugDescription)")
        XCTAssertFalse(app.staticTexts["Connect directly to \(Self.address) instead?"].exists)
        XCTAssertFalse(app.buttons["connect-directly"].exists)
    }

    private static func hex(_ text: String) -> String {
        Data(text.utf8).map { String(format: "%02x", $0) }.joined()
    }
}
