import AppKit
import SwiftUI

/// A workspace in the detail, at one of two levels (spec §4.3): its
/// orchestrator's conversation beside its board, or, drilled into a task or
/// a worktree, that alone with the conversation shrunk to a rail beside it.
///
/// Which is drawn is `WorkspaceColumns.layout`'s answer for the detail's own
/// width, measured here and never read from the window, which the detail
/// shares with the sidebar. The contents are the window's: each is handed in
/// whole, so this view decides where things go and nothing about what they
/// are.
struct WorkspaceView<Conversation: View, Rail: View, Board: View, Crumbs: View, Drilled: View>: View {
    /// Whether a task or a worktree is open: the drilled level.
    let drilled: Bool
    /// Whether this workspace has a conversation column at all: not a
    /// repository's implicit workspace on a runner without `workstreams`.
    let hasConversation: Bool
    /// The terminal font's cell width: the minimums are in columns.
    let cell: CGFloat
    /// Focus (⌃⌘↩): what's opened alone, without the rail.
    let focused: Bool
    /// The conversation popped open over what's opened.
    let peek: Bool
    /// Which column the one-column form shows.
    @Binding var pick: WorkspacePick
    @ViewBuilder let conversation: () -> Conversation
    @ViewBuilder let rail: () -> Rail
    @ViewBuilder let board: () -> Board
    /// The breadcrumb, across the top of the drilled level, over the rail
    /// and what's opened alike.
    @ViewBuilder let breadcrumb: () -> Crumbs
    @ViewBuilder let opened: () -> Drilled
    /// A click outside the popped-open conversation: it closes.
    var onDismissPeek: () -> Void = {}
    /// How the popped-open conversation moves: `OrchestratorPeek.spring`,
    /// slowed only by a test that reads it mid-flight.
    var peekMotion: Animation = OrchestratorPeek.spring

    var body: some View {
        GeometryReader { proxy in
            let arrangement = WorkspaceColumns.layout(
                width: proxy.size.width, drilled: drilled, cell: cell, hasConversation: hasConversation,
                focused: focused, peek: peek)
            // The workspace level stays drawn, hidden, while drilled in, so
            // the board keeps its place, its scroll and its visit (the
            // summary's "last visit" is when you left the workspace, not a
            // task), and the split its dividers, for Back. Never the
            // conversation itself: one terminal view per pane, and it's the
            // rail's to pop open.
            let base = WorkspaceColumns.layout(
                width: proxy.size.width, drilled: false, cell: cell, hasConversation: hasConversation)
            ZStack {
                workspaceLevel(base)
                    .opacity(drilled ? 0 : 1)
                    .allowsHitTesting(!drilled)
                    .accessibilityHidden(drilled)
                if arrangement.drilled {
                    drilledLevel(arrangement, width: proxy.size.width)
                }
            }
            .frame(width: proxy.size.width, height: proxy.size.height)
            .preference(key: WorkspaceArrangementPreference.self, value: arrangement)
            .preference(key: WorkspaceWidthPreference.self, value: proxy.size.width)
        }
    }

    private func drilledLevel(_ arrangement: WorkspaceColumns.Arrangement, width: CGFloat) -> some View {
        VStack(spacing: 0) {
            breadcrumb()
            Divider()
            HStack(spacing: 0) {
                if arrangement.conversation != .none {
                    rail()
                        .frame(width: WorkspaceColumns.rail)
                        .frame(maxHeight: .infinity)
                    Divider()
                }
                // Over what's opened, never beside it: popping the
                // conversation open doesn't resize a task's terminals, or
                // their tmux windows for every other client.
                opened()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .overlay(alignment: .leading) {
                        // Animated here alone: going anywhere else takes the
                        // drilled level down at once, so the conversation is
                        // never drawn twice while one fades.
                        if arrangement.conversation != .none {
                            OrchestratorPeekPanel(
                                open: arrangement.conversation == .peek,
                                width: WorkspaceColumns.peekWidth(in: width, cell: cell),
                                motion: peekMotion, content: conversation, onDismiss: onDismissPeek)
                        }
                    }
                    .accessibilityIdentifier("workspace-opened")
            }
        }
        .background(WorkspaceStyle.canvas)
    }

