import XCTest

/// The read-only Files browser (ov-259), against the canned runner: a worktree's
/// files from the pane's menu and a shared folder from Needs You, a screen to a
/// directory and a screen to a file. Needs no runner: `-phone-harness` answers
/// `worktree.list_dir` and `worktree.read_file` and checks what it is asked, so
/// it cannot skip itself green.
final class FilesBrowserTests: XCTestCase {
    private static let agent = "0198f2c0-0000-7000-8000-00000000d002"

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    /// The overflow button of the pane actually on screen, by frame: the shell
    /// keeps the neighboring panes mounted (see `NewTerminalTests`).
    private func visibleOverflow(_ app: XCUIApplication) -> XCUIElement? {
        let all = app.buttons.matching(identifier: "Pane options")
        for i in 0..<all.count {
            let button = all.element(boundBy: i)
            guard button.exists, button.isHittable else { continue }
            if button.frame.midX > app.frame.minX, button.frame.midX < app.frame.maxX { return button }
        }
        return nil
    }

    /// The sheet's own Back. By being hittable, because the screens under the
    /// sheet keep navigation bars of their own.
    private func back(_ app: XCUIApplication) {
        let all = app.navigationBars.buttons.matching(identifier: "BackButton")
        for i in 0..<all.count where all.element(boundBy: i).isHittable {
            all.element(boundBy: i).tap()
            return
        }
        XCTFail("no Back to tap: \(app.debugDescription)")
    }

    private func openFilesFromAWorktree(_ app: XCUIApplication) throws {
        XCTAssertTrue(
            app.buttons["worktree-back"].firstMatch.waitForExistence(timeout: 30),
            "the link did not land on the pane: \(app.debugDescription)")
        let overflow = try XCTUnwrap(visibleOverflow(app), "no pane menu on screen")
        overflow.tap()
        let files = app.buttons["pane-files"]
        XCTAssertTrue(files.waitForExistence(timeout: 10), "the menu has no Files: \(app.debugDescription)")
        files.tap()
        XCTAssertTrue(element(app, "files-sheet").waitForExistence(timeout: 10), "Files did not open")
    }

    /// **A worktree's files, down to a line**: Files, a folder, a file; Back
    /// walks up; the text is numbered and a CRLF file breaks into its lines.
    func testAWorktreesFilesOpenDownToALine() throws {
        let app = XCUIApplication.phoneHarness(["-deep-link", Self.agent])
        try openFilesFromAWorktree(app)
        let src = element(app, "files-row-src")
        XCTAssertTrue(src.waitForExistence(timeout: 10), "the root has no src")
        src.tap()
        let main = element(app, "files-row-main.rs")
        XCTAssertTrue(main.waitForExistence(timeout: 10), "src has no main.rs")
        main.tap()
        XCTAssertTrue(element(app, "files-code").waitForExistence(timeout: 10), "no code view")
        XCTAssertTrue(app.staticTexts["fn main() {"].exists, "line 1 is not on screen")
        XCTAssertTrue(app.staticTexts["3"].exists, "the gutter has no line 3")
        XCTAssertTrue(app.buttons["files-copy-path"].exists, "no Copy Path")
        back(app)
        XCTAssertTrue(element(app, "files-row-main.rs").waitForExistence(timeout: 10), "Back left src")
        back(app)
        XCTAssertTrue(element(app, "files-row-README.md").waitForExistence(timeout: 10), "Back left the root")
        element(app, "files-row-README.md").tap()
        XCTAssertTrue(app.staticTexts["Invoices, in PDF."].waitForExistence(timeout: 10), "CRLF did not break lines")
    }

    /// **What can't be drawn says why**: a binary file and a file past the
    /// runner's limit say their size, and a link opens what it points at.
    func testAFileThatCantBeDrawnSaysSoAndALinkOpensItsTarget() throws {
        let app = XCUIApplication.phoneHarness(["-deep-link", Self.agent])
        try openFilesFromAWorktree(app)
        element(app, "files-row-logo.png").tap()
        let note = element(app, "files-note")
        XCTAssertTrue(note.waitForExistence(timeout: 10), "no note for the binary file")
        XCTAssertTrue(note.label.hasPrefix("This is a binary file."), note.label)
        back(app)
        element(app, "files-row-big.log").tap()
        XCTAssertTrue(
            element(app, "files-note").waitForExistence(timeout: 10)
                && element(app, "files-note").label.hasPrefix("This file is too large to show here."),
            "no too-large note")
        back(app)
        element(app, "files-row-latest").tap()
        XCTAssertTrue(app.staticTexts["fn main() {"].waitForExistence(timeout: 10), "the link did not open main.rs")
    }

    /// **A shared folder opens from Needs You**, under the runner's name for it.
    func testASharedFolderOpensFromNeedsYou() throws {
        let app = XCUIApplication.phoneHarness([])
        let folder = element(app, "folder-row-logs")
        XCTAssertTrue(folder.waitForExistence(timeout: 30), "no logs folder on Needs You")
        folder.tap()
        let log = element(app, "files-row-today.log")
        XCTAssertTrue(log.waitForExistence(timeout: 10), "the folder lists no today.log")
        log.tap()
        XCTAssertTrue(app.staticTexts["12:01 ready"].waitForExistence(timeout: 10), "the log's text is not on screen")
    }

    /// **A runner that predates Files, or a Read grant, gets no door**: no
    /// Files in the pane's menu, though the menu itself opens (New Terminal).
    func testNoDoorFromARunnerThatCantServeFiles() throws {
        for flag in ["-phone-files-old", "-phone-read-scope"] {
            let app = XCUIApplication.phoneHarness(["-deep-link", Self.agent, flag])
            XCTAssertTrue(
                app.buttons["worktree-back"].firstMatch.waitForExistence(timeout: 30),
                "\(flag): the link did not land on the pane")
            let overflow = try XCTUnwrap(visibleOverflow(app), "\(flag): no pane menu on screen")
            overflow.tap()
            XCTAssertTrue(
                app.buttons["New Terminal"].waitForExistence(timeout: 10) || app.buttons["Remove Worktree…"].exists,
                "\(flag): the menu did not open: \(app.debugDescription)")
            XCTAssertFalse(app.buttons["pane-files"].exists, "\(flag): Files is offered")
            app.terminateRetrying()
        }
    }
}
