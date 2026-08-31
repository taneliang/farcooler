import Foundation
import SwiftUI

// The state mark: one drawing, on every surface that says what an agent is
// doing.
//
// **Ring is your side, core is the agent's.** That sentence is the whole of
// §03 of the design spec, and everything in this file follows from it:
//
//   - The RING's weight and hue are the person's side — is your attention
//     wanted, and what for. Three weights: needs-you, to-review, hairline.
//   - The CORE is the agent's — filled while producing, absent at a prompt.
//   - DASHES go on the ring, because the ring IS the channel. A broken ring is
//     a broken link, and it never disturbs the core.
//
// Three rules, twelve states, "so a combination you have not met still reads
// correctly". That last clause is why this is a struct of three independent
// axes rather than an enum of twelve cases: an enum has to have every
// combination written down before it can be drawn, and the one nobody wrote
// down is the one that turns up on a phone.
//
// **What this replaced.** Four surfaces drew four different things and shared
// nothing but the colour orange: `ShellMarkView` drew a capsule with four
// hard-coded states, the widget rows printed `agent.glyph` — a literal
// character off the wire — the circular accessory stacked an SF Symbol over a
// count, and the Live Activity and the watch complication each had their own
// again. A person seeing two of those within a minute of each other had no way
// to tell they were about the same thing.
//
// **It must typecheck for `arm64_32-apple-watchos`**, for the reason given in
// `apps/ios/generate-project.py:274-281`: the watch compiles AgentKit one file
// at a time and never the package. Plain SwiftUI shapes and `Double`
// arithmetic only.

/// What one mark says, on all three of its axes.
public struct GlanceMark: Hashable, Sendable {
    /// The ring: whether your attention is wanted, and what for.
    ///
    /// Weight AND hue together, never one without the other, because the
    /// accessory families flatten to a single tint and the mark has to keep
    /// working with hue removed — §03: "the mark distinguishes states by stroke
    /// weight, fill and dash, so hue was always redundant reinforcement."
    public enum Attention: Hashable, Sendable {
        /// An agent is stopped, waiting on a person. The heaviest ring, and
        /// the only amber thing in the product.
        case needsYou
        /// A finished diff nobody has read. A middle-weight ring in the low
        /// chroma `GlancePalette.review`.
        ///
        /// **Never an agent's state.** `reviewsWaiting` is a fleet-wide scalar
        /// and `unreadDiff` is per WORKSPACE; `ShellScreen.swift:141-147` and
        /// `ShellNavigation.swift:116-124` both refuse to invent a per-agent
        /// version, so this tier belongs to a fleet-level count or to a Diff
        /// tab and to nothing else. The convenience initialiser from
        /// `FleetSnapshot.Agent` below cannot produce it, on purpose.
        case toReview
        /// Nothing is wanted from you. A 1pt hairline at every size.
        case quiet
    }

    /// The core: what the agent itself is doing.
    ///
    /// Two values and no third, because §03 gives the core exactly one job.
    /// "Not stated" is `nil` on the `core` property rather than a case here —
    /// see that property.
    public enum Core: Hashable, Sendable {
        /// Producing. A filled disc at the centre.
        case producing
        /// At a prompt. No disc.
        case atAPrompt
    }

    /// The ring's continuity: whether the channel is currently carrying
    /// anything.
    public enum Link: Hashable, Sendable {
        case live
        /// Unreachable, or a claim about the present that has gone stale.
        /// Drawn as a dashed ring.
        ///
        /// One axis for both, because they are one fact: §08's staleness rule
        /// is that "decay applies only to claims about the present. Blocked and
        /// to-review hold at any age; working and idle go dashed" — which is
        /// the same statement as a broken link, made about time instead of
        /// about the network.
        case broken
    }

    public var attention: Attention

    /// What the agent is doing, or nil where this surface may not say.
    ///
    /// **Nil is what a widget uses, and it is not a thirteenth state.** §08 is
    /// explicit: "Working versus idle never appears on a widget. It flips every
    /// few seconds; at this refresh rate the claim would be false more often
    /// than true." A widget reloads about once every twenty minutes, so a core
    /// drawn there would be a statement about the present made from a snapshot
    /// that is usually a quarter of an hour old.
    ///
    /// So the twelve states are still the drawn vocabulary; nil is a SURFACE
    /// declining to state one of the three axes, which is a different thing
    /// from stating that the agent is at a prompt. It draws the same as
    /// `.atAPrompt` — there is no fourth thing a core can look like — but it
    /// says nothing in the accessibility label, where the difference is
    /// audible.
    public var core: Core?

