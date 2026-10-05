import SwiftUI

// Task keys inside running text (ov-299): a note's, an intent's, a page's or an
// agent's reply, where `TaskKeyLinker.linked` made each key a link. SwiftUI's
// `Text` says nothing about where in it the pointer is, so on the Mac each key
// is drawn as its own run, marked with `TaskKeyRun`, and a `TextRenderer` that
// draws the text exactly as `Text` would also notes where each marked run
// landed. A hover over one of those rects shows the key's card.

/// Text whose task keys show their cards: on the Mac, a hovercard; on the
/// iPhone, the text as it was (a link in running text keeps the system's own
/// long press). Either way each known key's title is in what VoiceOver says.
public struct TaskKeyText: View {
    public let text: AttributedString
    /// Whether `text` was linked already, or is plain text whose keys show
    /// cards without becoming links.
    let links: Bool
    @Environment(\.taskKeyLinker) private var linker

    /// `text` as `TaskKeyLinker.linked` returns it: its keys are links.
    public init(_ text: AttributedString) {
        self.text = text
        links = true
    }

    /// Plain text, as a plan row's reason: its keys show their cards, and
    /// stay drawn as the text around them, since the row is the button.
    public init(keysIn text: String) {
        self.text = AttributedString(text)
        links = false
    }

    public var body: some View {
        let linked = links ? text : linker.linked(text)
        let spoken = linker.spoken(linked)
        #if os(macOS)
        let drawn = TaskKeyHoverText(text: linked, linker: linker, links: links)
        #else
        let drawn = Text(text)
        #endif
        if let spoken {
            drawn.accessibilityLabel(spoken)
        } else {
            drawn
        }
    }
}

/// The key a run of text is, marked on its own `Text` so a renderer can find
/// it in the laid-out lines.
struct TaskKeyRun: TextAttribute {
    var key: String
}

extension TaskKeyLinker {
    /// `linked` as one `Text`, each key this linker has a card for in its own
    /// run marked `TaskKeyRun`. The same characters and attributes as
    /// `Text(linked)`; only the marks are new.
    ///
    /// With `links` false, the keys' links are dropped as they're drawn: the
    /// text looks as it did, and only the hovercard knows where they are.
    func markedText(_ linked: AttributedString, links: Bool = true) -> (text: Text, keys: Int) {
        var out: Text?
        var keys = 0
        func append(_ next: Text) { out = out.map { Text("\($0)\(next)") } ?? next }
        for run in linked.runs {
            var slice = AttributedString(linked[run.range])
            if let url = run.link, let card = card(for: url) {
                if !links { slice.link = nil }
                append(Text(slice).customAttribute(TaskKeyRun(key: card.key)))
                keys += 1
            } else {
                append(Text(slice))
            }
        }
        return (out ?? Text(""), keys)
    }

    /// What VoiceOver says for `linked`: each key with a card followed by its
    /// title, "ov-190 (Fix the login)", or nil when no key has one.
    public func spoken(_ linked: AttributedString) -> String? {
        var out = ""
        var any = false
        for run in linked.runs {
            out += String(linked[run.range].characters)
            if let url = run.link, let card = card(for: url) {
                out += " (\(card.title))"
                any = true
            }
        }
        return any ? out : nil
    }
}

/// Where each marked key was drawn, in the text's own coordinates. A class,
/// written while drawing and read on hover, so drawing never changes the
/// view's state.
final class TaskKeyRects: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: [(key: String, rect: CGRect)] = []

    var all: [(key: String, rect: CGRect)] {
        lock.withLock { stored }
    }

    func set(_ rects: [(key: String, rect: CGRect)]) {
        lock.withLock { stored = rects }
    }

    /// The key drawn under `point`, and the rect of the run it's in.
    func key(at point: CGPoint) -> (key: String, rect: CGRect)? {
        all.first { $0.rect.contains(point) }
    }
}

/// Draws text as `Text` does, noting where each `TaskKeyRun` landed.
struct TaskKeyRecorder: TextRenderer {
    let rects: TaskKeyRects

    func draw(layout: Text.Layout, in context: inout GraphicsContext) {
        var found: [(key: String, rect: CGRect)] = []
        for line in layout {
            for run in line {
                if let mark = run[TaskKeyRun.self] {
                    found.append((mark.key, run.typographicBounds.rect))
                }
            }
            context.draw(line)
        }
        rects.set(found)
    }
}

#if os(macOS)
/// The Mac's linked text: the hovercard of the key under the pointer, after
/// the system's hover delay, closed when the pointer leaves the key.
struct TaskKeyHoverText: View {
    let text: AttributedString
    let linker: TaskKeyLinker
    var links = true
    @State private var rects = TaskKeyRects()
    @State private var hovered: (key: String, rect: CGRect)?
    @State private var shown: TaskKeyCard?
    @State private var waiting: Task<Void, Never>?

    var body: some View {
        let marked = linker.markedText(text, links: links)
        if marked.keys == 0 {
            Text(links ? text : AttributedString(text.characters))
        } else {
            marked.text
                .textRenderer(TaskKeyRecorder(rects: rects))
                .onContinuousHover(coordinateSpace: .local) { phase in
                    switch phase {
                    case let .active(point): hover(rects.key(at: point))
                    case .ended: hover(nil)
                    }
                }
                .onDisappear { waiting?.cancel() }
                .task(id: TaskKeyHoverForcing.shared.key) { await forced() }
                .popover(
                    isPresented: Binding(get: { shown != nil }, set: { if !$0 { shown = nil } }),
                    attachmentAnchor: .rect(.rect(hovered?.rect ?? .zero)), arrowEdge: .bottom
                ) {
                    if let shown { TaskKeyCardView(card: shown) }
                }
        }
    }

    /// The capture harness's key, shown once the text has been drawn and
    /// the key's place is known.
    private func forced() async {
        guard let key = TaskKeyHoverForcing.key, let card = linker.card(forKey: key) else { return }
        for _ in 0..<20 {
            if let found = rects.all.first(where: { $0.key == key }) {
                guard !TaskKeyHoverForcing.shared.claimed else { return }
                TaskKeyHoverForcing.shared.claimed = true
                hovered = found
                shown = card
                return
            }
            try? await Task.sleep(for: .milliseconds(100))
        }
    }

    /// The pointer over `found`, or over no key. Moving within one key keeps
    /// its card; moving to another, or off, starts over.
    private func hover(_ found: (key: String, rect: CGRect)?) {
        guard found?.key != hovered?.key || found == nil else { return }
        waiting?.cancel()
        hovered = found
        shown = nil
        guard let found, let card = linker.card(forKey: found.key) else { return }
        waiting = Task { @MainActor in
            try? await Task.sleep(for: TaskKeyHoverDelay.current())
            if !Task.isCancelled, hovered?.key == found.key { shown = card }
        }
    }
}
#endif
