import Foundation
import Testing

@testable import AgentKit

/// The card's second rule, as arithmetic: one shared time axis down the card.
///
/// **What this suite is guarding.** Every trace on the wire snaps to the
/// shortest of three windows that contains its own activity, and the width it
/// picked is one nibble of its first byte. Two rows drawn side by side could
/// therefore be showing spans twenty-four times apart under one set of x
/// positions — so a column meant a different amount of time on each line, while
/// the drawing invited exactly the cross-row reading (two rows stopping in the
/// same column is a runner going away) that it could not support.
///
/// The fix is a sum, and it is exact because the three widths are whole
/// multiples of each other. These tests hold the sum to hand-computed values
/// rather than to whatever the code happens to produce, because a test that
/// asserts the implementation against itself is this repository's defining
/// failure mode.
///
/// Run by `swift test --package-path apps/shared/AgentKit`, which is the
/// "Test AgentKit" step of the `swift` job in `.github/workflows/ci.yml`.
struct TraceAxisTests {
    /// The premise everything else here rests on, checked rather than assumed.
    ///
    /// `farcooler_core::trace::BASE_WIDTH`: "Everything else is a whole multiple
    /// of this, so a coarser window is summed out of these rather than sampled
    /// on its own clock." If a fourth width were ever added that did not divide
    /// evenly, `rebucketed(to:)` would be splitting a bucket rather than adding
    /// whole ones, and it would have no way to know it.
    @Test func theWidthsAreWholeMultiplesOfEachOther() {
        // `farcooler_core::trace::WIDTHS`, quoted.
        #expect(ActivityTrace.Span.hour.bucketSeconds == 300)
        #expect(ActivityTrace.Span.sixHours.bucketSeconds == 1800)
        #expect(ActivityTrace.Span.day.bucketSeconds == 7200)

        for fine in ActivityTrace.Span.allCases {
            for coarse in ActivityTrace.Span.allCases
            where coarse.bucketSeconds > fine.bucketSeconds {
                #expect(
                    coarse.bucketSeconds % fine.bucketSeconds == 0,
                    "\(coarse) is not a whole number of \(fine) buckets, so it cannot be summed")
            }
        }

