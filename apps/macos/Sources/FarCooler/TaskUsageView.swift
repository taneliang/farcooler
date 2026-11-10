import AgentKit
import SwiftUI

/// A task's Usage section in its Overview (ov-195): what its agents spent,
/// read from the runner when the task opens.
///
/// Quiet on purpose: secondary text and no color, since nothing here needs
/// you. The lines are the client core's (`TaskUsage.said`), the same the
/// phones and `farcooler task usage` say; the breakdown by harness and model is the app's
/// one disclosure, `CollapsibleSection`, closed until opened and remembered.
struct TaskUsageView: View {
    let state: TaskUsageState
    /// Try Again, after a read that didn't come back.
    var onRetry: () -> Void = {}

    var body: some View {
        switch state {
        case .needsUpdate:
            section {
                Text(TaskUsageFormat.needsUpdate)
                    .font(TaskTypography.body)
                    .foregroundStyle(.quietInk)
                    .accessibilityIdentifier("task-usage-needs-update")
            }
        case .failed:
            section {
                HStack(alignment: .firstTextBaseline, spacing: ColumnGrid.rhythm) {
                    Text(TaskUsageFormat.couldntRead)
                        .font(TaskTypography.body)
                        .foregroundStyle(.quietInk)
                    Button(TaskUsageFormat.tryAgain, action: onRetry)
                        .controlSize(.small)
                        .accessibilityIdentifier("task-usage-retry")
                }
            }
        case .loading:
            section {
                ProgressView()
                    .controlSize(.small)
                    .accessibilityLabel("Loading usage")
            }
        case .loaded(let usage):
            section {
                if let nothing = usage.said.nothing {
                    Text(nothing)
                        .font(TaskTypography.body)
                        .foregroundStyle(.quietInk)
                        .accessibilityIdentifier("task-usage-empty")
                } else {
                    totals(usage.said)
                    if !usage.said.rows.isEmpty {
                        breakdown(usage.said.rows)
                    }
                }
            }
        }
    }

    private func section<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: TaskTypography.labelGap) {
            Text("Usage")
                .font(TaskTypography.label)
                .foregroundStyle(.quietInk)
                .accessibilityAddTraits(.isHeader)
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityIdentifier("task-usage")
    }

    private func totals(_ t: WireUsageSaid) -> some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm / 2) {
            Text(t.tokens)
                .font(TaskTypography.body)
            if let detail = t.tokenDetail {
                Text(detail)
                    .font(TaskTypography.meta)
                    .foregroundStyle(.quietInk)
            }
            Text(t.cost)
                .font(TaskTypography.meta)
                .foregroundStyle(.quietInk)
                .help(TaskUsageFormat.apiEquivalent)
            if let time = t.time {
                Text(time)
                    .font(TaskTypography.meta)
                    .foregroundStyle(.quietInk)
            }
        }
        .textSelection(.enabled)
        .accessibilityElement(children: .combine)
    }

    private func breakdown(_ rows: [WireUsageRowSaid]) -> some View {
        CollapsibleSection(
            "By Harness and Model", id: "task-usage-breakdown", style: .minor, metrics: .inline, tone: .quiet,
            key: "task.usage.breakdown.open", expandedByDefault: false
        ) {
            Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 2 * ColumnGrid.rhythm,
                verticalSpacing: ColumnGrid.rhythm / 2) {
                ForEach(rows) { row in
                    GridRow {
                        Text(row.title).accessibilityValue(row.model)
                        Text(row.detail)
                            .foregroundStyle(.quietInk)
                    }
                    .font(TaskTypography.meta)
                    .accessibilityElement(children: .combine)
                }
            }
            .textSelection(.enabled)
        }
    }
}
