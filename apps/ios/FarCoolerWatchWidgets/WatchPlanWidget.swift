import SwiftUI
import WidgetKit

/// The plan, in the Smart Stack and on a face (ov-310, ov-268 P8): the board
/// with a plan, what needs you on it, the lanes in Now and the one next up.
///
/// The watch has no sockets, so this hears the plan from the relay and from
/// nowhere else: `/v1/pulse` over HTTPS, with the credential the watch app
/// filed from the phone's context, as `WatchFleetWidget` asks which runners
/// are beating. The relay decided the board (`leadPlan`); `PlanGlanceView`
/// draws it, the same view the phone's widget and the Live Activity use.
struct WatchPlanWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "WatchPlanWidget", provider: WatchPlanProvider()) { entry in
            WatchPlanView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Plan")
        .description("What needs you on your plan, what’s running now, and what’s next.")
        .supportedFamilies([.accessoryRectangular, .accessoryInline])
    }
}

struct WatchPlanEntry: TimelineEntry {
    let date: Date
    /// What to show. See `PlanGlanceMemory.shown`.
    let shown: PlanGlanceShown
}

struct WatchPlanProvider: TimelineProvider {
    /// An hour between asks, the fleet complication's pace: this budget is
    /// tighter than the phone's, and the watch app's reloads spend it too.
    private static let lookEvery: TimeInterval = 60 * 60

    func placeholder(in context: Context) -> WatchPlanEntry {
        WatchPlanEntry(date: Date(), shown: .plan(WatchPlanSamples.main, caveat: nil))
    }

    func getSnapshot(in context: Context, completion: @escaping (WatchPlanEntry) -> Void) {
        if context.isPreview { return completion(placeholder(in: context)) }
        Task { completion(WatchPlanEntry(date: Date(), shown: PlanGlanceMemory.look(await Self.read()))) }
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<WatchPlanEntry>) -> Void) {
        Task {
            let now = Date()
            let reading = await Self.read()
            let policy: TimelineReloadPolicy =
                reading.nextPlanLook(at: now, every: Self.lookEvery).map { .after($0) } ?? .never
            completion(Timeline(entries: [WatchPlanEntry(date: now, shown: PlanGlanceMemory.look(reading, at: now))], policy: policy))
        }
    }

    /// A credential the watch holds but can't read while locked is a look
    /// that failed, so it looks again rather than parking.
    private static func read() async -> RunnerPulse.Reading {
        let vault = PulseStore.vault
        return await RunnerPulse.read(vault.flatMap(PulseStore.read(from:)), held: vault?.holds ?? false)
    }
}

struct WatchPlanView: View {
    @Environment(\.widgetFamily) private var family
    let entry: WatchPlanEntry

    var body: some View {
        switch entry.shown {
        case let .plan(glance, caveat):
            if family == .accessoryInline {
                Text([caveat?.line, glance.heading, glance.next.map { "Next: \($0)" }].compactMap { $0 }.joined(separator: " · "))
            } else {
                PlanGlanceView(glance, style: .lines, caveat: caveat)
            }
        default:
            Text(entry.shown.message ?? "")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}

enum WatchPlanSamples {
    static let main = PlanGlance(
        workspace: "Main", needsYou: 2,
        now: [.init(name: "mac-ux", state: .review), .init(name: "ov-310", state: .building)],
        next: "mac-fu3")
}

#if DEBUG
    #Preview("Rectangular", as: .accessoryRectangular) {
        WatchPlanWidget()
    } timeline: {
        WatchPlanEntry(date: .now, shown: .plan(WatchPlanSamples.main, caveat: nil))
        WatchPlanEntry(date: .now, shown: .plan(WatchPlanSamples.main, caveat: PlanCaveat(age: 3 * 3600, cantReach: "Studio")))
        WatchPlanEntry(date: .now, shown: .noPlan)
    }
#endif
