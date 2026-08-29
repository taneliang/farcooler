import XCTest

/// The branch header's pull request row, over canned state.
///
/// The assertion that matters is the one about SILENCE. "There is no pull
/// request for this branch" and "we could not ask GitHub" arrive as the same
/// absent value on every link — `gh` failing, being logged out, or never having
/// run all degrade to the same `Ok(None)` in the daemon on purpose — and
/// `pr_known` on the list is the only thing that separates them. Offering
/// "Create Pull Request" while a pull request already exists behind a
/// logged-out `gh` is the app confidently proposing the wrong action, and it is
/// the one failure on this row a screenshot of the happy path would never show.
///
/// Needs no runner. `-changes-layout-harness` mounts the review screen over a
/// canned change set (`ChangesLayoutHarness`) and the `-pr-*` arguments stand
/// the row on canned GitHub state, so this is the whole matrix in a few seconds
/// with nothing to connect to — unlike `TerminalScrollTests`, which is about a
/// live pane and skips itself when no demo host answers.
final class ChangesPullRequestTests: XCTestCase {
    private func launch(_ pullRequest: String...) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += ["-changes-layout-harness"] + pullRequest
        app.launch()
        return app
    }

    /// Either identifier, whatever element type SwiftUI decided a `Link` is.
    private func row(_ app: XCUIApplication, _ identifier: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: identifier).firstMatch
    }

    /// The case that distinguishes this from a guess.
    ///
    /// `gh` could not answer, so the header says nothing about a pull request
    /// AND offers nothing — not a row, not a button, not a placeholder.
    func testARepositoryGitHubCouldNotBeAskedAboutOffersNothing() {
        let app = launch()
        // The screen is up: the canned branch is on it.
        XCTAssertTrue(
            app.staticTexts["feat/handle-retries-on-429"].waitForExistence(timeout: 30),
            "the changes harness never mounted")
        XCTAssertFalse(
            row(app, "changes-pr-create").exists,
            "offered to create a pull request without knowing whether one exists")
        XCTAssertFalse(
            row(app, "changes-pr-row").exists,
            "claimed to know a pull request's state without having been told one")
    }

    /// `gh` answered and there is no pull request, which is the only case the
    /// offer may be made in.
    func testAnAnsweredRepositoryWithNoPullRequestOffersToCreateOne() {
        let app = launch("-pr-none")
        let create = row(app, "changes-pr-create")
        XCTAssertTrue(create.waitForExistence(timeout: 30))
        XCTAssertFalse(row(app, "changes-pr-row").exists)
    }

    /// A pull request that exists is drawn, and its number and state are the
    /// row — not a chevron beside one.
    func testAPullRequestIsDrawnWithItsNumberAndWhereItGotTo() {
        let app = launch("-pr-failing")
        let prRow = row(app, "changes-pr-row")
        XCTAssertTrue(prRow.waitForExistence(timeout: 30))
        // Said rather than spelled: no wire word, no underscore, and the number
        // read as a number.
        XCTAssertEqual(prRow.label, "Pull request 335, Changes requested, Checks failed")
        // And nothing offers to create a second one.
        XCTAssertFalse(row(app, "changes-pr-create").exists)
    }
}
