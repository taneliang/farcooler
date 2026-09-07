import Foundation
import SwiftUI

// The Live Activity card, drawn. §02's header / two rows / tail, and nothing
// else.
//
// **It is in AgentKit and it used to be in the widget extension**, which is a
// move rather than a copy: `FleetCard` and `CardRow` were `private struct`s in
// `AgentActivityWidget.swift`, and everything they touch — `AgentCardLayout`,
// `GlanceMarkView`, `GlanceTraceView`, `GlancePalette`, `glanceType` — was
// already here. Nothing about them was ActivityKit's.
//
// The reason for the move is the reason `AgentCardRows.swift` gives for being
// here at all: `swift test --package-path apps/shared/AgentKit` runs on every
// push and the iOS UI suite is compiled and never executed, so a `View` in the
// extension is a drawing nothing reads back. That mattered the moment the card
// grew a defect that was invisible to every assertion above the pixels — the
// card forced the dark palette onto a surface whose background the SYSTEM
// supplies, so on a Mac's menu bar, and on a phone set to Light, the secondary
// ink and the trace's upper half landed as pale grey on pale grey. See
// `GlanceCardContrastTests`, which renders this view over both grounds and
// reads the ink back out.
//
// **The card does not assert what is behind it.** `.activityBackgroundTint(nil)`
// leaves the background to the system's material — a flat fill of ours sits on
// top of somebody's photograph like a sticker — and a surface that does not
// choose its own background has no business choosing an appearance either. So
// every ink here resolves against `@Environment(\.colorScheme)`. The one place
// forcing dark stays correct is the Dynamic Island, which is a black pill
// whatever the phone is set to; that forcing is still in
// `AgentActivityWidget.swift`, at each of the presentations it is true of.

/// The card the design draws: header, two rows, tail.
///
/// **Every word and every figure on it is composed in `AgentCardLayout`**,
/// which is beside it in this package and has a test suite. Nothing below
/// decides what to say; it decides where things go. That split is the only way
/// any of this is checkable without a device, and it is the same arrangement
/// `GlanceTraceLayout` has with the trace it draws.
///
/// **The geometry is the design's, quoted.** 12/14 padding, a 9pt gap
/// everywhere, columns of 11 / flex / 52 / 64, and a body whose rows divide
/// whatever height the system gives the presentation. The two rules are two
/// weights on purpose: the heavier one separates the card's three parts and the
/// lighter one separates two agents, and drawn at one weight the card reads as a
/// list of five things.
///
/// **It follows the appearance rather than declaring one.** This card used to
/// carry `.environment(\.colorScheme, .dark)` over the whole of itself, on the
/// argument that "a Live Activity is drawn over the lock screen's wallpaper on
/// a dark material whatever the phone's appearance is set to". That is not
/// true, and the way it is not true is the ugliest one available: the card
/// takes the SYSTEM's material for its background, and the system draws that
/// material light on a Mac's menu bar and on a phone set to Light. The dark
/// palette on a pale ground put `text 2` (L 0.74) and the trace's upper half
/// (L 0.88) within a few percent of the material behind them, so the header's
/// count, both rows' detail lines, both rows' commit counts and the fleet line
/// were all invisible while the titles beside them read fine — the exact split
/// a person would report as "white text on a light background".
public struct GlanceCardView: View {
    /// Light mode is a different palette rather than the same one dimmed — §01
    /// is explicit that it is "Not a filter flip" — so the card has to know
    /// which appearance it is in, for the same reason and by the same mechanism
    /// `GlanceMarkView` and `GlanceTraceView` already do.
    @Environment(\.colorScheme) private var scheme

    private let layout: AgentCardLayout

    public init(layout: AgentCardLayout) {
        self.layout = layout
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
                .padding(.bottom, 9)
            hairline(GlancePalette.ruleInk(scheme))
            // `flex: 1` — the rows take whatever is left of the card, which is
            // the only figure in the design that is not a number: the
            // presentation's height belongs to the system.
            VStack(spacing: 0) {
                ForEach(Array(layout.rows.enumerated()), id: \.element.id) { index, row in
                    if index > 0 { hairline(GlancePalette.rowRuleInk(scheme)) }
                    GlanceCardRow(row: row)
                        .frame(maxHeight: .infinity)
                }
            }
            .frame(maxHeight: .infinity)
            hairline(GlancePalette.ruleInk(scheme))
            tail
                .padding(.top, 8)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }

    private var header: some View {
        HStack(spacing: 9) {
            // §03's header diameter, and the one place on this card where the
            // ring is about the whole fleet rather than one agent.
            GlanceMarkView(layout.mark, size: .header)
            Text(layout.title)
                .glanceType(.cardHeader)
                .foregroundStyle(GlancePalette.ink1(scheme))
                .lineLimit(1)
            Spacer(minLength: 4)
            if let counts = layout.counts {
                // Mono, because these are figures a machine counted. §02's test
                // is exactly that: if it came off a machine it is mono.
                Text(counts)
                    .glanceType(.monoFigures)
                    .foregroundStyle(GlancePalette.ink2(scheme))
                    .lineLimit(1)
            }
        }
    }

