package com.farcooler.model

/**
 * Markdown, split into the pieces a reply is drawn from.
 *
 * Ported from `apps/shared/AgentKit/Sources/AgentKit/MarkdownView.swift`. Only
 * the parser is here; the drawing is in the UI layer, because layout is each
 * platform's own job and a block list is not.
 *
 * Line-based, because the structures that matter here are line-based. A fenced
 * code block swallows everything until its closing fence, so its contents are
 * never mistaken for markup.
 */
object Markdown {
    /** One renderable piece of a reply. */
    sealed interface Block {
        data class Paragraph(val text: String) : Block
        data class Heading(val level: Int, val text: String) : Block
        data class Bullet(val text: String, val depth: Int) : Block
        data class Numbered(val number: String, val text: String, val depth: Int) : Block
        data class Code(val text: String, val language: String) : Block
        data class Quote(val text: String) : Block
        data object Rule : Block

        /**
         * A pipe table. The header row is kept apart from the body because it
         * is drawn differently, not because the parser needs it separated.
         */
        data class Table(val header: List<String>, val rows: List<List<String>>) : Block
    }

    /**
     * Split one `| a | b |` line into its cells.
     *
     * The outer pipes are optional in GitHub's dialect and agents write them
     * either way, so an empty cell from a leading or trailing pipe is dropped
     * rather than rendered as a blank column.
     */
    fun cells(line: String): List<String> {
        var trimmed = line.trim()
        if (trimmed.startsWith("|")) trimmed = trimmed.drop(1)
        if (trimmed.endsWith("|")) trimmed = trimmed.dropLast(1)
        return trimmed.split("|").map { it.trim() }
    }

    /**
     * Whether a line is a table's `|---|:--:|` separator.
     *
     * This is what distinguishes a table from a paragraph that happens to
     * contain a pipe — a shell command, most often, which must not be eaten as
     * markup.
     */
    fun isTableRule(line: String): Boolean {
        if (!line.contains("|")) return false
        val parts = cells(line)
        if (parts.isEmpty()) return false
        return parts.all { cell ->
            cell.isNotEmpty() && cell.all { it == '-' || it == ':' || it == ' ' } && cell.contains('-')
        }
    }

    fun blocks(text: String): List<Block> {
        val blocks = mutableListOf<Block>()
        val paragraph = mutableListOf<String>()

        fun flushParagraph() {
            // Joined with a NEWLINE, not a space.
            //
            // CommonMark folds a single line break into a space and needs two
            // to make one. That rule exists for hand-written source files, and
            // an agent does not write to it — it breaks lines where it means
            // them to break. Honouring that is the difference between a
            // readable reply and one run-on paragraph.
            val joined = paragraph.joinToString("\n").trim()
            if (joined.isNotEmpty()) blocks.add(Block.Paragraph(joined))
            paragraph.clear()
        }

        val lines = text.split("\n")
        var index = 0
        while (index < lines.size) {
            val line = lines[index]
            index += 1
            val trimmed = line.trim()

            // A fence takes precedence over everything: its contents are not
            // markup and must not be read as any.
            if (trimmed.startsWith("```")) {
                flushParagraph()
                val language = trimmed.drop(3).trim()
                val body = mutableListOf<String>()
                while (index < lines.size) {
                    val next = lines[index]
                    index += 1
                    if (next.trim().startsWith("```")) break
                    body.add(next)
                }
                blocks.add(Block.Code(body.joinToString("\n"), language))
                continue
            }

            if (trimmed.isEmpty()) {
                flushParagraph()
                continue
            }

            if (trimmed == "---" || trimmed == "***" || trimmed == "___") {
                flushParagraph()
                blocks.add(Block.Rule)
                continue
            }

            if (trimmed.startsWith("#")) {
                val level = trimmed.takeWhile { it == '#' }.length
                if (level < trimmed.length && trimmed[level] == ' ') {
                    flushParagraph()
                    blocks.add(Block.Heading(minOf(level, 3), trimmed.drop(level).trim()))
                    continue
                }
            }

            // A table is recognised by its SECOND line, not its first: the
            // header alone is indistinguishable from a sentence containing a
            // pipe. So the separator is peeked at before either is consumed.
            if (trimmed.contains("|") && index < lines.size && isTableRule(lines[index])) {
                flushParagraph()
                index += 1
                val header = cells(trimmed)
                val body = mutableListOf<List<String>>()
                while (index < lines.size) {
                    val row = lines[index].trim()
                    if (row.isEmpty() || !row.contains("|")) break
                    index += 1
                    body.add(cells(row))
                }
                blocks.add(Block.Table(header, body))
                continue
            }

            if (trimmed.startsWith("> ")) {
                flushParagraph()
                blocks.add(Block.Quote(trimmed.drop(2)))
                continue
            }

            // Indentation is what nests a list, so it is measured before the
            // marker is stripped.
            val indent = line.takeWhile { it == ' ' || it == '\t' }.length
            val depth = minOf(indent / 2, 3)

            val marker = listOf("- ", "* ", "+ ").firstOrNull { trimmed.startsWith(it) }
            if (marker != null) {
                flushParagraph()
                blocks.add(Block.Bullet(trimmed.drop(marker.length), depth))
                continue
            }

            val dot = trimmed.indexOf('.')
            if (dot > 0 &&
                trimmed.take(dot).all { it.isDigit() } &&
                dot + 1 < trimmed.length &&
                trimmed[dot + 1] == ' '
            ) {
                flushParagraph()
                blocks.add(Block.Numbered(trimmed.take(dot), trimmed.drop(dot + 2), depth))
                continue
            }

            paragraph.add(trimmed)
        }
        flushParagraph()
        return blocks
    }