    public var link: Link

    public init(attention: Attention, core: Core?, link: Link = .live) {
        self.attention = attention
        self.core = core
        self.link = link
    }

    /// Whether this mark is one of the ones a vibrancy fallback drops.
    ///
    /// §03's stated concession: "If 1pt hairlines wash out under vibrancy,
    /// quiet marks leave the accessory ribbon and only the attention marks are
    /// drawn; the count text carries the rest." A ribbon that has to make that
    /// trade filters on this rather than re-deriving what "quiet" means.
    public var isQuiet: Bool { attention == .quiet }

    // MARK: - Where a mark comes from

    /// One agent, as the wire describes it.
    ///
    /// **`status` is a String on the wire and stays one** — see
    /// `FleetSnapshot.Agent.status`, which explains that the daemon and the
    /// relay are not Swift and that a word none of them knew about must not
    /// take the whole snapshot down. So an unrecognised status lands on the
    /// quiet hairline, which is the tier that claims the least.
    ///
    /// This can never return `.toReview`. See `Attention.toReview`.
    public init(agent: FleetSnapshot.Agent, confidence: FleetSnapshot.Confidence = .known) {
        switch agent.status {
        // Blocked is latched — an agent stopped an hour ago is still stopped —
        // so it keeps its heavy ring however old the snapshot is, and the core
        // is genuinely absent rather than merely unstated: being at a prompt is
        // what "blocked" means.
        case "blocked": self.init(attention: .needsYou, core: .atAPrompt)
        case "working": self.init(attention: .quiet, core: .producing)
        default: self.init(attention: .quiet, core: .atAPrompt)
        }
        // Only the claim about the present decays. `confidence(in:at:)` has
        // already applied that rule — it vouches for blocked and done at any
        // age and stops vouching for working — so this is the ring following
        // the answer that type gives rather than a second staleness rule.
        if confidence == .lastSeen { link = .broken }
    }

    /// The fleet, as a surface with room for one thing sees it.
    ///
    /// `FleetSnapshot.Glance` is where the precedence "blocked beats review
    /// beats working" is decided (`FleetSnapshot.swift:556`), so this is a
    /// translation and not a second opinion.
    public init(glance: FleetSnapshot.Glance) {
        switch glance {
        case .blocked: self.init(attention: .needsYou, core: .atAPrompt)
        case .review: self.init(attention: .toReview, core: .atAPrompt)
        case .working: self.init(attention: .quiet, core: .producing)
        }
    }

    /// The same, with the core withheld — what every widget family draws.
    ///
    /// See `core` for why. Spelled as a function on the value rather than left
    /// to each call site, so that "widgets do not state working-versus-idle"
    /// is one decision in one place instead of a rule six families have to
    /// remember.
    public var withoutCore: GlanceMark {
        GlanceMark(attention: attention, core: nil, link: link)
    }

    // MARK: - Words

    /// The tier, in words, for VoiceOver.
    ///
    /// **Stroke weight is not exposed to VoiceOver, and neither is hue.** The
    /// entire mark is a difference in line width, fill and dash — which is to
    /// say the entire mark is invisible to a screen reader unless it is also
    /// said out loud. The words are the matrix's own row and column labels from
    /// §03, so what is spoken and what is drawn are the same table.
    public var phrase: String {
        let tier =
            switch attention {
            case .needsYou: "Needs you"
            case .toReview: "To review"
            case .quiet: "Nothing wanted"
            }
        if link == .broken { return "\(tier), unreachable" }
        switch core {
        case .producing: return "\(tier), producing"
        case .atAPrompt: return "\(tier), at a prompt"
        // The surface declined to say. Saying "at a prompt" here would be this
        // file inventing the very claim `core == nil` exists to withhold.
        case nil: return tier
        }
    }
}

