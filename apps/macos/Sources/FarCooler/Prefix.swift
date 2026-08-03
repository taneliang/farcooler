import AppKit
import SwiftUI

/// Everything tiling can be asked to do from the keyboard.
///
/// Shorter than it was, because the model underneath it got smaller. There is no
/// tile-all, un-tile, join or shift any more: those existed to manage membership
/// of a hand-built list, and a pane's membership is now simply which tmux window
/// it is in. What is left maps one-to-one onto `farcooler layout`, which is the
/// same set of verbs tmux itself has.
enum TileCommand: Equatable {
    case zoom
    case focusNext
    case focusPrevious
    case focus(TileDirection)
    case focusIndex(Int)
    case cycle
    case preset(TilePreset)
    case splitRight
    case splitDown
    case breakPane
    case closePane
    case newGroup
    case nextGroup
    case previousGroup
    /// Terminal ⟷ chat, for the focused pane. Not a layout verb — nothing
    /// about the arrangement changes — but it lives here anyway, with tmux's
    /// other pane-scoped bindings, because that is where the plan puts it:
    /// "no new permanent chrome on a pane" means this has to be a command
    /// reachable from the keyboard and the palette, not a button drawn on
    /// the pane itself.
    case toggleAgentPane
    case help

    static let notification = Notification.Name("farcooler.tile")

    func post() {
        NotificationCenter.default.post(name: Self.notification, object: Box(self))
    }

    /// A reference wrapper, because `NotificationCenter`'s object is `Any?` and
    /// an enum with associated values does not survive the round trip as one.
    final class Box: NSObject {
        let command: TileCommand
        init(_ command: TileCommand) { self.command = command }
    }
}

/// The tmux prefix, and what follows it.
///
/// A prefix key rather than a wall of ⌘-combinations, for two reasons. The first
/// is that the combinations are gone: a terminal wants nearly every ⌘ and ⌥
/// chord for itself, and the ones left are the ones nobody can remember. The
/// second is that a very large number of the people this is for already have
/// `⌃B z` in their fingers, and reusing it means the tiling has no learning
/// curve for them at all.
///
/// `⌃B ⌃B` sends a literal `⌃B` through to the program, exactly as tmux does, so
/// running tmux *inside* a Far Cooler pane still works. Without that this would
/// have quietly broken the one tool its own design is borrowed from.
@MainActor
final class PrefixMode: ObservableObject {
    static let shared = PrefixMode()

    /// Waiting for the second key. Published because the UI shows it — a prefix
    /// you cannot see you have pressed is a prefix you press twice.
    @Published private(set) var armed = false

    /// How many panes are on screen in the workspace being looked at.
    ///
    /// Set by the view, and it is what makes the prefix-less `⌃hjkl` bindings
    /// affordable. `⌃H` is backspace, `⌃J` is newline, `⌃K` kills to end of line
    /// and `⌃L` clears the screen — all four are real terminal keys, which is why
    /// tmux insists on a prefix in the first place. So they are only intercepted
    /// while there is somewhere to traverse TO: with one pane or none, every one
    /// of them passes straight through to the program, unchanged.
    var tiledPanes = 0

    /// What a keystroke turned out to be.
    enum Outcome {
        /// Swallowed: it armed the prefix, or it was a binding.
        case handled
        /// Not ours. The terminal should encode and send it as usual.
        case passThrough
    }

    private var disarm: Task<Void, Never>?

    /// A prefix left armed by a keystroke nobody followed up on is a trap: the
    /// next letter you type does something else. tmux can leave it armed forever
    /// because its prefix is never ambiguous with typing; here the same key is
    /// also how you get a literal one through.
    private static let armedTimeout = Duration.seconds(3)

    func handle(_ event: NSEvent) -> Outcome {
        // Direct traversal, before the prefix is considered: `⌃L` right is what
        // vim-tmux-navigator users have in their fingers, and having to reach for
        // a prefix to cross a boundary is the friction that binding exists to
        // remove. Only while tiled — see `tiledPanes`.
        if !armed, tiledPanes > 1, Preferences.shared.directTraversal,
            let direction = Self.traversal(event)
        {
            TileCommand.focus(direction).post()
            return .handled
        }

        if armed {
            setArmed(false)
            // The prefix again: tmux's escape hatch. Let it through so the
            // terminal encodes and sends a real ⌃B, which is what makes running
            // tmux inside a pane possible.
            if isPrefix(event) { return .passThrough }
            guard let command = binding(for: event) else {
                // An unbound key after the prefix does nothing, rather than
                // being typed into the agent. Typing `zsh` and having the `z`
                // eaten and `sh` run is worse than a keystroke going missing.
                NSSound.beep()
                return .handled
            }
            command.post()
            return .handled
        }

        guard isPrefix(event) else { return .passThrough }
        setArmed(true)
        return .handled
    }

    /// Cancel the armed state — on Esc, or when focus leaves.
    func cancel() {
        if armed { setArmed(false) }
    }

    private func setArmed(_ value: Bool) {
        armed = value
        disarm?.cancel()
        guard value else { return }
        disarm = Task { [weak self] in
            try? await Task.sleep(for: Self.armedTimeout)
            guard !Task.isCancelled else { return }
            self?.armed = false
        }
    }

    /// `⌃h`, `⌃j`, `⌃k`, `⌃l` and nothing else.
    ///
    /// Control alone: `⇧⌃L` and `⌥⌃L` are left for whatever wants them, and a
    /// prefix-less binding that swallowed every modifier combination would be
    /// taking far more than it was asked for.
    private static func traversal(_ event: NSEvent) -> TileDirection? {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard flags == .control else { return nil }
        switch event.charactersIgnoringModifiers?.lowercased() {
        case "h": return .left
        case "j": return .bottom
        case "k": return .top
        case "l": return .right
        default: return nil
        }
    }

