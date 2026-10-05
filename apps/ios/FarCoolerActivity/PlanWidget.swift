import SwiftUI
import WidgetKit

/// The plan, on a home screen and under a lock screen clock (ov-310, ov-268
/// P8): the board with a plan, what needs you on it, the lanes in Now and the
/// one next up.
///
/// The relay decided which board and what it says (`leadPlan`, from the
/// runner's own glance), and this asks it with the phone's pulse token, the
/// way `FleetWidget` asks which runners are beating. It never reaches a runner
/// and never re-derives anything: `PlanGlanceView` draws the answer.
///
/// Its own widget beside `FleetWidget` rather than more lines in it: the fleet
/// widget's families are already budgeted line by line, and someone who wants
/// the plan on a lock screen picks it on its own.
struct PlanWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "PlanWidget", provider: PlanProvider()) { entry in
            PlanWidgetView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Plan")
        .description("What needs you on your plan, what’s running now, and what’s next.")
        .supportedFamilies([.systemSmall, .systemMedium, .accessoryRectangular, .accessoryInline])
    }
}

struct PlanEntry: TimelineEntry {
    let date: Date
    /// The board the relay named, or nil when it named none: no plan on any
    /// runner, no answer, or no credential to ask with.
    let glance: PlanGlance?
}

struct PlanProvider: TimelineProvider {
    func placeholder(in context: Context) -> PlanEntry {
        PlanEntry(date: Date(), glance: PlanWidgetSamples.main)
    }

    func getSnapshot(in context: Context, completion: @escaping (PlanEntry) -> Void) {
        if context.isPreview { return completion(placeholder(in: context)) }
        Task { completion(PlanEntry(date: Date(), glance: await RunnerPulse.read(PulseStore.read()).glance)) }
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<PlanEntry>) -> Void) {
        Task {
            let now = Date()
            let reading = await RunnerPulse.read(PulseStore.read())
            let policy: TimelineReloadPolicy =
                reading.nextPlanLook(at: now, every: RunnerPulse.lookEvery).map { .after($0) } ?? .never
            completion(Timeline(entries: [PlanEntry(date: now, glance: reading.glance)], policy: policy))
        }
    }
}

struct PlanWidgetView: View {
    @Environment(\.widgetFamily) private var family
    let entry: PlanEntry

    var body: some View {
        if let glance = entry.glance {
            switch family {
            case .accessoryInline:
                Text([glance.heading, glance.nextLine].compactMap { $0 }.joined(separator: " · "))
            case .accessoryRectangular:
                PlanGlanceView(glance, style: .lines)
            default:
                VStack(alignment: .leading, spacing: 0) {
                    PlanGlanceView(glance, style: .rows)
                    Spacer(minLength: 0)
                }
            }
        } else {
            Text(PlanWidgetSamples.none)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }
}

/// What the widget says with no board, and the board its previews draw.
enum PlanWidgetSamples {
    static let none = "No board has a plan yet."

    static let main = PlanGlance(
        workspace: "Main", needsYou: 2,
        now: [.init(name: "mac-ux", state: .review), .init(name: "ov-310", state: .building)],
        next: "mac-fu3")

    static let quiet = PlanGlance(workspace: "Billing", needsYou: 0, now: [], next: "bil-export")
}

#if DEBUG
    #Preview("Small", as: .systemSmall) {
        PlanWidget()
    } timeline: {
        PlanEntry(date: .now, glance: PlanWidgetSamples.main)
        PlanEntry(date: .now, glance: PlanWidgetSamples.quiet)
        PlanEntry(date: .now, glance: nil)
    }

    #Preview("Rectangular", as: .accessoryRectangular) {
        PlanWidget()
    } timeline: {
        PlanEntry(date: .now, glance: PlanWidgetSamples.main)
        PlanEntry(date: .now, glance: nil)
    }
#endif