    /** One run of inline text, with whatever emphasis applies to it. */
    data class Span(
        val text: String,
        val bold: Boolean = false,
        val italic: Boolean = false,
        val code: Boolean = false,
        val link: String? = null,
    )

    /**
     * Inline syntax only — bold, italic, code spans, links.
     *
     * Hand-rolled, unlike Swift's, which hands the string to Foundation's own
     * markdown parser. Android has no equivalent, and pulling in a markdown
     * library to resolve four constructs would be a large dependency for a
     * small job.
     *
     * Text that cannot be parsed is returned as itself rather than dropped: a
     * stray bracket should cost a reader the emphasis, not the sentence. Code
     * spans are resolved first and their contents are never re-scanned, so an
     * asterisk inside `**` in a shell command stays an asterisk.
     */
    fun inline(text: String): List<Span> {
        val spans = mutableListOf<Span>()
        val literal = StringBuilder()

        fun flush() {
            if (literal.isNotEmpty()) {
                spans.add(Span(literal.toString()))
                literal.clear()
            }
        }

        var i = 0
        while (i < text.length) {
            val rest = text.substring(i)

            // Backticks first, and their contents are taken verbatim.
            if (rest.startsWith("`")) {
                val end = rest.indexOf('`', startIndex = 1)
                if (end > 0) {
                    flush()
                    spans.add(Span(rest.substring(1, end), code = true))
                    i += end + 1
                    continue
                }
            }

            if (rest.startsWith("**") || rest.startsWith("__")) {
                val fence = rest.take(2)
                val end = rest.indexOf(fence, startIndex = 2)
                if (end > 0) {
                    flush()
                    spans.add(Span(rest.substring(2, end), bold = true))
                    i += end + 2
                    continue
                }
            }

            if ((rest.startsWith("*") || rest.startsWith("_")) && rest.length > 1) {
                val fence = rest.take(1)
                val end = rest.indexOf(fence, startIndex = 1)
                if (end > 1) {
                    flush()
                    spans.add(Span(rest.substring(1, end), italic = true))
                    i += end + 1
                    continue
                }
            }

            if (rest.startsWith("[")) {
                val close = rest.indexOf(']')
                if (close > 0 && close + 1 < rest.length && rest[close + 1] == '(') {
                    val end = rest.indexOf(')', startIndex = close + 2)
                    if (end > 0) {
                        flush()
                        spans.add(
                            Span(
                                rest.substring(1, close),
                                link = rest.substring(close + 2, end),
                            )
                        )
                        i += end + 1
                        continue
                    }
                }
            }

            literal.append(text[i])
            i += 1
        }
        flush()
        return spans
    }
}

/**
 * How far through a plan entry is, as the agent reports it.
 *
 * The wire carries free-form strings from several adapters, so the mapping
 * lives in one place rather than in each surface that draws a checkmark.
 */
enum class PlanStatus {
    PENDING,
    ACTIVE,
    DONE;

    val isDone: Boolean get() = this == DONE

    companion object {
        fun parse(raw: String): PlanStatus = when (raw.lowercase()) {
            "completed", "done" -> DONE
            "in_progress", "inprogress", "active" -> ACTIVE
            else -> PENDING
        }
    }
}

val List<PlanEntry>.doneCount: Int
    get() = count { PlanStatus.parse(it.status) == PlanStatus.DONE }

val List<PlanEntry>.active: PlanEntry?
    get() = firstOrNull { PlanStatus.parse(it.status) == PlanStatus.ACTIVE }