        // And the windows the labels stand for, which is where the round numbers
        // stop being the real ones. 13 x 300 = 65 minutes, printed `1h`.
        #expect(ActivityTrace.Span.hour.bucketSeconds * ActivityTrace.buckets == 3900)
        #expect(ActivityTrace.Span.sixHours.bucketSeconds * ActivityTrace.buckets == 23400)
        #expect(ActivityTrace.Span.day.bucketSeconds * ActivityTrace.buckets == 93600)
    }

    /// Thirteen five-minute buckets summed onto thirty-minute ones, against
    /// values computed by hand.
    ///
    /// Six fine buckets to a coarse one, packed from the NEWEST end — bucket 12
    /// is the one the trace was closed in and it is the end two rows have in
    /// common. So sources 7…12 make column 12, sources 1…6 make column 11,
    /// source 0 makes column 10, and the nine columns before that are empty
    /// because a 65-minute history genuinely says nothing about the six hours
    /// behind it.
    ///
    /// The powers of two are deliberate: every subset of them has a distinct
    /// sum, so a wrong grouping cannot coincidentally produce a right total.
    @Test func fiveMinuteBucketsSumOntoThirtyMinuteOnes() throws {
        let code: [UInt16] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096]
        let output = [UInt16](repeating: 10, count: 13)
        let commits: [UInt8] = [0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 3]
        let fine = try #require(
            ActivityTrace(
                ActivityTraceTests.encoded(code: code, output: output, commits: commits, width: 0)))
        #expect(fine.span == .hour)

        let axis = fine.rebucketed(to: .sixHours)
        #expect(axis.span == .sixHours, "the re-bucketed trace has to declare the axis it is on")

        // 128 + 256 + 512 + 1024 + 2048 + 4096, by hand.
        #expect(axis.code(12) == 8064)
        // 2 + 4 + 8 + 16 + 32 + 64.
        #expect(axis.code(11) == 126)
        // The single oldest bucket, alone in its column.
        #expect(axis.code(10) == 1)

        // Six equal buckets, then six, then one — the same grouping seen from a
        // series where the grouping is the only thing that shows.
        #expect(axis.output(12) == 60)
        #expect(axis.output(11) == 60)
        #expect(axis.output(10) == 10)

        // Commits add like the bars do: sources 10 and 12 land together, source
        // 6 lands one column back.
        #expect(axis.commits(12) == 5)
        #expect(axis.commits(11) == 1)
        #expect(axis.commits(10) == 0)
    }

    /// **§04's rule 4, on the shared axis.** "An agent that has touched no files
    /// shows an empty upper half against a visible center rule — absence drawn,
    /// not omitted."
    ///
    /// A row with less history than the axis covers fills only the newest
    /// columns and leaves the rest EMPTY. Nothing pads, stretches or centers a
    /// short trace to fill the axis, because a filled column is a claim that
    /// something happened in it and the wire made no such claim.
    @Test func aShortTraceLeavesTheOlderColumnsEmpty() throws {
        let fine = try #require(
            ActivityTrace(
                ActivityTraceTests.encoded(
                    code: [UInt16](repeating: 7, count: 13),
                    output: [UInt16](repeating: 7, count: 13),
                    commits: [UInt8](repeating: 1, count: 13),
                    width: 0)))

        let axis = fine.rebucketed(to: .sixHours)
        for column in 0...9 {
            #expect(axis.code(column) == 0, "column \(column) was invented")
            #expect(axis.output(column) == 0, "column \(column) was invented")
            #expect(axis.commits(column) == 0, "column \(column) was invented")
        }
        // ...and the newest three are where all of it went, so nothing was lost
        // either: 13 buckets of 7 is 91 whichever way they are grouped.
        let total = (10...12).reduce(0) { $0 + Int(axis.code($1)) }
        #expect(total == 91, "summing must move the counts, not drop them")
    }

    /// The widest jump, where thirteen five-minute buckets fit inside ONE
    /// two-hour bucket.
    ///
    /// 65 minutes is less than two hours, so the whole of a `1h` row's history
    /// belongs in the newest column of a `24h` axis and the other twelve columns
    /// are empty. This is the case that reads most starkly on the card, and it
    /// is correct: an agent that started an hour ago has nothing to say about
    /// yesterday.
    @Test func anHourOfHistorySitsInOneColumnOfADayAxis() throws {
        let code: [UInt16] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096]
        let fine = try #require(
            ActivityTrace(ActivityTraceTests.encoded(code: code, width: 0)))

        let axis = fine.rebucketed(to: .day)
        #expect(axis.span == .day)
        #expect(axis.code(12) == 8191, "2^13 − 1: every bucket, in one column")
        for column in 0...11 { #expect(axis.code(column) == 0) }
    }

    /// Thirty-minute buckets onto two-hour ones: four to a column, from the
    /// newest end.
    ///
    /// The middle pairing, which the two tests above do not exercise — 7200 is
    /// 4 x 1800 and neither 6 nor 24, so a `per` hard-coded to either would
    /// still pass them.
    @Test func thirtyMinuteBucketsSumOntoTwoHourOnes() throws {
        let code: [UInt16] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096]
        let coarse = try #require(
            ActivityTrace(ActivityTraceTests.encoded(code: code, width: 1)))
        #expect(coarse.span == .sixHours)

        let axis = coarse.rebucketed(to: .day)
        // 512 + 1024 + 2048 + 4096, sources 9…12.
        #expect(axis.code(12) == 7680)
        // 32 + 64 + 128 + 256, sources 5…8.
        #expect(axis.code(11) == 480)
        // 2 + 4 + 8 + 16, sources 1…4.
        #expect(axis.code(10) == 30)
        // Source 0, alone.
        #expect(axis.code(9) == 1)
        for column in 0...8 { #expect(axis.code(column) == 0) }
    }

    /// The wire's saturation, applied again on this side.
    ///
    /// `farcooler_core::trace::encode` saturates a bucket busier than 65535
    /// lines "which is invisible, because the bars are scaled per row per half".
    /// Summing six buckets can reach that ceiling where one could not, so the
    /// accumulator has to be wider than the field and the write has to clamp.
    /// An accumulator that was itself `UInt16` would wrap, and a wrapped bucket
    /// draws SHORT — a busy agent rendered quiet, which is the failure that
    /// looks like nothing at all.
    @Test func aSummedBucketSaturatesRatherThanWrapping() throws {
        let fine = try #require(
            ActivityTrace(
                ActivityTraceTests.encoded(
                    code: [UInt16](repeating: 60000, count: 13),
                    output: [UInt16](repeating: 60000, count: 13),
                    commits: [UInt8](repeating: 200, count: 13),
                    width: 0)))

        let axis = fine.rebucketed(to: .sixHours)
        // 6 x 60000 is 360000, which is not a UInt16 and must not become one by
        // wrapping to 32928.
        #expect(axis.code(12) == UInt16.max)
        #expect(axis.output(12) == UInt16.max)
        // 6 x 200 is 1200, and commits are one byte.
        #expect(axis.commits(12) == UInt8.max)
        // The column holding a single bucket is under the ceiling and must be
        // left alone — a clamp that fired everywhere would pass the two above.
        #expect(axis.code(10) == 60000)
        #expect(axis.commits(10) == 200)
    }

    /// A trace already on the axis is returned untouched, and so is one asked to
    /// go the wrong way.
    ///
    /// Splitting a coarse bucket into finer ones is not arithmetic: it would
    /// have to decide WHEN inside two hours the work happened and nothing on
    /// the wire knows. `AgentCardLayout.axis(of:)` never asks for it, and this
    /// is the belt on that.
    @Test func aTraceIsNeverSplitIntoFinerBuckets() throws {
        let code: [UInt16] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096]
        let coarse = try #require(
            ActivityTrace(ActivityTraceTests.encoded(code: code, width: 2)))

        #expect(coarse.rebucketed(to: .day) == coarse, "already on the axis")
        #expect(coarse.rebucketed(to: .hour) == coarse, "a day cannot be cut into five minutes")
        #expect(coarse.rebucketed(to: .sixHours) == coarse)
    }

    // MARK: - The card's choice of axis

    /// The card draws every row on the COARSEST window any of its rows carries.
    ///
    /// Coarsest, because summing up is exact and splitting down is invention.
    /// The row with the most history therefore sets the axis and every shorter
    /// row is summed onto it.
    @Test func theCardPicksTheCoarsestWindowItsRowsCarry() throws {
        let hour = try #require(ActivityTrace(ActivityTraceTests.encoded(width: 0)))
        let sixHours = try #require(ActivityTrace(ActivityTraceTests.encoded(width: 1)))
        let day = try #require(ActivityTrace(ActivityTraceTests.encoded(width: 2)))

        #expect(AgentCardLayout.axis(of: [hour, day]) == .day)
        #expect(AgentCardLayout.axis(of: [day, hour]) == .day, "order must not decide it")
        #expect(AgentCardLayout.axis(of: [hour, sixHours]) == .sixHours)
        #expect(AgentCardLayout.axis(of: [hour, hour]) == .hour, "agreement changes nothing")

        // A row with no trace has no window and must not vote. Letting an absent
        // row count as the finest would narrow an axis it is not even on.
        #expect(AgentCardLayout.axis(of: [nil, day]) == .day)
        #expect(AgentCardLayout.axis(of: [nil, nil]) == nil, "no trace, no axis")
        #expect(AgentCardLayout.axis(of: []) == nil)
    }

    /// End to end, from the push's JSON to the traces the card hands the
    /// drawing: two rows at different windows come out on ONE.
    ///
    /// **This is the test that fails if the draw site ever reads the raw bytes
    /// again.** `AgentCardLayout.Row.trace` is the decoded, re-bucketed trace by
    /// design, and the row's own 66 bytes stay on `AgentCardRow` for anything
    /// that wants the wire rather than a picture.
    @Test func twoRowsAtDifferentWindowsAreDrawnOnOne() throws {
        let short = ActivityTraceTests.encoded(
            code: [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096], width: 0)
        let long = ActivityTraceTests.encoded(
            code: [UInt16](repeating: 5, count: 13), width: 1)
        let json = """
            {"terminal":"a","label":"a","machine":"m","status":"working","detail":"",
             "blocked":0,"review":0,"working":2,"more":0,
             "rows":[{"terminal":"a","label":"quick","status":"working","detail":"",
                      "trace":"\(short.base64EncodedString())"},
                     {"terminal":"b","label":"old","status":"working","detail":"",
                      "trace":"\(long.base64EncodedString())"}]}
            """
        let state = try JSONDecoder().decode(AgentCardState.self, from: Data(json.utf8))
        let card = try #require(AgentCardLayout(state: state))

        #expect(card.span == .sixHours, "the longer-running agent sets the axis")
        #expect(card.rows.count == 2)
        for row in card.rows {
            #expect(row.trace?.span == .sixHours, "\(row.name) is not on the card's axis")
        }

        // The short row's own values, summed — the same hand-computed numbers as
        // `fiveMinuteBucketsSumOntoThirtyMinuteOnes`, arrived at through the
        // whole decode rather than through `rebucketed(to:)` alone.
        let quick = try #require(card.rows.first?.trace)
        #expect(quick.code(12) == 8064)
        #expect(quick.code(11) == 126)
        #expect(quick.code(10) == 1)
        #expect(quick.code(9) == 0)

        // The long row was already on the axis and must be untouched: five in
        // every column, not thirty in some of them.
        let old = try #require(card.rows.last?.trace)
        for column in 0..<ActivityTrace.buckets { #expect(old.code(column) == 5) }

        // And the raw bytes are still there for the wire, unchanged by any of
        // this — `AgentCardRow` is what the relay sent and stays that.
        #expect(card.rows.first?.row.trace == short)
    }

    /// A card whose rows carry no trace has no axis and draws none.
    ///
    /// The compatibility path: a relay older than the trace field, or a fleet
    /// that has done nothing the trace can see. `nil` here has to stay `nil`
    /// rather than becoming a default window, because a default window is a
    /// surface deciding what a runner did not say.
    @Test func aCardWithNoTracesHasNoAxis() throws {
        let json = """
            {"terminal":"a","label":"a","machine":"m","status":"working","detail":"",
             "blocked":0,"review":0,"working":1,"more":0,
             "rows":[{"terminal":"a","label":"quick","status":"working","detail":""}]}
            """
        let state = try JSONDecoder().decode(AgentCardState.self, from: Data(json.utf8))
        let card = try #require(AgentCardLayout(state: state))
        #expect(card.span == nil)
        #expect(card.rows.first?.trace == nil)
    }

    // MARK: - The anchor: where a trace is, not only what shape it has

    /// **The phase.** A five-minute trace whose newest bucket is the third of
    /// its half hour, placed by its anchor rather than packed from the end.
    ///
    /// Anchor 6002 is absolute five-minute bucket 6002, so the thirteen are
    /// 5990 through 6002. Half hours are six of those: 5988–5993 is half hour
    /// 998, 5994–5999 is 999, 6000–6005 is 1000. So sources 0…3 land in 998,
    /// 4…9 in 999 and 10…12 in 1000 — the newest THREE in the newest column,
    /// where packing from the end puts six. Powers of two, so every grouping
    /// has its own sum.
    @Test func anAnchoredTraceIsPlacedByItsPhase() throws {
        let code: [UInt16] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096]
        let fine = try #require(
            ActivityTrace(ActivityTraceTests.encoded(code: code, width: 0)))

        let placed = fine.placed(on: .sixHours, anchor: 6002, newest: 1000)
        #expect(placed.span == .sixHours)
        // 1024 + 2048 + 4096, sources 10…12.
        #expect(placed.code(12) == 7168)
        // 16 + 32 + 64 + 128 + 256 + 512, sources 4…9.
        #expect(placed.code(11) == 1008)
        // 1 + 2 + 4 + 8, sources 0…3.
        #expect(placed.code(10) == 15)
        for column in 0...9 { #expect(placed.code(column) == 0) }

        // And that is NOT what packing from the end draws — the one-column
        // error the anchor exists to remove, pinned so the two cannot quietly
        // become the same function.
        #expect(fine.rebucketed(to: .sixHours).code(12) == 8064)
    }

    /// Where the phase happens to be the last bucket of its half hour, placing
    /// and packing agree exactly. That is `rebucketed(to:)`'s own claim — "exact
    /// when the phase is `per - 1`" — checked from the other side.
    @Test func anAnchorAtTheEndOfItsColumnDrawsWhatPackingDid() throws {
        let code: [UInt16] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096]
        let fine = try #require(
            ActivityTrace(ActivityTraceTests.encoded(code: code, width: 0)))
        // 6005 is the sixth five-minute bucket of half hour 1000.
        #expect(fine.placed(on: .sixHours, anchor: 6005, newest: 1000) == fine.rebucketed(to: .sixHours))
    }

    /// **The skew, made visible.** Two rows from two runners' last notices, end
    /// to end from the push's JSON: a five-minute row anchored at 6002 (half
    /// hour 1000) and a thirty-minute row anchored at 998, two half hours
    /// earlier.
    ///
    /// The newer row sets the card's newest column, 1000. The older row ends two
    /// columns before it: its thirteen buckets are half hours 986…998, so they
    /// fill columns 0…10, its two oldest fall off the left of the window, and
    /// columns 11 and 12 are EMPTY — the runner said nothing about them. Packing
    /// from the end drew both rows ending in column 12, as if they had been
    /// encoded together.
    ///
    /// **This is the test that fails if the card ignores the anchor.**
    @Test func twoAnchoredRowsShareOneAbsoluteGrid() throws {
        let short = ActivityTraceTests.encoded(
            code: [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096], width: 0)
        let long = ActivityTraceTests.encoded(
            code: [UInt16](repeating: 5, count: 13), width: 1)
        let json = """
            {"terminal":"a","label":"a","machine":"m","status":"working","detail":"",
             "blocked":0,"review":0,"working":2,"more":0,
             "rows":[{"terminal":"a","label":"quick","status":"working","detail":"",
                      "trace":"\(short.base64EncodedString())","traceAnchor":6002,"updatedAt":1800600},
                     {"terminal":"b","label":"old","status":"working","detail":"",
                      "trace":"\(long.base64EncodedString())","traceAnchor":998,"updatedAt":1800600}]}
            """
        let state = try JSONDecoder().decode(AgentCardState.self, from: Data(json.utf8))
        let card = try #require(AgentCardLayout(state: state))
        #expect(card.span == .sixHours)
        #expect(card.rows.first?.row.traceAnchor == 6002, "the anchor did not survive the decode")

        let quick = try #require(card.rows.first?.trace)
        #expect(quick.code(12) == 7168)
        #expect(quick.code(11) == 1008)
        #expect(quick.code(10) == 15)
        #expect(quick.code(9) == 0)

        let old = try #require(card.rows.last?.trace)
        for column in 0...10 { #expect(old.code(column) == 5, "column \(column)") }
        #expect(old.code(11) == 0, "the older runner said nothing about this half hour")
        #expect(old.code(12) == 0, "the older runner said nothing about this half hour")
    }

    /// A row from a runner too old to send an anchor, beside one that sent it.
    ///
    /// The anchored row is placed; the other is packed from its newest end into
    /// the card's newest column, exactly as every row was before anchors — the
    /// old drawing, not an invented placement. (`twoRowsAtDifferentWindowsAreDrawnOnOne`
    /// above is the card where NO row has one, and it is unchanged.)
    @Test func aRowWithNoAnchorIsPackedAsBefore() throws {
        let short = ActivityTraceTests.encoded(
            code: [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096], width: 0)
        let long = ActivityTraceTests.encoded(
            code: [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096], width: 1)
        let json = """
            {"terminal":"a","label":"a","machine":"m","status":"working","detail":"",
             "blocked":0,"review":0,"working":2,"more":0,
             "rows":[{"terminal":"a","label":"quick","status":"working","detail":"",
                      "trace":"\(short.base64EncodedString())","traceAnchor":6002,"updatedAt":1800600},
                     {"terminal":"b","label":"old","status":"working","detail":"",
                      "trace":"\(long.base64EncodedString())"}]}
            """
        let state = try JSONDecoder().decode(AgentCardState.self, from: Data(json.utf8))
        let card = try #require(AgentCardLayout(state: state))

        #expect(card.rows.first?.trace?.code(12) == 7168, "the anchored row was not placed")
        let old = try #require(card.rows.last?.trace)
        #expect(old.code(12) == 4096, "an unanchored row keeps its newest bucket in the newest column")
        #expect(old.code(0) == 1)
    }

    /// The anchor round-trips through the encoder ActivityKit persists a card
    /// with, and a malformed one is no anchor rather than a thrown decode —
    /// which would keep the whole activity out of `Activity.activities`.
    @Test func theAnchorSurvivesPersistenceAndAMalformedOneIsDropped() throws {
        let row = AgentCardRow(terminal: "t", trace: Data([0x10]), traceAnchor: 5_960_000)
        let back = try JSONDecoder().decode(
            AgentCardRow.self, from: JSONEncoder().encode(row))
        #expect(back.traceAnchor == 5_960_000)

        let odd = #"{"terminal":"t","traceAnchor":"5960000"}"#
        let lenient = try JSONDecoder().decode(AgentCardRow.self, from: Data(odd.utf8))
        #expect(lenient.traceAnchor == nil)
        #expect(lenient.terminal == "t")
    }

    // MARK: - An anchor is a clock, and a clock can be wrong

    /// Two rows as the relay sends them, the second with its own anchor and
    /// updatedAt. Anchor 6002 on the five-minute grid is Unix second 1,800,600,
    /// and that is when the relay heard from both.
    private func card(second: String) throws -> AgentCardLayout {
        let short = ActivityTraceTests.encoded(
            code: [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096], width: 0)
        let json = """
            {"terminal":"a","label":"a","machine":"m","status":"working","detail":"",
             "blocked":0,"review":0,"working":2,"more":0,
             "rows":[{"terminal":"a","label":"quick","status":"working","detail":"",
                      "trace":"\(short.base64EncodedString())","traceAnchor":6002,
                      "updatedAt":1800600},
                     \(second)]}
            """
        let state = try JSONDecoder().decode(AgentCardState.self, from: Data(json.utf8))
        return try #require(AgentCardLayout(state: state))
    }

    /// A thirty-minute row of fives, anchored at `anchor` and heard at 1,800,600.
    private func fives(anchor: Int) -> String {
        let long = ActivityTraceTests.encoded(code: [UInt16](repeating: 5, count: 13), width: 1)
        return """
            {"terminal":"b","label":"other","status":"working","detail":"",
             "trace":"\(long.base64EncodedString())","traceAnchor":\(anchor),"updatedAt":1800600}
            """
    }

    /// **A runner a day fast must not erase the card.** Its anchor, half hour
    /// 1048, claims a bucket that starts a day after the relay heard from it,
    /// so it is ignored: that row is packed as it was before anchors, and the
    /// honest row keeps its exact placement. Trusted, 1048 would have become the
    /// card's newest column and pushed every bucket of the other row off the
    /// left edge.
    @Test func aRunnerFarAheadDoesNotSetTheCardsNewestColumn() throws {
        let card = try card(second: fives(anchor: 1000 + 48))
        let quick = try #require(card.rows.first?.trace, "the honest row was pushed off the card")
        #expect(quick.code(12) == 7168)
        #expect(quick.code(11) == 1008)
        #expect(quick.code(10) == 15)
        let ahead = try #require(card.rows.last?.trace)
        for column in 0..<ActivityTrace.buckets { #expect(ahead.code(column) == 5) }
    }

    /// **A runner a day slow erases only itself**, and draws as NO trace rather
    /// than as thirteen quiet buckets. Half hour 952 is forty-eight columns
    /// before the card's newest, so not one of its buckets is in the window —
    /// it has told the card nothing about these six and a half hours.
    @Test func aRunnerFarBehindDrawsAsAbsentAndErasesNobodyElse() throws {
        let card = try card(second: fives(anchor: 1000 - 48))
        let quick = try #require(card.rows.first?.trace)
        #expect(quick.code(12) == 7168)
        #expect(quick.code(11) == 1008)
        #expect(quick.code(10) == 15)
        #expect(card.rows.last?.trace == nil, "a row with nothing in the window is absent, not quiet")
    }

    /// `placed` itself: nothing landed is nil, and something landed that was
    /// zero is thirteen measured buckets and draws.
    @Test func aPlacedTraceWithNothingInTheWindowIsNil() throws {
        let quiet = try #require(ActivityTrace(ActivityTraceTests.encoded(width: 1)))
        // Thirteen columns behind: the newest bucket is one left of column 0.
        #expect(quiet.placed(on: .sixHours, anchor: 987, newest: 1000) == nil)
        // Twelve behind: the newest bucket is column 0, so it landed — as zero.
        let edge = try #require(quiet.placed(on: .sixHours, anchor: 988, newest: 1000))
        for column in 0..<ActivityTrace.buckets { #expect(edge.code(column) == 0) }
    }

    /// The bound itself, at its edge. Anchor `a` at width `w` names a bucket
    /// starting at `a * w`, which may be at most `anchorSlack` past `heardAt`.
    @Test func anAnchorIsTrustedUpToTheSlackAndNoFurther() {
        let heard = Date(timeIntervalSince1970: 1_800_600)
        // (1_800_600 + 600) / 300 = 6004.
        #expect(ActivityTrace.trusted(6004, span: .hour, heardAt: heard) == 6004)
        #expect(ActivityTrace.trusted(6005, span: .hour, heardAt: heard) == nil)
        // (1_800_600 + 600) / 1800 = 1000.66…, so 1000.
        #expect(ActivityTrace.trusted(1000, span: .sixHours, heardAt: heard) == 1000)
        #expect(ActivityTrace.trusted(1001, span: .sixHours, heardAt: heard) == nil)
        // Nothing to check it against, so nothing to trust.
        #expect(ActivityTrace.trusted(6000, span: .hour, heardAt: nil) == nil)
        #expect(ActivityTrace.trusted(-1, span: .hour, heardAt: heard) == nil)
    }

    /// **Extreme values cost a row its placement, never the card.** This runs
    /// in a Live Activity extension, where an arithmetic trap is the whole card
    /// gone. Each of these would overflow somewhere without its guard.
    @Test func extremeAnchorsNeverTrap() throws {
        let trace = try #require(ActivityTrace(ActivityTraceTests.encoded(width: 0)))
        #expect(trace.placed(on: .day, anchor: Int.min, newest: 0) == nil)
        #expect(trace.placed(on: .day, anchor: Int.max, newest: Int.max) == nil)
        #expect(trace.placed(on: .day, anchor: 0, newest: Int.max) == nil)
        #expect(
            ActivityTrace.trusted(
                ActivityTrace.anchorLimit, span: .day,
                heardAt: Date(timeIntervalSince1970: .greatestFiniteMagnitude)) == nil)
        #expect(
            ActivityTrace.trusted(
                ActivityTrace.anchorLimit, span: .day, heardAt: Date(timeIntervalSince1970: 1e15))
                == nil)

        // And the decoder keeps anything outside `0...anchorLimit` off the row.
        for wire in ["-1", "9007199254740993", "\(Int.max)"] {
            let json = #"{"terminal":"t","traceAnchor":WIRE}"#.replacingOccurrences(
                of: "WIRE", with: wire)
            let row = try JSONDecoder().decode(AgentCardRow.self, from: Data(json.utf8))
            #expect(row.traceAnchor == nil, "\(wire) reached the row")
        }
    }
}
