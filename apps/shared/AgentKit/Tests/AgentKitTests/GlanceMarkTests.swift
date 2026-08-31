import Foundation
import Testing

@testable import AgentKit

/// The glance spec's literal values, held down.
///
/// This suite exists for one reason and it is written in the spec itself: "Do
/// not copy values out of them into your own constants file and then edit them
/// there — that duplication is what produced eight rounds of drift while these
/// documents were being written, and the same failure mode will hit the
/// codebase." The values now live in exactly one place; these are the
/// assertions that notice when somebody edits them there rather than in the
/// design document.
///
/// They are deliberately a transcription of §03's stroke and core tables rather
/// than a rederivation of them. A test that computed the expected stroke from a
/// ratio would agree with a ratio-based implementation and disagree with the
/// spec, which is the exact mistake §03 rules out — "Stroke is a literal value
/// per diameter, never a percentage."
struct GlanceMarkTests {
    /// §03: "On a phone: diameter 8pt in a ribbon, 10pt in a row, 11pt in a
    /// header, 15pt as a lone indicator … On a wrist … 14pt in a row … 22pt as
    /// a lone indicator … Six sizes across the two bodies, no others."
    @Test func thereAreSixDiametersAndNoOthers() {
        #expect(GlanceMarkSize.allCases.count == 6)
        #expect(GlanceMarkSize.allCases.map(\.diameter) == [8, 10, 11, 15, 14, 22])
    }

    /// §03: "Needs you: 2 / 2.5 / 2.5 / 3.5. To review: 2 / 2 / 2 / 3.
    /// Hairline: 1 at every size." Read down the phone's four diameters.
    @Test func thePhonesThreeStrokeLaddersAreTheSpecs() {
        let phone: [GlanceMarkSize] = [.ribbon, .row, .header, .lone]
        #expect(phone.map { $0.stroke(.needsYou) } == [2, 2.5, 2.5, 3.5])
        #expect(phone.map { $0.stroke(.toReview) } == [2, 2, 2, 3])
        #expect(phone.map { $0.stroke(.quiet) } == [1, 1, 1, 1])
    }

    /// §03: "14pt in a row — strokes 3 / 2.5 / 1, core 5 — and 22pt as a lone
    /// indicator — strokes 5 / 4 / 1.5, core 8".
    ///
    /// The 22pt hairline is 1.5, which is the one place the spec states
    /// "Hairline: 1 at every size" and then an exception in the same section.
    /// The explicit triple wins; see `GlanceMarkSize.stroke(_:)`.
    @Test func theWristCarriesItsOwnTwoTriples() {
        #expect(GlanceMarkSize.watchRow.stroke(.needsYou) == 3)
        #expect(GlanceMarkSize.watchRow.stroke(.toReview) == 2.5)
        #expect(GlanceMarkSize.watchRow.stroke(.quiet) == 1)
        #expect(GlanceMarkSize.watchLone.stroke(.needsYou) == 5)
        #expect(GlanceMarkSize.watchLone.stroke(.toReview) == 4)
        #expect(GlanceMarkSize.watchLone.stroke(.quiet) == 1.5)
    }

    /// §03: "At 8pt those first two collapse to one weight: there is no room
    /// for four distinguishable strokes, so an 8pt ribbon separates wants you
    /// from quiet and leaves amber-versus-review to hue."
    ///
    /// Checked as a property rather than as two numbers, because the property
    /// is what the fallback IS — and because the same property must NOT hold
    /// one rung up, or the three-step ladder §03 promises "from 10pt up" has
    /// quietly become a two-step one.
    @Test func onlyTheRibbonCollapsesTheTwoAttentionWeights() {
        #expect(GlanceMarkSize.ribbon.stroke(.needsYou) == GlanceMarkSize.ribbon.stroke(.toReview))
        for size in GlanceMarkSize.allCases where size != .ribbon {
            #expect(size.stroke(.needsYou) > size.stroke(.toReview))
            #expect(size.stroke(.toReview) > size.stroke(.quiet))
        }
    }

