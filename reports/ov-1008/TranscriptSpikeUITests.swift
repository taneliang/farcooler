import XCTest

/// Functional checks run separately from timing, so XCUI cannot cause hitches.
final class TranscriptSpikeUITests: XCTestCase {
    func testCollectionStreamsAndDisclosesTools() {
        let app = XCUIApplication()
        app.launchArguments = ["-appLock", "NO", "-native-agent-harness", "-native-live", "-native-live-real", "-native-long-reports", "-uikit-transcript"]
        app.launch()
        let transcript = app.collectionViews["native-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 30))
        let before = transcript.value as? String
        Thread.sleep(forTimeInterval: 4)
        XCTAssertNotEqual(before, transcript.value as? String, "live rows never reached the collection snapshot")
        var group = transcript.descendants(matching: .any)["native-tool-group"].firstMatch
        for _ in 0..<8 {
            if group.exists && group.isHittable { break }
            transcript.swipeDown()
            group = transcript.descendants(matching: .any)["native-tool-group"].firstMatch
        }
        XCTAssertTrue(group.exists && group.isHittable, "no production tool group")
        group.buttons.firstMatch.tap()
        XCTAssertTrue(transcript.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'native-tool-real:'")).firstMatch.waitForExistence(timeout: 5))
    }
}
