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
//
// **The activity trace is in here too, at the bottom.** The spec's title is
// "One mark, one graphic": §03's mark and §04's trace are the only two drawings
// in the system, and they are here together because a third file would have to
// be added to four separate lists in `generate-project.py` — the phone's, the
// watch's, the complication's and the Live Activity extension's — and every one
// of the five binaries that draws a mark also draws a trace. The two share
// nothing but `GlancePalette` and that rule; if this file ever grows a third
// drawing, split it then.

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
        /// A finished thing nobody has looked at. A middle-weight ring in the
        /// low chroma `GlancePalette.review`.
        ///
        /// **A finished diff nobody has read, and a finished turn nobody has
        /// opened.** It was the first of those alone until the `done` status
        /// was let in, and the widening is deliberate: what the tier says is
        /// "this is over, and you have not looked", which is true of a turn in
        /// exactly the sense it is true of a diff.
        ///
        /// **`reviewsWaiting` and `unreadDiff` are still fleet- and
        /// workspace-level and have no per-agent version.** `reviewsWaiting`
        /// is a fleet-wide scalar and `unreadDiff` is per WORKSPACE;
        /// `ShellScreen.swift:141-147` and `ShellNavigation.swift:116-124`
        /// both refuse to invent a per-agent version, and that refusal stands
        /// — a ring meaning "this AGENT has unread changes" would be a fact
        /// nothing on the wire has an opinion about. The narrowing that let
        /// `done` in does not reach those two counts, because the reason for
        /// the ban was invented data and `done` is not invented: the daemon
        /// sends it per terminal, and `FleetSnapshot.Confidence` already
        /// treats it as latched rather than passing. See the initializer from
        /// `FleetSnapshot.Agent` below, which produces this tier for `done`
        /// and for nothing else.
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
    /// `.toReview` comes out of exactly one status, `done`, and out of no
    /// other — see `Attention.toReview` for why that one is allowed and why a
    /// per-agent `reviewsWaiting` or `unreadDiff` still is not.
    public init(agent: FleetSnapshot.Agent, confidence: FleetSnapshot.Confidence = .known) {
        switch agent.status {
        // Blocked is latched — an agent stopped an hour ago is still stopped —
        // so it keeps its heavy ring however old the snapshot is, and the core
        // is genuinely absent rather than merely unstated: being at a prompt is
        // what "blocked" means.
        case "blocked": self.init(attention: .needsYou, core: .atAPrompt)
        // The turn is over and nobody has looked at it. `.atAPrompt` for the
        // same reason `blocked` is, one line up: the core is the agent's side
        // of the mark and a finished agent is not producing anything.
        //
        // Latched, like `blocked` and unlike `working` — `confidence(in:at:)`
        // vouches for both at any age, as the comment below this switch says,
        // so the ring stays solid however old the snapshot is. Drawing a state
        // the system latches as "nothing wanted" would contradict a decision
        // this codebase has already made.
        case "done": self.init(attention: .toReview, core: .atAPrompt)
        case "working": self.init(attention: .quiet, core: .producing)
        // An unrecognised status lands on the tier that claims the least, and
        // that rule is untouched by `done` joining the review tier above.
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
    /// Whether `core == .producing` is said with FILL rather than with a disc.
    ///
    /// Only ever true for the in-app ladder. See that initializer, which is
    /// where the deviation from §03 is argued.
    private let fillsCore: Bool
    private let elongated: Bool
    private let decorative: Bool

    /// One of §03's six sizes.
    public init(_ mark: GlanceMark, size: GlanceMarkSize, decorative: Bool = false) {
        self.mark = mark
        self.diameter = size.diameter
        self.ring = size.stroke(mark.attention)
        self.coreDiameter = mark.core == .producing ? size.core : nil
        self.fillsCore = false
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
    /// 8pt stroke collapse (2 / 2 / 1), and the core's INK and meaning — but
    /// not the core's geometry. See below.
    ///
    /// **The deviation from §03, stated plainly.** The spec says "below 7pt
    /// diameter the core comes off entirely", and it says nothing about
    /// putting anything in its place. This initializer used to obey that
    /// literally — `coreDiameter = nil`, full stop — and the result was a
    /// defect the owner reported: in the shell bar a WORKING agent and an idle
    /// one drew the same 1pt hairline ring, which reads as nothing at all. The
    /// Mac's sidebar showed the same agent's core correctly, because it draws
    /// at 8 and 15 where the core survives; the phone's bar said nothing until
    /// the turn finished. A person watching both had one surface telling them
    /// their agent was running and another implying it was not.
    ///
    /// So at these diameters `core: .producing` is drawn as FILL: the ring's
    /// whole interior in `ink1`, where an at-a-prompt mark leaves it empty.
    ///
    /// **Why this is §03's own argument rather than a hole in it.** The spec's
    /// reason for taking the core off is geometric and it still holds — at 7pt
    /// a 3pt disc inside a 2pt ring touches it on both sides, and there is no
    /// smaller disc that reads. What does not follow is that the AXIS goes
    /// silent: §03's stated method is that "the mark distinguishes states by
    /// stroke weight, fill and dash", and fill is the one of those three the
    /// in-app ladder had not spent. The core keeps its ink (`ink1`, the
    /// brightest neutral, never amber — the reason is in `body`) and its
    /// meaning (the agent is producing); only its shape changes, from a
    /// concentric disc to the interior it could not fit inside.
    ///
    /// **The fill is inset by the ring rather than drawn behind it**, which is
    /// what keeps `link: .broken` legible. A filled capsule with the ring
    /// stroked on top would show `ink1` through the dash GAPS, so a stale
    /// working agent would read as a solid disc with a slightly dimmer edge —
    /// the dash, which is the whole of what "we have not heard from this
    /// recently" looks like, would be gone. Inset, the gaps show the surface
    /// behind the mark, and the two inks differ, so a dashed ring around a
    /// filled interior stays a dashed ring.
    ///
    /// This is a deviation and it is confined to this one initializer. The six
    /// spec sizes above are untouched and still draw §03's disc.
    public init(_ mark: GlanceMark, inAppDiameter: CGFloat, elongated: Bool = false) {
        self.mark = mark
        self.diameter = inAppDiameter
        self.ring = GlanceMarkSize.ribbon.stroke(mark.attention)
        self.coreDiameter = nil
        self.fillsCore = mark.core == .producing
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
            // The in-app ladder's core, which is the interior rather than a
            // disc — see `init(_:inAppDiameter:elongated:)`. Inset by the ring
            // so the dash's gaps show the surface and not this.
            if fillsCore {
                Capsule()
                    .inset(by: ring)
                    .fill(GlancePalette.ink1(scheme))
            }
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

// MARK: - §04 · The activity trace

/// Where the trace is drawn, and how big it is there.
///
/// §04 names four and says so: "40 · island / 44 · island row / 52 · card row /
/// 76 · widget. Four shipping sizes, one form." Those figures are WIDTHS — the
/// floor a few lines above them is stated as "36pt wide", and §07's card row
/// gives its columns as "11 / flex / 52 / 64", where the 52 is this.
///
/// **The heights are derived here, and they are the one thing §04 does not
/// state.** It gives the specimen's box (156×41) and the band formula ((h−3)/2)
/// and then leaves `h` to the surface. Four numbers invented independently would
/// be four chances to draw the same graphic four shapes, so there is ONE rule
/// instead: hold the specimen's proportion. Its band is (41−3)/2 = 19 at 156
/// wide, so a band is `width * 19 / 156` and the height is that twice plus the
/// axis. It is a derivation rather than a quotation and it is flagged as one
/// wherever it appears; the design document is where it should be settled.
public enum GlanceTraceSize: Hashable, Sendable, CaseIterable {
    /// 40pt. The Dynamic Island's compact presentation, which §07 gives to the
    /// fleet trace: "Count leading, fleet trace trailing, thirteen buckets like
    /// every other."
    case island
    /// 44pt. A row of the Island's expanded presentation.
    case islandRow
    /// 52pt. A row of the Live Activity card. §07's third column.
    case cardRow
    /// 76pt. A row of a home screen widget. §05's medium and large tiles.
    case widget

    /// §04's four figures, quoted.
    public var width: CGFloat {
        switch self {
        case .island: 40
        case .islandRow: 44
        case .cardRow: 52
        case .widget: 76
        }
    }

    /// The specimen's proportion, held. See the type's own comment — this is
    /// derived, not quoted.
    public var height: CGFloat {
        GlanceTraceLayout.axis + 2 * (width * 19 / 156).rounded()
    }
}

/// §04's geometry, as arithmetic — separated from the drawing so it can be
/// asserted against the spec without rendering anything.
///
/// Every figure in here is quoted from §04 except `minimumBar`, which is
/// flagged where it is declared. The same discipline `GlanceMarkSize` keeps for
/// §03: the spec's numbers land in exactly one place, and a test reads them back.
public struct GlanceTraceLayout: Sendable, Equatable {
    /// Which half a bar belongs to. §04: "up is code, down is chat".
    public enum Half: Hashable, Sendable, CaseIterable {
        /// The upper half — lines of code touched.
        case code
        /// The lower half — output to the person.
        case output
    }

    /// §04: "Bands (h−3)/2 — Equal halves either side of a 3pt axis."
    public static let axis: CGFloat = 3

    /// §04: "Commits 3pt block. On the axis."
    public static let commitBlock: CGFloat = 3

    /// §04: "Floor 36pt wide — Below this, commit marks stop separating and
    /// come off."
    public static let commitFloor: CGFloat = 36

    /// The shortest a bar may be drawn.
    ///
    /// **The one figure here §04 does not give**, and it is forced by two of the
    /// ones it does. "A bucket with no activity" has a colour of its own in §01
    /// and §04 says it is "Drawn, not omitted", so a silent bucket has to be
    /// something rather than nothing — and a scaled height can be a hundredth of
    /// a point, which is nothing. 1pt, matching §03's hairline, is the smallest
    /// thing this system draws anywhere.
    ///
    /// It applies to LIT bars too: a bucket with one line of output beside a
    /// bucket with nine hundred scales to well under a point, and rounding it
    /// away would delete a real observation rather than an absent one.
    public static let minimumBar: CGFloat = 1

    public let size: CGSize

    public init(size: CGSize) {
        self.size = size
    }

    /// Half the drawable height, either side of the axis. §04's `(h−3)/2`.
    public var band: CGFloat { max(0, (size.height - Self.axis) / 2) }

    /// §04's floor, applied. Below it "commit marks stop separating and come
    /// off" — they are not shrunk, they are removed.
    public var drawsCommits: Bool { size.width >= Self.commitFloor }

    /// The x-range of one bucket's column, 0…12 oldest first.
    ///
    /// §04: "Bars are flex: 1, so they always fill the width exactly" and "Gap
    /// 0. No gaps. The fused silhouette is the point."
    ///
    /// Computed as two multiplications rather than one width times an index, so
    /// thirteen columns tile the box EXACTLY: `width/13` rounded and multiplied
    /// leaves a sliver of dead space at the recent end, which is precisely what
    /// §04's last NEVER forbids — "Never leave the container wider than the
    /// bars — trailing space sits at the recent end and reads as dead activity."
    public func column(_ bucket: Int) -> (x: CGFloat, width: CGFloat) {
        let leading = size.width * CGFloat(bucket) / CGFloat(ActivityTrace.buckets)
        let trailing = size.width * CGFloat(bucket + 1) / CGFloat(ActivityTrace.buckets)
        return (leading, trailing - leading)
    }

    /// How tall one bucket's bar is, and whether anything was in it.
    ///
    /// §04's scaling rule, and the whole of it: "Tallest bar = that row's
    /// busiest bucket, per half. Each half therefore reaches full height, so the
    /// two halves are not comparable to one another — only each against its own
    /// past. Never cross-row." So the denominator is `tallestCode` for the upper
    /// half and `tallestOutput` for the lower, and never the other, and never
    /// another row's.
    ///
    /// `lit` is false for a bucket that recorded nothing, which is what picks
    /// §01's `empty` tone over `code` or `chat`. It is NOT the same as a short
    /// bar: a bucket with one line in a row whose busiest has nine hundred is
    /// lit, at `minimumBar`.
    public func bar(_ bucket: Int, _ half: Half, in trace: ActivityTrace)
        -> (height: CGFloat, lit: Bool)
    {
        let value: UInt16
        let tallest: UInt16
        switch half {
        case .code:
            value = trace.code(bucket)
            tallest = trace.tallestCode
        case .output:
            value = trace.output(bucket)
            tallest = trace.tallestOutput
        }
        guard value > 0, tallest > 0 else { return (min(Self.minimumBar, band), false) }
        let scaled = band * CGFloat(value) / CGFloat(tallest)
        return (min(band, max(Self.minimumBar, scaled)), true)
    }

    /// Where that bar sits. The upper half grows up from the axis, the lower
    /// half down from it.
    public func barRect(_ bucket: Int, _ half: Half, in trace: ActivityTrace) -> CGRect {
        let column = column(bucket)
        let height = bar(bucket, half, in: trace).height
        switch half {
        case .code:
            return CGRect(x: column.x, y: band - height, width: column.width, height: height)
        case .output:
            return CGRect(
                x: column.x, y: band + Self.axis, width: column.width, height: height)
        }
    }

    /// The centre rule. §01: "The trace centre rule. Continuous, never dotted."
    public var axisRect: CGRect {
        CGRect(x: 0, y: band, width: size.width, height: Self.axis)
    }

    /// A commit mark, or nil where this bucket has none or this size has no
    /// room for any.
    ///
    /// §04: "Commits 3pt block. On the axis. Unlit buckets get height 0 — an
    /// unset height stretches." The block is the column's own width and sits in
    /// the axis rather than beside it, so a commit reads as the rule brightening
    /// under that bucket. §01 gives it the lightest neutral in the table, which
    /// is what makes it visible against the axis it replaces.
    public func commitRect(_ bucket: Int, in trace: ActivityTrace) -> CGRect? {
        guard drawsCommits, trace.commits(bucket) > 0 else { return nil }
        let column = column(bucket)
        return CGRect(x: column.x, y: band, width: column.width, height: Self.commitBlock)
    }
}

/// The trace, drawn. §04's one form, at whichever of its four sizes.
///
/// **It takes an `ActivityTrace` and not a `Data?`, and that is the contract
/// rather than an inconvenience.** A terminal with nothing to show sends no
/// bytes at all — see `ActivityTrace.init?` — and "no trace" and "a trace of
/// thirteen quiet buckets" are two different drawings. Making the absent case
/// unrepresentable here means a caller has to write `if let trace =
/// ActivityTrace(agent.trace)`, and the surface that forgets draws nothing
/// rather than a flat line at zero.
///
/// **Six shapes and no more**, which is the memory half of the same argument the
/// wire format makes. Thirteen buckets times two halves plus thirteen commit
/// marks is thirty-nine views per trace if each bar is one; at six rows per
/// widget family that is a view tree a memory-capped extension pays for on every
/// timeline rebuild. One `Path` per TONE collapses it to six, and a `Path` of
/// thirteen rectangles is a handful of points.
public struct GlanceTraceView: View {
    /// Same reason as `GlanceMarkView`: light mode is a different palette and
    /// §01 says so in as many words. The two trace tones swap ends of the scale.
    @Environment(\.colorScheme) private var scheme

    private let trace: ActivityTrace
    private let size: CGSize

    /// One of §04's four shipping sizes.
    public init(_ trace: ActivityTrace, size: GlanceTraceSize) {
        self.trace = trace
        self.size = CGSize(width: size.width, height: size.height)
    }

    /// An explicit box, for a preview or a surface measuring its own row.
    public init(_ trace: ActivityTrace, size: CGSize) {
        self.trace = trace
        self.size = size
    }

    public var body: some View {
        ZStack {
            // Order matters and it is the spec's silhouette: the two halves
            // first, then the axis over them so a bar that reaches the rule does
            // not eat it, then the commits over the axis because a commit is
            // drawn IN the rule rather than beside it.
            layer(.bars(.code, lit: false)).fill(GlancePalette.empty(scheme))
            layer(.bars(.code, lit: true)).fill(GlancePalette.code(scheme))
            layer(.bars(.output, lit: false)).fill(GlancePalette.empty(scheme))
            layer(.bars(.output, lit: true)).fill(GlancePalette.chat(scheme))
            layer(.axis).fill(GlancePalette.axis(scheme))
            layer(.commits).fill(GlancePalette.commitInk(scheme))
        }
        .frame(width: size.width, height: size.height)
        // History, spoken as what it covers. The bars themselves cannot be
        // read out — thirteen unlabelled figures in two units is not a
        // sentence — and every surface that draws one already says what the
        // agent is doing beside it.
        .accessibilityLabel("Activity over the last \(trace.span.spoken)")
    }

    private func layer(_ kind: TraceLayer.Kind) -> TraceLayer {
        TraceLayer(trace: trace, kind: kind)
    }
}

/// One tone's worth of the trace, as a single path.
private struct TraceLayer: Shape {
    enum Kind {
        case bars(GlanceTraceLayout.Half, lit: Bool)
        case axis
        case commits
    }

    let trace: ActivityTrace
    let kind: Kind

    func path(in rect: CGRect) -> Path {
        // Geometry off the rect SwiftUI actually laid out rather than off the
        // size asked for, so a surface that constrains this draws a correct
        // trace at the size it got instead of one clipped from a larger one.
        let layout = GlanceTraceLayout(size: rect.size)
        var path = Path()
        switch kind {
        case let .bars(half, lit):
            for bucket in 0..<ActivityTrace.buckets
            where layout.bar(bucket, half, in: trace).lit == lit {
                path.addRect(layout.barRect(bucket, half, in: trace).offsetBy(dx: rect.minX, dy: rect.minY))
            }
        case .axis:
            path.addRect(layout.axisRect.offsetBy(dx: rect.minX, dy: rect.minY))
        case .commits:
            for bucket in 0..<ActivityTrace.buckets {
                if let mark = layout.commitRect(bucket, in: trace) {
                    path.addRect(mark.offsetBy(dx: rect.minX, dy: rect.minY))
                }
            }
        }
        return path
    }
}

extension ActivityTrace.Span {
    /// The span in words, for VoiceOver. `label` is the two mono characters §04
    /// prints; this is the same fact said out loud, where "1h" would be read as
    /// a letter.
    public var spoken: String {
        switch self {
        case .hour: "hour"
        case .sixHours: "six hours"
        case .day: "24 hours"
        }
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

    /// §04's trace, at all four shipping sizes and in the three cases that are
    /// easy to draw wrong.
    ///
    /// **The three cases are the point of the preview.** A busy row shows the
    /// silhouette; a row with an empty upper half is §05's `docs-sweep`, "it has
    /// talked and touched nothing, drawn rather than omitted"; and sixty-six
    /// zero bytes is a real trace of thirteen quiet buckets, which is a
    /// different drawing from no trace at all — an absent one has no view here
    /// because `ActivityTrace.init?` refuses it, which is exactly the contract.
    ///
    /// **Look at it in light mode too.** The commit mark is the one tone §01
    /// gives a single figure that does not survive inversion; see
    /// `GlancePalette.commitInk`.
    #Preview("The trace · four sizes, three cases") {
        func encoded(
            code: [UInt16] = Array(repeating: 0, count: 13),
            output: [UInt16] = Array(repeating: 0, count: 13),
            commits: [UInt8] = Array(repeating: 0, count: 13),
            width: UInt8 = 0
        ) -> Data {
            var bytes = Data([(1 << 4) | width])
            for v in code { bytes.append(UInt8(v & 0xFF)); bytes.append(UInt8(v >> 8)) }
            for v in output { bytes.append(UInt8(v & 0xFF)); bytes.append(UInt8(v >> 8)) }
            bytes.append(contentsOf: commits)
            return bytes
        }
        let cases: [(String, Data)] = [
            (
                "busy",
                encoded(
                    code: [0, 0, 40, 210, 180, 30, 0, 90, 420, 260, 55, 0, 12],
                    output: [4, 30, 120, 260, 300, 180, 60, 140, 380, 500, 220, 90, 40],
                    commits: [0, 0, 0, 1, 0, 0, 0, 0, 2, 0, 1, 0, 0])
            ),
            (
                "empty upper half",
                encoded(output: [0, 12, 40, 90, 60, 30, 110, 200, 90, 20, 5, 0, 0], width: 1)
            ),
            ("66 zeroes · all quiet", encoded(width: 2)),
        ]
        return ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                ForEach(cases, id: \.0) { name, bytes in
                    Text(name).font(.caption2)
                    ForEach(GlanceTraceSize.allCases, id: \.self) { size in
                        HStack(spacing: 6) {
                            Text("\(Int(size.width))")
                                .font(.caption2.monospacedDigit())
                                .frame(width: 24, alignment: .trailing)
                            if let trace = ActivityTrace(bytes) {
                                GlanceTraceView(trace, size: size)
                                Text(trace.span.label).glanceType(.monoFigures)
                            }
                            Spacer(minLength: 0)
                        }
                    }
                }
            }
            .padding()
        }
    }
#endif
