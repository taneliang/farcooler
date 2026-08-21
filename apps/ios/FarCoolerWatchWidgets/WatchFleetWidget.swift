import SwiftUI
import WidgetKit

/// The extension's entry point.
///
/// A `WidgetBundle` with one widget in it rather than `@main` on the widget
/// itself, for the reason `FarCoolerActivityBundle` records: there is no way to
/// add a second widget to a `@main` widget without rewriting the file into
/// exactly this. WidgetKit discovers complications by asking the `@main` bundle
/// for them, so a `Widget` that compiles but is not named here is absent from
/// the face gallery and from the Smart Stack with no error anywhere.
@main
struct FarCoolerWatchWidgets: WidgetBundle {
    var body: some Widget {
        WatchFleetWidget()
    }
}

/// The fleet, on a watch face and in the Smart Stack.
///
/// This is the surface the watch app exists for. Almost nobody raises a wrist,
/// finds an app and taps it to learn that an agent is waiting; they see it on
/// the face, or they never see it.
///
/// Everything drawn here was decided on the host. `glyph`, `headline`, `line`
/// and the order come off the snapshot unchanged — `rank` in particular, which
/// is computed in `farcooler_core::feed` precisely so that a complication with
/// room for ONE agent and a fleet list with room for twelve name the same one.
/// Re-deriving "the most urgent agent" here would be the disagreement the whole
/// ladder exists to prevent, and it would be visible: the complication and the
/// list it opens are on screen within a second of each other.
///
/// The one judgement it makes is how confident to sound, and it makes it the
/// same way `FleetWidget` does on the phone and `FleetListView` does in the app
/// beside it — `FleetSnapshot.confidence(in:at:)`, so all three stop vouching
/// for an agent at one definition of stale rather than three.
///
/// **It reads the WATCH's container, which the phone never writes.** An App
/// Group is per device. `WatchLinkClient.receive` writes each arriving context
/// through `SnapshotStore` and then calls `WidgetCenter.reloadAllTimelines`,
/// and that write is the only thing that ever puts bytes in the file this
/// process opens. Which is why the "nothing ever written" state below is not a
/// corner case here the way it is on a phone: it is what every watch shows
/// until its own app has run once with the phone in range.
struct WatchFleetWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "WatchFleetWidget", provider: WatchFleetProvider()) { entry in
            WatchFleetView(entry: entry)
                // Required since watchOS 10: a widget that sets no container
                // background is drawn with none at all in the Smart Stack,
                // which reads as a rendering bug rather than as a style. The
                // system material and not a color of ours, so a watch face
                // shows through it the way every other complication does.
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Agents")
        .description("What your agents are doing, and which one needs you.")
        .supportedFamilies([.accessoryCircular, .accessoryRectangular, .accessoryInline])
    }
}

struct WatchFleetEntry: TimelineEntry {
    let date: Date
    let snapshot: FleetSnapshot

    /// Whether a snapshot has ever been written on THIS watch.
    ///
    /// `FleetSnapshot.empty` is not an empty fleet — it is the absence of an
    /// observation, and the two must not render alike. "No agents" is a claim
    /// about the runners this person keeps; a watch that has never heard from
    /// the phone has made no observation that entitles it to make one. The
    /// distinction is the same one `FleetEntry.hasSnapshot` draws on the phone
    /// and `WatchState.nothing` draws in the app, and it matters more here than
    /// in either: a fresh watch is in this state by default, so getting it
    /// wrong means the first thing this feature ever says is a confident "0".
    ///
    /// The epoch `capturedAt` is what tells them apart, because a real capture
    /// always has a real date.
    var hasSnapshot: Bool { snapshot.capturedAt.timeIntervalSince1970 > 0 }
}

struct WatchFleetProvider: TimelineProvider {
    func placeholder(in context: Context) -> WatchFleetEntry {
        WatchFleetEntry(date: Date(), snapshot: .empty)
    }

