import AgentKit
import SwiftUI

// Terminals as the owner's "just terminals" direction has them (ov-234): an
// ordinary shell in the project or a worktree, started by hand, that you can
// name, see the port of, open in a browser and close. Nothing here starts or
// closes one on its own.
//
// Worked out as values, so which terminals a task shows, what a port opens
// and what the rename sheet accepts are the ones `TerminalNamesTests` pins.

extension Terminal {
    /// The TCP ports this terminal is listening on, lowest first. Empty with
    /// nothing listening, and from a runner too old to say, whose name still
    /// carries the `web :PORT` text it always did (`terminal_ports`).
    var servedPorts: [Int] { (ports ?? []).sorted() }

    /// The one port a row shows, `:5173`: the lowest, because a dev server
    /// that also opens a debugger port reads as the server a person started.
    var portLabel: String? { servedPorts.first.map { ":\($0)" } }
}

enum TerminalPorts {
    /// What Open in Browser opens: `http://localhost:<lowest port>`, only for
    /// a terminal on this Mac. A remote runner's port isn't forwarded, so
    /// `localhost` would open whatever else is on this Mac's port, or nothing.
    /// `host` is the runner's, empty for this Mac.
    static func browserURL(for terminal: Terminal, host: String) -> URL? {
        guard host.isEmpty, let port = terminal.servedPorts.first else { return nil }
        return URL(string: "http://localhost:\(port)")
    }
}

/// A task's terminals (ov-234): the ones running in its worktree that are
/// the person's, each with Close. The task's agent, an orchestrator and a
/// changes pane have rows of their own elsewhere. Nothing closes on its own:
/// a Done task's server keeps running until it is closed here.
enum TaskTerminals {
    /// The terminals in the worktree `row` works in, in the runner's order.
    /// None when the task has no worktree the fleet lists.
    static func terminals(of row: TaskRow, host: String, in fleet: Fleet) -> [Terminal] {
        guard let worktree = WorkspaceWorktrees.worktree(of: row, host: host, in: fleet) else { return [] }
        return ProjectTerminals.terminals(in: worktree, fleet: fleet)
    }
}

/// What the rename sheet is about: a terminal, and the worktree it runs in.
struct RenamingTerminal: Identifiable, Equatable {
    var terminal: Terminal
    var worktree: Worktree
    var id: String { terminal.id }
}

enum TerminalName {
    /// The longest name the runner takes (`MAX_TERMINAL_NAME`).
    static let limit = 80

    /// What the field holds when the sheet opens: the name a person gave it,
    /// or nothing for one named for what runs in it.
    static func initial(_ terminal: Terminal) -> String {
        terminal.label == Terminal.name(of: terminal.preset) ? "" : terminal.label
    }

    /// Whether the runner would take `typed`: trimmed, within the limit,
    /// with no control character. An empty one is taken too; it clears the
    /// name.
    static func isValid(_ typed: String) -> Bool {
        let name = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.count <= limit && !name.unicodeScalars.contains { $0.properties.generalCategory == .control }
    }
}

/// Rename Terminal: what the name is for, a field, and Rename.
struct RenameTerminalSheet: View {
    let terminal: Terminal
    /// Receives what was typed. A refusal is the window's banner's to say, as
    /// for every action on a terminal, so the sheet closes either way.
    let onRename: (String) async -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var typed: String
    @State private var working = false

    init(terminal: Terminal, onRename: @escaping (String) async -> Void) {
        self.terminal = terminal
        self.onRename = onRename
        _typed = State(initialValue: TerminalName.initial(terminal))
    }

    var body: some View {
        SheetFrame(
            title: "Rename Terminal",
            subtitle: terminal.label,
            confirmTitle: "Rename",
            canConfirm: !working && TerminalName.isValid(typed),
            working: working,
            failure: nil,
            onCancel: { dismiss() },
            onConfirm: {
                working = true
                await onRename(typed)
                working = false
                dismiss()
            }
        ) {
            VStack(alignment: .leading, spacing: 10) {
                Text(
                    "A name helps you tell your terminals apart in the sidebar and the jump bar. "
                        + "Leave it empty to name the terminal for what’s running in it."
                )
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                TextField("", text: $typed, prompt: Text("Name"))
                    .textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier("rename-terminal-field")
            }
        }
    }
}

extension View {
    /// The Rename Terminal sheet while `renaming` holds a terminal; `rename`
    /// runs what was typed.
    func renameTerminalSheet(
        _ renaming: Binding<RenamingTerminal?>,
        rename: @escaping (RenamingTerminal, String) async -> Void
    ) -> some View {
        sheet(item: renaming) { target in
            RenameTerminalSheet(terminal: target.terminal) { typed in await rename(target, typed) }
        }
    }
}

/// A task's terminals, under its tab bar and over whichever tab is shown, so
/// a dev server stays in view while you read the diff (ov-234). Drawn only
/// while the worktree has one; each row has its name, its port with Open in
/// Browser on this Mac, and Close.
struct TaskTerminalsStrip: View {
    let terminals: [Terminal]
    /// The runner is this Mac, so a port opens in its browser.
    let onThisMac: Bool
    var onOpen: (Terminal) -> Void = { _ in }
    var onOpenInBrowser: (Terminal) -> Void = { _ in }
    var onRename: ((Terminal) -> Void)?
    var onClose: (Terminal) -> Void = { _ in }

    var body: some View {
        VStack(spacing: 0) {
            ForEach(terminals) { terminal in
                HStack(spacing: ColumnGrid.rhythm) {
                    Button(action: { onOpen(terminal) }) {
                        HStack(spacing: ColumnGrid.rhythm) {
                            Image(systemName: "terminal")
                                .font(TaskTypography.meta)
                                .foregroundStyle(.secondary)
                            Text(terminal.label)
                                .font(TaskTypography.body)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            if let port = terminal.portLabel {
                                Text(port)
                                    .font(TaskTypography.meta)
                                    .foregroundStyle(.secondary)
                                    .fixedSize()
                            }
                        }
                    }
                    .buttonStyle(.plain)
                    .help("Show this terminal in the task’s worktree")
                    Spacer(minLength: ColumnGrid.rhythm)
                    if onThisMac, terminal.portLabel != nil {
                        Button("Open in Browser") { onOpenInBrowser(terminal) }
                            .controlSize(.small)
                            .fixedSize()
                    }
                    Button("Close") { onClose(terminal) }
                        .controlSize(.small)
                        .fixedSize()
                        .help("Stop this terminal and remove it")
                }
                .padding(.horizontal, TaskTypography.inset.leading)
                .frame(minHeight: ColumnGrid.rowHeight)
                .contextMenu {
                    Button("Show") { onOpen(terminal) }
                    if onThisMac, terminal.portLabel != nil {
                        Button("Open in Browser") { onOpenInBrowser(terminal) }
                    }
                    if let onRename {
                        Button("Rename…") { onRename(terminal) }
                    }
                    Button("Close") { onClose(terminal) }
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("task-terminal-\(terminal.id)")
            }
        }
        .padding(.vertical, ColumnGrid.rhythm)
        .background(WorkspaceStyle.canvas)
    }
}
