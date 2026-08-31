import PhotosUI
import SwiftUI

// The bar that belongs to a PANE, which is the platform's own navigation bar.
//
// ## Why there are two bars, and why only one of them is the workspace
//
// `ShellBar` at the bottom IS the workspace — its name, its ribbon, and the
// surface you drag for the next workspace or lift for the column. It is
// navigation, and nothing else may be put on it: a control added there would
// be a control your thumb finds while it is trying to swipe, on the one
// surface in this app whose whole job is to be dragged.
//
// So what a PANE can do goes on a bar of the pane's own, at the top. Three
// things follow from that and all three are load-bearing:
//
// - **It is not in the thumb zone, and that is correct rather than a
//   compromise.** The thumb-zone rule this shell was built under is about
//   NAVIGATION — the thing you reach for constantly, which is why the bar that
//   changes workspace is at the bottom of the screen and not at the top. A
//   pane's own affordances at the top is ordinary iOS, and it is exactly what
//   `WorkspaceView` — the pane host the shell replaced — did with the same
//   controls before they were orphaned.
//
// - **The keyboard cannot take it away.** `DockedBar.swift:34-41`: an input
//   accessory lives in the KEYBOARD's window, so a composer rises with the
//   keyboard and the shell's bottom bar is simply covered by it —
//   `ShellRootView` deliberately stays full height so the bar and the track
//   never move under a keyboard, which means the bottom bar is unreachable for
//   as long as somebody is typing. A bar pinned to the TOP is untouched by all
//   of that: it keeps the material, and the pane's controls stay reachable
//   with the keyboard up. That is the whole reason the pane's bar is not
//   simply a second row on the shell's.
//
// - **It is glass, and the pane under it is matte.** Glass belongs to the
//   functional layer — navigation and controls — and content beneath it is
//   flat. This bar is functional; the diff's cards and the terminal's grid are
//   content and stay exactly as flat as they were. There is no glass on glass
//   anywhere here: the shell's bar is at the other end of the screen and the
//   two surfaces never overlap, which is the same arrangement a navigation bar
//   and a tab bar have always had.
//
// ## Why it is a `NavigationStack` and not a surface this file draws
//
// It WAS a surface this file drew: a rounded capsule with `GlassSurface` on
// it, applied as a `safeAreaInset`. That is the same object the shell's bottom
// bar is — same radius, same material, same inset from the edge — and the
// bottom bar is the one surface in this app you are meant to put a finger on
// and drag. Two of them, one at each end of the screen, does not read as "the
// workspace, and this pane"; it reads as two draggable bars, and the first
// question it got was why there was a second one.
//
// A navigation bar cannot be mistaken for that, because nothing else on the
// platform looks like it. And every property the hand-built version had to
// state, it gets for nothing and gets right: the material and its scroll-edge
// behavior, the 44-point row, the title's type and truncation, the safe area
// above it, the standard hit targets, and the way content passes underneath it
// rather than stopping at it.
//
// **The shell itself still has no `NavigationStack`, and must not grow one.**
// Phase 3 took the app's single stack out, and the overview has one of its own
// (`ShellOverview.overviewBody`). What is added here is one stack per PANE,
// inside the pane, so it travels with the pane on the track and no part of the
// shell — bar, track, overview — is inside anybody's navigation. See
// `ShellScreen.ShellPaneRealView.body` for where it is mounted and what the
// pane's own safe area is fed from.
//
// ## What it costs the pane, said out loud
//
// A navigation bar reduces the safe area of what it is over, so a scroll view
// treats it as a CONTENT inset — the diff's cards travel behind the material
// and only come to REST below it, which is what the overview's own chrome
// does. For the terminal the same inset is a real loss: a VT grid is not
// scrollable content, so rows that ran under the bar would be rows you cannot
// read, and the honest thing is for the grid to be a bar shorter.
//
// The measurement, because "a toolbar changes layout" is exactly the trap the
// overview hit — hiding its navigation bar moved every card 184 points. An
// inline navigation bar over a pane is **44 points**. The capsule this
// replaced took `ShellMetrics.barRow` plus a 12-point gap, which is **56**, so
// a pane is 12 points TALLER than it was yesterday and shorter than it was
// before any of this by exactly one navigation bar. Nothing about the TRACK
// changes: `ShellPaneTrack` sizes every pane to `page` × the full height and
// offsets it, and a bar inside a pane is invisible to all of that — the
// pane-retention tests (`testCommittingASwipeRebuildsNothing` and the two
// beside it) are what hold that down.

