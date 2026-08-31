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

/// One count, said in full: the mark, the number and the words.
///
/// `Label` rather than an `HStack`, so the mark and the text keep the system's
/// own spacing and stay together when the type size grows — these lines are
/// drawn at accessibility sizes on a lock screen, where a hand-spaced icon and
/// its text drift apart.
///
/// **The icon is the state mark now, not an SF Symbol.** It used to be
/// `glance.symbol` — a `▲` or a `±` — which made this one of the four
/// different drawings four surfaces used for one fact. The spec's §03 is a
/// single mark whose ring says whether your attention is wanted, and the whole
/// value of one mark is that a person who has learned it on the home screen has
/// learned it on the lock screen and the wrist at the same time.
///
/// **`.withoutCore`**, like every mark on a widget: §08 says
/// "Working versus idle never appears on a widget. It flips every few seconds;
/// at this refresh rate the claim would be false more often than true."
///
/// Never dimmed by `confidence`. Both counts this draws are latched: an agent
/// that was blocked an hour ago is still blocked, and a diff nobody has
/// reviewed is still unreviewed. Dimming them would make the two facts a person
/// opens this widget for look like the doubtful part of it.
private struct GlanceLabel: View {
    @Environment(\.colorScheme) private var scheme
    let glance: FleetSnapshot.Glance

    var body: some View {
        Label {
            Text(glance.phrase)
        } icon: {
            GlanceMarkView(GlanceMark(glance: glance).withoutCore, size: .row)
        }
        .font(.caption.weight(.medium))
        .foregroundStyle(GlancePalette.tint(glance, scheme))
        .lineLimit(1)
    }
}

/// The fleet, as one monochrome-safe row of state marks.
///
/// §06's accessoryRectangular: "Count, the names behind it, and the fleet as
/// one monochrome ribbon." Row size — `GlanceMarkSize.row` — because
/// accessoryRectangular takes the row size: §03, "it IS a row: mark, label,
/// trace."
///
/// **`WidgetRenderingMode` is read because color cannot be leaned on here.**
/// Lock screen accessories render `.vibrant` or `.accented` — the system
/// strips every hue a mark would otherwise carry — so what has to tell twelve
/// states apart is `GlanceMarkView`'s own vocabulary of stroke weight, fill
/// and dash, which §03 built for exactly this reason: "the mark distinguishes
/// states by stroke weight, fill and dash, so hue was always redundant
/// reinforcement." `.fullColor` is the widget-gallery/picker case, the one
/// place this ribbon is ever seen with its hue intact.
///
/// **Quiet marks drop out once color is gone.** §03's own concession: "If
/// 1pt hairlines wash out under vibrancy, quiet marks leave the accessory
/// ribbon and only the attention marks are drawn; the count text carries the
/// rest." A ribbon that is all 1pt hairlines when nobody is blocked would
/// also be pure decoration under §05's rule for an all-clear tile — there is
/// nothing left for the eye to sort, so the row is worth spending only on the
/// states that need a person.
///
/// Fixed order — `ranked`, never re-derived from activity here or anywhere
/// else in this file — so a ribbon glanced at twice in a row does not
/// reshuffle under a finger that touched nothing.
///
/// Every mark is `decorative`, the same choice `ShellRibbon` makes for its
/// own ribbon in the app: the headline line above already names the agent
/// that matters, and VoiceOver reading "Needs you, at a prompt" once per dot
/// in a row of eight would repeat one word eight times without saying which
/// agent any dot was.
private struct FleetRibbon: View {
    @Environment(\.widgetRenderingMode) private var renderingMode
    let snapshot: FleetSnapshot
    let at: Date

    var body: some View {
        let marks = snapshot.ranked.map {
            GlanceMark(agent: $0, confidence: snapshot.confidence(in: $0, at: at)).withoutCore
        }
        let shown = renderingMode == .fullColor ? marks : marks.filter { !$0.isQuiet }
        HStack(spacing: 3) {
            ForEach(Array(shown.enumerated()), id: \.offset) { _, mark in
                GlanceMarkView(mark, size: .row, decorative: true)
            }
        }
    }
}