    private func isPrefix(_ event: NSEvent) -> Bool {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard flags == .control else { return false }
        // Compared against the unmodified character, because ⌃B arrives with
        // `characters` set to the control code 0x02 rather than to "b".
        return event.charactersIgnoringModifiers?.lowercased() == Preferences.shared.prefixKey
    }

    /// The bindings, in tmux's spellings.
    private func binding(for event: NSEvent) -> TileCommand? {
        // Arrows first: they have no character, only a key code.
        switch event.keyCode {
        case 123: return .focus(.left)
        case 124: return .focus(.right)
        case 126: return .focus(.top)
        case 125: return .focus(.bottom)
        case 53: return nil  // Esc: cancel, already disarmed above
        default: break
        }

        guard let key = event.charactersIgnoringModifiers, let first = key.first else { return nil }
        let shifted = event.modifierFlags.contains(.shift)

        if let digit = first.wholeNumberValue, (1...9).contains(digit), !shifted {
            return .focusIndex(digit)
        }

        switch first {
        case " ": return .cycle
        case "z": return .zoom
        case "a": return .toggleAgentPane
        case "o": return .focusNext
        case ";": return .focusPrevious
        case "h": return .focus(.left)
        case "l": return .focus(.right)
        case "k": return .focus(.top)
        case "j": return .focus(.bottom)
        // tmux's split bindings, and they mean the same thing here: a new pane
        // beside the one you are looking at, in the layout you are looking at.
        case "%": return .splitRight
        case "\"": return .splitDown
        // tmux's break-pane. There is no matching join binding any more: a pane is
        // in a layout by being in that tmux window, so the way to put one beside
        // another is to say which one — a drag, or `layout move`. `⌃B a` used to
        // mean "bring this into the current layout", which could only ever append
        // it to the end and never say where.
        case "!": return .breakPane
        case "x": return .closePane
        case "c": return .newGroup
        case "n": return .nextGroup
        case "p": return .previousGroup
        case "?": return .help
        default: return nil
        }
    }
}

extension View {
    /// The prefix hint, over any terminal.
    ///
    /// On the tiled view AND the single one. `⌃B` has always worked from a terminal
    /// that is not tiled — `⌃B t` is how you tile in the first place — but the hint
    /// only rendered inside the tiled view, so from a solo terminal the prefix
    /// appeared to do nothing at all.
    func prefixHint() -> some View {
        modifier(PrefixHintOverlay())
    }

    func onTileCommand(_ perform: @escaping (TileCommand) -> Void) -> some View {
        onReceive(NotificationCenter.default.publisher(for: TileCommand.notification)) { note in
            guard let box = note.object as? TileCommand.Box else { return }
            perform(box.command)
        }
    }
}

/// What the prefix is waiting for, shown while it waits.
///
/// This is the discoverability story for a prefix key, and it is a better one
/// than a cheat sheet: the bindings appear at the moment they are usable and
/// vanish when they are not, so nobody has to remember that a list exists. It is
/// also how you learn a keystroke you were not reaching for.
///
/// It drops bindings rather than wrapping. A hint that reflows into three lines
/// of half-words is worse than a shorter hint: unreadable in the second you have
/// to read it, and it shoves the terminal around while you do.
struct PrefixHint: View {
    /// Longest first. `ViewThatFits` takes the first that does.
    private static let tiers: [[(String, String)]] = [
        [
            // Arrows work after the prefix, but this slot advertises the
            // prefix-LESS movement instead: it is the binding used most and the
            // one least likely to be guessed.
            ("z", "zoom"), ("o", "next"), ("\u{2303}hjkl", "move"),
            ("space", "layout"), ("%", "split"), ("\"", "split down"), ("!", "pop out"),
            ("a", "chat"), ("c", "new"), ("?", "all keys"),
        ],
        [
            ("z", "zoom"), ("o", "next"), ("space", "layout"), ("%", "split"),
            ("?", "all keys"),
        ],
        [("z", "zoom"), ("space", "layout"), ("?", "all keys")],
        [("?", "keys")],
    ]

    var body: some View {
        ViewThatFits(in: .horizontal) {
            ForEach(Array(Self.tiers.enumerated()), id: \.offset) { _, tier in
                row(tier)
            }
        }
    }

    private func row(_ bindings: [(String, String)]) -> some View {
        HStack(spacing: 11) {
            Text("\u{2303}\(Preferences.shared.prefixKey.uppercased())")
                .font(.system(size: 11, weight: .semibold, design: .monospaced))
                .foregroundStyle(.tint)

            ForEach(bindings, id: \.0) { key, label in
                HStack(spacing: 4) {
                    Text(key)
                        .font(.system(size: 10, weight: .medium, design: .monospaced))
                    Text(label)
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                }
            }
        }
        .lineLimit(1)
        .fixedSize()
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .background(.regularMaterial, in: Capsule())
        .overlay(Capsule().strokeBorder(Color.primary.opacity(0.08)))
        .shadow(color: .black.opacity(0.15), radius: 12, y: 4)
    }
}


/// Shows what the armed prefix is waiting for.
private struct PrefixHintOverlay: ViewModifier {
    @ObservedObject private var prefix = PrefixMode.shared

    func body(content: Content) -> some View {
        content
            .overlay(alignment: .bottom) {
                if prefix.armed {
                    PrefixHint()
                        .padding(.bottom, 14)
                        .transition(
                            .opacity.combined(with: .move(edge: .bottom))
                                .combined(with: .scale(scale: 0.96)))
                }
            }
            // A chip arriving on a keystroke, so the bouncier preset.
            .animation(.snappy(duration: 0.22, extraBounce: 0.08), value: prefix.armed)
    }
}