/// The six diameters this mark is drawn at, and no others.
///
/// §03: "Six sizes across the two bodies, no others." An enum rather than a
/// `CGFloat` because that sentence is a rule, and a rule expressed as a
/// parameter is a rule anybody can be one typo away from breaking.
///
/// **Stroke is a literal value per diameter, never a percentage.** §03 gives
/// the reason and it is not stylistic: "a ratio cannot land on the half-pixel
/// grid at these sizes." A 15pt mark whose ring is 22% of its diameter is
/// 3.3pt, which on a 3× screen is 9.9 device pixels and gets antialiased into
/// a smear; 3.5pt is 10.5 and lands on the half-pixel grid the renderer snaps
/// to. Every number in `metrics` below is quoted from the spec.
public enum GlanceMarkSize: Hashable, Sendable, CaseIterable {
    /// 8pt, in a ribbon. On a phone.
    case ribbon
    /// 10pt, in a row. On a phone.
    case row
    /// 11pt, in a header. On a phone.
    case header
    /// 15pt, as a lone indicator. On a phone — and the size §06 sends
    /// `accessoryCircular` to, because that is a lone indicator over a
    /// wallpaper.
    case lone
    /// 14pt, in a row. On a wrist — which is read at arm's length, so it has
    /// only two sizes rather than four. `accessoryRectangular` takes this one,
    /// "because it IS a row: mark, label, trace."
    case watchRow
    /// 22pt, as a lone indicator. On a wrist. The detail header and
    /// `accessoryCircular`.
    case watchLone

    public var diameter: CGFloat {
        switch self {
        case .ribbon: 8
        case .row: 10
        case .header: 11
        case .lone: 15
        case .watchRow: 14
        case .watchLone: 22
        }
    }

    /// The ring's width for one attention tier, in points, quoted.
    ///
    /// The phone's three ladders are §03's, read down 8 / 10 / 11 / 15:
    /// needs you `2 / 2.5 / 2.5 / 3.5`, to review `2 / 2 / 2 / 3`, hairline
    /// `1` at every size. The two watch sizes carry their own triples,
    /// `3 / 2.5 / 1` at 14 and `5 / 4 / 1.5` at 22.
    ///
    /// **The 22pt hairline is 1.5 and not 1**, which is the one place the spec
    /// states a rule and then an exception to it in the same paragraph: "Hairline:
    /// 1 at every size" sits under the PHONE's four diameters, and the wrist's
    /// two carry explicit triples of their own that end in 1.5. The explicit
    /// figure wins, and it is also the one that survives being read at arm's
    /// length on a 22pt mark.
    public func stroke(_ attention: GlanceMark.Attention) -> CGFloat {
        switch (self, attention) {
        // At 8pt the first two tiers collapse to ONE weight, and that is the
        // spec's own fallback rather than a rounding: "there is no room for
        // four distinguishable strokes, so an 8pt ribbon separates wants you
        // from quiet and leaves amber-versus-review to hue. In monochrome at
        // 8pt the count text carries that split instead." Which is why the two
        // arms below are both 2, written out rather than merged — the day a
        // ribbon gains a point of diameter, the two numbers part company again.
        case (.ribbon, .needsYou): 2
        case (.ribbon, .toReview): 2
        case (.row, .needsYou): 2.5
        case (.row, .toReview): 2
        case (.header, .needsYou): 2.5
        case (.header, .toReview): 2
        case (.lone, .needsYou): 3.5
        case (.lone, .toReview): 3
        case (.watchRow, .needsYou): 3
        case (.watchRow, .toReview): 2.5
        case (.watchLone, .needsYou): 5
        case (.watchLone, .toReview): 4
        case (.watchLone, .quiet): 1.5
        case (_, .quiet): 1
        }
    }

    /// The core's diameter, or nil where this size has none.
    ///
    /// §03, literally: "3 at 8pt, 4 at 11pt, 5 at 15pt. There is no 10pt core —
    /// a row shows the ring alone — and below 7pt diameter the core comes off
    /// entirely." Plus the wrist's own two, 5 at 14pt and 8 at 22pt.
    ///
    /// The missing 10pt core is not an oversight to be filled in by
    /// interpolation. A row is the densest thing in the system and the ring
    /// alone is what stays legible there; splitting the difference between 3
    /// and 4 would put a 3.5pt disc inside a 5pt hole and produce a mark that
    /// reads as a smudge at exactly the size it is read at most.
    public var core: CGFloat? {
        switch self {
        case .ribbon: 3
        case .row: nil
        case .header: 4
        case .lone: 5
        case .watchRow: 5
        case .watchLone: 8
        }
    }
}

