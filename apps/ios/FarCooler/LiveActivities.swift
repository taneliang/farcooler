import ActivityKit
import Foundation

/// The phone's half of the Live Activity contract: hand the relay the tokens it
/// cannot mint, and take them back when a card is gone.
///
/// Nothing here creates or draws a card. The relay starts them (see
/// `services/relay/src/push.ts`) and the widget extension draws them; this type
/// exists only because two of the three tokens ActivityKit issues can be
/// obtained nowhere but inside the running app:
///
///   - the push-to-start token, one per install, which is what lets the relay
///     raise a card while the app is not running — the whole point of the
///     feature, since an agent going quiet is not something the phone can
///     notice on its own;
///   - an update token per running activity, which is the only thing that can
///     change or dismiss THAT card. The relay has no way to derive it.
///
/// Both are handed over as they arrive rather than fetched when needed, because
/// there is no "when needed" — by the time the relay wants to end a card the app
/// may not have run for hours.
@MainActor
final class LiveActivities {
    static let shared = LiveActivities()

    /// Activities already being watched, by activity id.
    ///
    /// `activityUpdates` re-yields activities that are already running — on
    /// launch it replays every live one — and each `watch` starts two
    /// long-running loops. Without this, relaunching with three cards up would
    /// leave six token loops racing to file the same tokens.
    private var watched: Set<String> = []

    private init() {}

    /// Begin watching. Safe to call once per launch, from the app's `.task`.
    func start() {
        // Not a guard against `areActivitiesEnabled`. That reads false at
        // launch on a device where the person has never seen a card, and it can
        // turn true later without the app relaunching; bailing here would mean
        // the token never gets filed and the feature never starts working. The
        // sequences below simply yield nothing while it is off.
        Task {
            for await token in Activity<AgentActivityAttributes>.pushToStartTokenUpdates {
                PushRegistration.shared.liveActivityStartToken = Self.hex(token)
            }
        }

        // Already-running cards first, then new ones. A card outlives the app:
        // the relay can start one while the app is not running, and when the
        // app next launches that activity is already there with an update token
        // the relay has never been told about.
        for activity in Activity<AgentActivityAttributes>.activities { watch(activity) }
        Task {
            for await activity in Activity<AgentActivityAttributes>.activityUpdates {
                watch(activity)
            }
        }
    }

    private func watch(_ activity: Activity<AgentActivityAttributes>) {
        guard watched.insert(activity.id).inserted else { return }
        let terminal = activity.attributes.terminal

        Task {
            for await token in activity.pushTokenUpdates {
                await Account.shared.registerActivityToken(
                    terminal: terminal,
                    updateToken: Self.hex(token),
                    environment: PushRegistration.environment)
            }
        }

        Task {
            for await state in activity.activityStateUpdates {
                switch state {
                case .active, .stale:
                    // Stale means the content is old, not that the card is
                    // gone. Its token still works, and clearing it here would
                    // strand a card on the lock screen that nothing can dismiss.
                    continue
                default:
                    // Ended or dismissed — and anything a later iOS adds that
                    // is neither active nor stale, which is the safe way to
                    // guess for a non-frozen enum. Telling the relay to forget
                    // the token matters: keeping it means the next `done` for
                    // this terminal pushes to a token APNs has retired, and the
                    // relay counts a delivery that cannot have happened.
                    await Account.shared.registerActivityToken(
                        terminal: terminal,
                        updateToken: nil,
                        environment: PushRegistration.environment)
                    watched.remove(activity.id)
                    return
                }
            }
        }
    }

    /// APNs tokens travel as hex, the same way the device token does.
    private static func hex(_ token: Data) -> String {
        token.map { String(format: "%02x", $0) }.joined()
    }
}
