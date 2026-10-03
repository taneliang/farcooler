import SwiftUI

// A task's Usage section (ov-195): what its agents spent, read from the
// runner when the task opens. Quiet on purpose, all secondary text and no
// color, since nothing here needs you. The words are `TaskUsageFormat`'s,
// the same the Mac, Android and `farcooler report` say.

struct TaskUsageSection: View {
    let state: TaskUsageState

    var body: some View {
        switch state {
        case .unavailable:
            // A runner older than spend, or a read that didn't come back:
            // nothing to say, so no section.
            EmptyView()
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
                    Text(TaskUsageFormat.nothingYet)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
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
