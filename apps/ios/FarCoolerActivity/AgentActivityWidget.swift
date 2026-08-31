import ActivityKit
import AppIntents
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
            // Read once per render and handed down, the same way `FleetTail`
            // is. Both are App Group file reads on a render path, and both are
            // wanted by two presentations that must not disagree.
            let ask = LeaderAsk.current(for: context.state)
            LockScreenCard(
                state: context.state,
                // The fleet line steps aside while there is an answer on offer.
                // Not a preference: a lock screen card is capped at about 160
                // points and the leader already spends most of it, so a divider
                // and a line about everybody else are the difference between
                // the reject button being on the card and being clipped off the
                // bottom of it. The tail says how many others are running; the
                // buttons are the only thing on this surface that cannot be got
                // anywhere else.
                tail: ask.isPresent
                    ? FleetTail.unknown
                    : FleetTail.current(excluding: context.state.terminal),
                ask: ask)
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
            let ask = LeaderAsk.current(for: context.state)
            let tail =
                ask.isPresent
                ? FleetTail.unknown : FleetTail.current(excluding: context.state.terminal)
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
                        // The turn clock gives way to the answer, exactly as
                        // it does on the lock screen card and for the same
                        // reason: how long an agent has been stopped is worth
                        // less than being able to stop it being stopped.
                        if let started = context.state.startedAt, !ask.isPresent {
                            Text(started, style: .timer)
                                .font(.caption.monospacedDigit())
                                .foregroundStyle(.tertiary)
                        }
                        // §04's "44 · island row": the leader's own thirteen
                        // buckets, in the one Island presentation with a row to
                        // put them in. Absent when the runner sent no trace for
                        // this terminal, which draws nothing at all rather than
                        // a flat line at zero.
                        //
                        // The span is not printed beside it here. §04 asks for
                        // it "beside the trace", and the reason it is worth the
                        // width elsewhere is that rows at different windows are
                        // otherwise incomparable — there is exactly one row
                        // here, so there is nothing to compare it against, and
                        // an Island is the surface with the least room in the
                        // product.
                        if let trace = ActivityTrace(tail.leaderTrace) {
                            GlanceTraceView(trace, size: .islandRow)
                                .environment(\.colorScheme, .dark)
                                .padding(.top, 2)
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
                        // The same controls the lock screen card draws, from
                        // the same view. Expanded is the one Island
                        // presentation that can carry a decision — compact and
                        // minimal are a glyph and a word — and a card whose
                        // buttons appeared only when the phone was locked would
                        // be two different features wearing one name.
                        if ask.isPresent { AnswerControls(ask: ask) }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            } compactLeading: {
                // §07's compact presentation leads with the mark. 11pt, the
                // header diameter, which is what fits beside the pill's own
                // curve without the ring reading as part of it.
                //
                // Forced dark, because the Island is a black pill whatever the
                // phone's appearance is — §01's light palette answers a
                // different question, "what does this look like on a pale
                // backdrop", and there is no pale backdrop here.
                GlanceMarkView(status.mark, size: .header)
                    .environment(\.colorScheme, .dark)
            } compactTrailing: {
                // §07's compact presentation, which is about the FLEET: "Count
                // leading, fleet trace trailing, thirteen buckets like every
                // other." §04's 40pt size exists for this slot and no other.
                //
                // The name is what this drew before, and it is kept as the
                // fallback rather than deleted. The two are not alternatives
                // that were weighed: the trace is what §07 asks for, and the
                // name is what there is to say when the runner has sent no
                // fleet trace — a blank trailing region would be worse than
                // either. A fleet at rest sends no bytes at all, deliberately,
                // so this fallback is a state the product will really be in and
                // not a defensive branch.
                //
                // Forced dark for `compactLeading`'s reason: the Island is a
                // black pill whatever the phone's appearance is.
                if let trace = ActivityTrace(tail.fleetTrace) {
                    GlanceTraceView(trace, size: .island)
                        .environment(\.colorScheme, .dark)
                } else {
                    // The leader's name, and how many agents are behind it.
                    //
                    // The name alone was right when there was a card per
                    // terminal and the only question was which card this is.
                    // With one card the question changed: the icon carries the
                    // leader's state and the name carries who it is, so the
                    // fact worth the remaining few points is that the leader is
                    // not the whole story.
                    Text(
                        tail.others > 0
                            ? "\(context.state.label) +\(tail.others)" : context.state.label
                    )
                    .font(.caption2)
                    .foregroundStyle(status.tint)
                    .lineLimit(1)
                    .frame(maxWidth: 74)
                }
            } minimal: {
                // §07, verbatim: "MINIMAL: The ring alone at 15pt. No count, no
                // trace — history is unreadable at this size." The lone
                // indicator, which is the whole presentation.
                GlanceMarkView(status.mark, size: .lone)
                    .environment(\.colorScheme, .dark)
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
    /// The leader's own thirteen buckets, as the wire's bytes.
    ///
    /// **Off the snapshot and not off the push**, which is not a preference:
    /// ActivityKit caps a content state at 4KB and the trace is 66 bytes per
    /// agent, but the deciding fact is that the push is assembled by the relay
    /// from one notification and has never held a trace at all. The snapshot in
    /// the App Group is the only thing on this phone that has one, and this
    /// function is already reading it.
    var leaderTrace: Data? = nil
    /// The whole fleet's buckets, summed on the runner. §07's compact Island:
    /// "Count leading, fleet trace trailing, thirteen buckets like every other."
    var fleetTrace: Data? = nil

    /// Nothing known, which draws nothing.
    static let unknown = FleetTail(others: 0, blocked: 0, qualified: false)

    static func current(excluding leader: String, now: Date = Date()) -> FleetTail {
        guard let snapshot = SnapshotStore.read(),
            snapshot.capturedAt.timeIntervalSince1970 > 0
        else { return .unknown }

        // Both traces, read before the early return below. A fleet whose only
        // agent IS the leader has no tail and still has a history, and the two
        // questions are not the same one — `others` counts everybody else and
        // the trace is about time.
        let leaderTrace = snapshot.agents.first { $0.id == leader }?.trace

        // `done` agents are not "more working" — they are finished runs the
        // snapshot has not dropped yet, and counting them would make a card
        // claim an idle fleet is busy.
        let rest = snapshot.agents.filter {
            $0.id != leader && ($0.status == "working" || $0.status == "blocked")
        }
        guard !rest.isEmpty else {
            return FleetTail(
                others: 0, blocked: 0, qualified: false,
                leaderTrace: leaderTrace, fleetTrace: snapshot.fleetTrace)
        }

        return FleetTail(
            others: rest.count,
            blocked: rest.filter { $0.status == "blocked" }.count,
            // Blocked is latched, so `confidence` only ever withdraws a
            // `working` claim — which is why the words below qualify the verb
            // and not the number. An incomplete snapshot is the other half, and
            // it qualifies the number instead: there may be agents in it that
            // nothing has ever told this phone about.
            qualified: !snapshot.complete
                || rest.contains { snapshot.confidence(in: $0, at: now) == .lastSeen },
            leaderTrace: leaderTrace,
            // Carried, never summed here. The rows do not share a window width,
            // so adding bucket 4 of a five-minute row to bucket 4 of a two-hour
            // one would add two different spans of time — see
            // `FleetSnapshot.fleetTrace`, which is the runner's own sum.
            fleetTrace: snapshot.fleetTrace)
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

/// What the card's leader is waiting on, and what this phone last did about it.
///
/// The third thing this card draws and the only one it can ACT on. The leader
/// comes off the push, the tail comes off the fleet snapshot, and this comes
/// off a second file in the same App Group — `GlancePermissions`, whose own doc
/// comment sets out why the options cannot ride on the push and what it costs
/// that they do not.
///
/// **Gated on the PUSH, not on the file's age.** A permission record carries
/// the time it was written and that time is deliberately not a freshness test:
/// a permission left up over lunch is still live, and hiding the buttons after
/// ten minutes would hide them in exactly the case somebody wants them. What
/// decides whether an answer may be offered at all is `status`, which arrives
/// by push and is as fresh as the last thing that happened. A leader the runner
/// says is working or finished gets no buttons whatever this file holds.
///
/// **An empty terminal gets nothing**, which is not a theoretical case: a card
/// started by a build older than the fleet restructure has no terminal in its
/// content state at all, and a permission cannot be keyed to an agent the card
/// cannot name. That is also what keeps the overflow copy below honest — it
/// tells somebody to tap the card, and `terminalURL` returns nil for exactly
/// this state.
struct LeaderAsk {
    let terminal: String
    /// What the agent offered, if this phone has ever read it off the stream.
    let permission: GlancePermission?
    /// What this phone last sent about that permission, and how it went.
    let answer: GlanceAnswer?

    /// Nothing to say, which draws nothing and leaves the card as it was.
    static let none = LeaderAsk(terminal: "", permission: nil, answer: nil)

    static func current(
        for state: AgentActivityAttributes.ContentState, now: Date = Date()
    ) -> LeaderAsk {
        guard AgentStatus(state.status) == .blocked, !state.terminal.isEmpty else { return .none }
        let store = GlancePermissionStore.read()
        let permission = store.permission(for: state.terminal)
        var answer = store.answer(for: state.terminal).flatMap { $0.isFresh(at: now) ? $0 : nil }
        // An answer about a DIFFERENT request says nothing about this one, and
        // showing it beside these buttons would report on a question that is
        // already over. Dropped rather than drawn.
        if let permission, let standing = answer, standing.request != permission.request {
            answer = nil
        }
        guard permission != nil || answer != nil else { return .none }
        return LeaderAsk(terminal: state.terminal, permission: permission, answer: answer)
    }

    /// Whether the card has anything at all to add under the leader. What the
    /// timer and the fleet line give way to.
    var isPresent: Bool { permission != nil || answer != nil }

    /// Whether a tap may still write to this agent.
    ///
    /// False for every outcome except `nothingSent`, which is the one that
    /// established the runner was never written to. This is the card's version
    /// of what `PermissionView` gets from `@State` — buttons off for the
    /// duration of a send, handed back only on the failure that is safe to
    /// repeat — and it has to be persisted rather than held in memory, because
    /// the process that renders this card is not the process that sent the
    /// answer and may not have existed when it was sent.
    var offersButtons: Bool {
        guard permission != nil else { return false }
        guard let answer else { return true }
        return !answer.refusesAnotherTap
    }

    /// The sentence under the leader, if there is one to say.
    ///
    /// The three outcomes are kept apart by color as well as by words, on the
    /// reasoning `PermissionView` gives for drawing "Nothing to Answer" and
    /// "Couldn't Check" differently: two states that mean opposite things must
    /// not look alike at a glance. Green is the only one that claims anything
    /// happened.
    var note: (text: String, symbol: String, tint: Color)? {
        guard let answer else { return nil }
        switch answer.outcome {
        // No message is stored for a claim in flight — there is nothing known
        // yet to store — so the wait is named here. A row that simply went
        // quiet is indistinguishable from a tap that missed.
        case .inFlight: return ("Sending your answer…", "arrow.up.circle", .secondary)
        case .sent: return (answer.message, "checkmark.circle.fill", .green)
        case .unsure, .nothingSent: return (answer.message, "exclamationmark.triangle.fill", .red)
        }
    }
}

/// The agent's own answers, as buttons, and whatever came of the last one.
///
/// Drawn identically on the lock screen and in the expanded Dynamic Island,
/// from one view, because they are one decision presented twice — the same
/// reason `terminalURL` is one function.
///
/// **Nothing here shortens an option's name.** There is no `lineLimit` and no
/// `truncationMode` on a button label anywhere below, and there must not be:
/// `Allow Bash(cargo test…` is a button that hides what it allows, and
/// `PermissionView` refuses the same thing on a screen with more room than this
/// one. What gives way instead is the LIST — `GlancePermission.fit` decides how
/// many of the agent's answers there is room for, refuses to show a yes without
/// a no, and counts whatever it left off so the overflow line can say so.
private struct AnswerControls: View {
    let ask: LeaderAsk

    /// How much room a card has for buttons, in lines and in characters.
    ///
    /// **Estimated, and estimated LOW on purpose.** A widget extension cannot
    /// measure text before it lays it out, and the two directions cost
    /// differently: guessing small sends somebody into the app who could have
    /// answered from the card, while guessing large pushes a button off the
    /// bottom of a card that does not scroll — and the button at the bottom is
    /// the reject.
    ///
    /// Three lines is what is left of a lock screen card once the leader has
    /// had its name, one line of question and a badge, against the roughly 160
    /// points the system gives the presentation. Forty characters is a
    /// conservative reading of a `.footnote` line across a card that wide;
    /// SF Pro at 13 points fits nearer fifty. Neither number has been checked
    /// on a device — see this task's report — which is the other reason the
    /// labels below wrap freely: an underestimate costs a button, and an
    /// overestimate costs a second line rather than a clipped word.
    private static let lines = 3
    private static let columns = 40

    @ViewBuilder var body: some View {
        if ask.isPresent {
            VStack(alignment: .leading, spacing: 6) {
                if let note = ask.note {
                    Label(note.text, systemImage: note.symbol)
                        .font(.caption)
                        .foregroundStyle(note.tint)
                }
                if ask.offersButtons, let permission = ask.permission {
                    let fit = permission.fit(lines: Self.lines, columns: Self.columns)
                    ForEach(fit.shown) { option in
                        OptionButton(
                            terminal: ask.terminal,
                            request: permission.request,
                            option: option,
                            // The same derivation the phone and the watch run,
                            // so all three agree about which answer is the
                            // plain yes. Emphasis only — every word on every
                            // button is still the agent's.
                            emphasized: option.id == permission.plainYes?.id)
                    }
                    if let overflow = Self.overflow(fit) {
                        Text(overflow)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    /// What to say about the answers that are not on screen.
    ///
    /// Said rather than left out. `WatchPermission.init?` states the rule this
    /// follows: a person shown a shorter list than the agent offered will pick
    /// from what they were shown, believing it was everything. So the count is
    /// on the card, and the way to the rest is the tap target the card already
    /// had.
    private static func overflow(_ fit: GlanceOptionFit) -> String? {
        guard fit.hidden > 0 else { return nil }
        if fit.shown.isEmpty {
            // Either the agent's answers are too long to put here without
            // shortening them, or its vocabulary offers nothing this build
            // recognizes as a refusal. Both end the same way, and neither is
            // worth explaining on a lock screen.
            return "Tap the card to answer."
        }
        return "Tap the card for \(fit.hidden) more answer\(fit.hidden == 1 ? "" : "s")."
    }
}

/// One answer, as the agent worded it, wired to the intent that sends it.
///
/// `Button(intent:)` and never a `Link`. That is the whole of how these coexist
/// with the card's `.widgetURL`: a button owns its own frame and the URL covers
/// what is left, where two URL-based targets over one area have nothing to say
/// which wins — the hazard `FleetWidget`'s `Layout` enum exists to keep off
/// that widget.
private struct OptionButton: View {
    let terminal: String
    let request: String
    let option: GlancePermissionOption
    let emphasized: Bool

    var body: some View {
        Group {
            if emphasized {
                button.buttonStyle(.borderedProminent)
            } else {
                button.buttonStyle(.bordered)
            }
        }
        .controlSize(.small)
    }

    private var button: some View {
        Button(
            intent: AnswerPermissionIntent(
                terminal: terminal,
                request: request,
                option: option.id,
                // Carried so the card can name the answer that landed once the
                // permission it belonged to is gone and its words with it. See
                // `GlanceAnswer.optionName`.
                optionName: option.name)
        ) {
            Text(option.name)
                .font(.footnote)
                // No `lineLimit`, no `truncationMode`. A long name wraps; it is
                // never cut. See this view's enclosing type.
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// The lock screen presentation: the leader, then everyone else in one line.
private struct LockScreenCard: View {
    let state: AgentActivityAttributes.ContentState
    let tail: FleetTail
    let ask: LeaderAsk

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            LeaderRow(state: state, ask: ask, trace: tail.leaderTrace)
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
/// the part of this card that gets controls. The answer to a blocked run is now
/// one of them — `AnswerControls`, under `detail` where the target shape puts
/// it, and not in `LockScreenCard` beside the fleet line, which is about
/// everybody else. A `Review` on a finished run belongs in the same place.
///
/// **The controls are Buttons, and the card keeps its `widgetURL`.** Those do
/// not fight: a `Button` claims its own frame and the modifier covers whatever
/// is left, so tapping an option answers and tapping anywhere else opens the
/// agent. That is a different arrangement from the one `FleetWidget`'s `Layout`
/// enum exists to prevent, which was per-row `Link`s AND a `widgetURL` — two
/// URL-based targets over one area, with nothing to say which wins. There is no
/// `Link` here and there must not be one.
private struct LeaderRow: View {
    let state: AgentActivityAttributes.ContentState
    let ask: LeaderAsk
    /// The leader's thirteen buckets, off the snapshot. See `FleetTail`, which
    /// is where a card gets anything the push could not carry.
    let trace: Data?

    var body: some View {
        let status = AgentStatus(state.status)
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .center, spacing: 14) {
                VStack(alignment: .leading, spacing: 3) {
                    // Name and runner on one line rather than stacked. The card
                    // has a fleet line to fit now, and "claude · studio" is how
                    // every other surface in this product names an agent.
                    Text(runnerSuffixed)
                        .font(.headline)
                        .lineLimit(1)
                    let body = state.detail
                    if !body.isEmpty {
                        // One line rather than two while there is an answer to
                        // offer. The question stays — answering something you
                        // cannot see is worse than anything this saves — but the
                        // second line of it is the cheapest twenty points on a
                        // card that has to end with a reject button still on it.
                        Text(body)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .lineLimit(ask.isPresent ? 1 : 2)
                            .padding(.top, 2)
                    }
                    // How long this turn has been going. `.timer` and not a
                    // string we compute: the extension gets no wake-up per
                    // second, so anything we render ourselves is frozen at the
                    // moment of the last push. The system counts this one,
                    // network or not — and it counts from the LEADER's start,
                    // which is why that date moved onto the content state with
                    // the rest of the leader.
                    //
                    // Hidden while an answer is on offer, along with the fleet
                    // line: a clock counting an agent that is stopped is the
                    // least useful thing on a card whose buttons could start it
                    // again.
                    if let started = state.startedAt, !ask.isPresent {
                        Text(started, style: .timer)
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.tertiary)
                            .padding(.top, 2)
                    }
                }
                Spacer(minLength: 0)
                // §07 gives the card's rows columns of "11 / flex / 52 / 64",
                // and this is the 52: §04's card-row trace, between the agent's
                // words and its mark.
                //
                // Dropped while there is an answer on offer, along with the turn
                // clock and the fleet line above. The card's own rule is that a
                // question outranks everything that is merely true, and history
                // is the most merely-true thing on it.
                if !ask.isPresent, let trace = ActivityTrace(trace) {
                    GlanceTraceView(trace, size: .cardRow)
                }
                StatusBadge(status: status)
            }
            // Guarded at the call site as well as inside the view. A `VStack`
            // asked to lay out a child that draws nothing is one more thing
            // about this card's spacing that would have to be checked on a
            // device rather than reasoned about.
            if ask.isPresent { AnswerControls(ask: ask) }
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

/// The state mark and its word, drawn the same way in both presentations.
///
/// The mark replaces the SF Symbol this drew before — `circle.dotted`,
/// `exclamationmark.bubble.fill`, `checkmark.circle.fill` — which were three
/// glyphs saying what one mark now says on every surface in the product. §03's
/// whole argument is that a person learns the mark once; three symbols here
/// meant the lock screen card was the one place that learning did not transfer.
///
/// **This surface DOES state the core**, unlike every widget family. §08's rule
/// is about refresh rate — "Working versus idle never appears on a widget. It
/// flips every few seconds; at this refresh rate the claim would be false more
/// often than true" — and a Live Activity is pushed on every change rather than
/// reloaded twice an hour. The claim is true here when it is made.
private struct StatusBadge: View {
    let status: AgentStatus
    /// 11pt — §07 gives the card's rows a leading column of exactly 11, which
    /// is §03's header diameter.
    var size: GlanceMarkSize = .header

    var body: some View {
        VStack(spacing: 4) {
            GlanceMarkView(status.mark, size: size)
            Text(status.title)
                .glanceType(.monoFigures)
        }
    }
}

extension AgentStatus {
    /// This state as the one mark.
    ///
    /// Blocked is the only one that earns the heavy amber ring, and that is the
    /// point: amber is reserved for the state that is waiting on a person, so a
    /// glance at a locked phone answers "does this need me" without reading a
    /// word. Working and finished both sit on the quiet hairline and are told
    /// apart by the core, which is exactly the split §03 draws — "the core is
    /// the agent's: filled while producing, absent at a prompt."
    ///
    /// Green has come off. It was the third hue in a system §01 allows two, and
    /// what it was saying — "this finished" — is what an absent core says.
    var mark: GlanceMark {
        switch self {
        case .working: GlanceMark(attention: .quiet, core: .producing)
        case .blocked: GlanceMark(attention: .needsYou, core: .atAPrompt)
        case .done: GlanceMark(attention: .quiet, core: .atAPrompt)
        }
    }

    /// The color for the WORDS beside the mark. The mark colors itself.
    ///
    /// `darkColor` and not the scheme-resolved value: the Dynamic Island is a
    /// black pill whatever the phone's appearance is set to, and the lock
    /// screen card sits over a wallpaper on the same dark ground. §01's light
    /// values are for a pale backdrop, which is not what either of these is.
    var tint: Color {
        switch self {
        case .working: GlancePalette.text2.darkColor
        case .blocked: GlancePalette.amber.darkColor
        case .done: GlancePalette.text2.darkColor
        }
    }
}
