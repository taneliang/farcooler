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
///
/// **There is exactly one card per install.** Everything below is written to
/// that invariant rather than to a per-terminal one: the relay keeps one row per
/// install to address, and a second live activity here is a second card on the
/// lock screen that the relay does not know it has and can therefore never end.
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

    /// Cards this app ended itself as extras, by activity id.
    ///
    /// Their ending is not news the relay can use. The row it keeps is keyed on
    /// the INSTALL, and the card that survived shares it — so reporting
    /// `updateToken: nil` for an extra would throw away the surviving card's
    /// only address and leave it on the lock screen with nothing able to end it.
    ///
    /// The hazard got sharper with one card per install, not milder. When the
    /// row was keyed on a terminal this only mattered for two cards about the
    /// SAME agent; now every card on the phone points at the one row, so any
    /// extra reporting its own death takes the survivor's address with it.
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
        // Extras are cleared before any of them is watched, because every card
        // files its token against the same row and the loser becomes a card
        // nothing can ever end.
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

        Task {
            for await token in activity.pushTokenUpdates {
                await Account.shared.registerActivityToken(
                    terminal: Self.leader(of: activity),
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
                    // the token matters: keeping it means the next `done`
                    // pushes to a token APNs has retired, and the relay counts
                    // a delivery that cannot have happened.
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
                            terminal: Self.leader(of: activity),
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

    /// The terminal this card is currently leading with.
    ///
    /// Read off the CONTENT STATE at the moment of reporting rather than
    /// captured when the watcher started, because the leader changes over the
    /// card's life — a card that opened on one agent and now leads with another
    /// would otherwise file its token under the agent it has stopped being
    /// about.
    ///
    /// The relay no longer keys anything on it: `/v1/devices/activity` holds one
    /// row per install and ignores this field. It is still sent because a relay
    /// deployed before that rekey REQUIRES it — a missing `terminal` is a 400
    /// there — and the app and its relay are one deployment per channel but not
    /// one atomic one. Sending the leader costs nothing and is the most truthful
    /// thing the phone can put in a field whose meaning has moved on.
    private static func leader(of activity: Activity<AgentActivityAttributes>) -> String {
        activity.content.state.terminal
    }

    /// Which of two cards to keep. Larger wins.
    ///
    /// **Fleet-shaped first.** An upgrade lands with terminal-scoped cards from
    /// the old build still in flight, and those are the ones to end: they lead
    /// with an agent they cannot re-lead, they draw from a content state this
    /// build fills differently, and there may be one per running agent. A
    /// legacy card winning this comparison would end the live fleet card and
    /// leave the lock screen showing whichever single agent happened to have a
    /// card up when the app was updated. `AgentActivityAttributes.version` is
    /// what answers this, and it has to be asked rather than inferred — both
    /// shapes are the same ActivityKit type, and `ContentState` decodes an old
    /// card leniently, so an empty label is a guess and not an answer.
    ///
    /// Then `blocked`, and that is the load-bearing part. The extra card exists
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
    ) -> (Int, Int, Date, String) {
        (
            card.attributes.isFleetShaped ? 1 : 0,
            AgentStatus(card.content.state.status) == .blocked ? 1 : 0,
            card.content.staleDate ?? .distantPast,
            card.id
        )
    }

    /// Leave exactly ONE card, and end every other.
    ///
    /// Not one per terminal any more — one, full stop. Four running agents used
    /// to mean four stacked cards on the lock screen while the Dynamic Island,
    /// which can present a single activity, showed whichever it picked; the card
    /// now leads with one agent and counts the rest, so a second card is a
    /// second answer to a question that has one.
    ///
    /// Two things arrive here, and the grouping that used to separate them is
    /// gone because both have the same remedy:
    ///
    ///   - **Extras the relay raised.** It can start a card it cannot address —
    ///     the app was not running to report an update token — and when an agent
    ///     then blocks it starts a corrected one rather than leave the lock
    ///     screen reading "Working" beside a banner that says otherwise. That is
    ///     the right trade, and this is what keeps its cost to a moment: the
    ///     loser is ended the next time the app runs, which is almost always the
    ///     tap on that very banner.
    ///   - **In-flight cards from the build before this one.** Those are
    ///     terminal-scoped, there may be several, and nothing else will ever
    ///     take them down: the relay's rekey means it no longer holds a row per
    ///     terminal to end them with, so left alone they sit out their hour-long
    ///     stale date beside the new card. `precedence` refuses to let one of
    ///     them be the survivor.
    ///
    /// Which card loses is `precedence`, and it is not simply the older one.
    private func reapDuplicates() {
        let cards = Activity<AgentActivityAttributes>.activities
        guard cards.count > 1 else { return }
        let keep = cards.max { Self.precedence($0) < Self.precedence($1) }
        for card in cards where card.id != keep?.id {
            // Marked before it is ended, and in both sets on purpose.
            //
            // `watched` refuses this card a watcher: `watch` starts a token
            // loop as well as a state loop, and filing a dying card's push
            // token would overwrite the surviving card's address on the relay's
            // row, which is keyed on the install they share.
            //
            // `reaped` is for a loop that is ALREADY running, which cannot be
            // called off — it tells that loop not to report this ending, for
            // the same reason, and removes both ids on its way out. Nothing
            // removes them for a card nobody was watching: there is no loop to
            // notice it went. That leak is one string per extra card per
            // launch — extras are rare, and the largest burst is the one-off
            // sweep of an upgrade's terminal-scoped leftovers — and the only
            // way to collect it would be to watch the card, which is precisely
            // what must not happen.
            reaped.insert(card.id)
            watched.insert(card.id)
            // Immediately, not on a delay. This card is beside one the person
            // is looking at right now, and one that lingers a minute is the
            // second card this exists to avoid.
            Task { await card.end(nil, dismissalPolicy: .immediate) }
        }
    }

    /// APNs tokens travel as hex, the same way the device token does.
    private static func hex(_ token: Data) -> String {
        token.map { String(format: "%02x", $0) }.joined()
    }
}
