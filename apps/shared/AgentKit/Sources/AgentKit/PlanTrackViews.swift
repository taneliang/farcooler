import SwiftUI

// The track line, drawn for the iPhone (ov-331). The words and rules are
// `PlanThemeTrack.swift`'s; this lays them out. Neutral words with a glyph,
// and amber only for a budget gone over, which says so in words as well.
// (The Mac draws its own in the canvas's pane sizes: `PlanThemeEntry`.)

/// "mac-ux is in review", "Queued, 1st up", "No lane · quiet for 3 days".
public struct PlanTrackLine: View {
    public let track: PlanTrack
    public let now: Int64
    public var font: Font
    @Environment(\.colorScheme) private var scheme

    public init(track: PlanTrack, now: Int64, font: Font = .footnote) {
        self.track = track
        self.now = now
        self.font = font
    }

    public var body: some View {
        let attention = track.needsAttention
        HStack(alignment: .firstTextBaseline, spacing: Spacing.tight + 2) {
            Image(systemName: track.symbol)
                .foregroundStyle(attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
                .accessibilityHidden(true)
            Text(PlanWords.track(track, now: now))
                .fontWeight(attention ? .medium : .regular)
                .foregroundStyle(attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
        }
        .font(font)
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(PlanWords.trackSpoken(track, now: now))
        .accessibilityIdentifier(attention ? "plan-budget-over" : "plan-theme-track")
    }
}
