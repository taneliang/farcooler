import AgentKit
import SwiftUI

/// A task's Usage section in its Overview (ov-195): what its agents spent,
/// read from the runner when the task opens.
///
/// Quiet on purpose: secondary text and no color, since nothing here needs
/// you. The words are AgentKit's `TaskUsageFormat`, the same the phones and
/// `farcooler report` say; the breakdown by harness and model is the app's
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
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("task-usage-needs-update")
            }
        case .failed:
            section {
                HStack(alignment: .firstTextBaseline, spacing: ColumnGrid.rhythm) {
                    Text(TaskUsageFormat.couldntRead)
                        .font(TaskTypography.body)
                        .foregroundStyle(.secondary)
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
                if usage.totals.isEmpty {
                    Text(TaskUsageFormat.nothingYet)
                        .font(TaskTypography.body)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("task-usage-empty")
                } else {
                    totals(usage.totals)
                    if !usage.byHarnessModel.isEmpty {
                        breakdown(usage.rows)
                    }
                }
            }
        }
    }

    private func section<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: TaskTypography.labelGap) {
            Text("Usage")
                .font(TaskTypography.label)
                .foregroundStyle(.secondary)
                .accessibilityAddTraits(.isHeader)
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityIdentifier("task-usage")
    }

    private func totals(_ t: TaskSpend) -> some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm / 2) {
            Text(TaskUsageFormat.tokensLine(t))
                .font(TaskTypography.body)
            if let detail = TaskUsageFormat.tokenDetail(t) {
                Text(detail)
                    .font(TaskTypography.meta)
                    .foregroundStyle(.secondary)
            }
            Text(TaskUsageFormat.cost(t))
                .font(TaskTypography.meta)
                .foregroundStyle(.secondary)
                .help(TaskUsageFormat.apiEquivalent)
            if let time = TaskUsageFormat.time(t) {
                Text(time)
                    .font(TaskTypography.meta)
                    .foregroundStyle(.secondary)
            }
        }
        .textSelection(.enabled)
        .accessibilityElement(children: .combine)
    }

    private func breakdown(_ rows: [TaskSpendRow]) -> some View {
        CollapsibleSection(
            "By Harness and Model", id: "task-usage-breakdown", style: .minor, metrics: .inline, tone: .quiet,
            key: "task.usage.breakdown.open", expandedByDefault: false
        ) {
            Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 2 * ColumnGrid.rhythm,
                verticalSpacing: ColumnGrid.rhythm / 2) {
                ForEach(rows) { row in
                    GridRow {
                        Text(TaskUsageFormat.title(row))
                        Text(TaskUsageFormat.detail(row))
                            .foregroundStyle(.secondary)
                    }
                    .font(TaskTypography.meta)
                    .accessibilityElement(children: .combine)
                }
            }
            .textSelection(.enabled)
        }
    }
}