    /// §03: "Core is a literal value too: 3 at 8pt, 4 at 11pt, 5 at 15pt.
    /// There is no 10pt core — a row shows the ring alone". Plus the wrist's 5
    /// at 14pt and 8 at 22pt.
    @Test func theCoreTableIncludesItsOneHole() {
        #expect(GlanceMarkSize.ribbon.core == 3)
        #expect(GlanceMarkSize.row.core == nil)
        #expect(GlanceMarkSize.header.core == 4)
        #expect(GlanceMarkSize.lone.core == 5)
        #expect(GlanceMarkSize.watchRow.core == 5)
        #expect(GlanceMarkSize.watchLone.core == 8)
    }

    /// Every core fits inside its own heaviest ring with room left over.
    ///
    /// The arithmetic nobody does by hand: the ring is drawn INSIDE the
    /// diameter, so the hole a core sits in is `diameter - 2 * stroke`, and the
    /// heaviest ring is the needs-you one. A core equal to the hole touches the
    /// ring on both sides and the mark reads as a solid disc — which is the one
    /// state the mark must never be mistaken for, since a filled amber dot is
    /// what this product used to draw for "needs you".
    @Test func everyCoreFitsInsideItsHeaviestRing() {
        for size in GlanceMarkSize.allCases {
            guard let core = size.core else { continue }
            let hole = size.diameter - 2 * size.stroke(.needsYou)
            #expect(core < hole, "core \(core) does not clear the hole \(hole) at \(size)")
        }
    }

    /// The three axes are independent, which is what "twelve states from three
    /// rules, so a combination you have not met still reads correctly" means in
    /// code.
    @Test func twelveStatesComeOutOfThreeRules() {
        var seen = Set<GlanceMark>()
        for a in [GlanceMark.Attention.needsYou, .toReview, .quiet] {
            for c in [GlanceMark.Core.producing, .atAPrompt] {
                for l in [GlanceMark.Link.live, .broken] {
                    seen.insert(GlanceMark(attention: a, core: c, link: l))
                }
            }
        }
        #expect(seen.count == 12)
    }

