import SwiftUI

#if os(macOS)
import AppKit
#endif

// A task key's card (ov-299): the Mac's hovercard and the iPhone's long-press
// preview draw this one view, from `TaskKeyCard`.

/// The card: the title first and largest, then status, theme and lane on one
/// line, then the latest note or the intent, a few lines at most.
public struct TaskKeyCardView: View {
    public let card: TaskKeyCard

    public init(card: TaskKeyCard) {
        self.card = card
    }

    /// How wide the card is drawn: room for a title on two lines and an
    /// excerpt on three, narrow enough not to cover the text it's about.
    public static let width: CGFloat = 300

    public var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(card.title)
                .font(.headline)
                .lineLimit(3)
                .fixedSize(horizontal: false, vertical: true)
            Text("\(card.key) · \(card.details)")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
            if !card.excerpt.isEmpty {
                Text(card.excerpt)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 2)
            }
        }
        .frame(width: Self.width, alignment: .leading)
        .padding(14)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("task-key-card-\(card.key)")
    }
}

/// How long the pointer rests on a key before its card shows: the system's
/// tooltip delay, which a person can set (`NSInitialToolTipDelay`, in
/// milliseconds), and AppKit's one second when they haven't.
public enum TaskKeyHoverDelay {
    public static let fallback: Duration = .seconds(1)

    public static func current(_ defaults: UserDefaults = .standard) -> Duration {
        let ms = defaults.integer(forKey: "NSInitialToolTipDelay")
        return ms > 0 ? .milliseconds(ms) : fallback
    }
}

/// For the Mac's capture harness: the key whose card shows as if the pointer
/// had rested on it, with no input sent (ui-lane-common, "Captures never send
/// input"). Nil everywhere but a capture. Observed, so text already on screen
/// shows the card when a capture names the key.
@MainActor
@Observable
public final class TaskKeyHoverForcing {
    public static let shared = TaskKeyHoverForcing()

    public static var key: String? {
        get { shared.key }
        set { shared.key = newValue }
    }

    var key: String? {
        didSet { claimed = false }
    }
    /// Whether a place drawing `key` has shown its card: only the first
    /// does, as one pointer rests on one place.
    @ObservationIgnored var claimed = false

    private init() {}
}

extension View {
    /// A key drawn on its own, as a row's key column or a plan row's cards:
    /// its card on hover (the Mac, after the system's delay, gone on
    /// move-out) or as a long press's preview with Open (the iPhone), and
    /// the title in its accessibility label. Nothing changes for a key the
    /// linker has no card for, or for nil.
    ///
    /// `speaksTitle` false leaves the label alone, for a key drawn beside
    /// its own title, which VoiceOver already reads.
    public func taskKeyCard(_ key: String?, speaksTitle: Bool = true) -> some View {
        modifier(TaskKeyCardModifier(key: key, speaksTitle: speaksTitle))
    }
}

private struct TaskKeyCardModifier: ViewModifier {
    let key: String?
    let speaksTitle: Bool
    @Environment(\.taskKeyLinker) private var linker

    func body(content: Content) -> some View {
        if let card = key.flatMap(linker.card(forKey:)) {
            #if os(macOS)
            content
                .modifier(TaskKeyHovercard(card: card))
                .modifier(SpokenTitle(card: card, on: speaksTitle))
            #elseif os(iOS)
            content
                .contextMenu {
                    Button("Open", systemImage: "arrow.up.forward.square") { linker.open(key: card.key) }
                } preview: {
                    TaskKeyCardView(card: card)
                }
                .modifier(SpokenTitle(card: card, on: speaksTitle))
            #else
            content
            #endif
        } else {
            content
        }
    }
}

/// The key's label with its title, "ov-190, Fix the login", when `on`.
private struct SpokenTitle: ViewModifier {
    let card: TaskKeyCard
    let on: Bool

    func body(content: Content) -> some View {
        if on { content.accessibilityLabel(card.accessibilityLabel) } else { content }
    }
}

#if os(macOS)
/// A popover with `card` after the pointer has rested on the view for the
/// system's hover delay; it closes when the pointer leaves.
struct TaskKeyHovercard: ViewModifier {
    let card: TaskKeyCard
    @State private var shown = false
    @State private var waiting: Task<Void, Never>?

    func body(content: Content) -> some View {
        content
            .onHover { inside in
                waiting?.cancel()
                guard inside else {
                    shown = false
                    return
                }
                waiting = Task { @MainActor in
                    try? await Task.sleep(for: TaskKeyHoverDelay.current())
                    if !Task.isCancelled { shown = true }
                }
            }
            .onDisappear { waiting?.cancel() }
            .popover(isPresented: $shown, arrowEdge: .bottom) {
                TaskKeyCardView(card: card)
            }
    }
}
#endif
