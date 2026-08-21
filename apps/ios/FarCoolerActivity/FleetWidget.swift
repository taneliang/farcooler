import SwiftUI
import WidgetKit

/// The fleet, on a home screen and under a lock screen clock.
///
/// Everything drawn here was decided on the host. `glyph`, `headline`, `line`
/// and the order come off the snapshot unchanged — this file chooses which rung
/// fits the space and nothing else, which is the entire reason the ladder is
/// computed once in `farcooler_core::feed` rather than by each client. A widget
/// that derived its own headline would put itself into exactly the disagreement
/// with the app that the ladder exists to prevent, and the two are on screen at
/// the same time.
///
/// The one judgement it does make is how confident to sound. A snapshot is
/// always somewhat old, and `FleetSnapshot.confidence(in:at:)` answers whether
/// a given agent's status has stopped being trustworthy — blocked and done stay
/// true, working does not.
struct FleetWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "FleetWidget", provider: FleetProvider()) { entry in
            FleetWidgetView(entry: entry)
                // Required since iOS 17: a widget that sets no container
                // background is drawn with none at all in the gallery and on
                // the home screen, which reads as a rendering bug rather than a
                // style. The system material, not a color of ours, so the
                // wallpaper stays visible behind it.
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Agents")
        .description("What your agents are doing, and which one needs you.")
        .supportedFamilies([
            .systemSmall, .systemMedium, .systemLarge,
            .accessoryCircular, .accessoryRectangular, .accessoryInline,
        ])
    }
}

struct FleetEntry: TimelineEntry {
    let date: Date
    let snapshot: FleetSnapshot

    /// Whether a snapshot has ever been written.
    ///
    /// `FleetSnapshot.empty` is not an empty fleet — it is the absence of an
    /// observation, and the two must not render alike. "No agents" is a claim
    /// about every runner this person has; a phone that has never opened the
    /// app has made no such observation and may not make that claim. The epoch
    /// `capturedAt` is what tells them apart, because a real capture always has
    /// a real date.
    var hasSnapshot: Bool { snapshot.capturedAt.timeIntervalSince1970 > 0 }
}

struct FleetProvider: TimelineProvider {
    func placeholder(in context: Context) -> FleetEntry {
        FleetEntry(date: Date(), snapshot: .empty)
    }

    func getSnapshot(in context: Context, completion: @escaping (FleetEntry) -> Void) {
        completion(FleetEntry(date: Date(), snapshot: SnapshotStore.read() ?? .empty))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<FleetEntry>) -> Void) {
        let snapshot = SnapshotStore.read() ?? .empty
        let now = Date()

        // One entry for now, and one for every moment after it at which this
        // snapshot stops vouching for another agent.
        //
        // Refreshes arrive from outside — the app writes and reloads, the
        // notification service extension writes and reloads — so the first
        // entry would be enough to show what is known. But a widget that can
        // only learn it has gone stale from a message it is not receiving would
        // assert `working` forever, and on this surface that message never
        // comes: a working push sends no alert, so no extension runs and
        // nothing reloads anything.
        //
        // ALL the moments, not the first. Ages are per agent, so agents expire
        // at different times; a timeline that stopped at the earliest would
        // render that entry from then on, with every later agent still drawn as
        // current for good. Each entry re-renders from the same snapshot at its
        // own date, and `confidence(in:at:)` is what turns that date into what
        // the row is allowed to say.
        //
        // `.never` because nothing here gets better with time: after the last
        // moment every render says the same thing, and asking the system for
        // wake-ups that change nothing spends a budget the reloads from the app
        // and the extension need.
        let entries =
            [FleetEntry(date: now, snapshot: snapshot)]
            + Self.wakes(for: snapshot, after: now).map {
                FleetEntry(date: $0, snapshot: snapshot)
            }

        completion(Timeline(entries: entries, policy: .never))
    }

    /// How many staleness entries one timeline may carry.
    ///
    /// Twelve, which is twice the six rows the largest family draws — so a fleet
    /// big enough to reach it has already put most of its agents off every
    /// screen this widget has. The cap is a guard against a pathological fleet,
    /// not a working limit: there is at most one moment per unlatched agent, and
    /// agents sharing a second share a moment.
    ///
    /// It exists because a timeline is a whole snapshot per entry, serialized
    /// into a widget process with a hard memory ceiling, and a hundred entries
    /// there is a widget that fails to render at all — which says less than a
    /// stale one does.
    private static let wakeLimit = 12

    /// The dates to re-render at, capped.
    ///
    /// The earliest moments, plus the LAST one whenever the cap bites. That last
    /// entry is not decoration: past it every unlatched agent in the snapshot
    /// has expired, so it is the one that guarantees nothing is ever asserted
    /// beyond its hour forever — the most a dropped middle moment can cost is
    /// one agent reading as current for part of the gap.
    private static func wakes(for snapshot: FleetSnapshot, after now: Date) -> [Date] {
        let moments = snapshot.stalenessMoments(after: now)
        guard moments.count > wakeLimit else { return moments }
        return Array(moments.prefix(wakeLimit - 1)) + Array(moments.suffix(1))
    }
}

