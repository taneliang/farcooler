package com.farcooler.model

/** What the caret is currently inside, if anything. */
sealed interface ComposerToken {
    data object None : ComposerToken
    data class Slash(val prefix: String, val range: IntRange) : ComposerToken
    data class Mention(val prefix: String, val range: IntRange) : ComposerToken
}

/**
 * What the caret is currently inside, if anything.
 *
 * Ported from `apps/shared/AgentKit/Sources/AgentKit/Composer.swift`, which is
 * shared between the Apple apps for the reason it is ported verbatim here:
 * getting it subtly different on two platforms means the picker opens in one
 * app and not the other for the same keystrokes.
 *
 * [range] is a half-open range over code points expressed as an `IntRange` from
 * `start` to `end - 1`; an empty token yields `start until start`, which Kotlin
 * writes as an empty range. Callers replace `text.substring(range.first, end)`.
 */
fun activeToken(text: String, cursor: Int): ComposerToken {
    if (cursor < 0 || cursor > text.length) return ComposerToken.None

    // Scan back to whitespace: a token cannot span a space.
    var start = cursor
    while (start > 0 && !text[start - 1].isWhitespace()) start -= 1
    if (start == cursor) return ComposerToken.None

    val first = text[start]

    if (first == '/') {
        // Only at the very start of the message. A slash mid-sentence is a path
        // separator or an "and/or", and popping a command menu over that
        // interrupts ordinary writing.
        if (start != 0) return ComposerToken.None
        return ComposerToken.Slash(text.substring(start + 1, cursor), start until cursor)
    }

    if (first == '@') {
        // Legitimate mid-sentence, unlike a slash — but not when it is part of
        // a word, which is what an email address is.
        return ComposerToken.Mention(text.substring(start + 1, cursor), start until cursor)
    }

    return ComposerToken.None
}
