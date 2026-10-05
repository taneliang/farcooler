import AgentKit
import SwiftUI

// What's left of the old Fleet sidebar's views once the sidebar went
// (ov-178): the worktree page a worktree opened whole draws
// (`WorktreeDetail`), its status dot, and two things other views share.

extension View {
    /// `.help`, when there is something to say, and nothing at all otherwise.
    ///
    /// An empty tooltip string is not the same as no tooltip: it still hands
    /// AppKit a tracking rectangle for a box with nothing in it. A conditional
    /// modifier keeps a clean worktree's row from having one.
    @ViewBuilder
    func help(ifAny text: String?) -> some View {
        if let text {
            self.help(text)
        } else {
            self
        }
    }
}

/// A worktree's glyph, wherever one is drawn beside its name: the
/// navigator's rows and the breadcrumb's menu.
enum WorktreeSection {
    /// What a worktree is drawn as in its glyph column: a branch.
    static let glyph = OneTreeGlyph.worktree
}

/// A worktree's own status, for its heading.
///
/// Worktrees have three states worth showing and no agent of their own, so
/// this stays a small dot: it is context for the heading rather than something
/// to scan, and the terminals underneath carry the detail.
///
/// The three are the exceptions — gone, broken, hidden. `active` was a fourth,
/// painted green, and that was the drift: "three states worth showing" was
/// right and the switch had grown a case for the state that is worth showing
/// least. `derive_worktree` returns `Active` for any worktree with a live
/// terminal, which is nearly all of them nearly all of the time, so the green
/// was on almost every heading — and green in this app means `Status.done`,
/// "the turn ended and nobody has looked yet". A worktree being in use and an
/// agent having finished are close to opposites, and they drew the same mark.
/// That is the phones' bug — an idle `zsh` and a finished agent in one green —
/// which the Mac was credited with not having.
///
/// `Active` is also what `derive_worktree` returns when the runner is
/// unreadable and every terminal derives `Unknown`, deliberately, so the green
/// was vouching for worktrees nobody had heard from. `StatusGlyph` paints that
/// same not-yet-answered case `.secondary` for exactly this reason.
///
/// iOS has nothing to keep in sync here: its worktree heading draws the
/// rolled-up status of the terminals inside plus "worktree gone", and never the
/// worktree's own state. So this is a one-platform change by construction, not
/// one half of a pair.
struct WorktreeDot: View {
    let state: String

    private var color: Color {
        switch StateKind.parse(state) {
        case .error: return .red
        case .hidden: return SidebarInk.secondary
        // Red, not a dimmed amber. `StatusGlyph` spends amber on one state —
        // an agent is waiting on you — and a directory that is gone is not
        // waiting for anything. It is the worktree-level `Status.lost`, and
        // `lost` is red. The soft orange was a third reading from before there
        // was a rule: less alarming than `error`, warmer than `hidden`.
        //
        // It shares red with `error` now, which is affordable HERE and would
        // not be in a list: this dot appears once, beside a 24pt title, never
        // next to another one, and its help text names the state.
        case .worktreeMissing: return .red
        // `active` falls here with `ready` and `creating`: a healthy worktree
        // is a healthy worktree, and the difference between one with a live
        // pane and one without is a fact the rows underneath state outright.
        default: return SidebarInk.secondary
        }
    }

    var body: some View {
        Circle().fill(color).frame(width: 8, height: 8).help(state)
    }
}

/// Worktree detail.
///
/// Used to open with a State / Terminals / Worktree key-value table, which read
/// like a database inspector and gave the most screen to a filesystem path
/// nobody needs. The terminals ARE the worktree, so they lead; the path is a
/// footnote you can copy when you want it.
struct WorktreeDetail: View {
    /// An orchestrator running here, drawn in its workspace's conversation
    /// column instead, and how to go there.
    struct Hosted {
        let name: String
        let go: () -> Void
    }

    /// The worktree, without the orchestrators in `hosted`.
    let worktree: Worktree
    var hosted: [Hosted] = []
    let onNewTerminal: () -> Void
    let onHide: () -> Void
    let onUnhide: () -> Void
    let onRemove: () -> Void
    let onOpenTerminal: (Terminal) -> Void
    /// Restart or Dismiss, from a card's context menu (ov-191).
    var onTerminalAction: (TerminalAction, Terminal) -> Void = { _, _ in }
    /// Whether the runner can name a terminal, so a card offers Rename… (ov-234).
    var canRename = false
    /// The runner is this Mac, so a card with a port offers Open in Browser.
    var onThisMac = false
    /// Show Changes, which opens its changes pane on its own when it has no
    /// terminal (ov-78); nil when its runner can't read changes.
    var onShowChanges: (() -> Void)?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                header

                if !hosted.isEmpty { hostedNote }

                if worktree.terminals.isEmpty {
                    empty
                } else {
                    VStack(spacing: 8) {
                        // Creation order, for the reason `WorktreeSection`
                        // gives: cards that rearrange themselves under the
                        // pointer are worse than cards you have to read.
                        ForEach(worktree.terminals) { t in
                            terminalCard(t)
                        }
                    }
                }

