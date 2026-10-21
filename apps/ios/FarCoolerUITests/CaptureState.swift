import XCTest

/// What a capture is of, and how to tell the screen has got there (ov-642).
///
/// A capture of the wrong state is a valid image, so it passes every check
/// that looks at one file: four Mac "Done" captures once came out identical to
/// the code step's, because the test shot before the model got to Done. So
/// each capture helper takes the state as a required argument and shoots only
/// once it holds; when it never does, the test fails and nothing is written.
///
/// Name a state by something only it shows: an element, a label, or the
/// absence of the element the state before it had.
struct CaptureState {
    /// In words, for the failure.
    let name: String
    let reached: () -> Bool

    /// An element only this state has is on the screen.
    static func shows(_ element: XCUIElement, _ name: String? = nil) -> CaptureState {
        CaptureState(name: name ?? "\(element) is on screen") { element.exists }
    }

    /// An element the state before this one had is gone.
    static func gone(_ element: XCUIElement, _ name: String? = nil) -> CaptureState {
        CaptureState(name: name ?? "\(element) is gone") { !element.exists }
    }

    /// An element reads `text` (its label, or its value when it has one).
    static func reads(_ element: XCUIElement, _ text: String) -> CaptureState {
        CaptureState(name: "\(element) reads \"\(text)\"") {
            element.exists && ("\(element.label)" + "\(element.value ?? "")").contains(text)
        }
    }

    /// The screen has moved on from `before`, an earlier `XCUIScreen.main.screenshot().pngRepresentation`: for a
    /// capture of a scrolled or changed screen, where no element names the new state. A shot taken before the
    /// scroll lands would be byte-identical to the one before it.
    static func changed(from before: Data, _ name: String = "the screen has changed") -> CaptureState {
        CaptureState(name: name) { XCUIScreen.main.screenshot().pngRepresentation != before }
    }

    /// Every one of `states` holds.
    static func all(_ states: CaptureState...) -> CaptureState {
        CaptureState(name: states.map(\.name).joined(separator: " and ")) { states.allSatisfy { $0.reached() } }
    }

    /// A check of the test's own, with the words for it.
    static func when(_ name: String, _ check: @escaping () -> Bool) -> CaptureState {
        CaptureState(name: name, reached: check)
    }
}

extension XCTestCase {
    /// Waits up to `timeout` for `state`. When it never holds, the test fails
    /// (naming the shot and the state) and this says false, so the caller
    /// writes nothing.
    @discardableResult
    func requireState(_ state: CaptureState, for shot: String, timeout: TimeInterval = 20) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while !state.reached() {
            if Date() >= deadline {
                XCTFail("\(shot): the state \"\(state.name)\" was never reached, so nothing was shot")
                return false
            }
            Thread.sleep(forTimeInterval: 0.1)
        }
        return true
    }
}
