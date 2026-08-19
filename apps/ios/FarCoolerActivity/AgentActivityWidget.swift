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
/// runner name and the label as plain strings rather than ids to look up.
///
/// That field is still spelled `machine`: the relay encodes the payload by
/// field name, so renaming it here would only stop the push arriving.
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
                // The same tap target the Island gets, and it has to be applied
                // HERE as well: `.widgetURL` on the `dynamicIsland` builder
                // covers only that presentation. Without it a tap on the lock
                // screen card — the presentation this whole feature is named
                // for — opened the app's front door instead of the terminal the
                // card is about, which is indistinguishable from a card that
                // ignores taps.
                .widgetURL(terminalURL(context.attributes.terminal))
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
                        let body = context.state.detail
                        if !body.isEmpty {
                            Text(body)
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                        }
                        if let started = context.attributes.startedAt {
                            Text(started, style: .timer)
                                .font(.caption.monospacedDigit())
                                .foregroundStyle(.tertiary)
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
            .widgetURL(terminalURL(context.attributes.terminal))
        }
    }
}

/// Where a tap on either presentation lands.
///
/// `FleetView.onOpenURL` reads the id back out and opens that terminal. It is in
/// the URL rather than added later because the id is known here and nowhere
/// else, and a card already on someone's lock screen cannot be given a better
/// URL retroactively.
///
/// One function because there are two presentations and they must not drift:
/// the lock screen card spent a while with no `widgetURL` at all, since the
/// modifier had been written once, on the Island's builder, where it looks like
/// it covers both.
private func terminalURL(_ terminal: String) -> URL? {
    URL(string: "\(AppScheme.current)://terminal/\(terminal)")
}

/// This build's URL scheme.
///
/// `farcooler` for stable, `farcooler-canary` and friends for the rest. It has
/// to be read rather than written down: this file hardcoded `farcooler://`,
/// which every non-stable channel does NOT register — so tapping a canary
/// card opened stable if it happened to be installed, and opened nothing at all
/// if it did not. The app's own Info.plist documents the same hazard for
/// sign-in; the widget was missed because `ACTIVITY_COMMON` in
/// generate-project.py deliberately does not inherit `TARGET_COMMON`.
///
/// Read from this extension's own bundle, not the app's — `Bundle.main` in an
/// appex is the appex — which is why `FarCoolerURLScheme` has to be stamped
/// into `FarCoolerActivity/Info.plist` as well. The fallback is stable's
/// scheme and is unreachable in a generated build; it exists so a missing key
/// produces a link that opens the wrong channel rather than `://terminal/…`,
/// which opens nothing and cannot be told apart from a card that ignored the
/// tap.
enum AppScheme {
    static var current: String {
        Bundle.main.object(forInfoDictionaryKey: "FarCoolerURLScheme") as? String
            ?? "farcooler"
    }
}

/// The lock screen presentation: name and runner on one line, what it is doing
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
                let body = state.detail
                if !body.isEmpty {
                    Text(body)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .padding(.top, 2)
                }
                // How long this turn has been going. `.timer` and not a string
                // we compute: the extension gets no wake-up per second, so
                // anything we render ourselves is frozen at the moment of the
                // last push. The system counts this one, network or not.
                if let started = attributes.startedAt {
                    Text(started, style: .timer)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.tertiary)
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
