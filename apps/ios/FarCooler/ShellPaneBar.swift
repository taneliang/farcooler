import PhotosUI
import SwiftUI

// The bar that belongs to a PANE, which is the platform's own navigation bar.
//
// ## Why there are two bars, and why only one of them is the worktree
//
// `ShellBar` at the bottom IS the worktree — its name, its ribbon, and the
// surface you drag for the next worktree or lift for the column. It is
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
//   changes worktree is at the bottom of the screen and not at the top. A
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
// worktree, and this pane"; it reads as two draggable bars, and the first
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
// Phase 3 took the app's single stack out. What is added here is one stack per
// PANE, inside the pane, so it travels with the pane on the track and no part
// of the shell — bar, track — is inside anybody's navigation. See
// `ShellScreen.ShellPaneRealView.body` for where it is mounted and what the
// pane's own safe area is fed from.
//
// ## What it costs the pane, said out loud
//
// A navigation bar reduces the safe area of what it is over, so a scroll view
// treats it as a CONTENT inset — the diff's cards travel behind the material
// and only come to REST below it. For the terminal the same inset is a real loss: a VT grid is not
// scrollable content, so rows that ran under the bar would be rows you cannot
// read, and the honest thing is for the grid to be a bar shorter.
//
// The measurement, because "a toolbar changes layout" is a trap: an inline
// navigation bar over a pane is **44 points**. The capsule this
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
/// **Remove worktree** — worktree-scoped, and it lives on this bar because the
/// moment you decide a worktree is finished with is the moment you are looking
/// at it.
///
/// **The card's menu has it too now**, which is the Mac's own arrangement —
/// the sidebar row and the worktree detail both carry it — and it is not two
/// behaviors: the ceremony moved into `RemoveWorktreeFlow` so both doors ask
/// for exactly the same confirmation. See `RemoveWorktreeConfirmSheet` for the
/// typed name itself, recovered rather than rewritten.
///
/// **Terminal ↔ chat** is here too, and that was the least obvious of the
/// five. See `paneModeItem`.
///
struct ShellPaneChromeModifier: ViewModifier {
    /// The tab's own title, straight off the shell's model.
    ///
    /// Not looked up again from the fleet, and that matters: the ribbon, the
    /// column and this bar are then three renderings of ONE string, so a
    /// terminal cannot be "claude 2" in the column and something else here.
    let title: String
    /// The runner this pane is on, for the task chip's route.
    let runner: UUID
    @ObservedObject var connection: Connection
    /// Images on their way into a terminal. Owned by `ShellScreen`, so a
    /// transfer started here keeps running when you swipe to another pane.
    @ObservedObject var pastes: ImagePasteQueue
    /// The worktree this pane belongs to, read live off the fleet.
    let worktree: Worktree?
    /// The terminal this pane IS, read live off the fleet — nil on a
    /// worktree's Diff tab, which has no pane on the runner behind it.
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
    /// A terminal this bar just made, handed up so the shell can land on it.
    ///
    /// The pane cannot move the shell itself — nothing inside a pane knows
    /// where it sits on the track — so `ShellScreen` owns the arrival. See
    /// `newTerminalItem` for why landing on it is part of the action rather
    /// than a nicety on top of it.
    let onCreated: (String) -> Void

    @Environment(\.phoneNavigator) private var navigator

    @State private var showPhotoPicker = false
    @State private var pickedImage: PhotosPickerItem?
    /// Where a removal started here has got to. The ceremony itself is
    /// `RemoveWorktreeFlow`.
    @State private var removing: RemoveWorktreeRequest?
    @State private var newTerminalFailure: NewTerminalFailure?
    /// The read-only Files browser, over this worktree (ov-259).
    @State private var browsingFiles = false
    /// A refused switch between the chat and the terminal.
    @State private var paneModeFailure: ActionFailure?

    /// Which sentence a refused New Terminal shows.
    ///
    /// The app's OWN two, chosen from `Connection.NewTerminalResult` — never a
    /// string from the runner. `Identifiable` so `.alert`'s `presenting:` form
    /// can carry it, which is what keeps the title and the body from being
    /// derived twice out of the same optional.
    private enum NewTerminalFailure: String, Identifiable {
        case disconnected
        case refused
        var id: String { rawValue }
    }