                footnote
            }
            .padding(32)  // grid-exempt: a detail page's margin, not a row
            .frame(maxWidth: 720, alignment: .leading)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentCard()
        // Same title as the tiled and solo views, so switching between them does
        // not change what the window claims you are looking at.
        .navigationTitle(worktree.windowTitle)
        .navigationSubtitle(worktree.windowSubtitle)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 10) {
                    WorktreeDot(state: worktree.state)
                    Text(worktree.task).font(.system(size: 24, weight: .semibold))
                }
                Text(worktree.branch)
                    .font(.system(size: 13, design: .monospaced))
                    .foregroundStyle(SidebarInk.secondary)
            }

            HStack(spacing: 8) {
                Button(action: onNewTerminal) {
                    Label("New Terminal", systemImage: "plus")
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("t", modifiers: .command)

                if let onShowChanges {
                    Button(action: onShowChanges) {
                        Label("Show Changes", systemImage: "plusminus")
                    }
                }

                // No "Tile all" any more. It gathered every terminal into one
                // arrangement, which was possible only while membership was a list
                // this app maintained; a terminal is a tmux window now, and putting
                // twelve of them in one window means twelve nested splits nobody
                // asked for. Panes come together one drag at a time, where you can
                // see what you are making.

                Spacer()

                // Destructive actions live in a menu rather than sitting as
                // permanent buttons next to the one you press constantly.
                Menu {
                    if worktree.isHidden {
                        Button("Unhide", action: onUnhide)
                    } else {
                        Button("Hide", action: onHide)
                    }
                    // Absent, not disabled, for the main checkout. A
                    // daemon-side refusal is a safety net; the button should
                    // not be there to press.
                    if !worktree.isMainCheckout {
                        Button("Remove Worktree…", role: .destructive, action: onRemove)
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
            }
        }
    }

    /// Where the orchestrators that run here are drawn, since it isn't here.
    private var hostedNote: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: "person.wave.2")
                .foregroundStyle(SidebarInk.secondary)
            Text(Self.hostedSentence(hosted.map(\.name)))
                .font(.callout)
                .foregroundStyle(SidebarInk.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            ForEach(Array(hosted.enumerated()), id: \.offset) { _, seat in
                Button(hosted.count == 1 ? "Show Orchestrator" : "Show \(seat.name)", action: seat.go)
            }
        }
    }

    /// "The orchestrator runs here…", for the workspaces whose orchestrators
    /// run in this worktree.
    static func hostedSentence(_ names: [String]) -> String {
        switch names.count {
        case 0: return ""
        case 1: return "The orchestrator runs here. It’s in the Orchestrator column."
        default:
            let list = ListFormatter.localizedString(byJoining: names)
            return "The orchestrators for \(list) run here. Each is in its workspace’s Orchestrator column."
        }
    }

    private var empty: some View {
        VStack(spacing: 8) {
            Image(systemName: "terminal")
                .font(.system(size: 28))
                .foregroundStyle(SidebarInk.secondary)
            Text("No terminals").font(.callout.weight(.medium))
            Text("A terminal runs one agent, or one shell, inside this worktree.")
                .font(.caption)
                .foregroundStyle(SidebarInk.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 40)
        .surface(.inset, in: .card)
    }

    private func terminalCard(_ t: Terminal) -> some View {
        Button {
            onOpenTerminal(t)
        } label: {
            HStack(spacing: 12) {
                StatusGlyph(status: t.status)

                VStack(alignment: .leading, spacing: 2) {
                    Text(t.label).font(.system(size: 14, weight: .medium))
                    Text(t.preset).font(.system(size: 12)).foregroundStyle(SidebarInk.secondary)
                }

                Spacer()

                Ticking(paused: t.status != .working && t.status != .blocked, since: t.displayDurationSince) { now in
                    Text(t.displayDuration(at: now).map { "\(t.status.label) \($0)" } ?? t.status.label)
                        .font(.system(size: 12))
                        .foregroundStyle(SidebarInk.secondary)
                }

                Image(systemName: "chevron.right")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(SidebarInk.secondary)
            }
            .padding(.vertical, 12)
            .padding(.horizontal, 14)  // grid-exempt: a card's inset on the detail page
            .surface(.inset, in: .card)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu { cardMenu(t) }
    }

    /// Open, and for a terminal with no running pane the page's own two
    /// answers (ov-191): a lost card used to do nothing, clicked or not.
    @ViewBuilder
    private func cardMenu(_ t: Terminal) -> some View {
        Button("Open") { onOpenTerminal(t) }
        if onThisMac, t.portLabel != nil {
            Button("Open in Browser") { onTerminalAction(.openInBrowser, t) }
        }
        if canRename {
            Button("Rename…") { onTerminalAction(.rename, t) }
        }
        if LostPane.Kind(state: t.state) == nil, !t.isChangesPane {
            Button("Close") { onTerminalAction(.close, t) }
        }
        if let kind = LostPane.Kind(state: t.state) {
            Divider() // style-exempt: a context menu's section break
            ForEach(LostPane.actions(for: kind), id: \.title) { action in
                Button(action.title) { onTerminalAction(TerminalAction(action), t) }
            }
        }
    }

    /// The path, demoted to where it belongs.
    private var footnote: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "folder")
                .font(.system(size: 11))
                .foregroundStyle(SidebarInk.secondary)
            Text(worktree.path)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(SidebarInk.secondary)
                .textSelection(.enabled)
                .lineLimit(2)
                .truncationMode(.middle)
        }
    }
}
