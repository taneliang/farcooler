import Foundation

public enum ComposerToken: Sendable, Equatable {
    case none
    case slash(prefix: String, range: Range<String.Index>)
    case mention(prefix: String, range: Range<String.Index>)
}

/// What the caret is currently inside, if anything.
///
/// Shared because getting it subtly different on two platforms means the
/// picker opens in one app and not the other for the same keystrokes.
public func activeToken(in text: String, cursor: Int) -> ComposerToken {
    guard cursor >= 0, cursor <= text.count else { return .none }
    let caret = text.index(text.startIndex, offsetBy: cursor)
    let head = text[text.startIndex..<caret]

    // Scan back to whitespace: a token cannot span a space.
    let tokenStart = head.lastIndex(where: { $0.isWhitespace }).map { text.index(after: $0) }
        ?? text.startIndex
    let token = text[tokenStart..<caret]
    guard let first = token.first else { return .none }

    if first == "/" {
        // Only at the very start of the message. A slash mid-sentence is a
        // path separator or an "and/or", and popping a command menu over that
        // interrupts ordinary writing.
        guard tokenStart == text.startIndex else { return .none }
        return .slash(prefix: String(token.dropFirst()), range: tokenStart..<caret)
    }

    if first == "@" {
        // Legitimate mid-sentence, unlike a slash — but not when it is part of
        // a word, which is what an email address is.
        return .mention(prefix: String(token.dropFirst()), range: tokenStart..<caret)
    }

    return .none
}
