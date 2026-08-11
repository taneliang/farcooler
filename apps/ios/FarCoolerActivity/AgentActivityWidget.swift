import ActivityKit
import SwiftUI
import WidgetKit

/// The lock screen card, and the Dynamic Island that stands in for it when the
/// phone is unlocked.
///
/// A separate binary from the app on purpose — this is not a choice. WidgetKit
/// renders Live Activities out of process so the card keeps drawing when the app
/// is not running, which is the entire case this feature exists for: an agent
/// stops for an answer while the phone is in a pocket.
///
/// Nothing here reaches into the app. The extension has no network, no daemon
/// connection, and no access to the fleet; everything it draws arrives in
/// `AgentActivityAttributes` from a push. That is why the attributes carry the
/// machine name and the label as plain strings rather than ids to look up.
struct AgentActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: AgentActivityAttributes.self) { context in
            LockScreenCard(attributes: context.attributes, state: context.state)
                // The card's own background. Left to the system's material
                // rather than a color of ours: the lock screen wallpaper is
                // behind it and a flat fill sits on top of the photo like a
                // sticker.
                .activityBackgroundTint(nil)
                .activitySystemActionForegroundColor(.primary)
        } dynamicIsland: { context in
            let status = AgentStatus(context.state.status)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    StatusBadge(status: status)
                        .padding(.leading, 4)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    Text(context.attributes.machine)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.trailing, 4)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(context.attributes.label)
                            .font(.headline)
                        if !context.state.detail.isEmpty {
                            Text(context.state.detail)
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            } compactLeading: {
                Image(systemName: status.symbol)
                    .foregroundStyle(status.tint)
            } compactTrailing: {
                // The name, not the status: the icon already carries the status,
                // and with several agents running the only thing that tells two
                // cards apart is which one this is.
                Text(context.attributes.label)
                    .font(.caption2)
                    .foregroundStyle(status.tint)
                    .lineLimit(1)
                    .frame(maxWidth: 64)
            } minimal: {
                Image(systemName: status.symbol)
                    .foregroundStyle(status.tint)
            }
            // Where a tap lands. The app currently opens to wherever it was, so
            // this is only the front door — nothing routes on the terminal id
            // yet. It is in the URL rather than added later because the id is
            // known here and nowhere else, and a card already on someone's lock
            // screen cannot be given a better URL retroactively.
            .widgetURL(URL(string: "farcooler://terminal/\(context.attributes.terminal)"))
        }
    }
}

/// The lock screen presentation: name and machine on one line, what it is doing
/// under them, a colored badge on the right.
private struct LockScreenCard: View {
    let attributes: AgentActivityAttributes
    let state: AgentActivityAttributes.ContentState

    var body: some View {
        let status = AgentStatus(state.status)
        HStack(alignment: .center, spacing: 14) {
            VStack(alignment: .leading, spacing: 3) {
                Text(attributes.label)
                    .font(.headline)
                    .lineLimit(1)
                Text(attributes.machine)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                if !state.detail.isEmpty {
                    Text(state.detail)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .padding(.top, 2)
                }
            }
            Spacer(minLength: 0)
            StatusBadge(status: status)
        }
        .padding(16)
    }
}

/// The icon and word for a state, drawn the same way in both presentations.
private struct StatusBadge: View {
    let status: AgentStatus

    var body: some View {
        VStack(spacing: 4) {
            Image(systemName: status.symbol)
                .font(.title2)
            Text(status.title)
                .font(.caption2.weight(.medium))
        }
        .foregroundStyle(status.tint)
    }
}

extension AgentStatus {
    var symbol: String {
        switch self {
        case .working: "circle.dotted"
        case .blocked: "exclamationmark.bubble.fill"
        case .done: "checkmark.circle.fill"
        }
    }

    /// Blocked is the only one that gets a warm color, and that is the point.
    /// Working and finished are both states nobody has to act on; amber is
    /// reserved for the one that is waiting on a person, so a glance at a locked
    /// phone answers "does this need me" without reading a word.
    var tint: Color {
        switch self {
        case .working: .secondary
        case .blocked: .orange
        case .done: .green
        }
    }
}