    /// `done` is the only status that reaches the review tier, and every other
    /// one is still shut out of it.
    ///
    /// **This test used to say no agent could ever be `.toReview`.** The
    /// prohibition was narrowed rather than lifted, so the guard is narrowed
    /// with it rather than deleted: `reviewsWaiting` is still a fleet-wide
    /// scalar and `unreadDiff` is still per WORKSPACE, `ShellScreen` and
    /// `ShellNavigation` still refuse to invent a per-agent version of either,
    /// and a ring meaning "this agent has unread changes" would still be a fact
    /// nothing on the wire has an opinion about. What changed is that `done` was
    /// never that fact — the daemon sends it per terminal, so nothing is
    /// invented — and the tier now says "a finished thing nobody has looked at".
    ///
    /// The case this exists for: a turn that ended, unlooked-at, must not be
    /// spoken as "Nothing wanted".
    @Test func theReviewTierIsAFinishedTurnAndNothingElse() {
        #expect(Self.mark("done").attention == .toReview)
        // The turn is over, so the agent is not producing.
        #expect(Self.mark("done").core == .atAPrompt)
        #expect(Self.mark("done").phrase == "To review, at a prompt")
        #expect(Self.mark("done").isQuiet == false)

        for status in ["blocked", "working", "something new", "idle", ""] {
            #expect(
                Self.mark(status).attention != .toReview,
                "\(status) claimed the review tier")
        }
    }

    /// Hue is redundant reinforcement — §03 — and the accessory families
    /// flatten to a single tint, so a finished turn has to be separable from a
    /// quiet one by the drawing alone.
    @Test func aFinishedTurnReadsWithItsHueRemoved() {
        let done = Self.mark("done")
        let quiet = Self.mark("idle")
        for size in GlanceMarkSize.allCases {
            #expect(
                size.stroke(done.attention) > size.stroke(quiet.attention),
                "\(size) draws a finished turn at a quiet turn's weight")
        }
        // And not at the heaviest weight either, above the 8pt ribbon where
        // §03 deliberately collapses the two attention strokes into one.
        for size in GlanceMarkSize.allCases where size != .ribbon {
            #expect(
                size.stroke(done.attention) < size.stroke(.needsYou),
                "\(size) draws a finished turn as loudly as a blocked one")
        }
    }

    private static func mark(_ status: String) -> GlanceMark {
        GlanceMark(
            agent: FleetSnapshot.Agent(
                id: "t", label: "claude", machine: "orchard", status: status, glyph: "●",
                headline: "", line: "", feed: [], rank: 0, turnFailed: false,
                activityChangedAt: nil))
    }

    /// A status the daemon invents later must not become an attention tier
    /// nobody chose — see `FleetSnapshot.Agent.status`, which is a String
    /// precisely so an unknown word cannot take the snapshot down.
    @Test func anUnknownStatusLandsOnTheQuietestTier() {
        let agent = FleetSnapshot.Agent(
            id: "t", label: "claude", machine: "orchard", status: "compacting", glyph: "●",
            headline: "", line: "", feed: [], rank: 0, turnFailed: false, activityChangedAt: nil)
        #expect(GlanceMark(agent: agent).attention == .quiet)
        #expect(GlanceMark(agent: agent).core == .atAPrompt)
    }

    /// A snapshot that has stopped vouching breaks the RING and leaves the core
    /// alone — §03: "a broken ring is a broken link and never disturbs the
    /// core."
    @Test func stalenessBreaksTheRingAndNotTheCore() {
        let agent = FleetSnapshot.Agent(
            id: "t", label: "claude", machine: "orchard", status: "working", glyph: "●",
            headline: "", line: "", feed: [], rank: 0, turnFailed: false, activityChangedAt: nil)
        let fresh = GlanceMark(agent: agent, confidence: .known)
        let stale = GlanceMark(agent: agent, confidence: .lastSeen)
        #expect(fresh.link == .live)
        #expect(stale.link == .broken)
        #expect(stale.core == fresh.core)
        #expect(stale.attention == fresh.attention)
    }

    /// A widget states no core at all — §08: "Working versus idle never appears
    /// on a widget."
    @Test func aWidgetsMarkStatesNoCore() {
        let mark = GlanceMark(glance: .working(4)).withoutCore
        #expect(mark.core == nil)
        // And it says so out loud rather than claiming the agent is at a
        // prompt, which is what the same drawing means when a surface DOES
        // state the axis.
        #expect(mark.phrase == "Nothing wanted")
        #expect(GlanceMark(attention: .quiet, core: .atAPrompt).phrase == "Nothing wanted, at a prompt")
    }

    /// Stroke weight reaches VoiceOver through nothing but words, so every tier
    /// has to have some.
    @Test func everyTierIsSayableInWords() {
        #expect(GlanceMark(glance: .blocked(2)).phrase == "Needs you, at a prompt")
        #expect(GlanceMark(glance: .review(3)).phrase == "To review, at a prompt")
        #expect(GlanceMark(glance: .working(4)).phrase == "Nothing wanted, producing")
        #expect(
            GlanceMark(attention: .needsYou, core: .producing, link: .broken).phrase
                == "Needs you, unreachable")
    }
}

/// §02's type table and the one time format it allows.
struct GlanceTypeTests {
    /// §02, read down the SIZE / WEIGHT column. Tracking is stored in points,
    /// converted from the spec's ems at each row's own size.
    @Test func theTypeScaleIsTheSpecs() {
        #expect(GlanceType.headline.size == 17)
        #expect(GlanceType.headline.tracking == -0.016 * 17)
        #expect(GlanceType.cardHeader.size == 15)
        #expect(GlanceType.rowName.size == 13)
        #expect(GlanceType.terminal.size == 12.5)
        #expect(GlanceType.secondary.size == 11)
        #expect(GlanceType.count().size == 44)
        #expect(GlanceType.count().tracking == -0.035 * 44)
    }

