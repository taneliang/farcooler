import SwiftUI

struct BranchInfo: Decodable, Identifiable, Hashable {
    var name: String
    var local: Bool?
    var remote: String?
    var checkedOut: Bool?
    var subject: String?
    var updatedAt: Double?

    var id: String { name }

    var isLocal: Bool { local ?? false }
    var isCheckedOut: Bool { checkedOut ?? false }

    /// Where it lives, in the terms that matter when picking one.
    var origin: String {
        switch (isLocal, remote) {
        case (true, .some(let r)): return "local · \(r)"
        case (true, .none): return "local only"
        case (false, .some(let r)): return "on \(r)"
        case (false, .none): return "unknown"
        }
    }

    var age: String {
        guard let updatedAt, updatedAt > 0 else { return "" }
        let seconds = Date().timeIntervalSince1970 - updatedAt
        if seconds < 3600 { return "\(max(1, Int(seconds / 60)))m" }
        if seconds < 86_400 { return "\(Int(seconds / 3600))h" }
        return "\(Int(seconds / 86_400))d"
    }
}

struct BranchList: Decodable {
    var branches: [BranchInfo]
}

/// Pick up work that already exists.
///
/// The other half of starting a task. Work arrives on a branch at least as often
/// as it starts on one — pushed from a laptop, handed over by a colleague, or
/// produced by an agent running somewhere else entirely. Without this, picking it
/// up meant doing the git by hand outside Far Cooler, and then Far Cooler not
/// knowing the worktree existed.
///
/// A branch that exists only on a remote is the interesting case, and it is
/// labeled as one: it has no local ref here yet, so adopting it has to create a
/// tracking branch, which is what makes pushing back go where it came from.
struct ResumeBranch: View {
    /// Every machine's repositories, tagged the same way `FleetStore.repositories`
    /// tags them — see `QuickCreate.projects` and `NewWorkspaceSheet.repositories`
    /// for the same shape and the same reason: this picker is the machine
    /// selector for `resume` now, and a bare `[Repository]` cannot name one.
    let projects: [(host: String, repository: Repository)]
    @Binding var project: String
    /// Branches for a project on a given machine. `host` is handed in
    /// alongside `project` rather than re-derived from it downstream — see
    /// `chosen` below and `onAdopt`'s own doc comment.
    let load: (_ host: String, _ project: String) async -> [BranchInfo]
    /// The branch adopted, the machine and project it came from, and the
    /// chosen preset.
    ///
    /// `host` comes from `chosen`, the same picker selection that resolved
    /// `project` — not re-derived by the caller from `project` alone once
    /// it is stale. `NewWorkspaceSheet.Choice` carries host and repository
    /// together for the identical reason: a lookup repeated downstream, from
    /// a bare id, is exactly the shape that goes silently wrong when the
    /// picker's own list has since moved on.
    let onAdopt: (BranchInfo, String, String, String) -> Void

    @AppStorage("tasks.agent") private var agent = "claude"
    @AppStorage("tasks.model") private var model = ""

    @Environment(\.dismiss) private var dismiss
    @State private var branches: [BranchInfo] = []
    @State private var query = ""
    @State private var loading = true
    @State private var selection: String?

    /// The project `project` names, together with the machine it is on —
    /// same rule `QuickCreate.chosen` follows, and for the same reason: no
    /// fallback to `projects.first`, so a stale `project` (a repository
    /// that was removed, or one on a machine not yet re-read after a
    /// reconnect) resolves to nothing rather than to whichever project
    /// happened to be first.
    private var chosen: (host: String, repository: Repository)? {
        projects.first { $0.repository.id == project }
    }

    /// Whether more than one machine has a repository on offer — same rule
    /// `QuickCreate` and `NewWorkspaceSheet` follow.
    private var multipleHosts: Bool {
        Set(projects.map(\.host)).count > 1
    }

    private func label(for entry: (host: String, repository: Repository)) -> String {
        guard multipleHosts else { return entry.repository.displayName }
        let host = entry.host.isEmpty ? "This Mac" : entry.host
        return "\(entry.repository.displayName) — \(host)"
    }

