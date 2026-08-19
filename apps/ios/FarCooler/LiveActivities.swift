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

    /// Cards this app ended itself as duplicates, by activity id.
    ///
    /// Their ending is not news the relay can use. The row it keeps is keyed on
    /// the TERMINAL, and the card that survived has the same one — so reporting
    /// `updateToken: nil` for a duplicate would throw away the surviving card's
    /// only address and leave it on the lock screen with nothing able to end it.
    private var reaped: Set<String> = []

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
        //
        // Duplicates are cleared before any of them is watched, because two
        // cards for one terminal both file their token against the same row and
        // the loser becomes a card nothing can ever end.
        reapDuplicates()
        for activity in Activity<AgentActivityAttributes>.activities { watch(activity) }
        Task {
            for await activity in Activity<AgentActivityAttributes>.activityUpdates {
                reapDuplicates()
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
                    //
                    // Which of the two it was is the phone's to report, and only
                    // the phone can: `.dismissed` is a person swiping the card
                    // away. The relay pushes a working card every ten seconds,
                    // so a dismissal it was not told about was a card back on
                    // the lock screen ten seconds later, for the life of the
                    // run. Anything else — the relay's own `end`, a card iOS
                    // retired — is not a refusal and must not silence the next
                    // one.
                    if !reaped.contains(activity.id) {
                        await Account.shared.registerActivityToken(
                            terminal: terminal,
                            updateToken: nil,
                            environment: PushRegistration.environment,
                            dismissed: state == .dismissed)
                    }
                    watched.remove(activity.id)
                    reaped.remove(activity.id)
                    return
                }
            }
        }
    }

    /// Which of two cards for one terminal to keep. Larger wins.
    ///
    /// `blocked` first, and that is the load-bearing part. The duplicate exists
    /// because a `blocked` push replaced a card stuck on "Working", so ending
    /// the wrong one puts the lock screen back to saying the opposite of what
    /// the banner beside it says — the failure the replacement exists to fix.
    ///
    /// Then the stale date, which is the only date a card carries: the relay
    /// gives every card it starts one an hour out, so a later stale date is a
    /// later start. It cannot decide this alone, because that date is in whole
    /// SECONDS on the wire — see `stale-date` in `services/relay/src/push.ts` —
    /// so a working start and the blocked start that replaced it can carry the
    /// identical value.
    ///
    /// Then the id, so nothing is left to `Activity.activities`' order, which is
    /// not documented and is not stable.
    private static func precedence(
        _ card: Activity<AgentActivityAttributes>
    ) -> (Int, Date, String) {
        (
            AgentStatus(card.content.state.status) == .blocked ? 1 : 0,
            card.content.staleDate ?? .distantPast,
            card.id
        )
    }

    /// Leave one card per terminal, and end the rest.
    ///
    /// The relay can start a card it cannot address — the app was not running to
    /// report an update token — and when the agent then blocks it starts a
    /// corrected one rather than leave the lock screen reading "Working" beside
    /// a banner that says otherwise. That is the right trade, and this is what
    /// keeps its cost to a moment: the loser is ended the next time the app
    /// runs, which is almost always the tap on that very banner.
    ///
    /// Which card loses is `precedence`, and it is not simply the older one.
    private func reapDuplicates() {
        let live = Activity<AgentActivityAttributes>.activities
        for (_, cards) in Dictionary(grouping: live, by: { $0.attributes.terminal }) {
            guard cards.count > 1 else { continue }
            let keep = cards.max { Self.precedence($0) < Self.precedence($1) }
            for card in cards where card.id != keep?.id {
                // Marked before it is ended, and in both sets on purpose.
                //
                // `watched` refuses this card a watcher: `watch` starts a token
                // loop as well as a state loop, and filing a dying card's push
                // token would overwrite the surviving card's address on the
                // relay's row, which is keyed on the terminal they share.
                //
                // `reaped` is for a loop that is ALREADY running, which cannot
                // be called off — it tells that loop not to report this ending,
                // for the same reason, and removes both ids on its way out.
                // Nothing removes them for a card nobody was watching: there is
                // no loop to notice it went. That leak is one string per
                // duplicate per launch — duplicates are rare and at most one per
                // terminal — and the only way to collect it would be to watch
                // the card, which is precisely what must not happen.
                reaped.insert(card.id)
                watched.insert(card.id)
                // Immediately, not on a delay. This card is a duplicate of one
                // the person is looking at right now, and a duplicate that
                // lingers a minute is the second card this exists to avoid.
                Task { await card.end(nil, dismissalPolicy: .immediate) }
            }
        }
    }

    /// APNs tokens travel as hex, the same way the device token does.
    private static func hex(_ token: Data) -> String {
        token.map { String(format: "%02x", $0) }.joined()
    }
}
