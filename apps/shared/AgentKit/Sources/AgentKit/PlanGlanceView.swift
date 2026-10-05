import SwiftUI

/// The plan's glance, drawn (ov-310): the board and what needs you on it, the
/// lanes in Now and the one next up.
///
/// One view for every surface that says it, so the phone's widget, the Live
/// Activity, the watch app and its complication can't word it four ways.
/// Each style has a fixed number of lines, so it fits the smallest surface it
/// goes on at the largest text size it allows (review H3, `PlanGlanceFitTests`):
///
/// - `.lines`, four at most, for a lock screen or Smart Stack accessory: the
///   board, one line per Now lane, next up. Text size stops at Large, the
///   default, because an accessory has no room to grow.
/// - `.rows`, the same four with each lane's state set right, for a home
///   screen widget and the watch app's list. Text size stops at xxx-Large.
/// - `.card`, two lines in the card's own fixed type, for the Live Activity:
///   the board and next up, then Now.
///
/// A `caveat` says the plan isn't current: a quiet runner's Now becomes
/// "Can't reach Studio · 3h ago" (its lanes are a claim nobody vouches for),
/// and a remembered plan keeps its lanes with "As of 3h ago" on top.
///
/// Amber only on the count, the one part that asks for the owner. VoiceOver
/// reads the whole glance as one sentence.
public struct PlanGlanceView: View {
    public enum Style: Sendable { case lines, rows, card }

    @Environment(\.colorScheme) private var scheme
    @Environment(\.dynamicTypeSize) private var typeSize
    private let glance: PlanGlance
    private let style: Style
    private let caveat: PlanCaveat?
    private let stale: Bool

    /// `stale` is the Live Activity's hour of silence: Now goes, as "Working"
    /// does on the card (review L4).
    public init(_ glance: PlanGlance, style: Style = .lines, caveat: PlanCaveat? = nil, stale: Bool = false) {
        self.glance = glance
        self.style = style
        self.caveat = caveat
        self.stale = stale
    }

    /// The longest a lane's name is drawn, in characters, before it's cut
    /// with an ellipsis. Branch-like slugs fit; a long one keeps its state
    /// word on the line.
    public static let nameShown = 20

    public var body: some View {
        content
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(spoken)
    }

    @ViewBuilder private var content: some View {
        switch style {
        case .card:
            VStack(alignment: .leading, spacing: 2) {
                Text(cardFirst).glanceType(.secondary).foregroundStyle(GlancePalette.ink2(scheme)).lineLimit(1)
                if let second = cardSecond {
                    Text(second).glanceType(.secondary).foregroundStyle(GlancePalette.ink2(scheme)).lineLimit(1)
                }
            }
        case .lines, .rows:
            VStack(alignment: .leading, spacing: style == .rows ? 3 : 1) {
                heading
                if let caveat, caveat.cantReach != nil {
                    // Two lines: it stands for both lanes, so it keeps the
                    // style inside its four.
                    line(caveat.line, lines: 2)
                } else {
                    ForEach(Array(glance.now.enumerated()), id: \.offset) { _, each in laneRow(each) }
                }
                if let next = glance.next { line("Next: \(Self.shown(next))") }
            }
        }
    }

    private var heading: some View {
        HStack(spacing: 4) {
            Text(glance.workspace)
                .font(caption(.semibold))
                .lineLimit(1)
                .layoutPriority(2)
            if let needs = glance.needsYouWords {
                Text("·").font(caption()).foregroundStyle(.secondary)
                Text(needs)
                    .font(caption(.medium))
                    .foregroundStyle(GlancePalette.amber(scheme))
                    .lineLimit(1)
                    .layoutPriority(1)
            }
            // A remembered plan says how old it is here, where it costs no line.
            if let caveat, caveat.cantReach == nil {
                Text("· \(GlanceAge.stated(caveat.age))").font(caption()).foregroundStyle(.secondary).lineLimit(1)
            }
        }
    }

    @ViewBuilder private func laneRow(_ lane: PlanGlance.Lane) -> some View {
        switch style {
        case .rows:
            HStack(spacing: 6) {
                Text(Self.shown(lane.name)).font(caption(mono: true)).lineLimit(1)
                Spacer(minLength: 4)
                Text(lane.state.word).font(caption()).foregroundStyle(.secondary).lineLimit(1).layoutPriority(1)
            }
        default:
            line("\(Self.shown(lane.name)) · \(lane.state.word)")
        }
    }

    private func line(_ text: String, lines: Int = 1) -> some View {
        Text(text).font(caption()).foregroundStyle(.secondary).lineLimit(lines)
    }

    /// "Main · 2 need you · Next: mac-fu3".
    private var cardFirst: String {
        ([glance.heading] + [glance.next.map { "Next: \(Self.shown($0))" }].compactMap { $0 }).joined(separator: " · ")
    }

    /// Now, or why it isn't said: "Now: mac-ux In Review, ov-310 Building",
    /// "Can’t reach Studio · 3h ago", "As of 3h ago"; nil on a stale card.
    private var cardSecond: String? {
        if let caveat, caveat.cantReach != nil { return caveat.line }
        if stale { return nil }
        let now = glance.now.isEmpty ? nil : "Now: " + glance.now.map { "\(Self.shown($0.name)) \($0.state.word)" }.joined(separator: ", ")
        return [caveat?.line, now].compactMap { $0 }.joined(separator: " · ").nilIfEmpty
    }

    private var spoken: String {
        [glance.spoken, caveat?.line].compactMap { $0 }.joined(separator: " ")
    }

    /// The caption at the reader's text size, up to this style's largest:
    /// Large for an accessory, xxx-Large for a widget's rows. Sized here
    /// rather than by `.caption` and a `.dynamicTypeSize` clamp so the size a
    /// surface draws is the size `PlanGlanceFitTests` measures, on a Mac too.
    private func caption(_ weight: Font.Weight = .regular, mono: Bool = false) -> Font {
        let cap: DynamicTypeSize = style == .rows ? .xxxLarge : .large
        return .system(size: Self.captionPoints(min(typeSize, cap)), weight: weight, design: mono ? .monospaced : .default)
    }

    /// iOS's Caption 1 at each text size, in points.
    static func captionPoints(_ size: DynamicTypeSize) -> CGFloat {
        switch size {
        case .xSmall, .small, .medium: 11
        case .large: 12
        case .xLarge: 13
        case .xxLarge: 14
        case .xxxLarge: 15
        default: 18
        }
    }

    /// A name cut to `nameShown` characters, with an ellipsis when cut.
    public static func shown(_ name: String) -> String {
        name.count > nameShown ? String(name.prefix(nameShown - 1)) + "…" : name
    }
}

extension String {
    fileprivate var nilIfEmpty: String? { isEmpty ? nil : self }
}
