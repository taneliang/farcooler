import SwiftUI

// Resuming onto work that already exists, and seeing where a branch sits.
//
// Both were desktop-only by accident rather than by design: `branch.list`,
// `stack.get` and `pr.refresh` have been in the protocol since the review
// surface landed, and the phone reached the daemon through an FFI that never
// exposed them. Neither needs a big screen — a branch list is a list, and a
// stack is a short chain — so neither had a reason to stay on the Mac.

/// Pick a branch to take over, rather than typing its name exactly.
///
/// Adoption, not creation: the worktree is named after the branch, so there is
/// nothing else to fill in — which is why this is a list and not a form.
struct BranchPicker: View {
    let repository: String
    let connection: Connection
    let onPick: (Branch) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var branches: [Branch] = []
    @State private var loading = true
    @State private var search = ""

    private var shown: [Branch] {
        let query = search.trimmingCharacters(in: .whitespaces).lowercased()
        guard !query.isEmpty else { return branches }
        return branches.filter { $0.name.lowercased().contains(query) }
    }

    var body: some View {
        NavigationStack {
            List {
                if loading {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("Reading branches…").foregroundStyle(.secondary)
                    }
                } else if branches.isEmpty {
                    Text("This repository has no other branches.")
                        .foregroundStyle(.secondary)
                }

                ForEach(shown) { branch in
                    Button {
                        onPick(branch)
                        dismiss()
                    } label: {
                        row(branch)
                    }
                    // git refuses a second checkout of a branch, so the row is
                    // disabled rather than left tappable to fail afterwards.
                    .disabled(branch.checkedOut)
                }
            }
            .searchable(text: $search, prompt: "Filter branches")
            .navigationTitle("Resume a Branch")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
            .task {
                branches = await connection.branches(repository: repository)
                loading = false
            }
        }
    }

    private func row(_ branch: Branch) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(branch.name)
                    .font(.subheadline.monospaced())
                    .foregroundStyle(branch.checkedOut ? .secondary : .primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                if branch.isRemoteOnly {
                    Text("remote")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            if branch.checkedOut {
                // Said plainly, because "why can't I tap this" is otherwise the
                // only question the row raises.
                Text("Already checked out in another worktree")
                    .font(.caption2)
                    .foregroundStyle(.orange)
            } else if !branch.subject.isEmpty {
                Text(branch.subject)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }
}

/// Where a branch sits, and what GitHub says about it.
///
/// Read-only on purpose. Setting a parent is a decision that changes what every
/// diff in the stack compares against, and this screen exists to answer "what
/// is the state of this" — the question you ask from a phone.
struct StackView: View {
    let repository: String
    let branch: String
    let connection: Connection

    @Environment(\.dismiss) private var dismiss
    @State private var response: StackResponse?
    @State private var loading = true
    @State private var refreshing = false

    var body: some View {
        NavigationStack {
            List {
                if loading {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("Reading…").foregroundStyle(.secondary)
                    }
                } else if let response, response.links.isEmpty {
                    Text("This branch isn't part of a stack.")
                        .foregroundStyle(.secondary)
                } else if response == nil {
                    Text("This machine's Far Cooler is too old to answer.")
                        .foregroundStyle(.secondary)
                }

                if response?.cycleDetected == true {
                    // Reported rather than followed. A parent chain that loops
                    // is walked as far as it was walked, and drawing it as a
                    // clean stack would draw one that does not exist.
                    Label(
                        "These branches list each other as parents. Showing what was walked.",
                        systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(.orange)
                }

                ForEach(response?.links ?? []) { link in
                    Section {
                        LabeledContent("Parent") {
                            HStack(spacing: 6) {
                                Text(link.parentBranch.isEmpty ? "—" : link.parentBranch)
                                    .font(.footnote.monospaced())
                                if link.parentGuessed {
                                    Text("guessed")
                                        .font(.caption2)
                                        .foregroundStyle(.orange)
                                }
                            }
                        }
                        LabeledContent("Ahead / behind", value: "\(link.ahead) / \(link.behind)")
                        if let pr = link.pr {
                            pullRequest(pr)
                        }
                    } header: {
                        Text(link.branch).font(.footnote.monospaced())
                    }
                }
            }
            .navigationTitle("Stack")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        Task {
                            refreshing = true
                            // Asks GitHub again rather than answering from what
                            // was last read — the affordance that exists because
                            // a cached "passing" is the reading that misleads.
                            response = await connection.refreshPullRequests(
                                repository: repository) ?? response
                            refreshing = false
                        }
                    } label: {
                        if refreshing {
                            ProgressView()
                        } else {
                            Image(systemName: "arrow.clockwise")
                        }
                    }
                    .disabled(refreshing)
                    .accessibilityLabel("Refresh from GitHub")
                }
            }
            .task {
                response = await connection.stack(repository: repository, branch: branch)
                loading = false
            }
        }
    }

    @ViewBuilder
    private func pullRequest(_ pr: PullRequest) -> some View {
        LabeledContent("Pull request") {
            HStack(spacing: 6) {
                Text("#\(pr.number)")
                Text(pr.state.capitalized)
                    .font(.caption)
                    .foregroundStyle(Self.tint(forState: pr.state))
            }
        }
        LabeledContent("Checks") {
            Text(pr.checks.capitalized)
                .foregroundStyle(Self.tint(forChecks: pr.checks))
        }
        if pr.review != "unknown" {
            LabeledContent(
                "Review", value: pr.review.replacingOccurrences(of: "_", with: " ").capitalized)
        }
        if pr.stale {
            Text("Last read from GitHub a while ago.")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        if let url = URL(string: pr.url) {
            Link("Open on GitHub", destination: url)
        }
    }

    private static func tint(forState state: String) -> Color {
        switch state {
        case "open": return .green
        case "merged": return .purple
        case "draft": return .secondary
        case "closed": return .red
        default: return .secondary
        }
    }

    private static func tint(forChecks checks: String) -> Color {
        switch checks {
        case "passing": return .green
        case "failing": return .red
        case "pending": return .orange
        default: return .secondary
        }
    }
}