/// The mark, drawn.
///
/// One view for every surface in the product. Where a caller has a
/// `FleetSnapshot.Agent` or a `FleetSnapshot.Glance` it builds the mark from
/// that rather than deciding anything itself, so the widget on a home screen
/// and the complication on the wrist beside it cannot draw the same fleet two
/// ways.
public struct GlanceMarkView: View {
    /// Light mode is a different palette rather than the same one dimmed —
    /// §01 is explicit that it is "Not a filter flip" — so the mark has to know
    /// which appearance it is in. `@Environment(\.colorScheme)` rather than a
    /// dynamic colour provider because those are `UIColor`/`NSColor` and the
    /// watch has neither. See `GlanceInk`.
    @Environment(\.colorScheme) private var scheme

    private let mark: GlanceMark
    private let diameter: CGFloat
    private let ring: CGFloat
    private let coreDiameter: CGFloat?
    private let elongated: Bool
    private let decorative: Bool

    /// One of §03's six sizes.
    public init(_ mark: GlanceMark, size: GlanceMarkSize, decorative: Bool = false) {
        self.mark = mark
        self.diameter = size.diameter
        self.ring = size.stroke(mark.attention)
        self.coreDiameter = mark.core == .producing ? size.core : nil
        self.elongated = false
        self.decorative = decorative
    }

    /// The in-app tab ribbon's own ladder, which is smaller than any of the six
    /// and older than the spec.
    ///
    /// **This is the one caller allowed to name its own diameter, and it is not
    /// a loophole being left open.** `ShellRibbon` draws at 6, `ShellColumn` at
    /// 7 and `ShellOverview` at 5, and those three numbers are load-bearing
    /// somewhere other than here: the column reserves an 18pt gutter around a
    /// 7pt mark, and the flight between the ribbon and the menu is a
    /// `matchedGeometryEffect` whose source frame is literally `size` and
    /// `size * 2.5` (`ShellBar.swift`, `ShellRibbon.slot` and
    /// `ShellColumn.mark`). Growing the dot to 8pt to satisfy §03 would move
    /// three frames in a file another lane is editing, to fix a ribbon that is
    /// not a glance surface at all — it is in the app, where the person is
    /// already looking.
    ///
    /// What it inherits from the spec is everything except the diameter: the
    /// 8pt stroke collapse (2 / 2 / 1), and no core, because "below 7pt
    /// diameter the core comes off entirely" and because at 7pt a 3pt core
    /// inside a 2pt ring would touch it on both sides.
    public init(_ mark: GlanceMark, inAppDiameter: CGFloat, elongated: Bool = false) {
        self.mark = mark
        self.diameter = inAppDiameter
        self.ring = GlanceMarkSize.ribbon.stroke(mark.attention)
        self.coreDiameter = nil
        self.elongated = elongated
        self.decorative = true
    }

    public var body: some View {
        ZStack {
            // `strokeBorder` and not `stroke`: the border is drawn INSIDE the
            // shape, so the mark's outer diameter is exactly the diameter §03
            // names. `stroke` straddles the path and would make a 15pt
            // lone indicator with a 3.5pt ring 18.5pt across — which is the
            // kind of drift that only shows up when two surfaces sized from the
            // same table stop lining up.
            //
            // A `Capsule` rather than a `Circle` because at equal width and
            // height a capsule IS a circle, so one shape draws both the mark
            // and the elongated in-app variant of it. `ShellMarkView` has the
            // argument for why that variant exists.
            Capsule()
                .strokeBorder(ringColor, style: strokeStyle)
            if let coreDiameter {
                // Never amber. §03 reserves the ring for the person's side and
                // the core for the agent's, and amber is the person's colour:
                // an amber core would say "this needs you" about the half of
                // the mark that is only ever saying what the agent is doing.
                // The brightest neutral instead, which is what "present" looks
                // like against every one of these surfaces.
                Circle()
                    .fill(GlancePalette.ink1(scheme))
                    .frame(width: coreDiameter, height: coreDiameter)
            }
        }
        .frame(width: elongated ? diameter * 2.5 : diameter, height: diameter)
        .accessibilityHidden(decorative)
        .accessibilityLabel(decorative ? "" : mark.phrase)
    }