    func getSnapshot(in context: Context, completion: @escaping (WatchFleetEntry) -> Void) {
        completion(WatchFleetEntry(date: Date(), snapshot: SnapshotStore.read() ?? .empty))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<WatchFleetEntry>) -> Void)
    {
        let snapshot = SnapshotStore.read() ?? .empty
        let now = Date()

        // One entry for now, and one for every moment after it at which this
        // snapshot stops vouching for another agent.
        //
        // Reloads arrive from outside — `WatchLinkClient.receive` calls
        // `reloadAllTimelines` on every context that lands — so the first entry
        // alone would be enough to show what is KNOWN. It is not enough to stop
        // saying what is no longer known, and on a wrist that gap is wider than
        // on a phone: a watch out of range of its phone receives nothing at all
        // for a whole day of walking around, and a complication that could only
        // learn it had gone stale from a message it is not receiving would
        // assert `working` for the entire day.
        //
        // ALL the moments, not the first. Ages are per agent — see
        // `FleetSnapshot.age(of:at:)` — so agents expire at different times, and
        // a timeline that stopped at the earliest would render that entry from
        // then on with every LATER agent still drawn as current, permanently.
        // Each entry re-renders from the same snapshot at its own date, and
        // `confidence(in:at:)` is what turns that date into what the face is
        // allowed to say.
        //
        // `.never`, because nothing here gets better with time: past the last
        // moment every render says the same thing, and asking watchOS for
        // wake-ups that change nothing spends a budget that is tighter on this
        // hardware than on a phone and that the reloads from the app need.
        let entries =
            [WatchFleetEntry(date: now, snapshot: snapshot)]
            + Self.wakes(for: snapshot, after: now).map {
                WatchFleetEntry(date: $0, snapshot: snapshot)
            }

        completion(Timeline(entries: entries, policy: .never))
    }

    /// How many staleness entries one timeline may carry.
    ///
    /// Six, half the phone widget's twelve, because no family here draws more
    /// than one agent: past the first there is nothing on screen that a wake-up
    /// could change except the count, and the count is `blocked`, which is
    /// latched and never expires. The cap is a guard against a pathological
    /// fleet rather than a working limit — there is at most one moment per
    /// unlatched agent, and agents sharing a second share a moment.
    ///
    /// It exists because a timeline is a whole snapshot per entry serialized
    /// into an extension process with a hard memory ceiling, and watchOS gives
    /// that process less than iOS does. A hundred entries is a complication
    /// that fails to render at all, which says less than a stale one does.
    private static let wakeLimit = 6

    /// The dates to re-render at, capped.
    ///
    /// The earliest moments, plus the LAST one whenever the cap bites. That
    /// last entry is not decoration: past it every unlatched agent in the
    /// snapshot has expired, so it is the one that guarantees nothing is
    /// asserted beyond its hour forever — the most a dropped middle moment can
    /// cost is one agent reading as current for part of the gap.
    private static func wakes(for snapshot: FleetSnapshot, after now: Date) -> [Date] {
        let moments = snapshot.stalenessMoments(after: now)
        guard moments.count > wakeLimit else { return moments }
        return Array(moments.prefix(wakeLimit - 1)) + Array(moments.suffix(1))
    }
}

/// The one string a surface with room for one uses to name an agent.
///
/// `headline` normally. But it can be EMPTY, and on exactly the path this
/// complication exists for: an agent first seen through a push gets
/// `headline: previous?.headline ?? ""` from the notification service
/// extension, which has a status word and a name and no ladder to run. A
/// `top?.headline ?? fallback` treats only a nil agent as unknown, so a surface
/// drawing one string drew a BLANK complication for exactly the agent that had
/// just become news.
///
/// Falling back down the wire's own fields, never deriving one: `line` is the
/// notification's body and is populated in precisely that case, `label` is at
/// least a name, and `status` is the one field the relay guarantees is
/// non-empty. Picking among fields the host sent is not composing a headline
/// here — nothing is written, so this file cannot end up phrasing an agent
/// differently from the app one tap away, which runs the identical fallback.
///
/// A third copy of `FleetListView.agentTitle` and `FleetWidget.agentTitle`, and
/// deliberately not shared: this is a separate binary from both, and sharing it
/// would mean moving it into AgentKit, where a projection of the pane would be
/// living somewhere other than the host. The rule the three copies keep is
/// "read the wire's own fields, in this order", which is small enough to state
/// and is stated in all three.
private func agentTitle(_ agent: FleetSnapshot.Agent) -> String {
    [agent.headline, agent.line, agent.label, agent.status].first { !$0.isEmpty } ?? ""
}

