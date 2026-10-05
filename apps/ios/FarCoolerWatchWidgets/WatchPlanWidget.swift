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
    /// The board the relay named, or nil for none.
    let glance: PlanGlance?
}

struct WatchPlanProvider: TimelineProvider {
    /// An hour between asks, the fleet complication's pace: this budget is
    /// tighter than the phone's, and the watch app's reloads spend it too.
    private static let lookEvery: TimeInterval = 60 * 60

    func placeholder(in context: Context) -> WatchPlanEntry {
        WatchPlanEntry(date: Date(), glance: WatchPlanSamples.main)
    }

    func getSnapshot(in context: Context, completion: @escaping (WatchPlanEntry) -> Void) {
        if context.isPreview { return completion(placeholder(in: context)) }
        Task { completion(WatchPlanEntry(date: Date(), glance: await Self.read().glance)) }
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<WatchPlanEntry>) -> Void) {
        Task {
            let now = Date()
            let reading = await Self.read()
            let policy: TimelineReloadPolicy =
                reading.nextPlanLook(at: now, every: Self.lookEvery).map { .after($0) } ?? .never
            completion(Timeline(entries: [WatchPlanEntry(date: now, glance: reading.glance)], policy: policy))
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
        if let glance = entry.glance {
            if family == .accessoryInline {
                Text([glance.heading, glance.nextLine].compactMap { $0 }.joined(separator: " · "))
            } else {
                PlanGlanceView(glance, style: .lines)
            }
        } else {
            Text(WatchPlanSamples.none)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}

enum WatchPlanSamples {
    static let none = "No board has a plan yet."

    static let main = PlanGlance(
        workspace: "Main", needsYou: 2,
        now: [.init(name: "mac-ux", state: .review), .init(name: "ov-310", state: .building)],
        next: "mac-fu3")
}

#if DEBUG
    #Preview("Rectangular", as: .accessoryRectangular) {
        WatchPlanWidget()
    } timeline: {
        WatchPlanEntry(date: .now, glance: WatchPlanSamples.main)
        WatchPlanEntry(date: .now, glance: nil)
    }
#endif