    @ViewBuilder
    private func workspaceLevel(_ arrangement: WorkspaceColumns.Arrangement) -> some View {
        if arrangement.switcher {
            VStack(spacing: 0) {
                Picker("Show", selection: $pick) {
                    ForEach(WorkspacePick.allCases) { Text($0.title).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity)
                .background(WorkspaceStyle.canvas)
                Divider()
                Group {
                    if pick == .board {
                        board()
                    } else if !drilled {
                        conversation()
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .accessibilityIdentifier("workspace-one-column")
        } else {
            HSplitView {
                if arrangement.conversation == .column {
                    // The slot stays while drilled in, empty, so the split
                    // keeps its divider where it was for Back.
                    ZStack {
                        if !drilled { conversation() }
                    }
                    .frame(
                        minWidth: WorkspaceColumns.conversationMinimum(cell: cell),
                        maxWidth: .infinity, maxHeight: .infinity)
                    .layoutPriority(1)
                    .accessibilityIdentifier("workspace-conversation")
                }
                board()
                    .frame(
                        minWidth: WorkspaceColumns.boardMinimum,
                        idealWidth: WorkspaceColumns.boardIdeal,
                        maxWidth: arrangement.conversation == .none ? .infinity : nil,
                        maxHeight: .infinity)
                    .accessibilityIdentifier("workspace-board")
            }
        }
    }
}

/// The orchestrator popped open over a task or a worktree, from the rail.
///
/// Mounted on its first open and kept while drilled in, closed or not, so
/// each open slides the same terminal view in rather than building a new
/// one; it goes with the drilled level, and in Focus. It moves on an offset,
/// on one spring, never by inserting and removing it: a press mid-flight
/// retargets the spring from wherever the panel is.
///
/// Whether it's open is `open`, the window's state, and nothing here waits
/// on the motion. The moment it reads closed, neither the panel nor the
/// click-outside catcher takes a click or the accessibility tree's notice,
/// however far it has left to slide; and it's clipped to what's opened, so
/// in flight it never passes over the rail. Both of those are what made the
/// rail seem dead while it moved (ov-84): the old panel slid in and out by
/// its whole width, catcher and all, across the rail, still hit-testable
/// while removed, so a click on the rail mid-close landed on the catcher,
/// whose close found it already closed. The clip covers clicks as well as
/// drawing: a clip alone lets the panel's far side take clicks over the
/// rail and the sidebar.
struct OrchestratorPeekPanel<Content: View>: View {
    let open: Bool
    /// The conversation's width open (`WorkspaceColumns.peekWidth`).
    let width: CGFloat
    var motion: Animation = OrchestratorPeek.spring
    @ViewBuilder let content: () -> Content
    var onDismiss: () -> Void

    /// Opened once: from then on it stays, off to the side when closed.
    @State private var mounted = false

    var body: some View {
        // Out only once it's on screen to move: the first open mounts it
        // closed, and its appearing slides it out.
        let out = open && mounted
        ZStack(alignment: .leading) {
            // A click anywhere else over what's opened puts it away, under a
            // dimming that comes and goes with it.
            Color.black
                .opacity(out ? OrchestratorPeek.dimming : 0)
                .contentShape(Rectangle())
                .onTapGesture(perform: onDismiss)
                .allowsHitTesting(OrchestratorPeek.takesClicks(open: open))
                .accessibilityHidden(true)
            if mounted || open {
                HStack(spacing: 0) {
                    content()
                        .frame(width: width)
                        .frame(maxHeight: .infinity)
                        .background(WorkspaceStyle.canvas)
                    Divider()
                }
                .compositingGroup()
                .shadow(color: .black.opacity(out ? OrchestratorPeek.shadow : 0), radius: 8, x: 2)
                .offset(x: OrchestratorPeek.offset(open: out, width: width))
                .allowsHitTesting(OrchestratorPeek.takesClicks(open: open))
                .accessibilityHidden(!open)
                // Nothing in it takes the keyboard while it's closed: not the
                // terminal, not a chat composer, not a SwiftUI control.
                .environment(\.outOfSight, !open)
                .disabled(!open)
                .accessibilityIdentifier("workspace-conversation-peek")
                .onAppear { mounted = true }
            }
        }
        .animation(motion, value: out)
        // Drawn and clicked only over what's opened: in flight, the panel's
        // far side is over the rail and the sidebar, and a clip alone stops
        // the drawing there but not the clicks.
        .clipShape(Rectangle())
        .contentShape(Rectangle())
        .allowsHitTesting(OrchestratorPeek.takesClicks(open: open))
    }
}

extension EnvironmentValues {
    /// Mounted but out of sight: the orchestrator tucked away beside a task
    /// (ov-84). A terminal or a composer under it takes no keyboard, by
    /// click, Tab or its own claim, and lets go of one it holds.
    @Entry var outOfSight = false
}

/// What an AppKit view that can take the keyboard does about `outOfSight`.
enum KeyboardFence {
    /// Called as `takesKeyboard` turns off: the window's first responder,
    /// if it's `view`, lets go, and what's opened takes it up from there
    /// (`ContentView.closePeek`).
    @MainActor
    static func release(_ view: NSView) {
        guard let window = view.window, window.firstResponder === view else { return }
        window.makeFirstResponder(nil)
    }
}

/// How the popped-open orchestrator moves, and what a press of the rail,
/// ⌥⌘1, Esc or a click outside does to it: always the opposite of what it
/// is now, at once, with nothing waiting on the motion (ov-84).
enum OrchestratorPeek {
    /// One spring for the panel, its shadow and the dimming, retargeted from
    /// where it is by a press mid-flight.
    static let spring = Animation.spring(response: 0.32, dampingFraction: 0.86)
    /// What's opened, dimmed under it.
    static let dimming = 0.08
    static let shadow = 0.18
    /// Past its own width when closed, so the shadow is out of sight too.
    static let overhang: CGFloat = 24

    /// Where the panel sits: at the rail's edge open, wholly off to the
    /// side closed.
    static func offset(open: Bool, width: CGFloat) -> CGFloat {
        open ? 0 : -(width + WorkspaceColumns.divider + overhang)
    }

    /// Whether the panel and the catcher around it take clicks: exactly
    /// while it's open, whatever the motion is doing.
    static func takesClicks(open: Bool) -> Bool { open }

    /// The state after a press of the rail or ⌥⌘1, drilled in.
    static func pressed(open: Bool) -> Bool { !open }

    /// The layout the conversation's view draws, and whether it's on
    /// screen. Drilled in, the panel draws the conversation's layout open
    /// or closed, so it's one terminal view; it's on screen, seen, watched
    /// and given the keyboard only when `visible` (`WorkspaceScreen.visible`)
    /// has it, which is while it's popped open.
    static func conversation(
        drilled: Bool, visible: [ShownLayout], drawable: [ShownLayout]
    ) -> (layout: ShownLayout?, onScreen: Bool) {
        let shown = visible.first { $0.column == .conversation }
        guard drilled else { return (shown, shown != nil) }
        return (drawable.first { $0.column == .conversation }, shown != nil)
    }

    /// Whether a terminal view drawing `layout` has the keyboard: never
    /// while it's off screen, else as `WorkspaceScreen.hasKeyboard` says.
    static func takesKeyboard(_ layout: ShownLayout, onScreen: Bool, key: PaneRef?, onBoard: Bool) -> Bool {
        onScreen && WorkspaceScreen.hasKeyboard(layout, key: key, onBoard: onBoard)
    }
}

/// The breadcrumb over a task or a worktree opened: Back, then each level up
/// to the one you're at, every one but that a way back to it.
struct DrillBreadcrumb: View {
    let crumbs: [WorkspaceNavigation.Crumb]
    var onGo: (ContentView.Selection) -> Void
    var onBack: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Button(action: onBack) {
                Image(systemName: "chevron.left")
            }
            .buttonStyle(.borderless)
            .help("Back (⌃⌘←)")
            .accessibilityLabel("Back")
            ForEach(Array(crumbs.enumerated()), id: \.offset) { index, crumb in
                if index > 0 { Text("›").foregroundStyle(.tertiary) }
                if let target = crumb.target {
                    Button(crumb.title) { onGo(target) }
                        .buttonStyle(.plain)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .help("Go to \(crumb.title)")
                } else {
                    Text(crumb.title)
                        .fontWeight(.semibold)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            Spacer(minLength: 0)
        }
        .font(.system(size: 12))
        .padding(.horizontal, 12)
        .frame(height: 30)
        .background(WorkspaceStyle.canvas)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Breadcrumb")
    }
}

/// The one-column form's Orchestrator | Board control.
enum WorkspacePick: String, CaseIterable, Identifiable {
    case orchestrator, board
    var id: String { rawValue }
    var title: String { self == .orchestrator ? "Orchestrator" : "Board" }
}

/// The arrangement a `WorkspaceView` drew, published from the value it
/// switched on, so a test can read which columns a width got.
struct WorkspaceArrangementPreference: PreferenceKey {
    static let defaultValue: WorkspaceColumns.Arrangement? = nil
    static func reduce(value: inout WorkspaceColumns.Arrangement?, nextValue: () -> WorkspaceColumns.Arrangement?) {
        value = nextValue() ?? value
    }
}

/// The detail's width as a `WorkspaceView` measured it: what the window reads
/// to say which of a workspace's columns are on screen.
struct WorkspaceWidthPreference: PreferenceKey {
    static let defaultValue: CGFloat? = nil
    static func reduce(value: inout CGFloat?, nextValue: () -> CGFloat?) {
        value = nextValue() ?? value
    }
}
