import SwiftUI

/// The fleet, on a wrist.
///
/// Everything on this screen was decided on the host. `glyph`, `headline`,
/// `line` and the ORDER come off the snapshot unchanged — the same rule
/// `FleetWidget` states for the phone, and for the same reason: a watch, a
/// phone and a Mac get looked at within seconds of each other, and the only way
/// three clients cannot disagree about one pane is that none of them decides.
/// This file chooses what fits a 45mm screen and how confident to sound, and
/// nothing else. There is no `sorted`, no `filter` on `status`, and no glyph
/// derived from a status word anywhere below.
///
/// Confidence is the one judgement it makes. A snapshot is always somewhat old,
/// and `FleetSnapshot.confidence(in:at:)` answers whether an agent's status has
/// stopped being worth asserting — `blocked` and `done` stay true at any age
/// because only a person un-does them, `working` does not.
struct FleetListView<Client: FleetClient>: View {
    @ObservedObject var client: Client

    /// When this screen must redraw, and the fleet in the order it draws them.
    ///
    /// **Both are derived from the snapshot and never from a render**, which is
    /// the entire reason they are `@State` rather than expressions in `body`.
    ///
    /// The schedule was `.explicit(refreshes(for:from: .now))` written inline,
    /// so every body evaluation built a schedule whose leading entry was a new
    /// `.now`. A `TimelineView` handed a schedule it has not seen before
    /// invalidates the one it was running and starts the next one, which
    /// evaluates the body, which produces another new schedule: a loop with a
    /// three-second push cadence feeding it. Computed once per snapshot, the
    /// schedule holds still between snapshots, which is what an explicit
    /// schedule is for.
    ///
    /// `rows` is `ranked`, which sorts the whole fleet and copies it into a new
    /// array — every time `body` ran, for an order that can only change when a
    /// snapshot does. `FleetSnapshot.ranked`'s own comment explains why the
    /// sort is `(rank, id)` and why it must stay that; this changes how OFTEN
    /// it is asked for and nothing about the answer.
    ///
    /// Seeded with `initial: true` so the first render has both, and updated
    /// from `onChange`, which SwiftUI runs after the body that observed the
    /// change. So a snapshot's first frame can draw the previous fleet's rows
    /// against the new snapshot. That costs nothing that lasts — the state
    /// write schedules another pass in the same update — and every row carries
    /// its own text, so the worst a frame can show is an agent that was in the
    /// fleet a moment ago.
    @State private var schedule: [Date] = [.now]
    @State private var rows: [FleetSnapshot.Agent] = []

    var body: some View {
        NavigationStack {
            // Every confidence question below is asked at `context.date` rather
            // than at `Date()`. See `refreshes(for:from:)` for why this screen
            // needs a clock of its own at all.
            TimelineView(.explicit(schedule)) { context in
                content(at: context.date)
            }
            .navigationTitle("Agents")
            .watchRoutes(client: client)
        }
        .onChange(of: client.state.snapshot, initial: true) { _, snapshot in
            schedule = refreshes(for: snapshot, from: .now)
            rows = snapshot?.ranked ?? []
        }
    }

    /// The three states, kept three.
    ///
    /// `.nothing` is NOT drawn as an empty fleet. "No agents" is a claim about
    /// the runners this person keeps, and a watch that has never heard from the
    /// phone has made no observation that entitles it to make one — the same
    /// distinction `FleetEntry.hasSnapshot` draws on the phone. Here it needs no
    /// epoch check: `WatchState` already makes it structural, because a watch
    /// with no snapshot has a `nil` to show for it rather than an empty one.
    @ViewBuilder private func content(at now: Date) -> some View {
        switch client.state {
        case .nothing:
            ContentUnavailableView(
                "Nothing Yet",
                systemImage: "iphone",
                description: Text("Open \(appName) on your iPhone to see your agents."))
        case let .live(snapshot):
            fleet(snapshot, at: now, unreachable: false)
        case let .cached(snapshot):
            fleet(snapshot, at: now, unreachable: true)
        }
    }

