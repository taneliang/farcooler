import SwiftUI

/// The plan's glance, drawn (ov-310): the board and what needs you on it, the
/// lanes in Now and the one next up.
///
/// One view for every surface that says it, so the phone's widget, the Live
/// Activity, the watch app and its complication can't word it four ways:
///
/// - `.lines`: three one-line texts, for a lock screen card, a Smart Stack
///   slot or an accessory: "Main · 2 need you", "Now: mac-ux In Review,
///   ov-310 Building", "Next: mac-fu3".
/// - `.rows`: a row per Now lane with its state beside it, for a home screen
///   widget and the watch app's list.
///
/// Amber only on the count, the one part that asks for the owner. VoiceOver
/// reads the whole glance as one sentence (`PlanGlance.spoken`).
public struct PlanGlanceView: View {
    public enum Style: Sendable { case lines, rows }

    @Environment(\.colorScheme) private var scheme
    private let glance: PlanGlance
    private let style: Style

    public init(_ glance: PlanGlance, style: Style = .lines) {
        self.glance = glance
        self.style = style
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: style == .rows ? 3 : 1) {
            heading
            switch style {
            case .lines:
                if let now = glance.nowLine { line(now) }
            case .rows:
                ForEach(Array(glance.now.enumerated()), id: \.offset) { _, lane in
                    HStack(spacing: 6) {
                        Text(lane.name)
                            .font(.caption.monospaced())
                            .lineLimit(1)
                        Spacer(minLength: 4)
                        Text(lane.state.word)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
            }
            if let next = glance.nextLine { line(next) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(glance.spoken)
    }

    private var heading: some View {
        HStack(spacing: 4) {
            Text(glance.workspace)
                .font(.caption.weight(.semibold))
                .lineLimit(1)
            if let needs = glance.needsYouWords {
                Text("·").font(.caption).foregroundStyle(.secondary)
                Text(needs)
                    .font(.caption.weight(.medium))
                    .foregroundStyle(GlancePalette.amber(scheme))
                    .lineLimit(1)
                    .layoutPriority(1)
            }
        }
    }

    private func line(_ text: String) -> some View {
        Text(text)
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
    }
}
