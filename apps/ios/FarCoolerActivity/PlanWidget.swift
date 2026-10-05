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
    /// What to show: a board (with why it isn't current, if it isn't), or the
    /// reason there's none (review H2). See `PlanGlanceMemory.shown`.
    let shown: PlanGlanceShown
}

struct PlanProvider: TimelineProvider {
    func placeholder(in context: Context) -> PlanEntry {
        PlanEntry(date: Date(), shown: .plan(PlanWidgetSamples.main, caveat: nil))
    }

    func getSnapshot(in context: Context, completion: @escaping (PlanEntry) -> Void) {
        if context.isPreview { return completion(placeholder(in: context)) }
        Task { completion(PlanEntry(date: Date(), shown: PlanGlanceMemory.look(await RunnerPulse.read(PulseStore.read())))) }
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<PlanEntry>) -> Void) {
        Task {
            let now = Date()
            let reading = await RunnerPulse.read(PulseStore.read())
            let policy: TimelineReloadPolicy =
                reading.nextPlanLook(at: now, every: RunnerPulse.lookEvery).map { .after($0) } ?? .never
            // The last plan is remembered beside the fleet's snapshot, so a
            // failed ask draws it with its age rather than "no plan".
            completion(Timeline(entries: [PlanEntry(date: now, shown: PlanGlanceMemory.look(reading, at: now))], policy: policy))
        }
    }
}

struct PlanWidgetView: View {
    @Environment(\.widgetFamily) private var family
    let entry: PlanEntry

    var body: some View {
        switch entry.shown {
        case let .plan(glance, caveat):
            switch family {
            case .accessoryInline:
                Text(inline(glance, caveat))
            case .accessoryRectangular:
                PlanGlanceView(glance, style: .lines, caveat: caveat)
            default:
                VStack(alignment: .leading, spacing: 0) {
                    PlanGlanceView(glance, style: .rows, caveat: caveat)
                    Spacer(minLength: 0)
                }
            }
        default:
            Text(entry.shown.message ?? "")
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    /// One line, the age or the quiet runner first when there is one: the
    /// tail is what truncates.
    private func inline(_ glance: PlanGlance, _ caveat: PlanCaveat?) -> String {
        [caveat?.line, glance.heading, glance.next.map { "Next: \($0)" }].compactMap { $0 }.joined(separator: " · ")
    }
}

/// What the widget says with no board, and the board its previews draw.
enum PlanWidgetSamples {
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
        PlanEntry(date: .now, shown: .plan(PlanWidgetSamples.main, caveat: nil))
        PlanEntry(date: .now, shown: .plan(PlanWidgetSamples.main, caveat: PlanCaveat(age: 3 * 3600, cantReach: "Studio")))
        PlanEntry(date: .now, shown: .plan(PlanWidgetSamples.quiet, caveat: PlanCaveat(age: 3 * 3600)))
        PlanEntry(date: .now, shown: .noPlan)
    }

    #Preview("Rectangular", as: .accessoryRectangular) {
        PlanWidget()
    } timeline: {
        PlanEntry(date: .now, shown: .plan(PlanWidgetSamples.main, caveat: nil))
        PlanEntry(date: .now, shown: .unknown)
    }
#endif