    /// §01's floor: "nothing below 11px … treat both numbers as hard."
    @Test func nothingIsBelowTheElevenPointFloor() {
        let rows = [
            GlanceType.headline, .cardHeader, .rowName, .terminal, .secondary, .monoFigures,
            .count(),
        ]
        for row in rows { #expect(row.size >= 11) }
    }

    /// §02: "38–46 / 600 … The single count on a widget." A family that asks
    /// for more gets the largest size the system has rather than one nobody
    /// specified.
    @Test func theWidgetCountIsClampedToItsRange() {
        #expect(GlanceType.count(10).size == 38)
        #expect(GlanceType.count(100).size == 46)
        #expect(GlanceType.count(40).size == 40)
    }

    /// §02: "If it came off a machine it is mono … If a person wrote it, or the
    /// product did, it is SF."
    @Test func onlyMachineRowsAreMono() {
        #expect(GlanceType.terminal.mono)
        #expect(GlanceType.monoFigures.mono)
        #expect(!GlanceType.headline.mono)
        #expect(!GlanceType.secondary.mono)
    }

    /// §02: "Relative and coarse: 2m ago, 52m ago, 3h ago."
    @Test func ageIsRelativeAndCoarse() {
        #expect(GlanceAge.brief(30) == "now")
        #expect(GlanceAge.brief(120) == "2m")
        #expect(GlanceAge.brief(52 * 60) == "52m")
        #expect(GlanceAge.brief(3 * 3600) == "3h")
        #expect(GlanceAge.brief(50 * 3600) == "2d")
        #expect(GlanceAge.stated(120) == "2m ago")
        #expect(GlanceAge.stated(30) == "just now")
        // Never a negative age from a clock that has drifted backwards.
        #expect(GlanceAge.brief(-500) == "now")
    }

    /// §08: "Fresh — under two minutes." Distinct from
    /// `FleetSnapshot.staleAfter`, which is an hour and answers a different
    /// question.
    @Test func freshIsTwoMinutesAndIsNotTheStalenessHour() {
        #expect(GlanceAge.fresh == 120)
        #expect(GlanceAge.fresh != FleetSnapshot.staleAfter)
    }
}

/// §04's activity trace: the bytes, and the geometry drawn from them.
///
/// Two suites' worth of subject in one file for the reason `GlanceMark.swift`
/// holds both drawings — the spec's title is "One mark, one graphic" and these
/// are the two. Same discipline as the suite above: every assertion is a
/// TRANSCRIPTION of §04, never a rederivation, so a figure edited in the code
/// disagrees with the test instead of agreeing with it.
struct ActivityTraceTests {
    /// The wire's 66 bytes, built the way `farcooler_core::trace::Trace::encode`
    /// builds them.
    ///
    /// Written out here rather than imported from anywhere, because there is
    /// nothing to import: the producer is Rust and this is the only Swift that
    /// has ever had an opinion about the layout. If the two ever disagree, this
    /// function is the half that is wrong, and `crates/core/src/trace.rs` is
    /// where to read the layout comment.
    static func encoded(
        code: [UInt16] = Array(repeating: 0, count: 13),
        output: [UInt16] = Array(repeating: 0, count: 13),
        commits: [UInt8] = Array(repeating: 0, count: 13),
        version: UInt8 = 1,
        width: UInt8 = 0
    ) -> Data {
        var bytes = Data([(version << 4) | width])
        // Little-endian, which is the layout comment's word and NOT the obvious
        // guess for a wire format. Getting this backwards produces a trace whose
        // every bar is plausible and wrong.
        for value in code { bytes.append(UInt8(value & 0xFF)); bytes.append(UInt8(value >> 8)) }
        for value in output { bytes.append(UInt8(value & 0xFF)); bytes.append(UInt8(value >> 8)) }
        bytes.append(contentsOf: commits)
        return bytes
    }

    /// `1 + 13*2 + 13*2 + 13`, which is `farcooler_core::trace::ENCODED_LEN`.
    @Test func theWireIsThirteenBucketsInSixtySixBytes() {
        #expect(ActivityTrace.buckets == 13)
        #expect(ActivityTrace.encodedLength == 66)
        #expect(Self.encoded().count == ActivityTrace.encodedLength)
    }

    /// **The negative control the whole feature turns on.**
    ///
    /// `proto/farcooler.proto`: "Empty when the terminal has done nothing the
    /// trace can see, rather than 66 zero bytes: a fleet at rest costs nothing."
    /// So an absent trace and a silent one are two different facts and must be
    /// two different drawings — nothing at all, against §04's thirteen quiet
    /// buckets, "Drawn, not omitted".
    ///
    /// A decoder that treated the empty field as a zeroed trace would draw a
    /// flat line at zero for every terminal that has never done anything, which
    /// is a claim of thirteen observed buckets of silence about a terminal
    /// nobody observed.
    @Test func anEmptyTraceIsAbsentAndAZeroedOneIsFlat() {
        #expect(ActivityTrace(nil) == nil, "no field at all has to be no trace")
        #expect(ActivityTrace(Data()) == nil, "an empty field has to be no trace")

        // ...and the other side of it, which is what stops the fix for the above
        // from being "refuse anything quiet".
        let silent = ActivityTrace(Self.encoded())
        #expect(silent != nil, "sixty-six zeroes is a real trace of quiet buckets")
        let layout = GlanceTraceLayout(size: CGSize(width: 76, height: 21))
        for bucket in 0..<ActivityTrace.buckets {
            for half in GlanceTraceLayout.Half.allCases {
                let bar = layout.bar(bucket, half, in: silent!)
                #expect(!bar.lit, "a bucket with nothing in it is not lit")
                #expect(bar.height == GlanceTraceLayout.minimumBar, "but it is still drawn")
            }
        }
    }

