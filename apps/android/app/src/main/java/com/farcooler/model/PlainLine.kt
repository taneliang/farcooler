package com.farcooler.model

/**
 * An agent's reply as one plain line, for a place that draws no Markdown
 * (ov-495). The runner sends `said` flattened to one line, so a reply that
 * opened with a list arrives as "Four asks, in order: - **PDF export**
 * (ha-12): ...". Emphasis, code, links, headings, quotes and list markers go;
 * the words stay; only the first non-blank line is kept. The same rules as
 * AgentKit's `PlainLine.swift`.
 */
object PlainLine {
    private val rules: List<Pair<Regex, String>> =
        listOf(
            Regex("""!?\[([^\]]*)\]\([^)]*\)""") to "$1", // a link or image: its text
            Regex("""^\s*(?:#{1,6}|>)\s+""") to "", // a heading or quote
            Regex("""^\s*(?:[-*+•]|\d{1,3}[.)])\s+""") to "", // a list marker that opens the line
            Regex("""(?<=\s)(?:[-*+•]|\d{1,3}[.)])\s+(?=[*_`\[])""") to "", // one flattened into the line
            Regex("""(?<=:)\s+(?:[-*+•]|\d{1,3}[.)])\s+""") to " ", // or right after a colon
            Regex("""`+([^`]+)`+""") to "$1", // code
            Regex("""(\*\*|__)(?=\S)(.+?)(?<=\S)\1""") to "$2", // bold
            Regex("""(?<![\w*])\*(?=[^\s*])(.+?)(?<=[^\s*])\*(?![\w*])""") to "$1", // italic, asterisks
            Regex("""(?<![\w_])_(?=[^\s_])(.+?)(?<=[^\s_])_(?![\w_])""") to "$1", // italic, underscores
            Regex("""~~(?=\S)(.+?)(?<=\S)~~""") to "$1", // strikethrough
        )

    fun of(text: String): String {
        var line = text.lines().firstOrNull { it.isNotBlank() } ?: return ""
        for ((rule, template) in rules) line = rule.replace(line, template)
        return line.trim().split(Regex("\\s+")).joinToString(" ")
    }
}
