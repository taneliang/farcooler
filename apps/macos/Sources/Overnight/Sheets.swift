import SwiftUI

/// Create a workspace: one worktree plus branch for one task.
struct NewWorkspaceSheet: View {
    let repositories: [Repository]
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
        .onAppear { if repo.isEmpty { repo = repositories.first?.short ?? "" } }
    }
}

/// Launch a preset in a new tagged tmux window.
struct NewTerminalSheet: View {
    let workspaceName: String
    let onCreate: (String, String) async -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var preset = "shell"
    @State private var title = ""
    @State private var working = false

    private let presets: [(id: String, label: String, detail: String)] = [
        ("shell", "Shell", "An interactive login shell"),
        ("claude", "Claude Code", "Anthropic's coding agent"),
        ("codex", "Codex", "OpenAI's coding agent"),
        ("cursor", "Cursor", "Cursor's agent CLI"),
    ]

    var body: some View {
        SheetFrame(
            title: "New terminal",
            subtitle: "In \(workspaceName).",
            confirmTitle: "Launch",
            canConfirm: !working,
            working: working,
            error: nil,
            onCancel: { dismiss() },
            onConfirm: {
                working = true
                let name = title.trimmingCharacters(in: .whitespaces)
                await onCreate(preset, name.isEmpty ? preset : name)
                working = false
                dismiss()
            }
        ) {
            VStack(alignment: .leading, spacing: 14) {
                ForEach(presets, id: \.id) { p in
                    Button {
                        preset = p.id
                        } label: {
                        HStack(spacing: 11) {
                            Image(systemName: preset == p.id ? "largecircle.fill.circle" : "circle")
                                .foregroundStyle(preset == p.id ? Color.accentColor : .secondary)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(p.label).fontWeight(.medium)
                                Text(p.detail).font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                        }
                        .padding(.vertical, 8)
                        .padding(.horizontal, 12)
                        .background(
                            RoundedRectangle(cornerRadius: 8)
                                .fill(preset == p.id ? Color.accentColor.opacity(0.10) : .clear)
                        )
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }

                TextField("Title", text: $title, prompt: Text("optional"))
                    .textFieldStyle(.roundedBorder)
                    .padding(.top, 4)
            }
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