    @ViewBuilder private func fleet(
        _ snapshot: FleetSnapshot, at now: Date, unreachable: Bool
    ) -> some View {
        List {
            // Once, at the top, and only when the phone is out of reach. Not
            // keyed off how old the snapshot looks: a fresh fleet with no link
            // is still a fleet nobody can act on, and an old one with a link is
            // still live. That is `WatchState`'s whole rule.
            if unreachable { CachedBanner(capturedAt: snapshot.capturedAt) }
            // The other half of "what needs me", and until now the app never
            // mentioned it.
            //
            // The complication on the very same watch draws "3 to review" — see
            // `WatchFleetWidget`'s `Rectangular` and `Circular` — and tapping a
            // complication opens this screen, which had nothing whatsoever to
            // say about reviews. A face that reports something the app it opens
            // has never heard of reads as an app that is broken, and it costs
            // somebody the one glance the complication was supposed to save
            // them.
            //
            // It is a count and not a list, and it is not tappable, because
            // there is nothing honest to open: `reviewsWaiting` is a number the
            // host derived per WORKTREE and the snapshot deliberately carries
            // no rows — see its comment, and see the review section of
            // `docs/jobs-to-be-done.md`, which puts reviewing on a phone or a
            // Mac and nowhere near a wrist. Reassure is the watch's job here;
            // Review is not, and a row that promised otherwise would be worse
            // than this one.
            if let reviews = snapshot.needsReview, reviews > 0 {
                ReviewRow(count: reviews)
            }
            // Only when a real capture came back with nothing, which on this
            // surface is the only kind of capture there is. The widget's words,
            // deliberately.
            //
            // Asked of `rows` rather than of `snapshot.agents` so that the two
            // cannot disagree on the one frame `rows` is a snapshot behind —
            // see its declaration. They hold the same agents; this is only
            // about which of the two a single render reads.
            if rows.isEmpty {
                Text("No agents").font(.headline).foregroundStyle(.secondary)
            }
            ForEach(rows) { agent in
                NavigationLink(value: WatchRoute.agent(terminal: agent.id)) {
                    AgentRow(agent: agent, confidence: snapshot.confidence(in: agent, at: now))
                }
            }
            if !snapshot.complete { PartialFooter() }
        }
    }
}

/// One agent: its mark, what it is doing, and where it is.
///
/// Two lines at most, because the row under it has to stay reachable by a thumb.
/// `lineLimit(2)` rather than the widget's 1: a watch list scrolls, so wrapping
/// spends a few points of a screen that has more of them below, while truncating
/// costs the end of the only sentence saying what the agent is on.
private struct AgentRow: View {
    let agent: FleetSnapshot.Agent
    let confidence: FleetSnapshot.Confidence

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            // The one state mark, at the 14pt row size §03 gives the wrist —
            // "read at arm's length, two sizes only". Amber for the agent that
            // is waiting on a person and nothing else, which is the reservation
            // the widget and the Live Activity both make, so a glance at any of
            // the three answers "does this need me" without reading a word.
            //
            // Unlike the complication, this states the CORE. §08's rule that
            // "working versus idle never appears on a widget" is about a surface
            // that reloads twice an hour; this screen is open in front of a
            // person and polls, so the claim is true when it is made.
            GlanceMarkView(GlanceMark(agent: agent, confidence: confidence), size: .watchRow)
                // A shape has no baseline, so a `firstTextBaseline` stack would
                // hang it off its bottom edge a descender low.
                .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 1 }
            VStack(alignment: .leading, spacing: 1) {
                Text(stated(agent, confidence))
                    .font(.body.weight(.medium))
                    .lineLimit(2)
                // Skipped when it is already the line above: an agent first seen
                // through a push has no `headline`, so `agentTitle` falls back
                // to this very string, and a row that printed it twice would
                // spend both its lines saying one thing.
                if !agent.line.isEmpty, agent.line != agentTitle(agent) {
                    Text(agent.line)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
            Spacer(minLength: 0)
        }
        .opacity(confidence == .lastSeen ? 0.6 : 1)
    }
}