/// One pane's own chrome: what this pane is, and what it can do.
///
/// A wrapper around the pane rather than a bar drawn beside it, because
/// `.toolbar` is a modifier on a navigation stack's ROOT — so the thing that
/// owns the buttons has to be the thing that owns the content. That shape pays
/// for itself twice over: the picker and the sheets below hang off the pane,
/// which is a live view hierarchy that can present, rather than off a view
/// hosted inside a navigation bar, which is the same trap the old toolbar
/// carried a note about one level down (a `PhotosPicker` inside a `Menu`).
///
/// ## Which capabilities live here, and why each one
///
/// Five capabilities lost their door when the shell replaced the pane host.
/// Three of them are here, and each is here rather than somewhere else for a
/// reason worth writing down:
///
/// **Review options** (`ChangesToolbarMenu`) — the `DiffScope` picker is the
/// ONLY way to change what a diff is compared against, and one of two ways
/// into the commit history. It was the pane host's toolbar item and has been
/// unmounted since. The reason it was moved out of `ChangesView` in the first
/// place — two toolbar trees merging in a different order per pane — is not a
/// risk here: `ChangesView`'s body declares no toolbar of its own, and its
/// stacks are all inside sheets it presents.
///
/// **Send an image** — types a path into a tty, so it is offered only on a
/// PLAIN terminal pane. An agent pane has its own picker in the composer
/// (`AgentView`), and a second one up here would be two doors to one action
/// with different behavior behind them.
///
/// **Remove worktree** — `Connection.removeWorktree` had no iOS caller at all
/// after `FleetList` went. It is workspace-scoped, so the two candidate homes
/// were this bar and the overview card's context menu; this bar won because
/// the overview is being reworked into a multi-server grid by another lane and
/// a destructive action landing in a file that is being rewritten underneath
/// it is an action that quietly disappears again. It reads well here anyway:
/// the moment you decide a worktree is finished with is the moment you are
/// looking at it. The typed-name ceremony is unchanged — see
/// `RemoveWorktreeConfirmSheet`, recovered rather than rewritten.
///
/// **Terminal ↔ chat** is here too, and that was the least obvious of the
/// five. See `paneModeItem`.
///
/// Hidden-workspace disclosure is deliberately NOT here. It is a question
/// about which workspaces the fleet SHOWS, and a control on one pane that
/// changes what a different screen lists is a control nobody will find twice.
/// It belongs to the overview.
struct ShellPaneChromeModifier: ViewModifier {
    /// The tab's own title, straight off the shell's model.
    ///
    /// Not looked up again from the fleet, and that matters: the ribbon, the
    /// column and this bar are then three renderings of ONE string, so a
    /// terminal cannot be "claude 2" in the column and something else here.
    let title: String
    @ObservedObject var connection: Connection
    /// Images on their way into a terminal. Owned by `ShellScreen`, so a
    /// transfer started here keeps running when you swipe to another pane.
    @ObservedObject var pastes: ImagePasteQueue
    /// The workspace this pane belongs to, read live off the fleet.
    let workspace: Workspace?
    /// The terminal this pane IS, read live off the fleet — nil on a
    /// workspace's Diff tab, which has no pane on the runner behind it.
    ///
    /// Live rather than the snapshot `ShellPaneRealView` latched, and that
    /// distinction is `TerminalView.live`'s: the latched value is the pane's
    /// IDENTITY and is right to freeze, while the pane's MODE is exactly what
    /// the switch below changes. A frozen copy left the old button asking for
    /// the same switch every time.
    let live: Terminal?
    /// The review this pane shows, on the Diff tab and nowhere else.
    let changes: ChangesStore?
    /// Whether this pane is the one on screen. See `dismissEverything`.
    let isVisible: Bool

    @State private var showPhotoPicker = false
    @State private var pickedImage: PhotosPickerItem?
    @State private var removeCandidate: Workspace?
    @State private var confirmingRemove = false
    @State private var needsTypedConfirmation: Workspace?