    func body(content: Content) -> some View {
        content
            .modifier(PhoneBackItem())
            .navigationTitle(title)
            // Inline, and not a choice worth agonising over: a large title
            // belongs to a screen you scroll from the top of, and a pane is a
            // terminal or a diff you are already somewhere inside. It is also
            // what keeps the cost of this bar to one 44-point row, measured
            // above.
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if let chip = taskChip {
                    ToolbarItem(placement: .principal) { chipTitle(chip) }
                }
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
            .removeWorktreeFlow($removing)
            .sheet(isPresented: $browsingFiles) {
                if let worktree {
                    FilesSheet(connection: connection, root: FilesLocation(place: .worktree(worktree.id)))
                }
            }
            // Far Cooler's own two sentences, and nothing the runner wrote.
            //
            // An alert rather than the `SheetFailureSection` the remove flow
            // uses, because there is no sheet: a menu item acts immediately,
            // so the only surface a refusal has is one the platform puts up
            // over the pane. What that costs is the `DetailBox` the sheet has
            // for a transcript, and nothing is lost by it — see
            // `Connection.NewTerminalResult`, which never carries one.
            //
            // The disconnected sentence does NOT claim nothing was made.
            // `WatchLinkHost.reason` is careful about exactly this and it is
            // the same situation: losing the link is losing the ANSWER, not
            // stopping the call, and a create that landed a moment later into
            // a dropped connection is a real tab that this side would be
            // telling somebody does not exist.
            .alert(
                newTerminalFailure == .disconnected
                    ? "Far Cooler lost this runner"
                    : "That runner wouldn’t open a terminal",
                isPresented: Binding(
                    get: { newTerminalFailure != nil },
                    set: { if !$0 { newTerminalFailure = nil } }),
                presenting: newTerminalFailure
            ) { _ in
                Button("OK", role: .cancel) {}
            } message: { failure in
                switch failure {
                case .disconnected:
                    Text(
                        "The connection dropped before it answered, so the terminal may or "
                            + "may not have been made. Its tabs will say which, once the "
                            + "runner’s back.")
                case .refused:
                    Text("It answered, and the answer was no. Nothing was created.")
                }
            }
            .actionFailureAlert($paneModeFailure)
            .onChange(of: isVisible) { _, visible in
                if !visible { dismissEverything() }
            }
    }

    /// The task this pane is shown under, by `TaskLink`'s rule, with the
    /// workspace its screen is on. On the Changes tab, which has no pane, the
    /// worktree's one open task. Nil for an orchestrator, and for a pane with
    /// no task or two.
    private var taskChip: (task: NeedsYouTask, workspace: String?)? {
        guard let worktree else { return nil }
        let id: String?
        if let live {
            id = TaskLink.task(of: live, in: worktree)
        } else {
            let open = worktree.openTaskIDs
            id = open.count == 1 ? open[0] : nil
        }
        guard let id, let task = worktree.openTasks?.first(where: { $0.id == id }) else {
            return nil
        }
        let workspace = live.flatMap { connection.fleet.workspace(of: $0, in: worktree) }
            ?? worktree.workspace
            ?? (connection.fleet.workspaces == nil ? worktree.repository : nil)
        return (task, workspace)
    }

