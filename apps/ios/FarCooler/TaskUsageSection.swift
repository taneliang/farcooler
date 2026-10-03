import SwiftUI

// A task's Usage section (ov-195): what its agents spent, read from the
// runner when the task opens. Quiet on purpose, all secondary text and no
// color, since nothing here needs you. The words are `TaskUsageFormat`'s,
// the same the Mac, Android and `farcooler report` say.

struct TaskUsageSection: View {
    let state: TaskUsageState
    /// Try Again, after a read that didn't come back.
    var onRetry: () -> Void = {}

    var body: some View {
        switch state {
        case .needsUpdate:
            Section("Usage") {
                quiet(TaskUsageFormat.needsUpdate)
                    .accessibilityIdentifier("task-usage-needs-update")
            }
        case .failed:
            Section("Usage") {
                quiet(TaskUsageFormat.couldntRead)
                Button(TaskUsageFormat.tryAgain, action: onRetry)
                    .accessibilityIdentifier("task-usage-retry")
            }
        case .loading:
            Section("Usage") {
                ProgressView()
                    .controlSize(.small)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityLabel("Loading usage")
            }
        case .loaded(let usage):
            Section("Usage") {
                if usage.totals.isEmpty {
                    quiet(TaskUsageFormat.nothingYet)
                        .accessibilityIdentifier("task-usage-empty")
                } else {
                    totals(usage.totals)
                    if !usage.byHarnessModel.isEmpty {
                        DisclosureGroup("By Harness and Model") {
                            ForEach(usage.rows) { row in
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(TaskUsageFormat.title(row))
                                        .font(.subheadline)
                                    Text(TaskUsageFormat.detail(row))
                                        .font(.footnote)
                                        .foregroundStyle(.secondary)
                                }
                                .accessibilityElement(children: .combine)
                            }
                        }
                        .font(.subheadline)
                        .accessibilityIdentifier("task-usage-breakdown")
                    }
                }
            }
        }
    }

    private func quiet(_ text: String) -> some View {
        Text(text)
            .font(.subheadline)
            .foregroundStyle(.secondary)
    }

    private func totals(_ t: TaskSpend) -> some View {
        VStack(alignment: .leading, spacing: PaneMetrics.tight) {
            Text(TaskUsageFormat.tokensLine(t))
                .font(.body)
            if let detail = TaskUsageFormat.tokenDetail(t) {
                Text(detail)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            Text(TaskUsageFormat.cost(t))
                .font(.footnote)
                .foregroundStyle(.secondary)
            if TaskUsageFormat.isPriced(t) {
                Text(TaskUsageFormat.apiEquivalent)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
            if let time = TaskUsageFormat.time(t) {
                Text(time)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("task-usage")
    }
}
