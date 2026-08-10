import AppKit
import SwiftUI

/// Create a workspace: one worktree plus branch for one task.
struct NewWorkspaceSheet: View {
    /// Every machine's repositories, tagged the same way `FleetStore.repositories`
    /// tags them — carried together rather than flattened to a bare
    /// `[Repository]`. A repository's `short` is eight hex characters minted
    /// per daemon (see `FleetStore.repositories`'s own doc comment), so two
    /// machines can hand back the same one for two different repositories; the
    /// picker below has to choose host and repository together; a repository
    /// chosen without its host, submitted through whatever host happened to be
    /// "active" elsewhere, is exactly how a workspace gets created on the wrong
    /// machine with no error at all.
    let repositories: [(host: String, repository: Repository)]
    /// The repository to open on, when this was reached from a project header.
    /// Empty means "ask", which is what the sidebar's own `+` wants.
    var preselected: String = ""
    /// The machine `preselected` lives on. Only consulted alongside
    /// `preselected`, to disambiguate a display name that exists on more than
    /// one machine — once the picker below has been touched, its own
    /// selection carries its own host and this is never read again.
    var preselectedHost: String = ""
    /// Receives host and repository together with the rest of the form, and
    /// answers with the daemon's own failure message, or nil on success. A
    /// `String` return rather than swallowing the result is what lets this
    /// sheet stay open and say why, the same way `RemoveWorkspaceSheet` and
    /// `AddRepositorySheet` already do — `createWorkspace` failing used to
    /// dismiss the sheet exactly as if it had succeeded.
    /// What a given machine says branch names start with. Per machine, because
    /// the branch is created on the one holding the repository.
    var branchPrefix: (String) -> String = { _ in "" }
    let onCreate: (_ host: String, _ repo: String, _ name: String, _ branch: String, _ base: String)
        async -> String?
    @Environment(\.dismiss) private var dismiss

    /// Host and repository chosen together, so the picker can never leave
    /// them disagreeing — selecting a row changes both fields at once, unlike
    /// two independent pickers that could each point somewhere else.
    private struct Choice: Hashable {
        var host: String
        var short: String
    }

    @State private var choice: Choice?
    @State private var name = ""
    @State private var branch = ""
    @State private var base = "HEAD"
    @State private var working = false
    @State private var error: String?

    /// Whether more than one machine has a repository on offer — the picker
    /// names the machine alongside the repository only when that distinction
    /// is real, same rule the sidebar's own host labels follow.
    private var multipleHosts: Bool {
        Set(repositories.map(\.host)).count > 1
    }

    private func label(for entry: (host: String, repository: Repository)) -> String {
        guard multipleHosts else { return entry.repository.displayName }
        let host = entry.host.isEmpty ? "This Mac" : entry.host
        return "\(entry.repository.displayName) — \(host)"
    }

    /// Suggest a branch from the name, the way a person would write it.
    ///
    /// The prefix used to be `feat/` written out here, which is where this and
    /// the task composer disagreed: the composer used none. It is now whatever
    /// the chosen machine says, defaulting to `feat/` — so the two agree and the
    /// answer is configurable in one place.
    ///
    /// This one lowercases and the directory below does not. A branch is a
    /// slug by convention; a name is read back to the user, and the case they
    /// typed is theirs.
    private var suggestedBranch: String {
        let slug = name.lowercased()
            .map { $0.isLetter || $0.isNumber ? $0 : "-" }
            .reduce(into: "") { acc, c in
                if c == "-" && acc.hasSuffix("-") { return }
                acc.append(c)
            }
            .trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        return slug.isEmpty ? "" : branchPrefix(choice?.host ?? "") + slug
    }

    /// The worktree this is about to create. Shown under the field because
    /// naming a directory carefully is not something anyone does while the
    /// directory is invisible.
    private var worktreePath: String? {
        guard let choice,
            let entry = repositories.first(where: {
                $0.host == choice.host && $0.repository.short == choice.short
            })
        else { return nil }
        return WorktreeName.path(repository: entry.repository.displayName, name: name)
    }

