import SwiftUI

/// A worktree on disk that Far Cooler does not know about yet.
struct ExistingWorktree: Decodable, Identifiable, Hashable {
    var path: String
    var branch: String?
    var head: String?
    var name: String
    var locked: Bool?

    var id: String { path }

    var branchLabel: String { branch ?? "detached" }
    var isLocked: Bool { locked ?? false }

    /// The path with the user's home collapsed, because the full one is usually
    /// three quarters `/Users/…` and the tail is the part that identifies it.
    var shortPath: String {
        let home = NSHomeDirectory()
        return path.hasPrefix(home) ? "~" + path.dropFirst(home.count) : path
    }
}

struct WorktreeList: Decodable {
    var worktrees: [ExistingWorktree]
}

/// Bring worktrees you already have into Far Cooler.
///
/// The onboarding path, and the reason it matters: Far Cooler only ever knew
/// about work it started itself, so someone arriving with a repository they have
/// used for months spent their first hour re-creating by hand what was already
/// on disk — using a tool whose whole point is not doing that.
///
/// Everything is selected when the sheet opens. Someone who asked to import
/// their worktrees means all of them far more often than some of them, and
/// unticking two is less work than ticking six.
struct ImportWorktrees: View {
    let projects: [Repository]
    @Binding var project: String
    let load: (String) async -> [ExistingWorktree]
    /// Returns how many were imported.
    let onImport: ([ExistingWorktree], String) async -> Int

    @Environment(\.dismiss) private var dismiss
    @State private var found: [ExistingWorktree] = []
    @State private var chosen: Set<String> = []
    @State private var loading = true
    @State private var importing = false
    @State private var imported: Int?

    private var canImport: Bool { !chosen.isEmpty && !importing }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            body(for: found)
            Divider()
            footer
        }
        .frame(width: 580, height: 440)
        .task(id: project) { await reload() }
    }

    @ViewBuilder
    private func body(for worktrees: [ExistingWorktree]) -> some View {
        if loading {
            centered { ProgressView().controlSize(.small) }
        } else if worktrees.isEmpty {
            centered {
                VStack(spacing: 6) {
                    Text(imported == nil ? "Nothing to import" : "All imported")
                        .font(.callout.weight(.medium))
                    Text(
                        imported == nil
                            ? "Every worktree for this project is already here."
                            : "They are in the sidebar now."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(worktrees) { worktree in
                        row(worktree)
                    }
                }
                .padding(.vertical, 4)
            }
        }
    }

    private func centered<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack { Spacer(); content(); Spacer() }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Import existing worktrees").font(.headline)
                    Text("Nothing is created, moved, or checked out.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if projects.count > 1 {
                    Picker("", selection: $project) {
                        ForEach(projects) { Text($0.displayName).tag($0.id) }
                    }
                    .labelsHidden().fixedSize().controlSize(.small)
                }
            }
        }
        .padding(14)
    }

    private func row(_ worktree: ExistingWorktree) -> some View {
        let picked = chosen.contains(worktree.id)
        return HStack(spacing: 10) {
            Image(systemName: picked ? "checkmark.circle.fill" : "circle")
                .font(.system(size: 13))
                .foregroundStyle(picked ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))

            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 6) {
                    Text(worktree.name).font(.system(size: 13))
                    Text(worktree.branchLabel)
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.secondary)
                    if worktree.isLocked {
                        // git holds a lock, usually removable media. Importable,
                        // and worth saying before rather than failing after.
                        Text("locked")
                            .font(.system(size: 10))
                            .foregroundStyle(.tertiary)
                    }
                }
                Text(worktree.shortPath)
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 6)
        .contentShape(Rectangle())
        .onTapGesture {
            if picked { chosen.remove(worktree.id) } else { chosen.insert(worktree.id) }
        }
    }

    private var footer: some View {
        HStack(spacing: 10) {
            if let imported {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                Text("Imported \(imported)").foregroundStyle(.secondary)
            } else if !found.isEmpty {
                Button(chosen.count == found.count ? "Select none" : "Select all") {
                    chosen = chosen.count == found.count ? [] : Set(found.map(\.id))
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
            }

            Spacer()

            Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
            Button(importing ? "Importing…" : "Import \(chosen.count)") {
                Task { await runImport() }
            }
            .keyboardShortcut(.defaultAction)
            .disabled(!canImport)
        }
        .font(.system(size: 12))
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    private func reload() async {
        loading = true
        found = await load(project)
        // Preselected: someone who opened this meant all of them far more often
        // than some of them.
        chosen = Set(found.map(\.id))
        loading = false
    }

    private func runImport() async {
        importing = true
        let picked = found.filter { chosen.contains($0.id) }
        let count = await onImport(picked, project)
        importing = false
        imported = count
        // Re-read rather than removing what we sent: one of them may have failed,
        // and the honest list is the one the host still reports as unregistered.
        await reload()
    }
}
