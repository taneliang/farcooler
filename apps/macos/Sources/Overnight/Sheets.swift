import AppKit
import SwiftUI

/// Create a workspace: one worktree plus branch for one task.
struct NewWorkspaceSheet: View {
    let repositories: [Repository]
    /// The repository to open on, when this was reached from a project header.
    /// Empty means "ask", which is what the sidebar's own `+` wants.
    var preselected: String = ""
    let onCreate: (String, String, String, String) async -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var repo: String = ""
    @State private var task = ""
    @State private var branch = ""
    @State private var base = "HEAD"
    @State private var working = false
    @State private var error: String?

    /// Suggest a branch from the task name, the way a person would write it.
    private var suggestedBranch: String {
        let slug = task.lowercased()
            .map { $0.isLetter || $0.isNumber ? $0 : "-" }
            .reduce(into: "") { acc, c in
                if c == "-" && acc.hasSuffix("-") { return }
                acc.append(c)
            }
            .trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        return slug.isEmpty ? "" : "feat/\(slug)"
    }

    private var canCreate: Bool {
        !repo.isEmpty && !task.trimmingCharacters(in: .whitespaces).isEmpty
            && !effectiveBranch.isEmpty && !working
    }

    private var effectiveBranch: String {
        branch.isEmpty ? suggestedBranch : branch
    }

    var body: some View {
        SheetFrame(
            title: "New workspace",
            subtitle: "A worktree and branch for one task.",
            confirmTitle: "Create",
            canConfirm: canCreate,
            working: working,
            error: error,
            onCancel: { dismiss() },
            onConfirm: {
                working = true
                error = nil
                await onCreate(repo, task, effectiveBranch, base)
                working = false
                dismiss()
            }
        ) {
            Form {
                Picker("Repository", selection: $repo) {
                    ForEach(repositories) { r in
                        Text(r.displayName).tag(r.short)
                    }
                }

                TextField("Task", text: $task, prompt: Text("add authentication"))

                TextField(
                    "Branch", text: $branch,
                    prompt: Text(suggestedBranch.isEmpty ? "feat/my-task" : suggestedBranch))

                TextField("Base revision", text: $base, prompt: Text("HEAD"))
            }
            .formStyle(.grouped)
        }
        .onAppear {
            guard repo.isEmpty else { return }
            // The project whose header was clicked, if there was one. Matching
            // on display name because that is what the sidebar groups by.
            repo = repositories.first { $0.displayName == preselected }?.short
                ?? repositories.first?.short ?? ""
        }
    }
}

/// Destructive confirmation.
///
/// Worktree removal is the one action that can lose work that was never
/// committed, so it demands the exact workspace name typed out and says plainly
/// what it will and will not touch.
struct RemoveWorkspaceSheet: View {
    let workspace: Workspace
    let hasRunningTerminals: Bool
    /// Receives the exact text the user typed, which the daemon re-checks.
    let onRemove: (String) async -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var typed = ""
    @State private var working = false

    private var matches: Bool { typed == workspace.task }

    var body: some View {
        SheetFrame(
            title: "Remove worktree",
            subtitle: workspace.task,
            confirmTitle: "Remove worktree",
            confirmRole: .destructive,
            canConfirm: matches && !hasRunningTerminals && !working,
            working: working,
            error: nil,
            onCancel: { dismiss() },
            onConfirm: {
                working = true
                await onRemove(typed)
                working = false
                dismiss()
            }
        ) {
            VStack(alignment: .leading, spacing: 14) {
                if hasRunningTerminals {
                    Callout(
                        icon: "exclamationmark.triangle.fill",
                        tone: .warning,
                        text:
                            "Terminals are still running in this workspace. Stop them first; "
                            + "Overnight will not remove a worktree out from under a live process."
                    )
                } else {
                    Callout(
                        icon: "info.circle.fill",
                        tone: .neutral,
                        text:
                            "This deletes the working directory. The branch is kept, and nothing "
                            + "already committed or pushed is touched."
                    )
                }

                VStack(alignment: .leading, spacing: 6) {
                    Text("Type the workspace name to confirm")
                        .font(.callout)
                    TextField("", text: $typed, prompt: Text(workspace.task))
                        .textFieldStyle(.roundedBorder)
                        .disabled(hasRunningTerminals)
                }
            }
        }
    }
}

/// Confirms cancelling an in-flight turn to switch a pane's mode.
///
/// `DaemonClient.setPaneMode` answers `confirmationRequired` rather than just
/// failing precisely so this can exist: a flat refusal would leave switching
/// to terminal mode mid-turn simply impossible from the keyboard, and forcing
/// it through with no confirmation would cancel an agent's work with the same
/// keystroke that opens the layout menu.
struct PaneModeConfirmSheet: View {
    /// The daemon's own description of what is in flight, so this says
    /// something true about the specific turn rather than a generic warning.
    let message: String
    let onConfirm: () async -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var working = false

    var body: some View {
        SheetFrame(
            title: "Cancel the current turn?",
            confirmTitle: "Cancel Turn and Switch",
            confirmRole: .destructive,
            canConfirm: !working,
            working: working,
            error: nil,
            onCancel: { dismiss() },
            onConfirm: {
                working = true
                await onConfirm()
                working = false
                dismiss()
            }
        ) {
            Callout(icon: "exclamationmark.triangle.fill", tone: .warning, text: message)
        }
    }
}

// MARK: - Shared chrome

