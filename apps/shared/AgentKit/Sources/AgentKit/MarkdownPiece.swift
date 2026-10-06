import SwiftUI

// One run of a reply, drawn as its own view so a delta redraws only the run it
// changed (ov-382).
//
// `MarkdownText` used to draw every run from functions inside its own `body`,
// so each `body` pass rebuilt every run's attributed text and handed SwiftUI a
// new `Text` for each, and a streaming reply's row runs that `body` on every
// poll. At 25.6K characters that was 97% of the main thread (ov-358 §3). Each
// run is now a value SwiftUI compares before it draws: an unchanged run keeps
// its laid-out text and its TextKit selection state, and only the paragraph
// being written is drawn again.

extension Markdown {
    /// `runs`, for a reply still being written: the last prose run split into
    /// one run per paragraph.
    ///
    /// A delta lands at the end of the reply. Gathered into one prose run, it
    /// changes the whole run's `Text`, which is then measured and laid out
    /// again from its first character, however long the run is. Split, it
    /// changes only the paragraph it lands in.
    ///
    /// Only the last run, because it's the only one a delta can reach: an
    /// earlier run ended where a block began, and appending can't change it.
    /// So the runs before it are the same values, in the same places, as the
    /// settled reply's, and settling redraws only this one.
    public static func streamingRuns(_ runs: [Run]) -> [Run] {
        guard case let .prose(paragraphs)? = runs.last, paragraphs.count > 1 else { return runs }
        return runs.dropLast() + paragraphs.map { .prose([$0]) }
    }

    /// `MarkdownText.merged`, computed once per distinct run of paragraphs:
    /// a run is built again whenever its row is realized, and a long
    /// reply's merged text is the costliest thing in it to build.
    @MainActor
    static let mergedCache = RenderMemo<[String], AttributedString>(limit: 200)
}

/// One run of a Markdown view: a prose run, or one block.
///
/// Compared, not just rebuilt (`MarkdownText` draws it `.equatable()`): its
/// inputs are its run and how it's set, and the same inputs draw the same
/// thing. What it reads from the environment, the linker and the type size,
/// still redraws it when that changes.
struct MarkdownPiece: View, Equatable {
    let run: Markdown.Run
    var secondary = false
    /// Still being written: the last paragraph of a reply that's streaming.
    /// Drawn without text selection, and its text isn't memoized.
    ///
    /// Selection on a `Text` keeps a TextKit layout of it beside the drawn
    /// one, built again whenever the text changes, which a paragraph being
    /// written does five times a second. What's selectable while a reply
    /// streams is every paragraph before it.
    var open = false
    /// Whether its text goes through `Markdown.mergedCache`. Not for a
    /// streaming reply's pieces (ov-382 review): a paragraph of one is
    /// gathered into a different run once it settles, so its entry would
    /// never be read again, and a long stream's worth of them evicts the
    /// settled rows a reader is looking at. Left out of `==`: it changes
    /// where the text is kept, not what's drawn, so settling, which turns it
    /// on, redraws nothing for it.
    var memo = true
    /// The space above it: the gap after the run before it.
    var gap: CGFloat = 0

    @Environment(\.taskKeyLinker) private var linker

    @ScaledMetric(relativeTo: .body)
    private var h1Size = MarkdownTypeScale.h1
    @ScaledMetric(relativeTo: .body)
    private var h2Size = MarkdownTypeScale.h2
    @ScaledMetric(relativeTo: .body)
    private var h3Size = MarkdownTypeScale.h3

    nonisolated static func == (a: MarkdownPiece, b: MarkdownPiece) -> Bool {
        a.run == b.run && a.secondary == b.secondary && a.open == b.open && a.gap == b.gap
    }

    #if DEBUG
    /// The bytes of text whose piece's `body` has run: what
    /// `StreamingReplyPerfTests` measures a delta's redraw in. Debug builds
    /// only, which is what tests run, so a release body counts nothing.
    @MainActor static var drawnBytes = 0
    #endif

