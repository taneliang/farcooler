import Foundation
import SwiftUI

// §02 of the glance spec: six type styles, one rule about which font, and one
// format for the only number these surfaces are allowed to print about time.
//
// **SF for chrome, SF Mono for anything a machine produced.** §02 states it as
// a test rather than a list: "If it came off a machine it is mono — timestamps,
// counts, diff stats, commit hashes, identifiers. If a person wrote it, or the
// product did, it is SF." A headline the daemon's ladder composed is still
// words for a person, so it is SF; the count beside it is a figure, so it is
// mono.
//
// **Numerals are always tabular.** Not a preference: these surfaces re-render
// on a timeline and a column of proportional figures jitters sideways between
// refreshes, which reads as the layout being unstable rather than as the number
// having changed.
//
// Same rule as `GlancePalette`: every figure here lands in exactly one place,
// and the design document is where it changes first.

/// One row of §02's table.
///
/// Size, weight and tracking travel together because that is how the table is
/// written and because two of the three are meaningless alone — 17/600 with no
/// tracking is a different style from the one specified, and nothing on screen
/// would say so.
public struct GlanceType: Sendable, Equatable {
    public let size: CGFloat
    public let weight: Font.Weight
    /// Letter spacing in POINTS, converted from the spec's em figure at this
    /// style's own size. SwiftUI's `.tracking(_:)` takes points; the spec, like
    /// every type spec, is written in ems because that is the unit that stays
    /// true when a size changes.
    public let tracking: CGFloat
    /// Whether this row is for something a machine produced.
    public let mono: Bool

    private init(_ size: CGFloat, _ weight: Font.Weight, em: CGFloat, mono: Bool = false) {
        self.size = size
        self.weight = weight
        self.tracking = em * size
        self.mono = mono
    }

    /// 17 / 600, −0.016em. Live Activity headline, widget primary count label.
    public static let headline = GlanceType(17, .semibold, em: -0.016)

    /// 15 / 600, −0.014em. Card header, column row titles.
    public static let cardHeader = GlanceType(15, .semibold, em: -0.014)

    /// 13 / 600, −0.010em. Workspace names in rows.
    public static let rowName = GlanceType(13, .semibold, em: -0.010)

    /// 12.5 / 400, no tracking. Terminal output in the app. Mono, because it is
    /// the most literal case of "it came off a machine" in the product.
    public static let terminal = GlanceType(12.5, .regular, em: 0, mono: true)

    /// 11 / 400, no tracking. Secondary lines, notes, and all mono figures.
    ///
    /// **The absolute floor on device**, and §01's contrast rule leans on it:
    /// "nothing below L 0.7 on the card and nothing below 11px. The brief's
    /// test is one second at arm's length in sunlight; treat both numbers as
    /// hard." So this is the smallest thing that may appear on any of these
    /// surfaces, and `.caption2` — which is 11 today and is whatever a future
    /// iOS decides tomorrow — is not a substitute for saying so.
    ///
    /// Not itself mono: the row covers "secondary lines, notes" as well as "all
    /// mono figures", so the caller says which it has by asking for
    /// `.monoFigures` when the content came off a machine.
    public static let secondary = GlanceType(11, .regular, em: 0)

    /// The same size and weight, in mono — timestamps, counts, diff stats,
    /// commit hashes, identifiers.
    public static let monoFigures = GlanceType(11, .regular, em: 0, mono: true)

    /// 38–46 / 600, −0.035em, tabular. The single count on a widget.
    ///
    /// A RANGE in the spec rather than one figure, because the number it draws
    /// is one or two digits depending on the fleet and the tile it sits in is
    /// the same size either way. The caller picks inside the range; anything
    /// outside it is clamped, so a family that wants a bigger number gets the
    /// biggest one this system has rather than one nobody specified.
    public static func count(_ size: CGFloat = 44) -> GlanceType {
        GlanceType(min(46, max(38, size)), .semibold, em: -0.035)
    }

    /// The font, with the figure style §02 makes universal.
    ///
    /// **A fixed point size, which means these lines do not scale with Dynamic
    /// Type**, and that is a consequence of the spec rather than a decision
    /// taken here: §02 gives absolute figures and §01 makes one of them a hard
    /// floor — "nothing below 11px … treat both numbers as hard" — which a
    /// scaling ramp cannot honour in both directions at once. `Font.system(size:)`
    /// has no `relativeTo:`, so there is no way to say "this size at the default
    /// content size, scaled from there" without naming a font file.
    ///
    /// The trade is deliberate but it IS a trade, and it is why this is applied
    /// narrowly. The lines that a person reads at an accessibility size on a
    /// lock screen — the rectangular accessory's headline and its count labels
    /// — deliberately keep the system's text styles, which scale. What takes
    /// these figures is the home screen, where the tile is a fixed size and a
    /// headline that grew would simply be clipped.
    public var font: Font {
        Font.system(size: size, weight: weight, design: mono ? .monospaced : .default)
            .monospacedDigit()
    }
}

extension View {
    /// One row of §02, applied.
    ///
    /// A modifier rather than two calls at every site because size and tracking
    /// are one decision and were repeatedly not applied together — SwiftUI puts
    /// them on different modifiers, so "17/600" is easy to write and
    /// "−0.016em" is easy to forget, and the result looks almost right.
    public func glanceType(_ style: GlanceType) -> some View {
        font(style.font).tracking(style.tracking)
    }
}

/// How old a snapshot is, in the two words §02 allows.
///
/// **No clock, anywhere.** §02: "iOS prints the time 20pt above every one of
/// these surfaces. Where a clock would go, print the age of the snapshot."
///
/// **And never a running one.** "Relative and coarse: 2m ago, 52m ago, 3h ago.
/// Never a running clock on an idle agent — precision nobody needs implies
/// precision we do not have. A live `Text(timerInterval:)` is reserved for a
/// wait you are actively in."
///
/// That last sentence is the reason this exists as a string rather than as
/// SwiftUI's `Text(_:style:.relative)`, which is exactly the live ticking clock
/// it forbids: a widget drawing one is a surface counting seconds up from a
/// measurement it took a quarter of an hour ago, to a precision it does not
/// have. The Live Activity's turn timer keeps `Text(_:style:.timer)`, because
/// that IS a wait you are actively in.
public enum GlanceAge {
    /// "2m", "52m", "3h" — the bare figure, for a slot that already has
    /// something to separate it from.
    public static func brief(_ interval: TimeInterval) -> String {
        let seconds = max(0, interval)
        if seconds < 60 { return "now" }
        if seconds < 3600 { return "\(Int(seconds / 60))m" }
        if seconds < 86400 { return "\(Int(seconds / 3600))h" }
        return "\(Int(seconds / 86400))d"
    }

    /// "2m ago", "52m ago", "3h ago" — the sentence form, where the surface has
    /// the room. "now" keeps no "ago", which would be a contradiction.
    public static func stated(_ interval: TimeInterval) -> String {
        let brief = brief(interval)
        return brief == "now" ? "just now" : "\(brief) ago"
    }

    /// Under two minutes, which is §08's definition of a fresh snapshot: "Fresh
    /// — under two minutes. Full strength, age printed in mono."
    ///
    /// The threshold is here rather than at each surface so that "fresh" means
    /// one thing. It is deliberately NOT `FleetSnapshot.staleAfter`, which is an
    /// hour and answers a different question — whether a claim about the
    /// present may still be asserted at all. Two minutes is when a surface
    /// starts saying how old it is; an hour is when it stops vouching.
    public static let fresh: TimeInterval = 120
}