    /// **The second negative control.**
    ///
    /// `proto/farcooler.proto`: "A client that does not recognize the version
    /// must draw an empty trace rather than read the rest." A partial parse of
    /// an encoding this build does not know is a bar chart of somebody else's
    /// bytes, and it would look entirely plausible.
    ///
    /// The bodies here are deliberately NOT zeroes: a decoder that read them
    /// anyway would produce a trace with visible bars, so the assertion fails
    /// loudly rather than on a technicality.
    @Test func aVersionThisBuildDoesNotKnowIsRefusedWhole() {
        let loud = Array(repeating: UInt16(400), count: 13)
        #expect(ActivityTrace(Self.encoded(code: loud, version: 2)) == nil)
        #expect(ActivityTrace(Self.encoded(code: loud, version: 15)) == nil)
        // Version 0 is nobody's encoding — `ENCODING_VERSION` starts at 1 — so
        // a zero high nibble is a corrupt byte rather than an older producer.
        #expect(ActivityTrace(Self.encoded(code: loud, version: 0)) == nil)
        // And the width code on the same terms. Version 1 has exactly three
        // widths because `WIDTHS` has three entries, so a fourth is a later
        // encoding whose author forgot the nibble above it.
        #expect(ActivityTrace(Self.encoded(code: loud, width: 3)) == nil)
        // The one it does know still decodes, so the test above is refusing the
        // version rather than refusing everything.
        #expect(ActivityTrace(Self.encoded(code: loud, version: 1)) != nil)
    }

    /// A length that is not 66 is not a trace, whatever the version byte says.
    @Test func aTruncatedTraceIsRefused() {
        let whole = Self.encoded(code: Array(repeating: 7, count: 13))
        #expect(ActivityTrace(whole) != nil)
        #expect(ActivityTrace(whole.dropLast()) == nil)
        #expect(ActivityTrace(whole + Data([0])) == nil)
        #expect(ActivityTrace(whole.prefix(1)) == nil)
    }

    /// The three series come out of the three runs, in the byte order the
    /// producer wrote them.
    ///
    /// Distinct values in every slot, so a decoder that read the output run as
    /// the code run, or read big-endian, or lost the oldest-first ordering,
    /// fails here rather than in a screenshot.
    @Test func theThreeSeriesAreReadFromTheirOwnRuns() throws {
        let code = (0..<13).map { UInt16(($0 + 1) * 100) }
        let output = (0..<13).map { UInt16(($0 + 1) * 7) }
        let commits = (0..<13).map { UInt8($0) }
        let trace = try #require(
            ActivityTrace(Self.encoded(code: code, output: output, commits: commits)))
        #expect((0..<13).map { trace.code($0) } == code)
        #expect((0..<13).map { trace.output($0) } == output)
        #expect((0..<13).map { trace.commits($0) } == commits)
        #expect(trace.tallestCode == 1300)
        #expect(trace.tallestOutput == 91)
    }

    /// A `u16` past 255 proves the byte order rather than assuming it: 0x0102
    /// little-endian is `02 01`, and a big-endian reader would call it 0x0201.
    @Test func theSeriesAreLittleEndian() throws {
        var code = Array(repeating: UInt16(0), count: 13)
        code[4] = 0x0102
        let trace = try #require(ActivityTrace(Self.encoded(code: code)))
        #expect(trace.code(4) == 0x0102)
        // The byte the producer wrote first is the LOW half.
        let bytes = Self.encoded(code: code)
        #expect(bytes[1 + 4 * 2] == 0x02)
        #expect(bytes[1 + 4 * 2 + 1] == 0x01)
    }