/// Where a tap on one agent lands.
///
/// `FleetView.onOpenURL` reads the id back out of the last path component and
/// opens that terminal. The scheme is this build's, never the literal
/// `farcooler://` — see `AppScheme` for the channel this hardcoding broke.
///
/// Optional and percent-encoded rather than force-unwrapped. Terminal ids are
/// UUIDs today, so neither guard fires on any id that currently exists; both are
/// here because of where this code runs rather than what it is handed. A `!` in
/// a widget is a trap in a process with no console — WidgetKit draws "Unable to
/// Load" and says nothing about why — so the day an id gains a space or a `%`
/// must cost a dead link on one row, not the whole surface. (`.urlPathAllowed`
/// passes `/` through unchanged; the encoding covers the characters
/// `URL(string:)` rejects, not path structure.)
private func terminalURL(_ agent: FleetSnapshot.Agent) -> URL? {
    let id = agent.id.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? agent.id
    return URL(string: "\(AppScheme.current)://terminal/\(id)")
}

/// The one string a surface with room for one uses to name an agent.
///
/// `headline` normally. But it can be EMPTY, and on the path this widget exists
/// for: an agent first seen through a push gets `headline: previous?.headline
/// ?? ""` from the notification service extension, which has a status word and
/// a name and no ladder to run. A `top?.headline ?? fallback` treats only a nil
/// agent as unknown, so the surfaces that draw one string drew a blank
/// complication for exactly the agent that had just become news.
///
/// Falling back down the wire's own fields, never deriving one: `line` is the
/// notification's body and is populated in precisely that case, `label` is at
/// least a name, and `status` is the one field the relay guarantees is
/// non-empty. Picking among fields the host sent is not the same as computing a
/// headline here — nothing is composed, and the app cannot end up disagreeing.
private func agentTitle(_ agent: FleetSnapshot.Agent) -> String {
    [agent.headline, agent.line, agent.label, agent.status].first { !$0.isEmpty } ?? ""
}

/// That title, put in the past tense when the snapshot has stopped vouching for
/// it.
///
/// The qualifier is a PREFIX, and on the accessories that is the whole design.
/// Text truncates at the tail, so a marker appended to the end is the first
/// thing to vanish from exactly the surface too narrow to keep it — a lock
/// screen that drops "· stale" and keeps "claude 4m" says the opposite of what
/// it meant. Prefixed, the worst a narrow slot can do is eat the agent's own
/// detail while "last seen" survives, which degrades toward saying less rather
/// than toward saying something false.
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
private func stated(_ agent: FleetSnapshot.Agent, _ confidence: FleetSnapshot.Confidence) -> String {
    let title = agentTitle(agent)
    return confidence == .lastSeen ? "last seen \(title)" : title
}

/// This build's app name, for the one sentence that has to say it.
///
/// `CFBundleDisplayName` in this extension's own Info.plist, which
/// generate-project.py fills from `version.sh app-name-short`. A literal here
/// would tell someone running the canary to open "Far Cooler", which is either
/// a different app on their phone or no app at all — the same failure the
/// hardcoded URL scheme above it caused, in words instead of a link.
private var appName: String {
    Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "Far Cooler"
}

