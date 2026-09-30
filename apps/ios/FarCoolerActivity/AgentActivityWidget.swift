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
/// picking between them with no rule anybody wrote.
///
/// **So the card draws a LINE EACH for two agents and counts the rest**, which
/// is `GlanceCardView` in AgentKit and what the design asks for. It leads with
/// the top of
/// the relay's own ordering — blocked first, then to-review, then working, the
/// precedence the rest of this product uses — and the tail says how many agents
/// have no line and what the whole fleet has changed. The drawing is in AgentKit
/// rather than here because `swift test --package-path apps/shared/AgentKit` is
/// the suite that runs on every push and the iOS UI suite is compiled and never
/// executed; a `View` in this file is a drawing nothing reads back, which is how
/// the card came to force the dark palette onto a light material for a whole
/// release. See `GlanceCard.swift`.
///
/// **Every figure on that card comes off the push**, and the last of them only
/// now:
///
///   - **the rows** are `context.state.rows`, a line per agent accumulated by
///     the relay from the notices every runner on an account already sends it;
///   - **the counts** are the three tier totals beside them, over the whole
///     fleet rather than over what fits;
///   - **the traces** are 66 bytes of base64 on each row. They used to be read
///     out of the App Group snapshot, because the push had never carried one —
///     which meant the history on the card was as old as the last time this
///     phone ran the app.
///
/// The snapshot is what the OTHER card falls back to. A relay too old to have a
/// roster says nothing about the fleet, `knowsFleet` is false, and `FleetTail`
/// reads the file and hedges its wording — `FleetSnapshot.complete` and
/// `confidence(in:at:)`, the same rule every other surface outside the app
/// follows. That hedge exists because a snapshot is assembled from what this
/// phone happens to have been told, which on a phone in a pocket that has not
/// run the app today is not much; the pushed rows do not need it.
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
            // ActivityKit's hour of silence, or a card whose every line went
            // to a runner that stopped beating (ov-71): either way nothing on
            // it vouches for "Working" now. See `AgentCardState.unvouched`.
            let stale = context.state.unvouched(stale: context.isStale)
            // Which of the two cards this is. See `LockScreenCard`, which is
            // where the choice is argued: rows when the relay sent any and
            // there is nothing to answer, the headline otherwise.
            //
            // `isStale` is the relay's hour of silence; see
            // `AgentCardLayout.init(state:now:stale:)`. A runner that stopped
            // beating is the other outage the card now hears of, from the
            // relay's sweep: its rows are gone and the tail names it.
            let layout =
                ask.isPresent
                ? nil : AgentCardLayout(state: context.state, stale: stale)
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
                //
                // The rows card does not read it at all — every figure on it
                // came off the push — so the App Group file is not opened on
                // that path. It is one small JSON on a render thread, but it is
                // also the last thing this card needed a local file for.
                tail: ask.isPresent || layout != nil
                    ? FleetTail.unknown
                    : FleetTail.current(
                        for: context.state, snapshot: SnapshotStore.read(),
                        stale: stale),
                ask: ask,
                layout: layout,
                stale: stale)
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
                //
                // **The card's TOP ROW, when it is drawing rows**, and that is a
                // correction rather than a refinement. The headline is the agent
                // the notice was about and the rows are the fleet in tier order;
                // they usually coincide and need not, and a card whose first
                // line reads `auth-refactor` opening `docs-sweep` is
                // indistinguishable from a card that ignored the tap. The
                // Island keeps the headline below, because the headline is what
                // the Island draws.
                .widgetURL(terminalURL(layout?.rows.first?.terminal ?? context.state.terminal))
        } dynamicIsland: { context in
            let status = AgentStatus(context.state.status)
            let ask = LeaderAsk.current(for: context.state)
            // See the lock screen's `stale` above.
            let stale = context.state.unvouched(stale: context.isStale)
            let tail =
                ask.isPresent
                ? FleetTail.unknown
                : FleetTail.current(
                    for: context.state, snapshot: SnapshotStore.read(), stale: stale)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    StatusBadge(status: status, stale: stale)
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
                        Text(
                            AgentCardLayout.named(
                                context.state.label, in: context.state.workspace)
                        )
                        .font(.headline)
                        // Never a blocked leader's question, as on the lock
                        // screen card. See `CardAskWording.line`.
                        let body = CardAskWording.line(state: context.state, leader: ask)
                        if !body.isEmpty {
                            Text(body)
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                        }
                        // The turn clock gives way to the answer, exactly as
                        // it does on the lock screen card and for the same
                        // reason: how long an agent has been stopped is worth
                        // less than being able to stop it being stopped.
                        // And gives way on a stale card, for a working leader:
                        // a clock still counting is a claim about now. See
                        // `AgentCardLeader`.
                        if let started = context.state.startedAt, !ask.isPresent,
                            AgentCardLeader.isStated(
                                status: context.state.status, stale: stale)
                        {
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
                                .opacity(tail.dimsLine ? 0.6 : 1)
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
                GlanceMarkView(status.mark(stale: stale), size: .header)
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
                    // On a stale card with a count beside the name, the count
                    // is who the relay last knew about. See
                    // `FleetTail.dimsCompactCount`.
                    .opacity(tail.dimsCompactCount(stale: stale) ? 0.6 : 1)
                    .lineLimit(1)
                    .frame(maxWidth: 74)
                }
            } minimal: {
                // §07, verbatim: "MINIMAL: The ring alone at 15pt. No count, no
                // trace — history is unreadable at this size." The lone
                // indicator, which is the whole presentation.
                GlanceMarkView(status.mark(stale: stale), size: .lone)
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
typealias LeaderAsk = CardLeaderAsk

/// The store read and the colors. What to offer is `CardLeaderAsk`'s, in
/// AgentKit, where it runs under test: the push's ask (ov-57) or the store's
/// record, the answer that goes with it, and whether the buttons are on.
extension CardLeaderAsk {
    static func current(
        for state: AgentActivityAttributes.ContentState, now: Date = Date()
    ) -> CardLeaderAsk {
        // No file read for a leader that cannot have buttons.
        guard AgentStatus(state.status) == .blocked, !state.terminal.isEmpty else { return .none }
        return current(store: GlancePermissionStore.read(), state: state, now: now)
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
        // The ask is closed: not a failure of this phone's, and nothing to
        // retry, so neither red nor green. The sentence says where to answer.
        case .over: return (answer.message, "clock.badge.xmark", .secondary)
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
                    // The hold's end, counted by the system so it keeps
                    // counting with no push. Guarded: a range that ends before
                    // it starts traps.
                    if let until = ask.until, until > Date() {
                        Text(
                            "Answer within \(Text(timerInterval: Date()...until, countsDown: true, showsHours: false))"
                        )
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                    }
                    let fit = permission.fit(lines: Self.lines, columns: Self.columns)
                    ForEach(fit.shown) { option in
                        OptionButton(
                            terminal: ask.terminal,
                            request: permission.request,
                            option: option,
                            until: ask.until,
                            // The same derivation the phone and the watch run,
                            // so all three agree about which answer is the
                            // plain yes. Emphasis only; every word on every
                            // button is a fixed one (`CardAskWording.label`).
                            emphasized: option.id == permission.plainYes?.id)
                    }
                    if let overflow = Self.overflow(fit, withheld: ask.withheld) {
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
    ///
    /// `withheld` adds the options the card left off because they have no
    /// fixed word: the card never draws an agent's own option name, which can
    /// be the command line (ov-57).
    private static func overflow(_ fit: GlanceOptionFit, withheld: Int) -> String? {
        let hidden = fit.hidden + withheld
        guard hidden > 0 else { return nil }
        if fit.shown.isEmpty {
            // Either the agent's answers are too long to put here without
            // shortening them, or its vocabulary offers nothing this build
            // recognizes as a refusal. Both end the same way, and neither is
            // worth explaining on a lock screen.
            return "Tap the card to answer."
        }
        return "Tap the card for \(hidden) more answer\(hidden == 1 ? "" : "s")."
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
    /// The card ask's hold end, so the app can refuse a late tap without
    /// connecting. Nil for an ask only the store knows.
    let until: Date?
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

    /// An answer that allows goes through `AllowPermissionIntent`, which asks
    /// for an unlock first; any other through `AnswerPermissionIntent`, which
    /// doesn't (provisional D2, `GlancePermissionOption.needsUnlock`).
    ///
    /// `optionName` is carried so the card can name the answer that landed
    /// once the permission it belonged to is gone and its words with it. See
    /// `GlanceAnswer.optionName`.
    @ViewBuilder private var button: some View {
        if option.needsUnlock {
            Button(
                intent: AllowPermissionIntent(
                    terminal: terminal, request: request, option: option.id,
                    optionName: option.name, until: until)
            ) { label }
        } else {
            Button(
                intent: AnswerPermissionIntent(
                    terminal: terminal, request: request, option: option.id,
                    optionName: option.name, until: until)
            ) { label }
        }
    }

    private var label: some View {
        Text(option.name)
            .font(.footnote)
            // No `lineLimit`, no `truncationMode`. A long name wraps; it is
            // never cut. See this view's enclosing type.
            .multilineTextAlignment(.leading)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The lock screen presentation, which is TWO drawings behind one name.
///
/// **The rows card is what the design asks for**: a header counting the fleet,
/// a line each for two agents with their own trace and their own figures, and a
/// tail for everybody else. It needs a `rows` array on the push, so it is what
/// this build draws against this relay.
///
/// **The headline card is the other two cases**, and neither is a fallback in
/// the apologetic sense:
///
///   - **A relay too old to send rows.** An app updated ahead of its relay is
///     the ordinary case here, not a corner: `AgentCardLayout.init?` returns nil
///     and this draws exactly what it drew before rows existed.
///   - **An answer on offer.** A blocked agent whose options this phone has read
///     off the stream gets buttons, and the buttons win the card. That is the
///     rule this surface already had — the turn clock, the trace and the fleet
///     line all step aside for an answer — stated once more with more to stand
///     aside: a question outranks everything that is merely true, and two rows
///     of history are the most merely-true thing on the card. The design draws
///     no buttons because the design is drawing the other case.
///
/// One `View` and one choice rather than two widgets, so there is exactly one
/// place that decides and it is written down.
private struct LockScreenCard: View {
    let state: AgentActivityAttributes.ContentState
    let tail: FleetTail
    let ask: LeaderAsk
    /// The rows card, or nil for the headline card. Built in the widget's own
    /// body so the choice and the tap target are made from one value.
    let layout: AgentCardLayout?
    /// ActivityKit's `isStale`. See `AgentStatus.mark(stale:)`.
    let stale: Bool

    var body: some View {
        if let layout {
            GlanceCardView(layout: layout)
        } else {
            VStack(alignment: .leading, spacing: 10) {
                LeaderRow(state: state, ask: ask, trace: tail.leaderTrace, stale: stale)
                if let rest = tail.line {
                    Divider()
                    Text(rest)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .opacity(tail.dimsLine ? 0.6 : 1)
                        .lineLimit(1)
                }
            }
            .padding(16)
        }
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
    let stale: Bool

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
                    // A blocked leader draws its ask's tool and workspace, or a
                    // generic line, and never the question: the locked card
                    // never shows the command (provisional D3). See
                    // `CardAskWording.line`.
                    let body = CardAskWording.line(state: state, leader: ask)
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
                    //
                    // And on a stale card for a working leader: see
                    // `AgentCardLeader`.
                    if let started = state.startedAt, !ask.isPresent,
                        AgentCardLeader.isStated(status: state.status, stale: stale)
                    {
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
                StatusBadge(status: status, stale: stale)
            }
            // Guarded at the call site as well as inside the view. A `VStack`
            // asked to lay out a child that draws nothing is one more thing
            // about this card's spacing that would have to be checked on a
            // device rather than reasoned about.
            if ask.isPresent { AnswerControls(ask: ask) }
        }
    }

    /// "Billing · claude" when the agent is in a workspace, as the rows name
    /// theirs (`AgentCardLayout.named`); otherwise "claude · studio", or just
    /// the name when the runner is not known.
    ///
    /// A card started by a build older than the fleet restructure carries
    /// neither — see `ContentState.init(from:)` — so the separator has to be
    /// conditional or the card leads with a bare "·".
    private var runnerSuffixed: String {
        if !state.workspace.isEmpty, !state.label.isEmpty {
            return AgentCardLayout.named(state.label, in: state.workspace)
        }
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
    /// ActivityKit's `isStale`. See `AgentStatus.mark(stale:)`.
    let stale: Bool

    var body: some View {
        VStack(spacing: 4) {
            GlanceMarkView(status.mark(stale: stale), size: size)
            // No word for a leader the card can't state: "Working" beside a
            // "can't say" ring read as a contradiction, to the eye and to
            // VoiceOver, which now hears the ring's "Can’t say" alone.
            if AgentCardLeader.isStated(status: status.rawValue, stale: stale) {
                Text(status.title)
                    .glanceType(.monoFigures)
            }
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

    /// The same, on a card the relay has stopped vouching for.
    ///
    /// ActivityKit's `isStale`: an hour since the last push, which is what a
    /// runner that stays down looks like from here. Working is the claim about
    /// now, so it becomes "can't say", the dashed ring with no core that
    /// `AgentCardLayout` draws for the same card; blocked and finished hold.
    func mark(stale: Bool) -> GlanceMark {
        AgentCardLeader.isStated(status: rawValue, stale: stale) ? mark : .unsaid
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