    private var canCreate: Bool {
        choice != nil && WorktreeName.isValid(name) && !effectiveBranch.isEmpty && !working
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
                guard let choice else { return }
                working = true
                error = nil
                if let failure = await onCreate(
                    choice.host, choice.short, name, effectiveBranch, base)
                {
                    working = false
                    error = failure
                    return
                }
                working = false
                dismiss()
            }
        ) {
            Form {
                Picker("Repository", selection: $choice) {
                    ForEach(repositories, id: \.repository.id) { entry in
                        Text(label(for: entry))
                            .tag(Optional(Choice(host: entry.host, short: entry.repository.short)))
                    }
                }

                LabeledContent("Name") {
                    VStack(alignment: .leading, spacing: 4) {
                        TextField("", text: $name, prompt: Text("rate-limiting"))
                        if let worktreePath {
                            Text(worktreePath)
                                .font(.caption.monospaced())
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.head)
                        }
                    }
                }

                TextField(
                    "Branch", text: $branch,
                    prompt: Text(
                        suggestedBranch.isEmpty
                            ? branchPrefix(choice?.host ?? "") + "my-task" : suggestedBranch))

                TextField("Base revision", text: $base, prompt: Text("HEAD"))
            }
            .formStyle(.grouped)
        }
        .onAppear {
            guard choice == nil else { return }
            // The project whose header was clicked, if there was one. Matching
            // on display name because that is what the sidebar groups by, and
            // on host too, because two machines can share a display name and
            // only one of them is the project that header actually named.
            let preferred = repositories.first {
                $0.repository.displayName == preselected
                    && (preselectedHost.isEmpty || $0.host == preselectedHost)
            }
            if let entry = preferred ?? repositories.first {
                choice = Choice(host: entry.host, short: entry.repository.short)
            }
        }
    }
}

/// Destructive confirmation.
///
/// Worktree removal is the one action that can lose work that was never
/// committed. The client cannot tell whether this worktree is dirty — only the
/// daemon can see the working tree — so this starts with a plain Remove button
/// and no field. If the daemon refuses without a confirmation, it reveals the
/// field and says why. A worktree with nothing uncommitted in it is removed on
/// the first click. Demanding the typed name every time would train people to
/// type it without reading it, which spends the one gesture meant to stop a
/// mistake.
struct RemoveWorkspaceSheet: View {
    let workspace: Workspace
    /// How many terminals removal is about to close.
    ///
    /// A count rather than the flag this used to be. Running terminals no longer
    /// BLOCK removal — the daemon closes them — so what the sheet needs is not
    /// "may I proceed" but "how many am I about to close", which is the one thing
    /// the user cannot see from here and would want to know before clicking.
    let runningCount: Int
    /// Receives the exact text the user typed, which the daemon re-checks.
    ///
    /// Answers `.ok`, `.confirmationRequired` (the worktree is dirty and the
    /// name is needed), or `.failed` with the daemon's own message for every
    /// other refusal — `TmuxUnavailable`, the main checkout, a failed `git
    /// worktree remove`. Only the middle case means "there is uncommitted
    /// work here"; the others are refused for reasons that typing the name
    /// again does nothing about.
    let onRemove: (String) async -> DaemonClient.RemoveWorktreeResult
    @Environment(\.dismiss) private var dismiss

    @State private var typed = ""
    @State private var needsName = false
    @State private var errorMessage: String?
    @State private var working = false

    private var matches: Bool { typed == workspace.task }

    var body: some View {
        SheetFrame(
            title: "Remove worktree",
            subtitle: workspace.task,
            confirmTitle: "Remove worktree",
            confirmRole: .destructive,
            // No longer gated on running terminals: removal closes them, so
            // disabling the button over one meant refusing to do the thing the
            // sheet exists to do.
            canConfirm: !working && (!needsName || matches),
            working: working,
            error: errorMessage,
            onCancel: { dismiss() },
            onConfirm: {
                working = true
                errorMessage = nil
                let result = await onRemove(typed)
                working = false
                switch result {
                case .ok:
                    dismiss()
                case .confirmationRequired:
                    needsName = true
                case .failed(let message):
                    // Not "there is uncommitted work here" — that callout is
                    // reserved for a genuine confirmation-required refusal.
                    // The daemon's own message names what to actually fix.
                    errorMessage = message
                }
            }
        ) {
            VStack(alignment: .leading, spacing: 14) {
                // One callout, stating what the button will do. It used to be two,
                // and the first was an instruction — "Stop the terminals in this
                // workspace before removing it" — which told you to go and do by
                // hand the thing you had just asked for. The daemon closes them
                // now, so the only thing worth saying is how many.
                Callout(
                    icon: "info.circle.fill",
                    tone: .neutral,
                    text: runningCount == 0
                        ? "This deletes the working directory. The branch is kept, and nothing "
                            + "already committed or pushed is touched."
                        : "This closes \(runningCount) "
                            + (runningCount == 1 ? "terminal" : "terminals")
                            + " and deletes the working directory. The branch is kept, and "
                            + "nothing already committed or pushed is touched."
                )

                if needsName {
                    Callout(
                        icon: "exclamationmark.triangle.fill",
                        tone: .warning,
                        text: "There is uncommitted work here. Type the workspace name to confirm you want it gone."
                    )

                    VStack(alignment: .leading, spacing: 6) {
                        Text("Type the workspace name to confirm")
                            .font(.callout)
                        // Not disabled any more. It was gated on running
                        // terminals, which meant the one field standing between
                        // the user and a dirty worktree could not be typed into
                        // for a reason that no longer stops removal at all.
                        TextField("", text: $typed, prompt: Text(workspace.task))
                            .textFieldStyle(.roundedBorder)
                    }
                }
            }
        }
    }
}

