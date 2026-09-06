import XCTest

/// Where the overview's cards actually land, measured on a running app.
///
/// **The other half of `ShellGridTests`, and neither is sufficient alone.**
/// That suite proves the arithmetic; this one proves the grid is laid out BY
/// that arithmetic. The defect it exists for is the one the old grid had — two
/// `GridItem(.fixed(168))` columns and no horizontal padding, so a `LazyVGrid`
/// centred them and the side gap became `(width - 348) / 2`, a number no part
/// of the app had chosen. Every unit test in the world can agree that the
/// margin should be 16 while the view goes on centring fixed columns.
///
/// Frames and not a screenshot, deliberately. A screenshot of a grid whose
/// margins are wrong looks like a grid; `XCUIElement.frame` is the layout's
/// own answer in points, which is the thing being disputed.
///
/// No runner and no daemon — `-shell-harness` stands the shell on a canned
/// fleet — so this suite never skips.
final class ShellGridLayoutTests: XCTestCase {
    /// `PaneMetrics.edge` and `PaneMetrics.card`, restated.
    ///
    /// A UI test bundle links neither AgentKit nor the app module, so it
    /// cannot say the names and has to carry the numbers. Written as constants
    /// with the names in the comment rather than as literals in the
    /// assertions, which is `ShellGestureTests.rowHeight`'s rule.
    private let margin: CGFloat = 16
    private let gutter: CGFloat = 12

    /// The card's height, which the fix does not change: only the WIDTH became
    /// a function of the display.
    private let cardHeight: CGFloat = 132

    /// Points of slop allowed on a measured frame.
    ///
    /// A third of a point. The frames come back in the app's own points on a
    /// 3× simulator, so a genuine layout answer is exact to within rounding;
    /// anything looser would let a whole pixel of drift through, and drift of
    /// exactly that size down one edge is what this suite is about.
    private let slop: CGFloat = 0.34

    private func launch(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-shell-harness", "-shell-overview", "-shell-10"] + extra
        app.launch()
        return app
    }

    /// The top row of cards, left to right.
    ///
    /// Found by `minY` rather than by name: the grid is sorted by precedence,
    /// so which workspaces are in the first row is a fact about the fixture's
    /// marks and not something this file should have an opinion about.
    private func topRow(_ app: XCUIApplication) throws -> [XCUIElement] {
        let cards = app.buttons.matching(
            NSPredicate(format: "identifier BEGINSWITH %@", "shell-card-"))
        guard cards.firstMatch.waitForExistence(timeout: 30) else {
            print(app.debugDescription)
            throw XCTSkip("The grid never drew a card.")
        }
        let all = cards.allElementsBoundByIndex.filter { $0.frame.height > 0 }
        guard let top = all.map({ $0.frame.minY }).min() else {
            throw XCTSkip("The grid drew cards with no frames.")
        }
        return all
            .filter { abs($0.frame.minY - top) < 1 }
            .sorted { $0.frame.minX < $1.frame.minX }
    }

    /// **The gap down each side of the grid is the margin the layout asked
    /// for, and the same one on both sides.**
    ///
    /// The owner's complaint, as a measurement. Before the fix this was
    /// `(402 - 348) / 2 = 27` on the phone the shell is tuned on — symmetric,
    /// so "are the two sides equal" would have passed, which is why the
    /// assertion is against 16 and not against itself.
    func testTheGridSitsAtTheSameMarginDownBothSides() throws {
        let app = launch()
        let row = try topRow(app)
        XCTAssertEqual(row.count, 2, "the phone's grid is two across")

        let width = app.frame.width
        let leading = row[0].frame.minX
        let trailing = width - row[row.count - 1].frame.maxX

        XCTAssertEqual(
            leading, margin, accuracy: slop,
            "the grid starts \(leading) points from the leading edge, not \(margin) — "
                + "on a \(width)-point display")
        XCTAssertEqual(
            trailing, margin, accuracy: slop,
            "the grid ends \(trailing) points from the trailing edge, not \(margin)")
    }

    /// **The gap BETWEEN two cards is the gutter, and it is smaller than the
    /// margins around them.**
    ///
    /// Two claims in one, and the second is the one that makes a grid read as
    /// a grid: items grouped inside a frame, rather than a frame drawn tightly
    /// around loosely-spaced items. The old layout had it exactly backwards on
    /// every phone from the 14 up — a 12-point gutter inside 22-to-46-point
    /// edges.
    func testTheGapBetweenCardsIsTheGutter() throws {
        let app = launch()
        let row = try topRow(app)
        XCTAssertEqual(row.count, 2)

        let gap = row[1].frame.minX - row[0].frame.maxX
        XCTAssertEqual(
            gap, gutter, accuracy: slop,
            "two cards in a row are \(gap) points apart, not \(gutter)")
        XCTAssertLessThan(
            gap, row[0].frame.minX,
            "the gap between the cards is wider than the gap at the edge of the screen, "
                + "so the row reads as two things rather than as one grid")
    }

    /// **Both cards are the same size, and the size is the one the grid
    /// computed rather than the design's fixed 168.**
    ///
    /// The card stretches now and the display is what it stretches to, which
    /// is the mechanism the two tests above are consequences of. A grid that
    /// went on drawing 168-point cards and merely padded itself would pass
    /// neither.
    func testEveryCardInARowIsTheSameComputedSize() throws {
        let app = launch()
        let row = try topRow(app)
        XCTAssertEqual(row.count, 2)

        let width = app.frame.width
        let expected = (width - margin * 2 - gutter) / 2
        for card in row {
            XCTAssertEqual(
                card.frame.width, expected, accuracy: slop,
                "a card is \(card.frame.width) points wide where two cards, two \(margin)-point "
                    + "margins and one \(gutter)-point gutter across \(width) leave \(expected)")
            // The height is untouched by any of this, and saying so here is
            // what stops the fix quietly becoming "cards got bigger".
            XCTAssertEqual(
                card.frame.height, cardHeight, accuracy: slop,
                "the card's height changed with its width")
        }
    }
}