/// That title, put in the past tense once the snapshot has stopped vouching for
/// it.
///
/// The qualifier is a PREFIX and never a suffix, and on these three families
/// that is the whole design rather than a preference. Text truncates at the
/// tail, so a marker appended to the end is the first thing to vanish from
/// exactly the slot too narrow to keep it — an inline complication that drops
/// "· stale" and keeps "claude needs you" says the opposite of what it meant,
/// and the inline slot on a 40mm face is the narrowest text this project draws
/// anywhere. Prefixed, the worst that slot can do is eat the agent's own detail
/// while "last seen" survives, which degrades toward saying less rather than
/// toward saying something false.
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
private func stated(_ agent: FleetSnapshot.Agent, _ confidence: FleetSnapshot.Confidence) -> String
{
    let title = agentTitle(agent)
    return confidence == .lastSeen ? "last seen \(title)" : title
}

/// This build's app name, for the one sentence that has to say it.
///
/// `CFBundleDisplayName` in THIS extension's own Info.plist, which
/// generate-project.py fills from `version.sh app-name-short`. `Bundle.main`
/// inside an `.appex` is the appex, so the watch app's merged plist is not
/// reachable from here even in principle — the key has to be stamped into this
/// target too, or the fallback below is what ships.
///
/// A literal would tell somebody running the canary to open "Far Cooler", which
/// is either a different app on their watch or no app at all. That exact
/// mistake already shipped once in the Live Activity; see `ACTIVITY_COMMON` in
/// generate-project.py.
private var appName: String {
    Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "Far Cooler"
}

/// The one line the two text families spend their whole budget on.
///
/// Free rather than a method on the view, so the inline arm and `Rectangular`
/// cannot come to word this differently — they are one sentence in two type
/// sizes, and the day they disagree is the day a face and the Smart Stack above
/// it name the same agent two ways.
private func headline(
    entry: WatchFleetEntry, top: FleetSnapshot.Agent?,
    confidence: FleetSnapshot.Confidence, appName: String
) -> String {
    guard let top, !agentTitle(top).isEmpty else {
        // Two different sentences because they are two different facts: a fleet
        // that has been looked at and has nothing in it, and a watch that has
        // never been told anything. There is no room here for the footer the
        // phone's larger families draw, so the distinction lives in this one
        // line or it does not exist at all.
        //
        // "Open <app>" means the WATCH app, which is the action that fixes
        // this: the watch's container is written only by `WatchLinkClient`, and
        // it writes on activation from the context the system is already
        // holding. Naming the iPhone here — as `FleetListView` correctly does,
        // because by then the watch app IS open — would send somebody to the
        // wrong device.
        return entry.hasSnapshot ? "No agents" : "Open \(appName)"
    }
    return stated(top, confidence)
}

