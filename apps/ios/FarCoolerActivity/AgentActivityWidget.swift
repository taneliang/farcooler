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
/// **One card, however many agents there are.** It used to be one card per
/// terminal, which put four stacked cards on a lock screen for four running
/// agents and left the Dynamic Island — which presents exactly one activity —
/// picking between them with no rule anybody wrote. So the card LEADS with one
/// agent, chosen by the relay on the same precedence the rest of this product
/// uses (blocked first), and COUNTS the others.
///
/// The two halves come from two places, and that is deliberate:
///
///   - **the leader** is `context.state`, which arrives by push and is therefore
///     always as fresh as the last thing that happened;
///   - **the tail** is the fleet snapshot in the App Group, because nothing on
///     the push side can honestly count a fleet — the relay stores none, and a
///     daemon knows only its own runner. This extension already reads that file
///     for `FleetWidget`, and `FarCoolerNotify` refreshes it from every push
///     that alerts.
///
/// A snapshot is older than the push that arrived with the leader, sometimes by
/// a lot, so `FleetTail` qualifies what it says rather than asserting it — the
/// same rule `FleetSnapshot.complete` and `confidence(in:at:)` already put on
/// every other surface outside the app.
///
/// Nothing here reaches into the app. The extension has no network, no daemon
/// connection, and no way to ask about the fleet beyond that one file.
///
/// The runner's field is still spelled `machine`: the relay encodes the payload
/// by field name, so renaming it here would only stop the push arriving. It is
/// **runner** in every word a person reads.
struct AgentActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: AgentActivityAttributes.self) { context in
            LockScreenCard(state: context.state, tail: FleetTail.current(excluding: context.state.terminal))
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
                //
                // Off the CONTENT STATE now rather than the attributes, which is
                // what makes it follow a change of leader: the card is rendered
                // again on every push, so the URL is rebuilt with it.
                .widgetURL(terminalURL(context.state.terminal))
        } dynamicIsland: { context in
            let status = AgentStatus(context.state.status)
            let tail = FleetTail.current(excluding: context.state.terminal)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    StatusBadge(status: status)
                        .padding(.leading, 4)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    Text(context.state.machine)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.trailing, 4)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(context.state.label)
                            .font(.headline)
                        let body = context.state.detail
                        if !body.isEmpty {
                            Text(body)
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                        }
                        if let started = context.state.startedAt {
                            Text(started, style: .timer)
                                .font(.caption.monospacedDigit())
                                .foregroundStyle(.tertiary)
                        }
                        // The rest of the fleet gets one line here, the same
                        // line the lock screen card ends with. Expanded is the
                        // presentation with room for it, and without it the
                        // Island would be the one surface that still claims a
                        // single agent is all there is.
                        if let rest = tail.line {
                            Text(rest)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .opacity(tail.qualified ? 0.6 : 1)
                                .padding(.top, 2)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            } compactLeading: {
                Image(systemName: status.symbol)
                    .foregroundStyle(status.tint)
            } compactTrailing: {
                // The leader's name, and how many agents are behind it.
                //
                // The name alone was right when there was a card per terminal
                // and the only question was which card this is. With one card
                // the question changed: the icon carries the leader's state and
                // the name carries who it is, so the fact worth the remaining
                // few points is that the leader is not the whole story.
                Text(tail.others > 0 ? "\(context.state.label) +\(tail.others)" : context.state.label)
                    .font(.caption2)
                    .foregroundStyle(status.tint)
                    .lineLimit(1)
                    .frame(maxWidth: 74)
            } minimal: {
                Image(systemName: status.symbol)
                    .foregroundStyle(status.tint)
            }
            .widgetURL(terminalURL(context.state.terminal))
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
///
/// Nil for an empty id, which is not a theoretical case: a card started by a
/// build older than the fleet restructure has no terminal in its content state
/// at all — see `AgentActivityAttributes.ContentState.init(from:)` — and
/// `farcooler://terminal/` opens nothing while looking exactly like a card that
/// ignored the tap. No URL at least leaves the system's own behavior.
private func terminalURL(_ terminal: String) -> URL? {
    guard !terminal.isEmpty else { return nil }
    return URL(string: "\(AppScheme.current)://terminal/\(terminal)")
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

/// Everyone the card is NOT leading with, counted rather than listed.
///
/// A count and not a list because of two hard limits pulling the same way.
/// ActivityKit caps a content state at 4KB and the Dynamic Island's expanded
/// presentation is a few lines tall, so a card that grew a row per agent would
/// be a card that stops rendering on the fleet that most needs it. And a count
/// is what the surface is for: the lock screen answers "does this need me", and
/// the app is one tap away for the rest.
///
/// **Read from the snapshot, not from the push.** The relay stores no fleet and
/// a daemon knows only its own runner, so neither can count this honestly; the
/// snapshot in the App Group is the only thing on the phone that has ever seen
/// the whole fleet. That it is a local file read on a render path is fine — it
/// is one small JSON, and `FleetWidget` in this same extension reads it the same
/// way on every timeline.
///
/// **It qualifies rather than asserts**, which is the rule every surface outside
/// the app follows. Two separate things make it unsure and they are not the
/// same: `complete` false means the snapshot was assembled from pushes and may
/// not know about every agent, so the count can be LOW; a working agent past
/// `FleetSnapshot.staleAfter` means the snapshot no longer vouches for what that
/// agent is doing. Either one drops the line to the 60% treatment
/// `FleetWidget` and `WatchFleetWidget` already use for "last seen", and the
/// second one puts the words there too.
///
/// Nothing at all when no snapshot has ever been written. `FleetSnapshot.empty`
/// is the absence of an observation, not an empty fleet, and a phone that has
/// never opened the app may not claim the leader is the only agent there is.
struct FleetTail {
    /// How many other agents are working or waiting.
    let others: Int
    /// How many of those are waiting on a person.
    let blocked: Int
    /// Whether this is a claim or a recollection.
    let qualified: Bool

    /// Nothing known, which draws nothing.
    static let unknown = FleetTail(others: 0, blocked: 0, qualified: false)

    static func current(excluding leader: String, now: Date = Date()) -> FleetTail {
        guard let snapshot = SnapshotStore.read(),
            snapshot.capturedAt.timeIntervalSince1970 > 0
        else { return .unknown }

        // `done` agents are not "more working" — they are finished runs the
        // snapshot has not dropped yet, and counting them would make a card
        // claim an idle fleet is busy.
        let rest = snapshot.agents.filter {
            $0.id != leader && ($0.status == "working" || $0.status == "blocked")
        }
        guard !rest.isEmpty else { return .unknown }

        return FleetTail(
            others: rest.count,
            blocked: rest.filter { $0.status == "blocked" }.count,
            // Blocked is latched, so `confidence` only ever withdraws a
            // `working` claim — which is why the words below qualify the verb
            // and not the number. An incomplete snapshot is the other half, and
            // it qualifies the number instead: there may be agents in it that
            // nothing has ever told this phone about.
            qualified: !snapshot.complete
                || rest.contains { snapshot.confidence(in: $0, at: now) == .lastSeen })
    }

    /// The one line the card gives everyone else, or nil when there is nobody
    /// else to give it to.
    ///
    /// "+3 more working" and "+3 more · 1 needs you", because those are the two
    /// questions a lock screen answers: how much is running, and is any of it
    /// waiting on me. The blocked half wins the second clause even though the
    /// leader is almost always the blocked one — a second agent blocking while
    /// the first is unanswered is the case where a person most needs to know the
    /// card is not the whole story.
    var line: String? {
        guard others > 0 else { return nil }
        if blocked > 0 {
            return "+\(others) more · \(blocked) need\(blocked == 1 ? "s" : "") you"
        }
        return qualified ? "+\(others) more last seen working" : "+\(others) more working"
    }
}

/// The lock screen presentation: the leader, then everyone else in one line.
private struct LockScreenCard: View {
    let state: AgentActivityAttributes.ContentState
    let tail: FleetTail

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            LeaderRow(state: state)
            if let rest = tail.line {
                Divider()
                Text(rest)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .opacity(tail.qualified ? 0.6 : 1)
                    .lineLimit(1)
            }
        }
        .padding(16)
    }
}