/// Removing a repository — which really means removing the root it lives
/// under, since the daemon has no operation narrower than that.
///
/// Solo on its root, this is a plain destructive click: the sheet already
/// knows and shows the exact folder name being revoked, so it supplies that
/// as the confirmation itself rather than asking someone to retype what is
/// already on screen — the same trust `RemoveWorkspaceSheet` places in a
/// click that follows a name it already displayed. Sharing a root with other
/// repositories is the one case with real blast radius — removing one takes
/// every sibling with it — and gets the friction worktree removal reserves
/// for a dirty one: every name that would disappear, said once, and the
/// folder name typed by hand before the button does anything.
struct RemoveRepositorySheet: View {
    let repository: Repository
    let root: RepositoryRoot
    /// Every other repository sharing `root`, on the same host. Empty means
    /// this repository has the root to itself.
    let siblings: [Repository]
    /// Receives the confirmation text — the app's own for a solo root, typed
    /// by hand for a shared one — and answers what the daemon said.
    let onRemove: (_ confirm: String) async -> DaemonClient.RemoveRootResult
    @Environment(\.dismiss) private var dismiss

    @State private var typed = ""
    @State private var errorMessage: String?
    @State private var working = false

    /// The daemon confirms by folder name, not repository name — `root.path`
    /// when this Mac can see it, the repository's own name otherwise, since
    /// a remote root's path is only ever visible to a `host_admin` client.
    private var rootName: String {
        root.path.map { URL(fileURLWithPath: $0).lastPathComponent } ?? repository.displayName
    }
    private var isShared: Bool { !siblings.isEmpty }
    private var matches: Bool { typed == rootName }
    private var siblingNames: String {
        ListFormatter.localizedString(byJoining: siblings.map(\.displayName).sorted())
    }