    /// §04: "Snaps to 1h / 6h / 24h — the shortest window containing the
    /// activity — with the span printed as two mono characters beside the
    /// trace." The three width codes are `WIDTHS`' three positions.
    @Test func theWidthCodePicksTheSpan() throws {
        let spans = try (0...2).map { code -> ActivityTrace.Span in
            try #require(ActivityTrace(Self.encoded(width: UInt8(code)))).span
        }
        #expect(spans == [.hour, .sixHours, .day])
        #expect(spans.map(\.label) == ["1h", "6h", "24h"])
    }

    // MARK: - §04's geometry

    /// §04: "Bands (h−3)/2 — Equal halves either side of a 3pt axis."
    @Test func theBandsAreEqualHalvesEitherSideOfAThreePointAxis() {
        #expect(GlanceTraceLayout.axis == 3)
        let layout = GlanceTraceLayout(size: CGSize(width: 156, height: 41))
        // The arithmetic is spelled in CGFloat on both sides. `#expect`
        // captures its operands separately, which breaks the inference that
        // would otherwise make `(41 - 3) / 2` a CGFloat, and the resulting
        // heterogeneous comparison silently answered FALSE for 19.0 against 19
        // — a test that fails for a reason that has nothing to do with its
        // subject is the same defect as one that passes for one.
        #expect(layout.band == CGFloat(41 - 3) / 2)
        #expect(layout.axisRect == CGRect(x: 0, y: 19, width: 156, height: 3))
    }

    /// §04: "Bars are flex: 1, so they always fill the width exactly" and "Gap
    /// 0. No gaps. The fused silhouette is the point."
    ///
    /// The last NEVER of §04 is what this is really holding down: "Never leave
    /// the container wider than the bars — trailing space sits at the recent end
    /// and reads as dead activity." A width of 76 over 13 columns does not
    /// divide, so a `width/13` implementation leaves 0.4pt of nothing at the
    /// newest end — which is the end a person reads.
    @Test func thirteenColumnsTileTheWidthExactlyAndTouch() {
        for width in [CGFloat(40), 44, 52, 76, 156] {
            let layout = GlanceTraceLayout(size: CGSize(width: width, height: 21))
            #expect(layout.column(0).x == 0)
            for bucket in 0..<(ActivityTrace.buckets - 1) {
                let here = layout.column(bucket)
                let next = layout.column(bucket + 1)
                #expect(abs(here.x + here.width - next.x) < 0.0001, "gap 0, at \(width)")
            }
            let last = layout.column(ActivityTrace.buckets - 1)
            #expect(abs(last.x + last.width - width) < 0.0001, "no dead space at \(width)")
        }
    }