    var body: some View {
        #if DEBUG
        let _ = Self.drawnBytes += bytes
        #endif
        Group {
            if open {
                content
            } else {
                content.textSelection(.enabled)
            }
        }
        .padding(.top, gap)
    }

    @ViewBuilder
    private var content: some View {
        switch run {
        case let .prose(paragraphs):
            TaskKeyText(linker.linked(merged(paragraphs)))
                .fixedSize(horizontal: false, vertical: true)
                .lineSpacing(3)
        case let .block(block):
            view(for: block)
        }
    }

    #if DEBUG
    /// Its text's size, in UTF-8 bytes.
    private var bytes: Int {
        switch run {
        case let .prose(paragraphs): paragraphs.reduce(0) { $0 + $1.utf8.count }
        case let .block(block):
            switch block {
            case let .paragraph(text), let .heading(_, text), let .bullet(text, _), let .numbered(_, text, _),
                let .code(text, _), let .quote(text):
                text.utf8.count
            case .rule: 0
            case let .table(header, rows): (header + rows.joined()).reduce(0) { $0 + $1.utf8.count }
            }
        }
    }
    #endif

    private func merged(_ paragraphs: [String]) -> AttributedString {
        if open || !memo { return MarkdownText.merged(paragraphs) }
        return Markdown.mergedCache.value(for: paragraphs) { MarkdownText.merged($0) }
    }

    @ViewBuilder
    private func view(for block: Markdown.Block) -> some View {
        switch block {
        case let .paragraph(text):
            TaskKeyText(linker.linked(Markdown.inline(text)))
                .fixedSize(horizontal: false, vertical: true)
                .lineSpacing(3)

        case let .heading(level, text):
            TaskKeyText(linker.linked(Markdown.inline(text)))
                .font(headingFont(level))
                .fixedSize(horizontal: false, vertical: true)
                .lineSpacing(2)

        case let .bullet(text, depth):
            marker("•", text: text, depth: depth)

        case let .numbered(number, text, depth):
            marker("\(number).", text: text, depth: depth)

        case let .code(text, _):
            // The same box a tool's output gets: one way of showing
            // monospaced text, not two.
            DetailBox(text: text)

        case let .quote(text):
            HStack(spacing: 8) {
                Rectangle().fill(.quaternary).frame(width: 2)
                TaskKeyText(linker.linked(Markdown.inline(text)))
                    .fixedSize(horizontal: false, vertical: true)
                    .lineSpacing(3)
            }

        case .rule:
            Divider()  // style-exempt: a Markdown rule is the author's content

        case let .table(header, rows):
            MarkdownTable(header: header, rows: rows)
        }
    }

    /// A marker and its text, aligned so a wrapped line does not slide back
    /// under the bullet.
    private func marker(_ symbol: String, text: String, depth: Int) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(symbol)
                .foregroundStyle(.secondary)
                .frame(minWidth: 14, alignment: .trailing)
            TaskKeyText(linker.linked(Markdown.inline(text)))
                .fixedSize(horizontal: false, vertical: true)
                .lineSpacing(3)
            Spacer(minLength: 0)
        }
        .padding(.leading, CGFloat(depth) * 14)
    }

    private func headingFont(_ level: Int) -> Font {
        // A heading must be distinct from both body and inline bold. Semantic
        // `.headline` is the same point size as body on Apple platforms, so a
        // weight-only H3 looked exactly like a bold phrase. This small custom
        // ramp keeps all three levels legible without turning chat into a title
        // page; `@ScaledMetric` preserves accessibility scaling.
        if secondary {
            switch level {
            case 1, 2: return .caption.weight(.semibold)
            default: return .caption.weight(.medium)
            }
        }

        switch level {
        case 1: return .system(size: h1Size, weight: .semibold)
        case 2: return .system(size: h2Size, weight: .semibold)
        default: return .system(size: h3Size, weight: .semibold)
        }
    }
}