/// The one color rule this feature turns on.
///
/// **Amber means "needs you", and nothing else may wear it.** A blocked agent
/// is stopped until a person answers it; a worktree with an unreviewed diff is
/// merely waiting, and nothing is worse for having waited another hour. The
/// moment reviews are amber too, amber stops meaning anything and the glance
/// this widget exists to support becomes a thing to be read rather than seen.
///
/// A second copy of this switch is in `WatchFleetWidget`, and it has to be a
/// copy: that is a separate binary, a color is a SwiftUI type, and
/// `FleetSnapshot` — the file both targets compile and where the glyph, the
/// words and the precedence do live — is the wire's shape and has no business
/// importing SwiftUI. Three cases is the whole rule, and it is stated in both.
///
/// `AnyShapeStyle` because the three tints are not all colors: tertiary is a
/// hierarchical style, which is what makes the all-clear rung recede against
/// whatever wallpaper or watch face is behind it rather than sitting at a fixed
/// gray that is invisible on one and shouting on another.
private func glanceTint(_ glance: FleetSnapshot.Glance) -> AnyShapeStyle {
    switch glance {
    case .blocked: AnyShapeStyle(Color.orange)
    case .review: AnyShapeStyle(Color.accentColor)
    case .working: AnyShapeStyle(HierarchicalShapeStyle.tertiary)
    }
}

/// One count, said in full: the mark, the number and the words.
///
/// `Label` rather than an `HStack`, so the mark and the text keep the system's
/// own spacing and stay together when the type size grows — these lines are
/// drawn at accessibility sizes on a lock screen, where a hand-spaced icon and
/// its text drift apart.
///
/// Never dimmed by `confidence`. Both counts this draws are latched: an agent
/// that was blocked an hour ago is still blocked, and a diff nobody has
/// reviewed is still unreviewed. Dimming them would make the two facts a person
/// opens this widget for look like the doubtful part of it.
private struct GlanceLabel: View {
    let glance: FleetSnapshot.Glance

    var body: some View {
        Label(glance.phrase, systemImage: glance.symbol)
            .font(.caption.weight(.medium))
            .foregroundStyle(glanceTint(glance))
            .lineLimit(1)
    }
}

struct FleetWidgetView: View {
    @Environment(\.widgetFamily) private var family
    let entry: FleetEntry

    private var top: FleetSnapshot.Agent? { entry.snapshot.ranked.first }

    /// How much this widget may still assert about the agent it leads with.
    ///
    /// Asked once, here, because every family draws that agent and every family
    /// therefore has to answer for it. The accessories originally did not ask
    /// at all, which made the second timeline entry — scheduled precisely so a
    /// snapshot can go stale with no news — change nothing whatsoever on half
    /// the surfaces: a lock screen still reading "claude 4m" an hour after
    /// anyone last heard from claude.
    private var confidence: FleetSnapshot.Confidence {
        top.map { entry.snapshot.confidence(in: $0, at: entry.date) } ?? .known
    }

    /// How this family is drawn.
    ///
    /// One mapping, read twice: `content` picks the layout from it and `body`
    /// asks it where the deep link goes. Those used to be two switches, and a
    /// family added to `supportedFamilies` later would have fallen into the
    /// rows arm's `default:` while still counting as link-less — earning it
    /// per-row `Link`s AND a `widgetURL`, which is the double tap target the
    /// comment in `body` exists to prevent.
    private enum Layout {
        case inline, circular, rectangular, count
        case rows(limit: Int, feed: Bool)

        var hasRows: Bool {
            if case .rows = self { return true }
            return false
        }

        /// Whether this family can state BOTH counts at once, or has to pick
        /// the higher-precedence one.
        ///
        /// On the enum beside `hasRows` rather than in a switch of its own, for
        /// the reason `hasRows` is: a family added to `supportedFamilies` later
        /// falls into a `default:` arm here once, and a second switch on
        /// `family` somewhere else is how it ends up drawn as a rows widget
        /// that also thinks it has room for one number only.
        var statesBothCounts: Bool {
            switch self {
            case .rectangular, .rows: true
            case .inline, .circular, .count: false
            }
        }
    }

