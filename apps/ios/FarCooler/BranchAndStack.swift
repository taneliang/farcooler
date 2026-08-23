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
    /// What the ages in the rows are measured from, fixed when the sheet opens.
    ///
    /// Deliberately NOT a `TimelineView` and not a `Date()` read inside the row.
    /// The clock on a terminal row ticks because the difference between four
    /// seconds and forty is the thing you are watching; a branch last touched
    /// three days ago is not going to become four while a picker is open, and a
    /// timeline redrawing the whole list every second to prove it would cost
    /// more than it tells anyone. `@State` rather than a stored `let`, because a
    /// stored one is re-initialized every time the parent re-evaluates its body.
    @State private var now = Date()

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
                    // A branch name and its subject are content, not an action
                    // — without this they inherit the button's accent tint, and
                    // the hierarchical styles inside the row resolve against it
                    // rather than against the label color. See
                    // `CommitHistorySheet` in `ChangesView`.
                    .buttonStyle(.plain)
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
                // How stale it is, trailing, where a list of branches can be
                // read down. Between two branches you half-remember, this and
                // the name are the whole decision — and the runner has been
                // sending it since this screen existed. Pushed to the trailing
                // edge rather than sitting after the name, so the column of
                // ages lines up instead of starting wherever each name ended.
                //
                // Drawn only when there is one: `age(at:)` is empty for a
                // branch git had no committer date for, and a blank caption
                // still takes the space it would have used.
                let age = branch.age(at: now)
                if !age.isEmpty {
                    Spacer(minLength: 6)
                    Text(age)
                        .font(.caption2.monospacedDigit())
                        .foregroundStyle(.tertiary)
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
        // A plain label sizes to its text. Give the tap the whole visible row,
        // including the blank trailing space beside a short branch name.
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
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
                    Text("This branch isn’t part of a stack.")
                        .foregroundStyle(.secondary)
                } else if response == nil {
                    Text("This runner’s Far Cooler is too old to answer.")
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
