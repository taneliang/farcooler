import Foundation
import Testing

@testable import AgentKit

/// The overview grid's division of a display, checked without a simulator.
///
/// The third of the shell's pure-arithmetic suites, beside `ShellNavigation`'s
/// and `ShellFlight`'s, and it exists for the sharpest version of their shared
/// reason: what this arithmetic decides is a GAP, and a gap is the one kind of
/// defect that can sit on a screen for months being looked at. The grid's side
/// margins were 27 points on the phone this shell is tuned on — against a
/// 12-point gutter and a 16-point vertical padding — and every one of those
/// numbers had been read off a screenshot and approved.
///
/// So the assertions here are about RELATIONSHIPS rather than about 179 being
/// a nice width for a card. A row fills the space between its margins exactly;
/// the margin is the number the app asked for and not a remainder; and neither
/// depends on the display.
struct ShellGridTests {
    /// The app's own spacing scale, restated. `PaneMetrics` lives in the iOS
    /// target and this package cannot see it — which is why the functions take
    /// these as arguments rather than knowing them.
    private let margin: CGFloat = 16
    private let gutter: CGFloat = 12

    /// Every width this app is actually laid out at, portrait.
    ///
    /// Named, because an assertion that fails on "393" is an assertion nobody
    /// can act on. The two iPads are why `columns` is a function at all.
    private static let displays: [(name: String, width: CGFloat)] = [
        ("iPhone SE", 375), ("iPhone 17", 393), ("iPhone 17 Pro", 402),
        ("iPhone 17 Pro Max", 440), ("iPad 11-inch", 834), ("iPad 13-inch", 1024),
    ]

    /// **A row of cards fills the display exactly between its two margins.**
    ///
    /// The whole of the fix, stated as the one equation it has to satisfy.
    /// What it forbids is the old layout: two fixed 168-point columns in a
    /// container 402 points wide leave 54 points over, a `LazyVGrid` splits
    /// them between the outer edges, and the grid's most visible measurement
    /// becomes a remainder.
    @Test func aRowOfCardsFillsTheWidthBetweenTheMargins() {
        for display in Self.displays {
            let columns = CGFloat(
                ShellGrid.columns(width: display.width, margin: margin, gutter: gutter))
            let card = ShellGrid.cardWidth(
                width: display.width, margin: margin, gutter: gutter)
            let spanned = margin * 2 + card * columns + gutter * (columns - 1)
            #expect(
                abs(spanned - display.width) < 0.001,
                """
                \(display.name) at \(display.width): \(Int(columns)) cards of \(card) span \
                \(spanned), leaving \(display.width - spanned) points nobody chose to put \
                anywhere
                """)
        }
    }

    /// **The side gap is the margin, on every display, and it is the same
    /// number above and below the grid.**
    ///
    /// Said as its own test because it is the owner's actual complaint. The
    /// leading inset the old grid produced is `(width - 348) / 2`, which is
    /// 13.5 on the narrowest phone and 243 on an iPad — so this is also the
    /// assertion that the gap has stopped moving with the hardware.
    @Test func theSideGapIsTheMarginAndNotARemainder() {
        for display in Self.displays {
            let card = ShellGrid.cardWidth(
                width: display.width, margin: margin, gutter: gutter)
            let columns = CGFloat(
                ShellGrid.columns(width: display.width, margin: margin, gutter: gutter))
            // Where the first card starts and the last one ends, laid out
            // left to right from the margin.
            let leading = margin
            let trailing = display.width - (margin + card * columns + gutter * (columns - 1))
            #expect(abs(leading - trailing) < 0.001, "\(display.name) is not symmetric")
            #expect(
                abs(trailing - margin) < 0.001,
                """
                \(display.name) puts \(trailing) points down its trailing edge where the \
                layout asked for \(margin)
                """)
            // The arithmetic this replaced, for the record: two fixed
            // 168-point columns and a gutter centred in the display, which is
            // `(width - 348) / 2` and is 16 at exactly one width in the world.
            let centred = (display.width - (ShellGrid.card.width * 2 + gutter)) / 2
            #expect(
                abs(centred - margin) > 0.5,
                """
                \(display.name) happens to be the width where centring two fixed columns \
                already gave \(margin) points, so it proves nothing here
                """)
        }
    }

    /// **Two across on a phone, and more only where there is honestly room.**
    ///
    /// The floor is the design's: the card was sized so two fit side by side,
    /// and on a 375-point phone a 168-point minimum with 16-point margins
    /// admits exactly one — which would silently turn the grid into a list on
    /// the smallest device this app supports.
    @Test func everyPhoneGetsTwoColumnsAndAnIPadGetsMore() {
        for display in Self.displays where display.width <= 440 {
            #expect(
                ShellGrid.columns(width: display.width, margin: margin, gutter: gutter) == 2,
                "\(display.name) did not get two columns")
        }
        #expect(ShellGrid.columns(width: 834, margin: margin, gutter: gutter) == 4)
        #expect(ShellGrid.columns(width: 1024, margin: margin, gutter: gutter) == 5)
    }

    /// **A card is never drawn narrower than the space a column claims.**
    ///
    /// The invariant that keeps `columns` and `cardWidth` honest about each
    /// other: adding a column must not make the cards so thin that the count
    /// was a lie. Every width where the count goes up is a width where the
    /// card is still at least the design's own 168 — except the clamped phone
    /// case, which is stated above and is deliberate.
    @Test func addingAColumnNeverMakesTheCardsNarrowerThanTheDesign() {
        for width in stride(from: CGFloat(390), through: 1400, by: 1) {
            let card = ShellGrid.cardWidth(width: width, margin: margin, gutter: gutter)
            #expect(
                card >= ShellGrid.card.width - 0.001,
                """
                a \(width)-point display drew \(card)-point cards, under the design's \
                \(ShellGrid.card.width)
                """)
        }
    }

    /// **The card grows with the display and never shrinks as it widens.**
    ///
    /// Monotonic except where a column is added, which is the one place it is
    /// allowed to step back — and even there it may not fall under the design
    /// width, which the test above pins. Without this a grid can widen and
    /// draw smaller cards, which reads as the layout coming apart rather than
    /// as a rule being applied.
    @Test func cardsNeverShrinkExceptWhereAColumnIsAdded() {
        var previous = ShellGrid.cardWidth(width: 320, margin: margin, gutter: gutter)
        var previousColumns = ShellGrid.columns(width: 320, margin: margin, gutter: gutter)
        for width in stride(from: CGFloat(321), through: 1400, by: 1) {
            let card = ShellGrid.cardWidth(width: width, margin: margin, gutter: gutter)
            let columns = ShellGrid.columns(width: width, margin: margin, gutter: gutter)
            if columns == previousColumns {
                #expect(
                    card >= previous - 0.001,
                    """
                    widening to \(width) shrank the card from \(previous) to \(card) \
                    without adding a column
                    """)
            }
            previous = card
            previousColumns = columns
        }
    }
}