    private var visible: [BranchInfo] {
        guard !query.isEmpty else { return branches }
        let q = query.lowercased()
        return branches.filter {
            $0.name.lowercased().contains(q)
                || ($0.subject ?? "").lowercased().contains(q)
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            body(for: visible)
            Divider()
            footer
        }
        .frame(width: 580, height: 470)
        .task(id: project) { await reload() }
    }

    @ViewBuilder
    private func body(for branches: [BranchInfo]) -> some View {
        if loading {
            centered { ProgressView().controlSize(.small) }
        } else if branches.isEmpty {
            centered {
                VStack(spacing: 5) {
                    Text(self.branches.isEmpty ? "No branches" : "Nothing matches")
                        .font(.callout.weight(.medium))
                    if !self.branches.isEmpty {
                        Text("“\(query)”").font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        } else {
            List(branches, selection: $selection) { branch in
                row(branch)
                    .tag(branch.id)
                    .contentShape(Rectangle())
                    .onTapGesture(count: 2) { adopt(branch) }
            }
            .listStyle(.inset)
            .onKeyPress(.return) {
                if let branch = branches.first(where: { $0.id == selection }) {
                    adopt(branch)
                    return .handled
                }
                return .ignored
            }
        }
    }

    private func centered<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack { Spacer(); content(); Spacer() }
    }

    private var header: some View {
        VStack(spacing: 9) {
            HStack {
                Text("Resume a branch").font(.headline)
                Spacer()
                if projects.count > 1 {
                    Picker("", selection: $project) {
                        ForEach(projects, id: \.repository.id) { entry in
                            Text(label(for: entry)).tag(entry.repository.id)
                        }
                    }
                    .labelsHidden().fixedSize().controlSize(.small)
                }
            }
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 11)).foregroundStyle(.tertiary)
                TextField("Filter branches", text: $query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 12))
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(0.05)))
        }
        .padding(14)
    }

    private func row(_ branch: BranchInfo) -> some View {
        HStack(spacing: 10) {
            // Remote-only is the handoff case, so it is the one that gets a
            // distinct symbol — it is about to be pulled down, not just opened.
            Image(systemName: branch.isLocal ? "arrow.triangle.branch" : "arrow.down.circle")
                .font(.system(size: 12))
                .foregroundStyle(branch.isLocal ? .tertiary : .secondary)
                .frame(width: 15)

            VStack(alignment: .leading, spacing: 1) {
                Text(branch.name).font(.system(size: 13)).lineLimit(1)
                Text((branch.subject?.isEmpty ?? true) ? branch.origin : branch.subject!)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 8)

            // git refuses a second checkout of the same branch, so this has to
            // be visible before someone picks it, not an error afterwards.
            Text(branch.isCheckedOut ? "checked out" : branch.age)
                .font(.system(size: 10))
                .foregroundStyle(.tertiary)
        }
        .padding(.vertical, 2)
        .opacity(branch.isCheckedOut ? 0.4 : 1)
    }

    private var footer: some View {
        HStack(spacing: 8) {
            Picker("", selection: $agent) {
                ForEach(Agents.all) { Text($0.name).tag($0.id) }
            }
            .labelsHidden().fixedSize().controlSize(.small)
            .onChange(of: agent) { _, _ in model = "" }

            Picker("", selection: $model) {
                Text("Default model").tag("")
                ForEach(Agents.agent(agent).models, id: \.self) { Text($0).tag($0) }
            }
            .labelsHidden().fixedSize().controlSize(.small)

            Spacer()
            Text("↩ resume").font(.system(size: 11)).foregroundStyle(.tertiary)
            Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
    }

    private func reload() async {
        loading = true
        // No fallback host, matching `chosen`'s own doc comment: a `project`
        // that names nothing any machine currently has loads no branches
        // rather than guessing which machine to ask.
        if let chosen {
            branches = await load(chosen.host, project)
        } else {
            branches = []
        }
        loading = false
    }

    private func adopt(_ branch: BranchInfo) {
        guard !branch.isCheckedOut, let chosen else { return }
        onAdopt(branch, chosen.host, project, Agents.preset(agent: agent, model: model))
        dismiss()
    }
}