    func body(content: Content) -> some View {
        content
            .navigationTitle(title)
            // Inline, and not a choice worth agonising over: a large title
            // belongs to a screen you scroll from the top of, and a pane is a
            // terminal or a diff you are already somewhere inside. It is also
            // what keeps the cost of this bar to one 44-point row, measured
            // above.
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                // One group rather than an item each, so the system spaces
                // them the way it spaces every other trailing group on this
                // platform instead of this file inventing a gap.
                ToolbarItemGroup(placement: .topBarTrailing) {
                    if let changes {
                        ChangesToolbarMenu(store: changes)
                    }
                    if let live, !live.isAgentPane, !live.isChangesPane {
                        imageMenu(live)
                    }
                    if hasOverflow {
                        overflowMenu
                    }
                }
            }
            // The picker hangs off the PANE rather than off the menu that
            // opens it, and now rather than off the bar either. A
            // `PhotosPicker` placed directly in a `Menu` renders as a row and
            // can never present, because menu content is not a live view
            // hierarchy to present FROM — the same note the pane host's
            // toolbar carried, and the reason this type wraps the pane instead
            // of being handed to `.toolbar` as a view.
            .photosPicker(isPresented: $showPhotoPicker, selection: $pickedImage, matching: .images)
            .onChange(of: pickedImage) { _, item in
                guard let item else { return }
                // No terminal to type a path into — only reachable if the pane
                // changed while the picker was up. Cleared rather than left
                // set, or picking the same photo again would not read as a
                // change and nothing would happen twice.
                guard let target = live?.id else {
                    pickedImage = nil
                    return
                }
                let core = connection.core
                Task {
                    // Loaded as data rather than as an `Image`: the picker
                    // hands back the original file, and re-encoding a
                    // screenshot through SwiftUI would smear the small text
                    // that is usually the whole reason someone is sending one.
                    guard let data = try? await item.loadTransferable(type: Data.self),
                        let image = UIImage(data: data)
                    else {
                        pickedImage = nil
                        return
                    }
                    pastes.send(image, terminal: target, core: core)
                    pickedImage = nil
                }
            }
            .confirmationDialog(
                "Remove worktree for \(removeCandidate?.task ?? "")?",
                isPresented: $confirmingRemove,
                titleVisibility: .visible
            ) {
                Button("Remove", role: .destructive) {
                    guard let ws = removeCandidate else { return }
                    Task {
                        switch await connection.removeWorktree(ws, confirm: "") {
                        case .ok:
                            break
                        case .confirmationRequired, .failed:
                            // The typed-name sheet also handles and displays a
                            // `.failed` result — route every non-.ok outcome
                            // there so there is one place this is shown, not
                            // two.
                            needsTypedConfirmation = ws
                        }
                    }
                }
                Button("Cancel", role: .cancel) {}
            }
            .sheet(item: $needsTypedConfirmation) { ws in
                RemoveWorktreeConfirmSheet(workspace: ws) { typed in
                    await connection.removeWorktree(ws, confirm: typed)
                }
            }
            .onChange(of: isVisible) { _, visible in
                if !visible { dismissEverything() }
            }
    }

    /// Every pane in the track stays MOUNTED when it is not on screen — that is
    /// the whole point of `ShellPaneTrack` — and a presentation is not part of
    /// the pane's own view, it is on the window. So a picker or a sheet left up
    /// by a pane that has gone would be a sheet belonging to a workspace nobody
    /// is looking at, and in the destructive case a typed-name confirmation for
    /// a worktree that is no longer the one on screen.
    ///
    /// In practice a swipe cannot happen while a sheet is up — the sheet has
    /// the touches. This exists for the paths that do not go through a finger:
    /// a deep link, a Live Activity tap, a workspace disappearing underneath
    /// the pane.
    private func dismissEverything() {
        showPhotoPicker = false
        pickedImage = nil
        confirmingRemove = false
        needsTypedConfirmation = nil
        removeCandidate = nil
    }

    /// An image, sent by typing its path into the tty.
    ///
    /// Plain-terminal only. `AgentView`'s composer has the agent-pane version
    /// and always has; this is the case the shell left with no door at all,
    /// even though `ImagePasteQueue` is still owned above the panes and
    /// `ImagePasteChips` is still drawn over them — every part of the transfer
    /// survived except the way to start one.
    ///
    /// No hand-set 44-point frame on the glyph, unlike the capsule this
    /// replaced: a toolbar item already has the platform's hit target, and
    /// forcing a frame on top of it is how a toolbar ends up with one button
    /// visibly larger than the one beside it.
    private func imageMenu(_ terminal: Terminal) -> some View {
        Menu {
            Button {
                showPhotoPicker = true
            } label: {
                Label("Choose Photo", systemImage: "photo")
            }
            if UIPasteboard.general.hasImages {
                Button {
                    if let image = UIPasteboard.general.image {
                        pastes.send(image, terminal: terminal.id, core: connection.core)
                    }
                } label: {
                    Label("Paste Image", systemImage: "doc.on.clipboard")
                }
            }
        } label: {
            Image(systemName: "photo.badge.plus")
        }
        .accessibilityLabel("Send an image")
    }

    /// Whether the overflow has anything in it. A `Menu` that opens onto
    /// nothing is worse than one that is not there.
    private var hasOverflow: Bool { canSwitchMode || canRemove }

    private var canSwitchMode: Bool {
        guard let live else { return false }
        // `|| isAgentPane`, and it is not redundant. `canSwitchPaneMode` is
        // `chatCapable`, which is a fact about whether a chat can be STARTED
        // here; a pane already in chat must be able to get back to its terminal
        // whatever that flag says. The Mac's own guard reads exactly this way —
        // see `ContentView.togglePaneMode` — and a phone that disagreed with it
        // would strand somebody in a chat on the device they reach for when
        // they are away from the Mac.
        return live.canSwitchPaneMode || live.isAgentPane
    }

    private var canRemove: Bool {
        guard let workspace else { return false }
        // Offering to remove the repository's own checkout would offer to
        // delete the directory the repository itself lives in.
        return !workspace.isPrimaryCheckout
    }

    private var overflowMenu: some View {
        Menu {
            if canSwitchMode, let live { paneModeItem(live) }
            if canRemove, let workspace {
                Divider()
                Button(role: .destructive) {
                    removeCandidate = workspace
                    confirmingRemove = true
                } label: {
                    Label("Remove Worktree…", systemImage: "trash")
                }
            }
        } label: {
            Image(systemName: "ellipsis")
        }
        .accessibilityLabel("Pane options")
    }

    /// Terminal or chat, on the pane that can be either.
    ///
    /// **This one was nearly not restored, and the argument is worth keeping.**
    /// The shell's model already treats an agent and a terminal as one thing
    /// with a flag — a tab is a tab, the ribbon draws them identically, and
    /// `ShellPaneRealView` picks a renderer off `isAgentPane` without anybody
    /// choosing. So the case for dropping this was that the model no longer
    /// has a terminal/chat DISTINCTION for a person to manage.
    ///
    /// It does not survive contact with what the flag actually is. The shell
    /// unified how a pane is REACHED; it did not give anybody a way to change
    /// what a pane IS, and those are different questions. An ACP adapter that
    /// will not surface a prompt, a permission the chat has no widget for, a
    /// TUI that wants a keystroke — every one of those is answered by looking
    /// at the tty, and the phone is precisely the device you are holding when
    /// the Mac is not in front of you. macOS has this on the pane and on ⌃B a;
    /// Android has it; iOS would have been the only client that could see an
    /// agent stuck and not look underneath it.
    ///
    /// In the OVERFLOW rather than on the bar, which is the part the old
    /// toolbar had wrong. The daemon respawns the pane to do this — a new
    /// epoch, and a refusal if a turn is in flight — so it is not a view
    /// toggle however much its old icon looked like one, and it does not
    /// belong one stray tap from a thumb resting near the top of the screen.
    ///
    /// **Known gap, and it is pre-existing:** `Connection.setPaneMode`
    /// swallows the daemon's answer with `try?`, so a refusal ("a turn is in
    /// flight") is silent here where the Mac shows it and offers to force. The
    /// phone has needed that since the call was written; giving the call a
    /// result type and a confirmation sheet is a change to `Connection` and to
    /// the wire's error handling, not to a bar, so it is not folded in here.
    private func paneModeItem(_ terminal: Terminal) -> some View {
        Button {
            Task {
                await connection.setPaneMode(
                    terminal, to: terminal.isAgentPane ? "terminal" : "agent")
            }
        } label: {
            Label(
                terminal.isAgentPane ? "Show the Terminal" : "Show the Chat",
                systemImage: terminal.isAgentPane
                    ? "terminal" : "bubble.left.and.text.bubble.right")
        }
    }
}