/// How many workspaces have moved since anybody looked at them.
///
/// **Every word and the mark come off `FleetSnapshot.Glance.review`**, which is
/// the table the complication draws from too. That is the point of the case
/// carrying its own glyph and its own phrasing: a wrist and the face on the
/// same wrist are looked at within seconds of each other, and two spellings of
/// one count would be read as two different counts.
///
/// **Never amber, and the accent color rather than a fixed one.** The
/// cross-surface law is that amber means an agent has stopped and is waiting on
/// a person; a diff waiting to be read is not that. `glanceTint` in
/// `WatchFleetWidget` makes the identical choice for the identical reason, and
/// this is the second half of it on the same device.
///
/// **Not dimmed by `confidence`, and it takes no date.** A diff nobody has
/// reviewed is still unreviewed an hour later — `reviewsWaiting` is latched,
/// the same way `blocked` is — so age does not make this less true, and
/// graying it would make the one fact somebody raised their wrist for look like
/// the doubtful part of the screen.
private struct ReviewRow: View {
    let count: Int

    var body: some View {
        Label {
            Text(FleetSnapshot.Glance.review(count).phrase)
        } icon: {
            GlanceMarkView(GlanceMark(glance: .review(count)), size: .watchRow)
        }
        .font(.caption.weight(.medium))
        // `darkColor` because watchOS has no light appearance — see the same
        // note in `PermissionView`.
        .foregroundStyle(GlancePalette.review.darkColor)
    }
}