    private var layout: Layout {
        switch family {
        case .accessoryInline: .inline
        case .accessoryCircular: .circular
        case .accessoryRectangular: .rectangular
        case .systemSmall: .count
        case .systemLarge: .rows(limit: 6, feed: true)
        default: .rows(limit: 3, feed: false)
        }
    }

    var body: some View {
        content
            // Small and the three accessories are a single tap target — there
            // is no room for a per-row link — so the whole widget opens the
            // agent it is about, rather than the app's front door and a fleet
            // list to re-find it in.
            .widgetURL(layout.hasRows ? nil : top.flatMap(terminalURL))
    }

    @ViewBuilder private var content: some View {
        switch layout {
        case .inline:
            // The QUALIFIER LEADS, which on this family is the whole design.
            // There is no styling channel here at all — the system draws this
            // line in the clock's own tint and ignores color and opacity — so
            // amber cannot do any of the work, and the word has to. Truncation
            // eats the tail, so a line that put the agent's name first and
            // "needs you" after would drop the only part that says what to do.
            //
            // The agent's own headline is what this says when there is nothing
            // to lead with, which includes the case a stale snapshot creates:
            // once no working agent can still be asserted, `glance(at:)`
            // returns nil and this falls back to "last seen claude 4m" — the
            // prefix that is this surface's entire degradation, kept intact.
            if let glance = entry.snapshot.glance(at: entry.date) {
                Label(glance.phrase, systemImage: glance.symbol)
            } else {
                Text(headline)
            }
        case .circular:
            // The mark and the number are ONE fact, so they come from one
            // answer. A glyph belonging to the top agent above a number
            // counting blocked agents was two facts stacked, and the pair
            // "▲ 2" / "± 3" is only tellable apart at a glance if the mark and
            // the count are about the same thing.
            //
            // Nothing here dims. Both rungs that can reach this surface with a
            // number are latched — an agent blocked an hour ago is still
            // blocked, a diff nobody reviewed is still unreviewed — and the
            // volatile rung is already gone by then: `glance(at:)` drops
            // working agents this snapshot can no longer vouch for, and returns
            // nil once none is left.
            VStack(spacing: 0) {
                if let glance = entry.snapshot.glance(at: entry.date) {
                    Image(systemName: glance.symbol)
                        .font(.title3)
                        .foregroundStyle(glanceTint(glance))
                    Text("\(glance.count)")
                        .font(.caption2.monospacedDigit())
                } else {
                    Text(top?.glyph ?? "·")
                        .font(.title3)
                        // The top agent's own mark, which CAN be a `working`
                        // one that has expired — so this one does dim.
                        .opacity(confidence == .lastSeen ? 0.6 : 1)
                    // A dash, not a zero, before anything has been written. "0"
                    // under a lock screen clock is a statement that nothing
                    // needs you, and this surface has no footer to qualify it
                    // with.
                    Text(entry.hasSnapshot ? "\(entry.snapshot.needingYou)" : "–")
                        .font(.caption2.monospacedDigit())
                }
            }
        case .rectangular:
            // Three lines: who, and then each count that has something to say.
            // This family has room for BOTH, and showing both is the point —
            // "2 need you" and "3 to review" are different work in different
            // places, and a surface that collapsed them into one number would
            // send somebody to the wrong screen.
            let blocked = entry.snapshot.needingYou
            // `?? 0` here and not in the snapshot: nil means this build was
            // never told about reviews, and zero lines is exactly what "not
            // told" should draw. What must never happen is the reverse — a
            // rendered "0 to review" asserting something nobody said.
            let reviews = entry.snapshot.needsReview ?? 0
            VStack(alignment: .leading, spacing: 1) {
                Text(headline)
                    .font(.headline)
                    .lineLimit(1)
                    // Only the agent's own words dim. The counts below are
                    // latched and must not be made to look doubtful.
                    .opacity(confidence == .lastSeen ? 0.6 : 1)
                if layout.statesBothCounts {
                    if blocked > 0 { GlanceLabel(glance: .blocked(blocked)) }
                    if reviews > 0 { GlanceLabel(glance: .review(reviews)) }
                }
                if blocked == 0, reviews == 0, let top, !top.line.isEmpty,
                    top.line != agentTitle(top)
                {
                    // What the agent is actually doing, which is worth the line
                    // only when no count wants it. Skipped when it is already
                    // the line above: an agent pushed before the app has ever
                    // seen it has no `headline`, so `agentTitle` falls back to
                    // this very string.
                    Text(top.line)
                        .font(.caption)
                        .lineLimit(1)
                        .opacity(confidence == .lastSeen ? 0.6 : 1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case .count:
            SmallFleet(entry: entry)
        case let .rows(limit, feed):
            RowsFleet(
                entry: entry, limit: limit, showFeed: feed,
                statesBothCounts: layout.statesBothCounts)
        }
    }

    /// The one line the two text accessories spend their whole budget on.
    private var headline: String {
        guard let top, !agentTitle(top).isEmpty else { return nothingKnown }
        return stated(top, confidence)
    }

    /// What the accessories say with no top agent to name.
    ///
    /// Two different sentences because they are two different facts: a fleet
    /// that has been looked at and has nothing in it, and a phone that has
    /// never looked. The accessories are too small for the footer the home
    /// screen families draw, so the distinction has to live in this one line.
    private var nothingKnown: String {
        entry.hasSnapshot ? "No agents" : "Open \(appName)"
    }
}

/// One number and the agent behind it.
private struct SmallFleet: View {
    let entry: FleetEntry

    var body: some View {
        // One number, by precedence: blocked, then reviews, then working. This
        // family has room for exactly one, and which one it picks is decided in
        // `FleetSnapshot.glance(at:)` so that this widget and the complication
        // on a wrist beside it cannot pick differently.
        let glance = entry.snapshot.glance(at: entry.date)
        VStack(alignment: .leading, spacing: 6) {
            // Gated on having a snapshot at all: a large "0" over "agents need
            // you" is a confident answer to a question this phone has never
            // asked anyone. With nothing known the footer's sentence is the
            // whole widget, which is sparse and true.
            //
            // And gated on there being something to say, which is new and is
            // the same rule pointed at a different failure. This used to draw
            // "0 · agents need you" for every fleet with nobody blocked in it,
            // including a fleet whose four working agents this snapshot had
            // stopped vouching for an hour ago — a reassuring zero standing in
            // for "I have not heard anything in a while". A nil glance is
            // exactly that state, and the agent line and the footer below say
            // it properly.
            if entry.hasSnapshot, let glance {
                Text("\(glance.count)")
                    .font(.system(size: 44, weight: .semibold, design: .rounded))
                    .foregroundStyle(glanceTint(glance))
                Text(glance.caption)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            if let top = entry.snapshot.ranked.first {
                AgentLine(agent: top, snapshot: entry.snapshot, at: entry.date)
            }
            StaleFooter(entry: entry)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A list of agents, most urgent first.
private struct RowsFleet: View {
    let entry: FleetEntry
    let limit: Int
    let showFeed: Bool
    /// Whether this family may state both counts at once. See
    /// `FleetWidgetView.Layout.statesBothCounts`.
    let statesBothCounts: Bool

    var body: some View {
        let blocked = entry.snapshot.needingYou
        // `?? 0` here and not in the snapshot: nil is "this build was never
        // told about reviews", and no line at all is what that should draw. The
        // reverse — rendering "0 to review" — would assert something no host
        // said.
        let reviews = entry.snapshot.needsReview ?? 0
        VStack(alignment: .leading, spacing: 6) {
            // Only when a real capture came back with nothing. Before the first
            // one, the footer says which of the two this is; "No agents" here
            // as well would answer the question wrongly in the larger type.
            if entry.snapshot.agents.isEmpty, entry.hasSnapshot {
                Text("No agents").font(.headline).foregroundStyle(.secondary)
            }
            // Both counts, above the rows, whenever both have something to say.
            // Side by side rather than stacked because these families are wide
            // and their rows are the expensive part of the layout; the rule the
            // narrow accessory keeps — show BOTH, never collapse them into one
            // number — is the same one, since "2 need you" and "3 to review"
            // are different work in different places.
            if statesBothCounts, blocked > 0 || reviews > 0 {
                HStack(spacing: 10) {
                    if blocked > 0 { GlanceLabel(glance: .blocked(blocked)) }
                    if reviews > 0 { GlanceLabel(glance: .review(reviews)) }
                    Spacer(minLength: 0)
                }
            }
            ForEach(entry.snapshot.ranked.prefix(limit)) { agent in
                if let url = terminalURL(agent) {
                    Link(destination: url) {
                        AgentLine(agent: agent, snapshot: entry.snapshot, at: entry.date)
                    }
                } else {
                    AgentLine(agent: agent, snapshot: entry.snapshot, at: entry.date)
                }
            }
            // The large family's whole reason for being: three lines of what
            // the agent actually said answers "what did it do while I was
            // away", which is the question a widget gets looked at to answer.
            if showFeed, let top = entry.snapshot.ranked.first, !top.feed.isEmpty {
                Divider()
                ForEach(Array(top.feed.enumerated()), id: \.offset) { _, said in
                    Text(said).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer(minLength: 0)
            StaleFooter(entry: entry)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// One agent: its mark, its name, and where it is.
private struct AgentLine: View {
    let agent: FleetSnapshot.Agent
    let snapshot: FleetSnapshot
    let at: Date

    var body: some View {
        let confidence = snapshot.confidence(in: agent, at: at)
        let title = agentTitle(agent)
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(agent.glyph)
                .font(.caption.monospaced())
                // Amber for the one that is waiting on a person, and nothing
                // else — the same reservation the Live Activity badge makes, so
                // a glance at either surface answers "does this need me"
                // without reading a word.
                .foregroundStyle(agent.status == "blocked" ? Color.orange : Color.secondary)
            VStack(alignment: .leading, spacing: 0) {
                // "last seen working" rather than "working": an agent that was
                // working an hour ago has very likely finished, and a widget
                // that keeps asserting it is a widget telling you something
                // untrue in the calmest possible voice.
                Text(stated(agent, confidence))
                    .font(.caption.weight(.medium))
                    .lineLimit(1)
                // Skipped when it is already the line above. An agent pushed
                // before the app has ever seen it has no `headline`, so
                // `agentTitle` falls back to this very string — and a row that
                // printed it twice would spend both of its lines saying one
                // thing.
                if !agent.line.isEmpty, agent.line != title {
                    Text(agent.line)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .opacity(confidence == .lastSeen ? 0.6 : 1)
    }
}

/// How old this is, and whether it is the whole fleet.
private struct StaleFooter: View {
    let entry: FleetEntry

    var body: some View {
        // A snapshot assembled only from pushes knows about the agents that
        // happened to notify. Saying so is the difference between "these are
        // your agents" and "these are the ones I have heard from".
        let source = entry.snapshot.complete ? "" : " · from notifications"
        if entry.hasSnapshot {
            Text("\(entry.snapshot.capturedAt, style: .relative) ago\(source)")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        } else {
            Text("Open \(appName) to see your agents")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
    }
}

#if DEBUG
    /// The four states every family here has to be looked at in.
    ///
    /// One timeline per family rather than four previews per family. The canvas
    /// steps through a timeline, so this is 6 canvases instead of 24 — and the
    /// question worth answering is whether "2 need you" and "3 to review" are
    /// tellable apart WITHOUT reading them, which is a question about two
    /// renders of one slot and is easiest to see by stepping one family through
    /// all four.
    ///
    /// The fourth state is the one that is easy to forget and expensive to get
    /// wrong: a snapshot whose `reviewsWaiting` is nil, written by an older
    /// build or by a phone whose runner cannot answer `changes.inbox`. It must
    /// draw as the widget always did — no review line anywhere, and never a
    /// "0 to review".
    private enum PreviewFleet {
        static let now = Date()

        static func agent(
            _ id: String, _ status: String, _ headline: String, _ line: String,
            _ glyph: String, rank: UInt32
        ) -> FleetSnapshot.Agent {
            FleetSnapshot.Agent(
                id: id, label: "claude", machine: "orchard", status: status,
                glyph: glyph, headline: headline, line: line,
                feed: [
                    "Reading crates/daemon/src/review_ops.rs.",
                    "Ran cargo test — 214 passed.",
                    "Writing the inbox gate.",
                ],
                rank: rank, turnFailed: false, activityChangedAt: now)
        }

        /// Two stopped, two getting on with it. `rank` ascending puts the
        /// blocked pair first, exactly as the host would.
        static let blocked: [FleetSnapshot.Agent] = [
            agent("t1", "blocked", "claude asks", "Run the migration?", "▲", rank: 10),
            agent("t2", "blocked", "codex asks", "Overwrite fruit.txt?", "▲", rank: 20),
            agent("t3", "working", "claude 4m", "Writing review_ops.rs", "●", rank: 300),
            agent("t4", "working", "claude 1m", "Reading watch.rs", "●", rank: 310),
        ]

        /// Nobody stopped. Whether anything is waiting is the review count's
        /// business, which is the whole point of the middle two states.
        static let quiet: [FleetSnapshot.Agent] = [
            agent("t3", "working", "claude 4m", "Writing review_ops.rs", "●", rank: 300),
            agent("t4", "working", "claude 1m", "Reading watch.rs", "●", rank: 310),
            agent("t5", "working", "codex 12m", "Running cargo test", "●", rank: 320),
            agent("t6", "working", "claude 2m", "Reading FleetWidget.swift", "●", rank: 330),
        ]

        static func snapshot(_ agents: [FleetSnapshot.Agent], reviews: Int?) -> FleetSnapshot {
            FleetSnapshot(
                agents: agents, capturedAt: now, complete: true, reviewsWaiting: reviews)
        }

        /// The four entries, one per state, a minute apart so the canvas can be
        /// stepped through them.
        /// The four entries, one per state, a minute apart so the canvas can
        /// be stepped through them. Named separately because the `timeline:`
        /// builder takes entries and not an array of them.
        static let blockedState = FleetEntry(date: now, snapshot: snapshot(blocked, reviews: 3))
        static let reviewsOnlyState = FleetEntry(
            date: now.addingTimeInterval(60), snapshot: snapshot(quiet, reviews: 3))
        static let allClearState = FleetEntry(
            date: now.addingTimeInterval(120), snapshot: snapshot(quiet, reviews: 0))
        static let reviewsUnknownState = FleetEntry(
            date: now.addingTimeInterval(180), snapshot: snapshot(quiet, reviews: nil))
    }

    #Preview("Small · blocked, reviews, clear, unknown", as: .systemSmall) {
        FleetWidget()
    } timeline: {
        PreviewFleet.blockedState
        PreviewFleet.reviewsOnlyState
        PreviewFleet.allClearState
        PreviewFleet.reviewsUnknownState
    }

    #Preview("Medium · blocked, reviews, clear, unknown", as: .systemMedium) {
        FleetWidget()
    } timeline: {
        PreviewFleet.blockedState
        PreviewFleet.reviewsOnlyState
        PreviewFleet.allClearState
        PreviewFleet.reviewsUnknownState
    }

    #Preview("Large · blocked, reviews, clear, unknown", as: .systemLarge) {
        FleetWidget()
    } timeline: {
        PreviewFleet.blockedState
        PreviewFleet.reviewsOnlyState
        PreviewFleet.allClearState
        PreviewFleet.reviewsUnknownState
    }

    #Preview("Circular · blocked, reviews, clear, unknown", as: .accessoryCircular) {
        FleetWidget()
    } timeline: {
        PreviewFleet.blockedState
        PreviewFleet.reviewsOnlyState
        PreviewFleet.allClearState
        PreviewFleet.reviewsUnknownState
    }

    #Preview("Rectangular · blocked, reviews, clear, unknown", as: .accessoryRectangular) {
        FleetWidget()
    } timeline: {
        PreviewFleet.blockedState
        PreviewFleet.reviewsOnlyState
        PreviewFleet.allClearState
        PreviewFleet.reviewsUnknownState
    }

    #Preview("Inline · blocked, reviews, clear, unknown", as: .accessoryInline) {
        FleetWidget()
    } timeline: {
        PreviewFleet.blockedState
        PreviewFleet.reviewsOnlyState
        PreviewFleet.allClearState
        PreviewFleet.reviewsUnknownState
    }
#endif