    var body: some View {
        SheetFrame(
            title: "Remove \(repository.displayName)",
            confirmTitle: "Remove",
            confirmRole: .destructive,
            canConfirm: !working && (!isShared || matches),
            working: working,
            error: errorMessage,
            onCancel: { dismiss() },
            onConfirm: {
                working = true
                errorMessage = nil
                let result = await onRemove(isShared ? typed : rootName)
                working = false
                switch result {
                case .ok:
                    dismiss()
                case .confirmationRequired:
                    // The app's own idea of the name and the daemon's
                    // disagree — not the ordinary "you mistyped it" this
                    // reads as for a worktree, but there is nothing more
                    // specific to say from here.
                    errorMessage = "That did not match — try again."
                case .failed(let message):
                    errorMessage = message
                }
            }
        ) {
            VStack(alignment: .leading, spacing: 14) {
                Callout(
                    icon: "info.circle.fill",
                    tone: .neutral,
                    text: "Revokes access to \(rootName). Nothing on disk is touched."
                )

                if isShared {
                    Callout(
                        icon: "exclamationmark.triangle.fill",
                        tone: .warning,
                        text:
                            "\(siblings.count == 1 ? "\(siblingNames) shares" : "\(siblingNames) share") "
                            + "this folder and would be removed too."
                    )

                    VStack(alignment: .leading, spacing: 6) {
                        Text("Type the folder name to confirm")
                            .font(.callout)
                        TextField("", text: $typed, prompt: Text(rootName))
                            .textFieldStyle(.roundedBorder)
                    }
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
/// Far Cooler will only touch directories the user has allowlisted, so adding a
/// repository can involve two steps. The sheet does not make the user
/// understand that: it works out whether an allowlist entry is needed, names
/// the exact folder it would add, and asks once. What it never does is
/// silently grant access to a directory the user did not see.
///
/// This Mac is browsed with an `NSOpenPanel`, because it is the one machine
/// whose disk this process can actually see. Any other machine is typed as a
/// path instead — the app has no filesystem to browse there, only the
/// daemon on that machine does, and it is the one that decides whether the
/// path exists and looks like a repository.
struct AddRepositorySheet: View {
    /// Every machine that could take a new repository.
    let hosts: [String]
    /// Allowlisted roots across every machine, tagged the same way
    /// `FleetStore.roots` tags them — filtered here to whichever machine is
    /// currently chosen, so a root on one machine can never look like it
    /// covers a path on another that merely happens to share the string.
    let roots: [(host: String, root: RepositoryRoot)]
    /// Allowlist a folder on `host`. Returns a message on failure.
    let onAddRoot: (_ host: String, _ path: String) async -> String?
    /// Register a repository on `host`. Returns a message on failure.
    let onRegister: (_ host: String, _ path: String) async -> String?
    /// Called with the host just registered on, after a successful
    /// registration, so the worktrees that repository already has can be
    /// offered immediately. That is the moment they matter: the person has
    /// just pointed at a project and it very likely has three or four checked
    /// out already. The host is handed back rather than assumed, because it
    /// is this sheet's own `host` state that decided which machine registered
    /// — nothing upstream chose it.
    var onRegistered: (_ host: String) -> Void = { _ in }

    @Environment(\.dismiss) private var dismiss
    /// Empty means this Mac, same convention as everywhere else host-tagged.
    @State private var host: String = ""
    @State private var chosen: URL?
    @State private var remotePath: String = ""
    @State private var working = false
    @State private var error: String?

    /// The path being proposed, however it was chosen. Building this from a
    /// `URL` even for a typed remote path is pure string manipulation — the
    /// path segment work below never touches a filesystem, remote or
    /// otherwise, so it is safe to share between both cases.
    private var path: URL? {
        host.isEmpty ? chosen : (remotePath.isEmpty ? nil : URL(fileURLWithPath: remotePath))
    }

    /// This machine's own allowlisted roots. A root string-identical to one
    /// on a different machine still must not answer for this one.
    private var rootsOnHost: [RepositoryRoot] {
        roots.filter { $0.host == host }.map(\.root)
    }

    /// The allowlisted root that already covers the chosen path, if any.
    private var coveringRoot: RepositoryRoot? {
        guard let path else { return nil }
        return rootsOnHost.first { root in
            guard let rootPath = root.path else { return false }
            return path.path == rootPath || path.path.hasPrefix(rootPath + "/")
        }
    }

    /// The folder that would be allowlisted: the repository's parent, so one
    /// entry covers its siblings too and the next repository needs no
    /// permission at all.
    private var rootToAdd: URL? {
        guard let path, coveringRoot == nil else { return nil }
        return path.deletingLastPathComponent()
    }

    /// Only meaningful on this Mac — the one machine whose disk this process
    /// can read. A remote path is taken on faith here; the daemon that owns
    /// that disk is what actually confirms it, when `onRegister` reaches it.
    private var looksLikeRepository: Bool {
        guard let chosen else { return false }
        return FileManager.default.fileExists(atPath: chosen.appendingPathComponent(".git").path)
    }

    private var canConfirm: Bool {
        guard path != nil, !working else { return false }
        return host.isEmpty ? looksLikeRepository : true
    }

    var body: some View {
        SheetFrame(
            title: "Add repository",
            subtitle: "Far Cooler creates a worktree per task, so it needs an existing repository.",
            confirmTitle: "Add repository",
            canConfirm: canConfirm,
            working: working,
            error: error,
            onCancel: { dismiss() },
            onConfirm: { await confirm() }
        ) {
            VStack(alignment: .leading, spacing: 14) {
                if hosts.count > 1 {
                    Picker("Machine", selection: $host) {
                        ForEach(hosts, id: \.self) { h in
                            Text(h.isEmpty ? "This Mac" : h).tag(h)
                        }
                    }
                    .pickerStyle(.menu)
                }

                if host.isEmpty {
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
                } else {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Path on \(host)")
                            .font(.callout)
                        TextField("/home/you/src/project", text: $remotePath)
                            .textFieldStyle(.roundedBorder)
                        Text("Checked on that machine when you add it — this Mac cannot see its disk.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                if host.isEmpty && chosen != nil && !looksLikeRepository {
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
                            "This also grants access to \(rootToAdd.lastPathComponent), "
                            + "the enclosing folder."
                    )
                } else if let coveringRoot, let rootPath = coveringRoot.path {
                    Callout(
                        icon: "checkmark.circle.fill",
                        tone: .neutral,
                        text: "Already inside the allowlisted folder \(URL(fileURLWithPath: rootPath).lastPathComponent)."
                    )
                }
            }
        }
        .onChange(of: host) { _, _ in error = nil }
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
        guard let path else { return }
        working = true
        defer { working = false }

        if let rootToAdd, await onAddRoot(host, rootToAdd.path) != nil {
            // The enclosing folder can be refused for reasons that say
            // nothing about the repository itself — it is a whole home
            // directory, a system path, already covered by a root nested
            // the other way. None of those make the repository's own folder
            // un-allowlistable, and it is exactly what "This also grants
            // access to…" already showed, just narrower than what was
            // offered rather than something unseen. A repository living
            // directly inside `$HOME` — an ordinary way to keep one — would
            // otherwise be impossible to add from this sheet at all: its
            // enclosing folder always fails and there is no error to read to
            // find out why.
            if let fallbackFailure = await onAddRoot(host, path.path) {
                error = fallbackFailure
                return
            }
        }
        if let failure = await onRegister(host, path.path) {
            error = failure
            return
        }
        dismiss()
        onRegistered(host)
    }
}