/// One frame for every sheet, so padding, button order and rhythm match.
struct SheetFrame<Content: View>: View {
    let title: String
    var subtitle: String? = nil
    let confirmTitle: String
    var confirmRole: ButtonRole? = nil
    let canConfirm: Bool
    let working: Bool
    let error: String?
    let onCancel: () -> Void
    let onConfirm: () async -> Void
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.title2.weight(.semibold))
                if let subtitle {
                    Text(subtitle).font(.callout).foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 24)
            .padding(.top, 22)
            .padding(.bottom, 16)

            content
                .padding(.horizontal, 24)

            if let error {
                Callout(icon: "xmark.octagon.fill", tone: .error, text: error)
                    .padding(.horizontal, 24)
                    .padding(.top, 12)
            }

            Divider().padding(.top, 20)

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel", action: onCancel)
                    .keyboardShortcut(.cancelAction)
                Button(confirmTitle, role: confirmRole) {
                    Task { await onConfirm() }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!canConfirm)
                .overlay(alignment: .leading) {
                    if working {
                        ProgressView().controlSize(.small).offset(x: -26)
                    }
                }
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 14)
        }
        .frame(width: 460)
    }
}

struct Callout: View {
    enum Tone { case neutral, warning, error }

    let icon: String
    let tone: Tone
    let text: String

    private var color: Color {
        switch tone {
        case .neutral: return .secondary
        case .warning: return .orange
        case .error: return .red
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 9) {
            Image(systemName: icon).foregroundStyle(color)
            Text(text).fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .font(.callout)
        .padding(11)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(color.opacity(tone == .neutral ? 0.07 : 0.11))
        )
    }
}

/// Add a repository, without ever needing the terminal.
///
/// Overnight will only touch directories the user has allowlisted, so adding a
/// repository can involve two steps. The sheet does not make the user
/// understand that: it works out whether an allowlist entry is needed, names
/// the exact folder it would add, and asks once. What it never does is
/// silently grant access to a directory the user did not see.
struct AddRepositorySheet: View {
    let roots: [RepositoryRoot]
    /// Allowlist a folder. Returns a message on failure.
    let onAddRoot: (String) async -> String?
    /// Register a repository. Returns a message on failure.
    let onRegister: (String) async -> String?
    /// Called after a successful registration, so the worktrees that repository
    /// already has can be offered immediately. That is the moment they matter:
    /// the person has just pointed at a project and it very likely has three or
    /// four checked out already.
    var onRegistered: () -> Void = {}

    @Environment(\.dismiss) private var dismiss
    @State private var chosen: URL?
    @State private var working = false
    @State private var error: String?

    /// The allowlisted root that already covers the chosen folder, if any.
    private var coveringRoot: RepositoryRoot? {
        guard let chosen else { return nil }
        return roots.first { root in
            guard let path = root.path else { return false }
            return chosen.path == path || chosen.path.hasPrefix(path + "/")
        }
    }

    /// The folder that would be allowlisted: the repository's parent, so one
    /// entry covers its siblings too and the next repository needs no
    /// permission at all.
    private var rootToAdd: URL? {
        guard let chosen, coveringRoot == nil else { return nil }
        return chosen.deletingLastPathComponent()
    }

    private var looksLikeRepository: Bool {
        guard let chosen else { return false }
        return FileManager.default.fileExists(atPath: chosen.appendingPathComponent(".git").path)
    }

    var body: some View {
        SheetFrame(
            title: "Add repository",
            subtitle: "Overnight creates a worktree per task, so it needs an existing repository.",
            confirmTitle: "Add repository",
            canConfirm: chosen != nil && looksLikeRepository && !working,
            working: working,
            error: error,
            onCancel: { dismiss() },
            onConfirm: { await confirm() }
        ) {
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 10) {
                    Button("Choose folder…") { choose() }
                    if let chosen {
                        Text(chosen.lastPathComponent)
                            .font(.callout.weight(.medium))
                            .lineLimit(1)
                            .truncationMode(.head)
                    } else {
                        Text("No folder chosen").font(.callout).foregroundStyle(.secondary)
                    }
                }

                if chosen != nil && !looksLikeRepository {
                    Callout(
                        icon: "exclamationmark.triangle.fill",
                        tone: .warning,
                        text: "That folder is not a git repository. Choose the folder containing .git."
                    )
                } else if let rootToAdd {
                    // Naming the folder is the point. Granting access to a
                    // directory tree is not something to do behind a spinner.
                    Callout(
                        icon: "lock.open.fill",
                        tone: .neutral,
                        text:
                            "Overnight will also allowlist \(rootToAdd.lastPathComponent), the "
                            + "folder containing it. Overnight only ever operates inside folders "
                            + "you have allowlisted, and this one covers its siblings too."
                    )
                } else if let coveringRoot, let path = coveringRoot.path {
                    Callout(
                        icon: "checkmark.circle.fill",
                        tone: .neutral,
                        text: "Already inside the allowlisted folder \(URL(fileURLWithPath: path).lastPathComponent)."
                    )
                }
            }
        }
    }

    private func choose() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.message = "Choose a git repository."
        if panel.runModal() == .OK {
            chosen = panel.url
            error = nil
        }
    }

    private func confirm() async {
        guard let chosen else { return }
        working = true
        defer { working = false }

        if let rootToAdd, let failure = await onAddRoot(rootToAdd.path) {
            error = failure
            return
        }
        if let failure = await onRegister(chosen.path) {
            error = failure
            return
        }
        dismiss()
        onRegistered()
    }
}