/// The one color rule this feature turns on.
///
/// **Amber means "needs you", and nothing else may wear it.** A blocked agent
/// is stopped until a person answers it; a worktree with an unreviewed diff is
/// merely waiting, and nothing is worse for having waited another hour. On a
/// wrist that reservation is worth more than anywhere else — this is the
/// surface people see without deciding to look, so the mark has to answer
/// "does this need me" before any word is read.
///
/// A second copy of the switch in `FleetWidget`, and it has to be a copy: that
/// is a separate binary, a color is a SwiftUI type, and `FleetSnapshot` — the
/// file both targets compile, and where the glyph, the words and the precedence
/// DO live, so they cannot drift — is the wire's shape and has no business
/// importing SwiftUI. Three cases is the whole rule and it is stated in both.
///
/// `AnyShapeStyle` because the three tints are not all colors: tertiary is a
/// hierarchical style, which is what lets the all-clear rung recede against
/// whatever watch face is behind it instead of sitting at a fixed gray that
/// disappears on one face and shouts on another.
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
/// own spacing and stay together as the type size grows — watchOS's largest
/// dynamic type sizes are what a Smart Stack card is read at, and a hand-spaced
/// icon drifts away from its text there.
///
/// Never dimmed by `confidence`. Both counts it draws are latched: an agent
/// blocked an hour ago is still blocked, and a diff nobody has reviewed is
/// still unreviewed. Dimming them would make the two facts somebody raises a
/// wrist for look like the doubtful part of the card.
private struct GlanceLabel: View {
    let glance: FleetSnapshot.Glance

    var body: some View {
        Label(glance.phrase, systemImage: glance.symbol)
            .font(.caption.weight(.medium))
            .foregroundStyle(glanceTint(glance))
            .lineLimit(1)
    }
}

struct WatchFleetView: View {
    @Environment(\.widgetFamily) private var family
    let entry: WatchFleetEntry

    /// The agent every family here is about.
    ///
    /// `ranked.first`, which is `rank` ascending with the id breaking ties —
    /// the order the host computed. Not "the first blocked one", not "the most
    /// recently changed": those are two more definitions of urgency, and the
    /// reason `rank` is on the wire at all is that there may only be one.
    private var top: FleetSnapshot.Agent? { entry.snapshot.ranked.first }

    /// How much this complication may still assert about that agent.
    ///
    /// Asked once, here, because all three families draw that agent and all
    /// three therefore have to answer for it. The phone's accessories
    /// originally did not ask at all, which made the staleness timeline — built
    /// precisely so a snapshot can go stale with no news — change nothing
    /// whatsoever on the surfaces that needed it most.
    private var confidence: FleetSnapshot.Confidence {
        top.map { entry.snapshot.confidence(in: $0, at: entry.date) } ?? .known
    }

    var body: some View {
        switch family {
        case .accessoryCircular:
            Circular(entry: entry, top: top, confidence: confidence)
        case .accessoryRectangular:
            Rectangular(entry: entry, top: top, confidence: confidence, appName: appName)
        default:
            // `.accessoryInline`, and anything a future watchOS adds.
            //
            // The QUALIFIER LEADS, exactly as it does on the phone's inline
            // accessory. There is no styling channel here at all — the system
            // draws this line in the face's own tint and ignores color and
            // opacity — so amber can do none of the work and the word has to do
            // all of it. Truncation eats the tail, and the inline slot on a
            // 40mm face is the narrowest text this project draws anywhere, so a
            // line that led with the agent's name would drop the half that says
            // what to do about it.
            //
            // The agent's own headline is what this says when there is nothing
            // to lead with, which includes the state a stale snapshot creates:
            // once no working agent can still be asserted, `glance(at:)`
            // returns nil and this falls back to "last seen claude 4m" — the
            // prefix that is this surface's entire degradation, kept intact.
            if let glance = entry.snapshot.glance(at: entry.date) {
                Label(glance.phrase, systemImage: glance.symbol)
            } else {
                Text(headline(entry: entry, top: top, confidence: confidence, appName: appName))
            }
        }
    }
}

/// A mark and a number, in a corner of the face.
///
/// **This is the ask this whole table was written for.** The slot holds one
/// glyph and one number, so "2 agents are stopped waiting for you" and "3
/// worktrees are waiting to be looked at" arrive in the same shape — and if
/// they arrive in the same COLOR with the same MARK, a glance cannot tell
/// urgent from merely pending, which is worse than showing neither. The pair is
/// what separates them: a filled amber triangle over 2, or an accented `±` over
/// 3, decided by `FleetSnapshot.glance(at:)` so this and the phone's circular
/// accessory cannot disagree about which of the two a fleet is currently about.
private struct Circular: View {
    let entry: WatchFleetEntry
    let top: FleetSnapshot.Agent?
    let confidence: FleetSnapshot.Confidence

