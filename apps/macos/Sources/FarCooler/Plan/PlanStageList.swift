import AgentKit
import SwiftUI

// A pipeline stage's tickets nobody is on (ov-874): what the plan column's
// "N more waiting for review" line opens, in the plan's navigation stack as a
// theme's page is. Each row opens its ticket's task, as the column's do.

struct PlanStageListPage: View {
    @ObservedObject var plan: PlanStore
    /// A `PlanColumn.Stage` raw value.
    let stage: String
    let context: PlanPageContext

    /// The stage's tickets with no agent on them, as the column reads them.
    private var tickets: [PlanColumn.Ticket] {
        let rows = context.rows
        let column = PlanColumn(
            plan: plan.plan, statuses: rows.mapValues(\.status), titles: rows.mapValues(\.title))
        return column.pipeline.first { $0.stage.rawValue == stage }?.unattended ?? []
    }

    var body: some View {
        let found = tickets
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                PlanColumnHeading(
                    title: PlanColumn.Stage(rawValue: stage)?.title ?? "Stage", count: "\(found.count)", prominent: true, top: 0)
                if found.isEmpty {
                    Text("Nothing here now.")
                        .font(.callout)
                        .foregroundStyle(.quietInk)
                        .identified("plan-stage-list-empty")
                } else {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(found) { ticket in
                            PlanTicketRow(ticket: ticket) {
                                if let row = context.rows[ticket.task] { context.onTask(row) } else { context.onOpen(.ticket(ticket.task)) }
                            }
                        }
                    }
                    .padding(.vertical, Spacing.tight)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .surface(.content, in: .card, fill: PlanColumnMetrics.cardFill)
                }
            }
            .frame(maxWidth: PlanMetrics.widest, alignment: .leading)
            .padding(PlanColumnMetrics.gutter)
            .frame(maxWidth: .infinity, alignment: .topLeading)
        }
        .contentCard()
        .task { await plan.readIfNeverRead() }
        .onAppear { plan.attach() }
        .onDisappear { plan.detach() }
        .identified("plan-stage-list")
    }
}
