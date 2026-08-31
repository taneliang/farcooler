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

    /// An agent's mark can never be the review tier.
    ///
    /// `reviewsWaiting` is a fleet-wide scalar and `unreadDiff` is per
    /// WORKSPACE; `ShellScreen` and `ShellNavigation` both refuse to invent a
    /// per-agent version. A cyan ring on an agent would mean "this agent has
    /// unread changes", which is not a fact anything on the wire has an opinion
    /// about.
    @Test func noAgentIsEverInTheReviewTier() {
        for status in ["blocked", "working", "done", "something new"] {
            let agent = FleetSnapshot.Agent(
                id: "t", label: "claude", machine: "orchard", status: status, glyph: "●",
                headline: "", line: "", feed: [], rank: 0, turnFailed: false,
                activityChangedAt: nil)
            #expect(GlanceMark(agent: agent).attention != .toReview)
        }
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