    /// The pane's title with its task under it: "bil-9 Invoice PDF export",
    /// which opens the task (spec §3.2).
    private func chipTitle(_ chip: (task: NeedsYouTask, workspace: String?)) -> some View {
        VStack(spacing: 0) {
            Text(title)
                .font(.headline)
                .lineLimit(1)
            Button {
                guard let workspace = chip.workspace else { return }
                navigator?.open(
                    .task(
                        PhoneWorkspace(runner: runner.uuidString, workspace: workspace),
                        task: chip.task.id))
            } label: {
                HStack(spacing: 4) {
                    Text(chip.task.key).font(.caption.monospaced())
                    Text(chip.task.title).font(.caption).lineLimit(1)
                }
                .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .disabled(navigator == nil || chip.workspace == nil)
            .accessibilityLabel("Task \(chip.task.key), \(chip.task.title)")
            .accessibilityIdentifier("pane-task-chip")
        }
    }

    /// Every pane in the track stays MOUNTED when it is not on screen — that is
    /// the whole point of `ShellPaneTrack` — and a presentation is not part of
    /// the pane's own view, it is on the window. So a picker or a sheet left up
    /// by a pane that has gone would be a sheet belonging to a worktree nobody
    /// is looking at, and in the destructive case a typed-name confirmation for
    /// a worktree that is no longer the one on screen.
    ///
    /// In practice a swipe cannot happen while a sheet is up — the sheet has
    /// the touches. This exists for the paths that do not go through a finger:
    /// a deep link, a Live Activity tap, a worktree disappearing underneath
    /// the pane.
    private func dismissEverything() {
        showPhotoPicker = false
        pickedImage = nil
        removing = nil
        newTerminalFailure = nil
        browsingFiles = false
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
    private var hasOverflow: Bool { canCreateTerminal || canSwitchMode || canRemove || canBrowseFiles }

    /// Whether this worktree's files can be read from here: the runner serves
    /// them and this phone's grant may read them (`DaemonBuild.offersFiles`).
    /// Hidden otherwise, and until the runner has said what it can do.
    private var canBrowseFiles: Bool { worktree != nil && (connection.knownBuild?.offersFiles ?? false) }

    /// Every worktree can take another terminal — including this one when the
    /// pane in front of you is the Diff, which has no terminal behind it and is
    /// a perfectly ordinary place to decide you want one.
    private var canCreateTerminal: Bool { worktree != nil }

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
        guard let worktree else { return false }
        // Offering to remove the repository's own checkout would offer to
        // delete the directory the repository itself lives in.
        return !worktree.isPrimaryCheckout
    }

    private var overflowMenu: some View {
        Menu {
            if canCreateTerminal, let worktree { newTerminalItem(worktree) }
            if canSwitchMode, let live { paneModeItem(live) }
            if canBrowseFiles {
                Button {
                    browsingFiles = true
                } label: {
                    Label("Files", systemImage: "folder")
                }
                .accessibilityIdentifier("pane-files")
            }
            if canRemove, let worktree {
                Divider()
                Button(role: .destructive) {
                    removing = .confirming(worktree, on: connection)
                } label: {
                    Label("Remove Worktree…", systemImage: "trash")
                }
            }
        } label: {
            Image(systemName: "ellipsis")
        }
        .accessibilityLabel("More")
        // The identifier the UI suite finds it by, unchanged: an identifier
        // isn't spoken, and VoiceOver hears "More".
        .accessibilityIdentifier("Pane options")
    }

    /// Another terminal in this worktree.
    ///
    /// **The capability existed everywhere except here.** `terminal.create` has
    /// been a wire method since the protocol had one (`proto/farcooler.proto`,
    /// tag 24); the CLI, the Mac (⌘T, the sidebar, the palette, ⌃B c) and
    /// Android all call it, and `Connection.createTerminal` was already written
    /// on this side — with no caller. So a worktree on the phone could show
    /// its terminals, switch between them, scroll them and type into them, and
    /// the one thing you want first in a fresh worktree was the one thing there
    /// was no way to ask for. Nothing new crosses the wire for this; it is a
    /// door on a room that was already built.
    ///
    /// ## Why here and not in the column you drag up
    ///
    /// The obvious home looks like `ShellColumn`: it already lists this
    /// worktree's terminals and it is the surface you are on at the moment you
    /// realise you want another one. Three things in that column's own design
    /// say no, and each is load-bearing rather than a taste:
    ///
    /// - **A row there is chosen by RELEASING a finger.** `ShellRootView`'s bar
    ///   gesture maps the finger's height to a row every frame and
    ///   `ShellFleet.barRelease` commits whatever it is over — that is a
    ///   selection mechanism, and hanging a create off it means an overshot
    ///   drag makes a tmux window. Selections are free to be wrong; this is not.
    /// - **`tabCount` is the column's arithmetic, not a list length.**
    ///   `ShellGesture.columnFull` and `pageRise` are both written against it,
    ///   so a synthetic row moves the point where the page starts to rise by a
    ///   whole `rowHeight` and re-tunes a gesture `ShellNavigationTests` and
    ///   `ShellGestureTests` pin between them.
    /// - **One dot per tab, in two places.** The ribbon and the column share
    ///   marks through a `matchedGeometryEffect` keyed on `tab.id`; a row with
    ///   no tab behind it has no dot to fly, and inventing one would put a mark
    ///   in the ribbon for a terminal that does not exist. The column already
    ///   refuses a non-tab row on the same grounds — see its note on why there
    ///   is deliberately no worktree row in it.
    ///
    /// This bar is where the file header says a pane's capabilities go, and the
    /// bottom bar is the one surface nothing may be added to. A menu item is
    /// also simply the right shape: it is tapped on purpose, it can be titled,
    /// and it can put an alert up when the runner says no.
    ///
    /// ## Landing on it is part of the action
    ///
    /// `onCreated` hands the id up so the shell moves to the new tab, which is
    /// the Mac's own argument in `openTerminalInNewLayout`: you made a terminal
    /// because you want to type in it, and leaving the selection where it was
    /// means going and finding it. On a phone that is a drag and a release into
    /// a column, which is most of the cost of the thing you just did.
    private func newTerminalItem(_ worktree: Worktree) -> some View {
        Button {
            Task {
                switch await connection.createTerminal(in: worktree) {
                case .created(let id): onCreated(id)
                case .disconnected: newTerminalFailure = .disconnected
                case .refused: newTerminalFailure = .refused
                }
            }
        } label: {
            // "plus", the symbol the Mac's own New terminal carries in the
            // sidebar and in the palette. Two clients drawing one action with
            // two glyphs is the drift `ShellMarkView`'s header describes.
            Label("New Terminal", systemImage: "plus")
        }
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
    /// A refusal ("a turn is in flight") is said in an alert now (ov-179).
    /// The Mac also offers to force the switch; the phone only says it
    /// wouldn't.
    private func paneModeItem(_ terminal: Terminal) -> some View {
        Button {
            Task {
                paneModeFailure = await connection.setPaneMode(
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

/// Back, on a pane's own bar, when the worktree it's in was pushed onto the
/// phone's stack (`WorktreeScreen`). That screen hides the stack's bar, since
/// every pane has a bar of its own, so the way back has to be on this one.
struct PhoneBackItem: ViewModifier {
    @Environment(\.phoneBack) private var back

    func body(content: Content) -> some View {
        content.toolbar {
            if let back {
                ToolbarItem(placement: .topBarLeading) {
                    Button(action: back) {
                        Image(systemName: "chevron.backward")
                            .fontWeight(.semibold)
                    }
                    .accessibilityLabel("Back")
                    .accessibilityIdentifier("worktree-back")
                }
            }
        }
    }
}
