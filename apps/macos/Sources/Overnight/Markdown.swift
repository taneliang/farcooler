import SwiftUI

/// Markdown, rendered as blocks rather than as one run of text.
///
/// `AttributedString(markdown:)` handles inline syntax — bold, italic, code
/// spans, links — and nothing else. Headings arrive as plain text, list items
/// lose their bullets, and every block is concatenated into a single
/// paragraph, which is why a reply full of structure rendered as one long
/// line with some words emphasised.
///
/// So blocks are split here and each is drawn as itself; inline parsing is
/// still `AttributedString`'s job, which it does well. This is not a complete
/// CommonMark implementation and is not trying to be — it covers what a chat
/// reply actually contains.
enum Markdown {
    /// One renderable piece of a reply.
    enum Block: Equatable {
        case paragraph(String)
        case heading(level: Int, text: String)
        case bullet(text: String, depth: Int)
        case numbered(number: String, text: String, depth: Int)
        case code(text: String, language: String)
        case quote(String)
        case rule
    }

    /// Split markdown into blocks.
    ///
    /// Line-based, because the structures that matter here are line-based. A
    /// fenced code block swallows everything until its closing fence, so its
    /// contents are never mistaken for markup.
    static func blocks(_ text: String) -> [Block] {
        var blocks: [Block] = []
        var paragraph: [String] = []

        func flushParagraph() {
            // Joined with a NEWLINE, not a space.
            //
            // CommonMark folds a single line break into a space and needs two
            // to make one. That rule exists for hand-written source files, and
            // Claude does not write to it — it breaks lines where it means
            // them to break. Honouring that is the difference between a
            // readable reply and one run-on paragraph.
            let joined = paragraph.joined(separator: "\n").trimmingCharacters(in: .whitespaces)
            if !joined.isEmpty { blocks.append(.paragraph(joined)) }
            paragraph = []
        }

        var lines = text.components(separatedBy: .newlines)[...]
        while let line = lines.first {
            lines = lines.dropFirst()
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            // A fence takes precedence over everything: its contents are not
            // markup and must not be read as any.
            if trimmed.hasPrefix("```") {
                flushParagraph()
                let language = String(trimmed.dropFirst(3)).trimmingCharacters(in: .whitespaces)
                var body: [String] = []
                while let next = lines.first {
                    lines = lines.dropFirst()
                    if next.trimmingCharacters(in: .whitespaces).hasPrefix("```") { break }
                    body.append(next)
                }
                blocks.append(.code(text: body.joined(separator: "\n"), language: language))
                continue
            }

            if trimmed.isEmpty {
                flushParagraph()
                continue
            }

            if trimmed == "---" || trimmed == "***" || trimmed == "___" {
                flushParagraph()
                blocks.append(.rule)
                continue
            }

            if let hashes = trimmed.firstIndex(where: { $0 != "#" }),
                trimmed.starts(with: "#"),
                trimmed[hashes] == " "
            {
                flushParagraph()
                let level = trimmed.distance(from: trimmed.startIndex, to: hashes)
                let body = String(trimmed[hashes...]).trimmingCharacters(in: .whitespaces)
                blocks.append(.heading(level: min(level, 3), text: body))
                continue
            }

            if trimmed.hasPrefix("> ") {
                flushParagraph()
                blocks.append(.quote(String(trimmed.dropFirst(2))))
                continue
            }

            // Indentation is what nests a list, so it is measured before the
            // marker is stripped.
            let indent = line.prefix(while: { $0 == " " || $0 == "\t" }).count
            let depth = min(indent / 2, 3)

            if let marker = ["- ", "* ", "+ "].first(where: { trimmed.hasPrefix($0) }) {
                flushParagraph()
                blocks.append(.bullet(text: String(trimmed.dropFirst(marker.count)), depth: depth))
                continue
            }

            if let dot = trimmed.firstIndex(of: "."),
                trimmed[trimmed.startIndex..<dot].allSatisfy(\.isNumber),
                trimmed.index(after: dot) < trimmed.endIndex,
                trimmed[trimmed.index(after: dot)] == " "
            {
                flushParagraph()
                let number = String(trimmed[trimmed.startIndex..<dot])
                let body = String(trimmed[trimmed.index(dot, offsetBy: 2)...])
                blocks.append(.numbered(number: number, text: body, depth: depth))
                continue
            }

            paragraph.append(trimmed)
        }
        flushParagraph()
        return blocks
    }

    /// Inline syntax only — bold, italic, code spans, links.
    ///
    /// Text that cannot be parsed is returned as itself rather than dropped: a
    /// stray bracket should cost a reader the emphasis, not the sentence.
    static func inline(_ text: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(
            allowsExtendedAttributes: true,
            interpretedSyntax: .inlineOnlyPreservingWhitespace,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }
}

/// A rendered markdown reply.
struct MarkdownText: View {
    let text: String
    /// Reasoning is set smaller and dimmer than a reply, but is otherwise the
    /// same markdown — agents write lists and code in their thinking too.
    var secondary: Bool = false

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            ForEach(Array(Markdown.blocks(text).enumerated()), id: \.offset) { _, block in
                view(for: block)
            }
        }
        .font(secondary ? .caption : .body)
        .foregroundStyle(secondary ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private func view(for block: Markdown.Block) -> some View {
        switch block {
        case let .paragraph(text):
            Text(Markdown.inline(text))
                .fixedSize(horizontal: false, vertical: true)

        case let .heading(level, text):
            Text(Markdown.inline(text))
                .font(headingFont(level))
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 2)

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
                Text(Markdown.inline(text))
                    .fixedSize(horizontal: false, vertical: true)
            }

        case .rule:
            Divider()
        }
    }

    /// A marker and its text, aligned so a wrapped line does not slide back
    /// under the bullet.
    private func marker(_ symbol: String, text: String, depth: Int) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(symbol)
                .foregroundStyle(.secondary)
                .frame(minWidth: 14, alignment: .trailing)
            Text(Markdown.inline(text))
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.leading, CGFloat(depth) * 14)
    }

    private func headingFont(_ level: Int) -> Font {
        switch level {
        case 1: secondary ? .callout.bold() : .title3.bold()
        case 2: secondary ? .caption.bold() : .headline
        default: secondary ? .caption.bold() : .subheadline.bold()
        }
    }
}

/// The app's motion, in one place.
///
/// Springs rather than timed curves, and short ones. A disclosure that takes
/// a third of a second to open reads as the app thinking about it; the whole
/// point of a fold is that it costs nothing to look.
enum Motion {
    /// Opening, closing, and anything else that should feel instant.
    static let snap = Animation.spring(response: 0.22, dampingFraction: 0.82)
    /// A little overshoot, for something arriving rather than resizing.
    static let arrive = Animation.spring(response: 0.26, dampingFraction: 0.7)
}