    /// §04: "Tallest bar = that row's busiest bucket, per half. Each half
    /// therefore reaches full height, so the two halves are not comparable to
    /// one another — only each against its own past."
    ///
    /// The two halves here are deliberately three orders of magnitude apart. A
    /// scale shared between them — which is the obvious implementation, and the
    /// wrong one — would draw the smaller half as thirteen invisible slivers.
    ///
    /// **Run twice with the halves swapped**, and that is not symmetry for its
    /// own sake. Breaking only the `.code` arm of the implementation while the
    /// loud half WAS the code half changed nothing and the test still passed —
    /// a shared denominator is invisible from whichever side happens to supply
    /// it. Each half has to be the quiet one once.
    @Test(arguments: [false, true]) func eachHalfIsScaledAgainstItsOwnBusiestBucket(
        swapped: Bool
    ) throws {
        var loud = Array(repeating: UInt16(0), count: 13)
        var quiet = Array(repeating: UInt16(0), count: 13)
        loud[0] = 1000
        loud[1] = 500
        quiet[0] = 4
        quiet[1] = 2
        let trace = try #require(
            ActivityTrace(
                Self.encoded(
                    code: swapped ? quiet : loud, output: swapped ? loud : quiet)))
        let layout = GlanceTraceLayout(size: CGSize(width: 76, height: 23))
        let band = layout.band
        for half in GlanceTraceLayout.Half.allCases {
            #expect(
                layout.bar(0, half, in: trace).height == band,
                "the busiest bucket of \(half) reaches full height")
            #expect(layout.bar(1, half, in: trace).height == band / 2)
        }
    }

    /// §04: "up is code, down is chat". The upper half grows up from the axis
    /// and the lower half down from it, so the two never overlap the rule.
    @Test func upIsCodeAndDownIsOutput() throws {
        var code = Array(repeating: UInt16(0), count: 13)
        var output = Array(repeating: UInt16(0), count: 13)
        code[6] = 10
        output[6] = 10
        let trace = try #require(ActivityTrace(Self.encoded(code: code, output: output)))
        let layout = GlanceTraceLayout(size: CGSize(width: 76, height: 23))
        let upper = layout.barRect(6, .code, in: trace)
        let lower = layout.barRect(6, .output, in: trace)
        #expect(upper.maxY == layout.band, "the code half ends where the axis begins")
        #expect(lower.minY == layout.band + GlanceTraceLayout.axis, "and chat begins after it")
        #expect(upper.minY == 0, "a full bar reaches the top edge")
        #expect(lower.maxY == 23, "and the other reaches the bottom")
    }

    /// §01: "A bucket with no activity. Drawn, not omitted." And §05, about the
    /// medium tile: "docs-sweep has an empty upper half — it has talked and
    /// touched nothing, drawn rather than omitted."
    ///
    /// A lit bucket that scales below a point is still lit. It is a real
    /// observation, and rounding it to nothing would delete it — which is a
    /// different thing from a bucket that observed nothing.
    @Test func aSilentBucketIsDrawnAndATinyOneIsStillLit() throws {
        var code = Array(repeating: UInt16(0), count: 13)
        code[0] = 60_000
        code[1] = 1
        let trace = try #require(ActivityTrace(Self.encoded(code: code)))
        let layout = GlanceTraceLayout(size: CGSize(width: 76, height: 23))
        let tiny = layout.bar(1, .code, in: trace)
        #expect(tiny.lit, "one line is an observation")
        #expect(tiny.height == GlanceTraceLayout.minimumBar)
        let silent = layout.bar(2, .code, in: trace)
        #expect(!silent.lit)
        #expect(silent.height == GlanceTraceLayout.minimumBar, "drawn, not omitted")
        // The whole lower half of this row is silent, which is §05's docs-sweep
        // case with the halves swapped.
        #expect((0..<13).allSatisfy { !layout.bar($0, .output, in: trace).lit })
    }

    /// §04: "Commits 3pt block. On the axis. Unlit buckets get height 0 — an
    /// unset height stretches." And the floor: "36pt wide — Below this, commit
    /// marks stop separating and come off."
    @Test func commitsAreThreePointBlocksThatComeOffBelowTheFloor() throws {
        var commits = Array(repeating: UInt8(0), count: 13)
        commits[3] = 2
        let trace = try #require(ActivityTrace(Self.encoded(commits: commits)))
        #expect(GlanceTraceLayout.commitFloor == 36)
        #expect(GlanceTraceLayout.commitBlock == 3)

        let wide = GlanceTraceLayout(size: CGSize(width: 40, height: 13))
        #expect(wide.drawsCommits)
        let mark = try #require(wide.commitRect(3, in: trace))
        #expect(mark.height == 3)
        #expect(mark.minY == wide.band, "on the axis")
        #expect(mark.width == wide.column(3).width, "the bucket's own column")
        #expect(wide.commitRect(4, in: trace) == nil, "an unlit bucket gets no block")

        // One point under the floor and every mark comes off — removed, not
        // shrunk.
        let narrow = GlanceTraceLayout(size: CGSize(width: 35, height: 13))
        #expect(!narrow.drawsCommits)
        #expect(narrow.commitRect(3, in: trace) == nil)
    }

    /// §04: "40 · island / 44 · island row / 52 · card row / 76 · widget. Four
    /// shipping sizes, one form."
    ///
    /// The heights are NOT quoted — §04 leaves `h` to the surface — so what is
    /// asserted here is that they come from one rule rather than four guesses:
    /// the specimen's own proportion, 19pt of band at 156pt wide.
    @Test func thereAreFourShippingSizesAndNoOthers() {
        #expect(GlanceTraceSize.allCases.count == 4)
        #expect(GlanceTraceSize.allCases.map(\.width) == [40, 44, 52, 76])
        for size in GlanceTraceSize.allCases {
            let band = (size.width * 19 / 156).rounded()
            #expect(size.height == GlanceTraceLayout.axis + 2 * band)
        }
        // All four clear §04's commit floor, which is why the spec can list
        // them together as one form: none of them has to drop the marks.
        #expect(
            GlanceTraceSize.allCases.allSatisfy {
                GlanceTraceLayout(size: CGSize(width: $0.width, height: $0.height)).drawsCommits
            })
    }
}