    private var ringColor: Color {
        switch mark.attention {
        case .needsYou: GlancePalette.amber(scheme)
        case .toReview: GlancePalette.review(scheme)
        case .quiet: GlancePalette.ink2(scheme)
        }
    }

    private var strokeStyle: StrokeStyle {
        StrokeStyle(lineWidth: ring, dash: mark.link == .broken ? Self.dash(ring) : [])
    }

    /// The dash, in one place, for every size and every state.
    ///
    /// **§03's rule here could not be implemented as written, and this is the
    /// nearest thing that draws.** The spec says: "Dash pattern is the platform
    /// default; only the stroke changes." SwiftUI has no platform default —
    /// `StrokeStyle.dash` defaults to an empty array, which is a solid line —
    /// so a figure had to be chosen. A single CONSTANT array was chosen first,
    /// 2 on and 2 off, and rendering the whole matrix is what showed why it
    /// cannot be one: at the 1pt hairline on an 8pt ribbon it reads correctly,
    /// and at the 5pt ring on a 22pt lone indicator each 2pt segment is a bar
    /// two points long and five points wide — the mark turns into a sunburst,
    /// which is to say it turns into a spinner, which is to say the drawing for
    /// "unreachable" reads as "working". One array cannot serve both a 1pt
    /// stroke and a 5pt one; the spec's own ladder spans exactly that range.
    ///
    /// So: one RULE rather than one array — the segment is half again as long
    /// as the stroke is wide, and the gap matches the stroke. Which keeps the
    /// half of the sentence that is a design decision (there is one dash in the
    /// system, and states differ by stroke weight and nothing else) and gives
    /// up the half that is an implementation detail of a platform that does not
    /// have it.
    ///
    /// **This is not the "never a percentage" rule being broken.** That rule is
    /// about stroke width, and its reason is grid alignment: "a ratio cannot
    /// land on the half-pixel grid at these sizes." A dash runs ALONG the path,
    /// around a curve, where there is no pixel grid to land on — the constraint
    /// simply does not apply to it.
    ///
    /// It remains the one figure in this file not quoted from the design
    /// document, and the document is where it should be settled.
    private static func dash(_ stroke: CGFloat) -> [CGFloat] {
        [stroke * 1.5, stroke]
    }
}

#if DEBUG
    /// §03's matrix, drawn — every state at every size, which is the only way
    /// to check a drawing whose whole vocabulary is half-point differences in
    /// line width.
    ///
    /// **Look at it in monochrome too.** The accessory families flatten to one
    /// tint, so the question that matters is whether the twelve states are
    /// still tellable apart with hue removed. Toggle the canvas to a lock
    /// screen accessory, or read the second row of this preview, which is the
    /// same grid desaturated.
    #Preview("The matrix · every state, every size") {
        let states: [GlanceMark] = [.needsYou, .toReview, .quiet].flatMap { attention in
            [GlanceMark.Core.producing, .atAPrompt].flatMap { core in
                [GlanceMark.Link.live, .broken].map {
                    GlanceMark(attention: attention, core: core, link: $0)
                }
            }
        }
        return ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                ForEach([false, true], id: \.self) { mono in
                    Text(mono ? "monochrome" : "colour").font(.footnote)
                    ForEach(Array(states.enumerated()), id: \.offset) { _, mark in
                        HStack(spacing: 0) {
                            Text(mark.phrase)
                                .font(.caption2)
                                .frame(width: 170, alignment: .leading)
                            ForEach(GlanceMarkSize.allCases, id: \.self) { size in
                                GlanceMarkView(mark, size: size)
                                    .frame(width: 44, height: 24)
                            }
                        }
                        .grayscale(mono ? 1 : 0)
                    }
                }
            }
            .padding()
        }
    }
#endif