    private var tail: some View {
        HStack(spacing: 8) {
            HStack(spacing: 4) {
                ForEach(Array(layout.rings.enumerated()), id: \.offset) { _, mark in
                    GlanceMarkView(mark, size: .ribbon, decorative: true)
                }
            }
            // Decorative as a group and not only one ring at a time: twelve
            // marks each announcing "Nothing wanted" is a screen reader reading
            // a texture aloud, and the header above says the same three numbers
            // in words that were written to be heard.
            .accessibilityHidden(true)
            Spacer(minLength: 4)
            if let line = layout.line {
                Text(line)
                    .glanceType(.monoFigures)
                    .foregroundStyle(GlancePalette.ink2(scheme))
                    .lineLimit(1)
            }
        }
    }

    /// One of the card's two rules, at the width it was given.
    ///
    /// A `Rectangle` and not a `Divider`: the two weights are the design's and
    /// `Divider` draws the system's separator, which is one weight and a color
    /// nobody chose.
    ///
    /// `AnyShapeStyle` because §01 states no light value for either rule and
    /// `GlancePalette.ruleInk` defers rather than invent one. See that
    /// declaration.
    private func hairline(_ ink: AnyShapeStyle) -> some View {
        Rectangle()
            .fill(ink)
            .frame(height: 1)
    }
}

/// One agent's line: ring, name and detail, its own thirteen buckets, its
/// figures.
///
/// **The columns are fixed widths, and that is only half of what makes the
/// traces line up.** §07 gives the row 11 / flex / 52 / 64, so every trace on
/// the card starts at the same x and is the same width. That buys a shared set
/// of x positions; it does not buy a shared axis, and for a while this comment
/// claimed the second from the first. The other half is that the rows share a
/// WINDOW — `AgentCardLayout` picks the coarsest its drawn rows carry and sums
/// the rest onto it — without which thirteen buckets sat under thirteen other
/// buckets that meant a different thirteen minutes.
///
/// **A row with no trace still holds the column open.** `ActivityTrace` refuses
/// to build from an absent field — a terminal with nothing to show sends no
/// bytes, deliberately, and that is not the same as thirteen quiet buckets — so
/// this draws nothing in a box of exactly the same size rather than letting the
/// figures beside it slide left. The row that HAS a trace and has touched no
/// files is the other case, and it draws itself: an empty upper half against a
/// visible center rule, absence drawn rather than omitted.
private struct GlanceCardRow: View {
    @Environment(\.colorScheme) private var scheme

    let row: AgentCardLayout.Row

    var body: some View {
        HStack(spacing: 9) {
            // §07's first column. The ring is where state lives on this card;
            // the trace two columns along never carries amber or blue, because
            // history is not urgent.
            GlanceMarkView(row.mark, size: .row)
                .frame(width: 11)
            VStack(alignment: .leading, spacing: 0) {
                Text(row.name)
                    .glanceType(.rowName)
                    .foregroundStyle(GlancePalette.ink1(scheme))
                    .lineLimit(1)
                if !row.detail.isEmpty {
                    Text(row.detail)
                        .glanceType(.secondary)
                        .foregroundStyle(GlancePalette.ink2(scheme))
                        .lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            trace
            VStack(alignment: .trailing, spacing: 0) {
                if let diff = row.diff {
                    Text(diff)
                        .glanceType(.monoFigures)
                        .foregroundStyle(GlancePalette.ink1(scheme))
                        .lineLimit(1)
                }
                if let footnote = row.footnote {
                    Text(footnote)
                        .glanceType(.monoFigures)
                        .foregroundStyle(GlancePalette.ink2(scheme))
                        .lineLimit(1)
                }
            }
            .frame(width: 64, alignment: .trailing)
        }
    }

    @ViewBuilder private var trace: some View {
        // Off the PUSH, which is what changed. The trace used to be read out of
        // the App Group snapshot because the push had never carried one; the
        // relay sends 66 bytes of base64 per row now, so the history on the card
        // is as fresh as everything else on it and does not depend on this phone
        // having run the app today.
        //
        // **Already on the card's own axis.** The two drawn rows used to
        // disagree about their SPAN — a trace snaps to the shortest of three
        // windows containing its own activity, so column 4 of a five-minute row
        // and column 4 of a two-hour row were spans twenty-four times apart
        // under one set of x positions. `AgentCardLayout` now picks the coarsest
        // window its rows carry and sums the finer ones onto it, so `row.trace`
        // is the drawn trace and the raw bytes are not reachable from here on
        // purpose. See `ActivityTrace.rebucketed(to:)` for the sum, and for the
        // one thing the wire does not carry: the second a trace's newest bucket
        // starts at, without which the placement is right to within one column
        // and no better.
        if let read = row.trace {
            GlanceTraceView(read, size: .cardRow)
        } else {
            Color.clear
                .frame(
                    width: GlanceTraceSize.cardRow.width,
                    height: GlanceTraceSize.cardRow.height)
        }
    }
}