/// What the watch is showing when it cannot reach the phone.
///
/// Said once and at the top rather than per row, because it is one fact about
/// the link and not eleven facts about eleven agents. The age lives here for the
/// same reason, and it is the snapshot's own `capturedAt` — when the phone last
/// assembled this, never when the watch last drew it.
private struct CachedBanner: View {
    let capturedAt: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("Can’t reach your iPhone")
                .font(.caption.weight(.medium))
            Text("Showing what it last sent, \(capturedAt, style: .relative) ago.")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// That these may not be all the agents there are.
///
/// A snapshot assembled only from pushes knows about the agents that happened to
/// notify. Drawing it as the fleet would assert that the others do not exist,
/// which is the same claim "No agents" makes and just as untrue.
private struct PartialFooter: View {
    var body: some View {
        Text("From notifications, so other agents may be missing.")
            .font(.caption2)
            .foregroundStyle(.tertiary)
    }
}

/// Every screen this app can push, in one type.
///
/// A route rather than a destination view built at each link, so `watchRoutes`
/// is the single place that knows what a button opens — one file to edit when a
/// screen is added, and no second navigation path to keep in step with this one.
enum WatchRoute: Hashable {
    /// A terminal id and never an `Agent` value. The detail screen re-reads the
    /// agent out of the CURRENT snapshot on every render, so a fleet that
    /// changes while somebody is reading changes what they are reading. A pushed
    /// copy would freeze at the moment of the tap and go on saying `working`
    /// after the fleet had said otherwise — the exact failure the staleness rule
    /// exists to prevent, reintroduced by navigation.
    case agent(terminal: String)
    case compose(terminal: String)
    case permission(terminal: String)
    /// What the agent said. A terminal id like the rest, and for the same
    /// reason twice over: the screen asks the phone for the words when it
    /// opens, so there is nothing to carry, and the snapshot it reads the
    /// agent's NAME out of has to be the current one.
    case transcript(terminal: String)
}

extension View {
    /// The routes, registered once for whichever `NavigationStack` encloses
    /// them.
    ///
    /// A modifier rather than three `navigationDestination`s written out at the
    /// call site: a preview and the app then push the same screens, so whatever
    /// anybody looks at in Xcode is what ships.
    func watchRoutes<Client: FleetClient>(client: Client) -> some View {
        navigationDestination(for: WatchRoute.self) { route in
            switch route {
            case let .agent(terminal):
                AgentDetailView(client: client, terminal: terminal)
            case let .compose(terminal):
                ComposeView(client: client, terminal: terminal)
            case let .permission(terminal):
                PermissionView(client: client, terminal: terminal)
            case let .transcript(terminal):
                TranscriptView(client: client, terminal: terminal)
            }
        }
    }
}

/// When a screen must redraw with no news arriving.
///
/// A watch app is not a widget, but it has the widget's problem: a screen
/// that re-renders only when a snapshot lands would keep asserting `working`
/// for an agent nobody has heard from in an hour, and the case this exists
/// for is precisely the one with no news in it — a phone out of range sends
/// nothing, so nothing would ever move this screen off what it last drew.
/// `stalenessMoments(after:)` is the same list `FleetProvider` builds its
/// timeline from, so a wrist and a lock screen stop vouching for an agent at
/// the same instant rather than at two definitions of it.
///
/// `now` leads, always, so the schedule is never empty and always carries an
/// entry that has already arrived — an explicit schedule with no past date has
/// nothing to draw. That is what makes it safe to compute this once per
/// snapshot and hold it: the leading entry only gets further into the past,
/// and a `TimelineView` wants exactly one of those. Past the last moment
/// nothing changes again, and the schedule ending there is correct rather than
/// a gap.
///
/// No cap, unlike the widget's twelve. That cap is about serializing a whole
/// snapshot per entry into a process with a hard memory ceiling; this is an
/// array of dates inside the app that is already holding the fleet.
///
/// At file scope rather than on `FleetListView`, alongside `agentTitle` and
/// `stated` below and for their reason: `AgentDetailView` runs the same clock
/// on the same snapshot, and it used to say so by rewriting the expression.
/// Two screens that must go stale at one instant should not be two statements
/// of when that is.
func refreshes(for snapshot: FleetSnapshot?, from now: Date) -> [Date] {
    [now] + (snapshot?.stalenessMoments(after: now) ?? [])
}

/// The one string a row uses to name an agent.
///
/// `headline` normally. But it can be EMPTY: an agent first seen through a push
/// gets `headline: previous?.headline ?? ""` from the notification service
/// extension, which has a status word and a name and no ladder to run. Falling
/// back down the wire's own fields is not deriving one — nothing is composed
/// here, so this screen cannot end up phrasing an agent differently from the
/// widget running the identical fallback on the phone in the same pocket.
func agentTitle(_ agent: FleetSnapshot.Agent) -> String {
    [agent.headline, agent.line, agent.label, agent.status].first { !$0.isEmpty } ?? ""
}

/// That title, put in the past tense once the snapshot has stopped vouching for
/// it.
///
/// The qualifier is a PREFIX and never a suffix. Text truncates at the tail, so
/// a marker appended to the end is the first thing to vanish from exactly the
/// row too narrow to keep it — a watch that drops "· stale" and keeps "claude"
/// says the opposite of what it meant. Prefixed, the worst a narrow screen can
/// do is eat the agent's own detail while "last seen" survives, which degrades
/// toward saying less rather than toward saying something false.
///
/// **There is no 28-character bound**, though this said there was one.
/// `HEADLINE_WIDTH` is 18 and "last seen " is 10, so the arithmetic was right
/// about the case it looked at and wrong about the function: `agentTitle` falls
/// back past `headline` to `line` (`feed::WIDTH`, 40) and then to `label`,
/// which the host bounds nowhere — a worktree-suffixed label is the ordinary
/// case here, not the edge one.
///
/// Nothing truncates it, and nothing needs to. Being a PREFIX is the bound:
/// whatever the slot cannot hold falls off the tail, and what falls off is the
/// agent's own detail rather than the qualifier that changes what the sentence
/// means. A cut made here would only pick a worse place than the layout picks.
func stated(_ agent: FleetSnapshot.Agent, _ confidence: FleetSnapshot.Confidence) -> String {
    let title = agentTitle(agent)
    return confidence == .lastSeen ? "last seen \(title)" : title
}

/// This build's app name, for the one sentence that has to say it.
///
/// `CFBundleDisplayName`, which `generate-project.py` stamps into this target
/// from `version.sh app-name-short`. A literal would tell somebody running the
/// canary to open "Far Cooler", which is either a different app on their phone
/// or no app at all — the mistake a hardcoded name already made once in the Live
/// Activity.
var appName: String {
    Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "Far Cooler"
}