    var body: some View {
        // The mark and the number are ONE fact, so they come from one answer.
        // A glyph belonging to the top agent above a number counting blocked
        // agents was two facts stacked in one slot, and a reader had no way to
        // know they were about different things.
        //
        // Nothing here dims. Both rungs that reach this slot with a number are
        // latched, and the volatile rung is already gone by then: `glance(at:)`
        // counts only working agents this snapshot can still vouch for, and
        // returns nil once there are none.
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
                    // The top agent's own mark, which CAN be a `working` one
                    // that has expired — so this one does dim.
                    .opacity(confidence == .lastSeen ? 0.6 : 1)
                // A dash, not a zero, before anything has been written. "0" on
                // a watch face is a statement that nothing needs you, and this
                // slot has no second line to qualify it with. It is also the
                // state every watch is in the moment this complication is first
                // added — which is the moment somebody decides whether the
                // feature works.
                Text(entry.hasSnapshot ? "\(entry.snapshot.needingYou)" : "–")
                    .font(.caption2.monospacedDigit())
            }
        }
    }
}

/// A headline and what the agent is doing, in the Smart Stack.
private struct Rectangular: View {
    let entry: WatchFleetEntry
    let top: FleetSnapshot.Agent?
    let confidence: FleetSnapshot.Confidence
    let appName: String