/// The one agent the card is about: name and runner on one line, what it is
/// doing under them, its own clock, and a colored badge on the right.
///
/// Its own view rather than inlined, and that is worth keeping: the leader is
/// the part of this card that gets controls. A `Review` on a finished run and
/// an answer on a blocked one both belong in this `VStack`, under `detail` and
/// beside the timer, where the target shape puts them — not in `LockScreenCard`
/// beside the fleet line, which is about everybody else.
private struct LeaderRow: View {
    let state: AgentActivityAttributes.ContentState

    var body: some View {
        let status = AgentStatus(state.status)
        HStack(alignment: .center, spacing: 14) {
            VStack(alignment: .leading, spacing: 3) {
                // Name and runner on one line rather than stacked. The card has
                // a fleet line to fit now, and "claude · studio" is how every
                // other surface in this product names an agent.
                Text(runnerSuffixed)
                    .font(.headline)
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
                // last push. The system counts this one, network or not — and
                // it counts from the LEADER's start, which is why that date
                // moved onto the content state with the rest of the leader.
                if let started = state.startedAt {
                    Text(started, style: .timer)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.tertiary)
                        .padding(.top, 2)
                }
            }
            Spacer(minLength: 0)
            StatusBadge(status: status)
        }
    }

    /// "claude · studio", or just the name when the runner is not known.
    ///
    /// A card started by a build older than the fleet restructure carries
    /// neither — see `ContentState.init(from:)` — so the separator has to be
    /// conditional or the card leads with a bare "·".
    private var runnerSuffixed: String {
        guard !state.machine.isEmpty else { return state.label }
        guard !state.label.isEmpty else { return state.machine }
        return "\(state.label) · \(state.machine)"
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
