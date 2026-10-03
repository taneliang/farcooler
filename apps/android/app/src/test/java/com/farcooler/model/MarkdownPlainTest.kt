package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Test

/** What TalkBack reads for a line of task text: its words, not its markup (ov-98 review). */
class MarkdownPlainTest {
    @Test
    fun aLineReadsAsItsWordsNotItsMarkup() {
        assertEquals(
            "Renders bold, italic, code and a link",
            Markdown.plain("Renders **bold**, *italic*, `code` and [a link](https://x.y)"),
        )
    }
}