    var body: some View {
        let blocked = entry.snapshot.needingYou
        // `?? 0` here and not in the snapshot: nil is "this build was never told
        // about reviews", and no line at all is what that should draw. The
        // reverse — rendering "0 to review" — would assert something no host
        // said, on the surface least able to qualify it.
        let reviews = entry.snapshot.needsReview ?? 0
        VStack(alignment: .leading, spacing: 1) {
            Text(headline(entry: entry, top: top, confidence: confidence, appName: appName))
                .font(.headline)
                .lineLimit(1)
                // Only the agent's own words dim. The counts below are latched
                // and must not be made to look doubtful — see `Circular`.
                .opacity(confidence == .lastSeen ? 0.6 : 1)
            // Both counts, stacked, whenever both have something to say. This
            // family has the room and the phone's rectangular accessory spends
            // it the same way: "2 need you" and "3 to review" are different
            // work in different places, and one number standing for both would
            // send somebody to the wrong screen.
            if blocked > 0 { GlanceLabel(glance: .blocked(blocked)) }
            if reviews > 0 { GlanceLabel(glance: .review(reviews)) }
            if blocked == 0, reviews == 0, let top, !top.line.isEmpty,
                top.line != agentTitle(top)
            {
                // The host's signal line — "3/7 · Designing test matrix · 2
                // agents" — which answers "what is it actually doing" and is
                // the only reason this family is worth more than the inline
                // one. Skipped when it is already the line above: an agent
                // pushed before the app has ever seen it has no `headline`, so
                // `agentTitle` falls back to this very string, and a card that
                // printed it twice would spend both its lines saying one thing.
                Text(top.line)
                    .font(.caption)
                    .lineLimit(1)
                    .opacity(confidence == .lastSeen ? 0.6 : 1)
            } else if !entry.hasSnapshot {
                // The one slot with room to say WHICH empty state this is. The
                // line above says what to do about it; this says why it is
                // being said, so a Smart Stack card showing it does not read as
                // a watch that has looked and found nothing.
                //
                // Two lines rather than one, and short enough that it needs
                // only one on a 46mm face. Everything else here truncates at
                // the tail because it is the host's words and the tail is the
                // least of them; this is a whole sentence of ours, and
                // "Nothing has reached t…" is the one kind of clipping that
                // reads as a broken widget rather than as a narrow screen.
                Text("Nothing has arrived yet.")
                    .font(.caption)
                    .lineLimit(2)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

#if DEBUG
    /// The four states all three families here have to be looked at in.
    ///
    /// The same fixtures the phone's `FleetWidget` previews use, and
    /// deliberately the same four states in the same order: the point of this
    /// task is that a person seeing both surfaces within a minute must not have
    /// to work out whether they mean the same thing, and the way to check that
    /// is to step the two canvases through the same timeline side by side.
    ///
    /// The fourth state is the one that is easy to forget and expensive to get
    /// wrong: a snapshot whose `reviewsWaiting` is nil, sent by a phone whose
    /// runner cannot answer `changes.inbox`. It must draw as the complication
    /// always did — no review line, and never a "0 to review".
    private enum PreviewFleet {
        static let now = Date()

        static func agent(
            _ id: String, _ status: String, _ headline: String, _ line: String,
            _ glyph: String, rank: UInt32
        ) -> FleetSnapshot.Agent {
            FleetSnapshot.Agent(
                id: id, label: "claude", machine: "orchard", status: status,
                glyph: glyph, headline: headline, line: line, feed: [],
                rank: rank, turnFailed: false, activityChangedAt: now)
        }

        static let blocked: [FleetSnapshot.Agent] = [
            agent("t1", "blocked", "claude asks", "Run the migration?", "▲", rank: 10),
            agent("t2", "blocked", "codex asks", "Overwrite fruit.txt?", "▲", rank: 20),
            agent("t3", "working", "claude 4m", "Writing review_ops.rs", "●", rank: 300),
            agent("t4", "working", "claude 1m", "Reading watch.rs", "●", rank: 310),
        ]

        static let quiet: [FleetSnapshot.Agent] = [
            agent("t3", "working", "claude 4m", "Writing review_ops.rs", "●", rank: 300),
            agent("t4", "working", "claude 1m", "Reading watch.rs", "●", rank: 310),
            agent("t5", "working", "codex 12m", "Running cargo test", "●", rank: 320),
            agent("t6", "working", "claude 2m", "Reading WatchFleetWidget.swift", "●", rank: 330),
        ]

        static func snapshot(_ agents: [FleetSnapshot.Agent], reviews: Int?) -> FleetSnapshot {
            FleetSnapshot(
                agents: agents, capturedAt: now, complete: true, reviewsWaiting: reviews)
        }

        /// The four entries, one per state, a minute apart so the canvas can
        /// be stepped through them. Named separately because the `timeline:`
        /// builder takes entries and not an array of them.
        static let blockedState = WatchFleetEntry(date: now, snapshot: snapshot(blocked, reviews: 3))
        static let reviewsOnlyState = WatchFleetEntry(
            date: now.addingTimeInterval(60), snapshot: snapshot(quiet, reviews: 3))
        static let allClearState = WatchFleetEntry(
            date: now.addingTimeInterval(120), snapshot: snapshot(quiet, reviews: 0))
        static let reviewsUnknownState = WatchFleetEntry(
            date: now.addingTimeInterval(180), snapshot: snapshot(quiet, reviews: nil))
    }

    #Preview("Circular · blocked, reviews, clear, unknown", as: .accessoryCircular) {
        WatchFleetWidget()
    } timeline: {
        PreviewFleet.blockedState
        PreviewFleet.reviewsOnlyState
        PreviewFleet.allClearState
        PreviewFleet.reviewsUnknownState
    }

    #Preview("Rectangular · blocked, reviews, clear, unknown", as: .accessoryRectangular) {
        WatchFleetWidget()
    } timeline: {
        PreviewFleet.blockedState
        PreviewFleet.reviewsOnlyState
        PreviewFleet.allClearState
        PreviewFleet.reviewsUnknownState
    }

    #Preview("Inline · blocked, reviews, clear, unknown", as: .accessoryInline) {
        WatchFleetWidget()
    } timeline: {
        PreviewFleet.blockedState
        PreviewFleet.reviewsOnlyState
        PreviewFleet.allClearState
        PreviewFleet.reviewsUnknownState
    }
#endif