struct FleetWidgetView: View {
    @Environment(\.widgetFamily) private var family
    /// Light mode is a different palette rather than a filter over the dark one
    /// — §01: "Not a filter flip." See `GlanceInk`.
    @Environment(\.colorScheme) private var scheme
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
            //
            // **Words only, and BOTH tiers.** §06: "accessoryInline has no
            // styling channel at all, so it is words only: `2 need you · 3 to
            // review`. Highest tier first, and it appends `· 2m` when the
            // snapshot is not fresh." The SF Symbol this used to lead with was
            // the one part of the line the system is free to draw in its own
            // tint at its own size, so it spent the narrowest slot in the
            // product on the only glyph that could not be relied on.
            Text(inlineLine)
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
            VStack(spacing: 2) {
                if let glance = entry.snapshot.glance(at: entry.date) {
                    // §06: "Heavy ring plus the count, at the lone-indicator
                    // size and stroke given in §03 — the heaviest mark in the
                    // system, which is both what a blocked agent earns and what
                    // survives vibrancy on a photograph." So 15pt with the
                    // 3.5pt ring, not a `.title3` SF Symbol.
                    GlanceMarkView(GlanceMark(glance: glance).withoutCore, size: .lone)
                        .foregroundStyle(GlancePalette.tint(glance, scheme))
                    Text("\(glance.count)")
                        .font(.caption2.monospacedDigit())
                } else {
                    // No count to make, so the mark is the top agent's own
                    // state rather than the fleet's — the same drawing, asked a
                    // narrower question.
                    GlanceMarkView(
                        top.map { GlanceMark(agent: $0, confidence: confidence).withoutCore }
                            ?? GlanceMark(attention: .quiet, core: nil),
                        size: .lone)
                    // No opacity dim any more, and that is the mark earning its
                    // keep rather than a rule being dropped. The top agent's
                    // mark CAN be a `working` one that has expired; a dashed
                    // ring is what §03 says that looks like — "a broken ring is
                    // a broken link" — and it survives the vibrancy flattening
                    // that a 60% opacity does not.
                    //
                    // A dash, not a zero, before anything has been written —
                    // AND once the top agent's own mark above has gone dashed
                    // too. `entry.snapshot.needingYou` is provably 0 on every
                    // path that reaches this `else`: `glance(at:)` returns
                    // `.blocked` the moment it is not, so printing it plainly
                    // was printing a guaranteed zero regardless of how old the
                    // snapshot answering it is. "0" under a lock screen clock
                    // is a confident claim that nobody needs you; once
                    // `confidence` has lapsed to `.lastSeen` this snapshot can
                    // no longer back that claim — a blocked agent since would
                    // arrive by the same unreliable push that let this snapshot
                    // go stale in the first place — and the digit has to say
                    // "I don't know" the same way the ring above it already
                    // does with its dash.
                    Text(entry.hasSnapshot && confidence == .known ? "\(entry.snapshot.needingYou)" : "–")
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
                // §06: "Count, the names behind it, and the fleet as one
                // monochrome ribbon." One agent alone would just be the
                // headline drawn twice, so the ribbon earns its row only once
                // there is a fleet to be a picture of.
                if entry.snapshot.agents.count > 1 {
                    FleetRibbon(snapshot: entry.snapshot, at: entry.date)
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

    /// The whole of `accessoryInline`, as §06 specifies it.
    ///
    /// Both counts, highest tier first, joined by the separator the spec draws
    /// — and the snapshot's age appended in the same way when it is no longer
    /// fresh. `glance(at:)` is deliberately NOT what picks here: it answers
    /// "the one thing this fleet is about", which is the right question for a
    /// slot holding one number and the wrong one for a line that has room for
    /// two facts.
    ///
    /// With neither count to make, it falls back to naming the top agent — the
    /// same fallback this family always had, including the case a stale
    /// snapshot creates, where `stated` prefixes "last seen".
    private var inlineLine: String {
        var parts: [String] = []
        if entry.snapshot.needingYou > 0 {
            parts.append(FleetSnapshot.Glance.blocked(entry.snapshot.needingYou).phrase)
        }
        // `?? 0` here and not in the snapshot: nil means this build was never
        // told about reviews, and saying nothing is what "not told" should
        // draw. What must never happen is the reverse — "0 to review" asserting
        // something nobody said.
        if let reviews = entry.snapshot.needsReview, reviews > 0 {
            parts.append(FleetSnapshot.Glance.review(reviews).phrase)
        }
        if parts.isEmpty { parts.append(headline) }
        let age = entry.snapshot.age(at: entry.date)
        // Only when it is not fresh. A line that appended "· now" to every
        // render would spend the narrowest slot in the product restating that
        // nothing is wrong.
        if entry.hasSnapshot, age >= GlanceAge.fresh {
            parts.append(GlanceAge.brief(age))
        }
        return parts.joined(separator: " · ")
    }

    /// The one line the rectangular accessory spends its whole budget on.
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
    @Environment(\.colorScheme) private var scheme
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
                // §02's own row: "38–46 / 600, −0.035em. The single count on a
                // widget. Tabular numerals." Not `.rounded`, which this drew
                // before — §02's rule is SF for chrome and SF Mono for anything
                // a machine produced, and rounded is neither.
                Text("\(glance.count)")
                    .glanceType(.count())
                    .foregroundStyle(GlancePalette.tint(glance, scheme))
                Text(glance.caption)
                    .glanceType(.secondary)
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
    @Environment(\.colorScheme) private var scheme
    let agent: FleetSnapshot.Agent
    let snapshot: FleetSnapshot
    let at: Date

    var body: some View {
        let confidence = snapshot.confidence(in: agent, at: at)
        let title = agentTitle(agent)
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            // The state mark at the row size — 10pt with no core, which §03
            // spells out: "There is no 10pt core — a row shows the ring alone."
            //
            // This used to be `Text(agent.glyph)`, a literal character off the
            // wire, which made a widget row the third of four different
            // drawings of one fact. The glyph is still on the wire and still
            // right for the surfaces that are text and nothing else; a row has
            // room to draw the mark, and drawing it is what makes a person who
            // has learned it here able to read the lock screen.
            //
            // `.withoutCore` for §08's reason: at one reload per twenty
            // minutes, working-versus-idle "would be false more often than
            // true".
            GlanceMarkView(GlanceMark(agent: agent, confidence: confidence).withoutCore, size: .row)
                // A shape has no baseline of its own, so a `firstTextBaseline`
                // stack would hang it off its bottom edge and leave it sitting
                // a descender low. One point below the baseline centres it
                // against the x-height of the caption beside it.
                .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 1 }
            VStack(alignment: .leading, spacing: 0) {
                // "last seen working" rather than "working": an agent that was
                // working an hour ago has very likely finished, and a widget
                // that keeps asserting it is a widget telling you something
                // untrue in the calmest possible voice.
                Text(stated(agent, confidence))
                    .glanceType(.rowName)
                    // **Stated, because a `Link` tints its label.** The rows on
                    // the medium and large families are wrapped in one so a tap
                    // opens that agent, and a `Link` label with no colour of
                    // its own is drawn in the accent colour — so every agent's
                    // name on the two biggest tiles was blue, which in a
                    // palette whose whole rule is that one hue is reserved is a
                    // second hue nobody chose. §01's `text 1`: "Names, counts,
                    // anything you read first."
                    .foregroundStyle(GlancePalette.ink1(scheme))
                    .lineLimit(1)
                // Skipped when it is already the line above. An agent pushed
                // before the app has ever seen it has no `headline`, so
                // `agentTitle` falls back to this very string — and a row that
                // printed it twice would spend both of its lines saying one
                // thing.
                if !agent.line.isEmpty, agent.line != title {
                    // Same reason as the line above it: inside a `Link`,
                    // `.secondary` is a secondary ACCENT rather than a secondary
                    // ink. §01's `text 2` is the floor for on-device text and is
                    // what this row is for.
                    Text(agent.line)
                        .glanceType(.secondary)
                        .foregroundStyle(GlancePalette.ink2(scheme))
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
            // **A figure, not a running clock.** This was
            // `Text(_, style: .relative)`, which ticks: a widget counting
            // seconds up from a measurement it took a quarter of an hour ago,
            // to a precision it does not have. §02: "Relative and coarse: 2m
            // ago, 52m ago, 3h ago. Never a running clock on an idle agent —
            // precision nobody needs implies precision we do not have."
            //
            // Mono because it came off a machine, at §02's 11pt floor.
            Text("\(GlanceAge.stated(entry.snapshot.age(at: entry.date)))\(source)")
                .glanceType(.monoFigures)
                .foregroundStyle(.tertiary)
        } else {
            Text("Open \(appName) to see your agents")
                .glanceType(.secondary)
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

        /// One working agent, alone, whose news is older than
        /// `FleetSnapshot.staleAfter`. `confidence(in:at:)` has lapsed to
        /// `.lastSeen` for it, so `glance(at:)` will not count it and the
        /// circular accessory falls into the "no count to make" branch this
        /// fixture exists to exercise.
        ///
        /// **The fixture that proves the circular fix.** Before it, this
        /// state rendered a dashed ring over a confident "0" — the ring said
        /// "I don't know" and the digit beneath it said "definitely zero" in
        /// the same breath. The digit now dashes with the ring.
        static let staleAgent = FleetSnapshot.Agent(
            id: "t9", label: "claude", machine: "orchard", status: "working",
            glyph: "●", headline: "claude 3h", line: "Writing review_ops.rs",
            feed: [], rank: 300, turnFailed: false,
            activityChangedAt: now.addingTimeInterval(-FleetSnapshot.staleAfter - 60))
        static let staleState = FleetEntry(
            date: now,
            snapshot: FleetSnapshot(
                agents: [staleAgent],
                capturedAt: now.addingTimeInterval(-FleetSnapshot.staleAfter - 60),
                complete: true, reviewsWaiting: 0))
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

    /// A fifth state, on its own rather than folded into the four above: it
    /// is the one this task exists to fix, and stepping past it quickly in a
    /// four-frame timeline is exactly how a regression here would go
    /// unnoticed. Watch the ring go dashed AND the digit go dashed with it —
    /// before the fix, only the ring did.
    #Preview("Circular · stale — dash, not zero", as: .accessoryCircular) {
        FleetWidget()
    } timeline: {
        PreviewFleet.staleState
    }

    /// **Rendered, not reasoned about.** Lock screen accessories render
    /// `.vibrant`, and the tinted Home Screen renders `.accented` — both
    /// strip hue — so this steps `FleetRibbon` through `.fullColor`,
    /// `.vibrant` and `.accented` directly via `.environment`, the same
    /// mechanism `FleetRibbon` itself reads at `\.widgetRenderingMode`.
    ///
    /// The bottom two rows are additionally desaturated with `.grayscale(1)`.
    /// That is not what the system does internally — real accessory vibrancy
    /// is a compositing pass in the accessory host, outside SwiftUI, and
    /// cannot be reproduced inside a canvas preview — but it removes hue the
    /// same way `GlanceMark.swift`'s own matrix preview does, "the same grid
    /// desaturated", which is enough to answer the one question that
    /// matters here: with color gone, can the ribbon still be read? What
    /// should survive per row: row 1 in full colour; rows 2–3 down to
    /// stroke weight and presence only, with the two hairline "quiet" marks
    /// dropped and only the two heavier "needs you" rings left.
    #Preview("Ribbon · monochrome legibility") {
        let fixture = PreviewFleet.snapshot(PreviewFleet.blocked, reviews: 3)
        let rows: [(String, WidgetRenderingMode)] = [
            ("full colour — widget gallery", .fullColor),
            ("vibrant — Lock Screen", .vibrant),
            ("accented — tinted Home Screen", .accented),
        ]
        return VStack(alignment: .leading, spacing: 20) {
            ForEach(rows, id: \.0) { label, mode in
                VStack(alignment: .leading, spacing: 6) {
                    Text(label).font(.caption2).foregroundStyle(.white)
                    FleetRibbon(snapshot: fixture, at: PreviewFleet.now)
                        .environment(\.widgetRenderingMode, mode)
                        .grayscale(mode == .fullColor ? 0 : 1)
                        .padding(10)
                        .background(Color.black, in: RoundedRectangle(cornerRadius: 8))
                }
            }
        }
        .padding()
        .background(Color(white: 0.08))
    }
#endif
