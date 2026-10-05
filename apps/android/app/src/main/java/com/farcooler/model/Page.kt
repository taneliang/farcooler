package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull

// Orchestrator pages on Android (ov-269 design 3 and 7, ov-285): a small JSON
// document of typed blocks an orchestrator publishes to a slot on a board,
// drawn here in Material's own type. The runner validates every page it stores
// (`crates/core/src/page_doc.rs`); this is the reader. The phone reads it
// through the client core's `page.list`, in the shape `page_json.rs` writes and
// `farcooler page list --json` prints, held to `test/fixtures/page.json`.
//
// The Android twin of AgentKit's `PageModel.swift`, so a page reads the same on
// the Mac, the iPhone and here, and `PageConformanceTest` holds it to every
// fixture in `test/fixtures/pages/` as AgentKit's own test does.
//
// EXPERIMENTAL, behind `board_pages`, and removable: delete Page.kt,
// PageLive.kt, PageReads.kt and the Page views.
//
// A reader is lenient where the runner is strict. A runner newer than this app
// may send a block or a field this build doesn't know, so an unknown block is
// [PageBlock.Unknown] (drawn as its `alt`), an unknown state is NONE, an
// unknown tone is neutral, and a block that won't decode costs that block, not
// the page. And it holds the design's caps itself: a page past them draws what
// fits, then one row saying the rest didn't.

/** One page as the runner lists it: its row in the overview, and its document when it was read with one. */
data class BoardPage(
    val id: String,
    val slot: String,
    val title: String,
    val short: String = "",
    val summary: String = "",
    /** `""`, or `"theme"` when it's drawn inside a theme's page. */
    val anchorKind: String = "",
    /** The theme's id when anchored. */
    val anchor: String = "",
    val revision: Long = 0,
    val ordinal: Long = 0,
    val actor: String = "",
    val updatedAtMs: Long = 0,
    /** Null when it was listed without its document, or the document is one this build can't read at all. */
    val doc: PageDoc? = null,
) {
    /** The theme it's drawn inside, by id, or null when it's a page of its own. */
    val themeAnchor: String? get() = if (anchorKind == "theme" && anchor.isNotEmpty()) anchor else null

    companion object {
        fun decode(o: JsonObject) = BoardPage(
            id = o.str("id") ?: "",
            short = o.str("short") ?: "",
            slot = o.str("slot") ?: "",
            title = o.str("title") ?: "",
            summary = o.str("summary") ?: "",
            anchorKind = o.str("anchor_kind") ?: "",
            anchor = o.str("anchor") ?: "",
            revision = o.num("revision") ?: 0,
            ordinal = o.num("ordinal") ?: 0,
            actor = o.str("actor") ?: "",
            updatedAtMs = o.num("updated_at_ms") ?: 0,
            doc = o["doc"]?.let { runCatching { PageDoc.decode(it) }.getOrNull() },
        )

        /** `page.list`: `{"pages": [...]}`, in the runner's order. */
        fun list(o: JsonObject): List<BoardPage> =
            (o["pages"] as? JsonArray).orEmpty().mapNotNull { (it as? JsonObject)?.let(::decode) }

        fun list(text: String): List<BoardPage> = list(Json.parseToJsonElement(text).jsonObject)
    }
}

/** The document: a title, a one-line summary and its blocks. */
data class PageDoc(
    val v: Int = 1,
    val title: String,
    val summary: String = "",
    /** At most 60 characters, for a glance; nothing in v1 draws it. */
    val glance: String? = null,
    /** Minutes after which "Updated …" reads "Not updated for …". */
    val staleAfterMin: Int? = null,
    val blocks: List<PageBlock>,
) {
    /**
     * The blocks to draw under a header that already says [title]: a first
     * heading that only repeats it is left out, so an anchored section doesn't
     * say "Risks" twice.
     */
    fun under(title: String): PageDoc {
        val first = blocks.firstOrNull() as? PageBlock.Heading
        return if (first != null && first.text.trim().equals(title.trim(), ignoreCase = true)) copy(blocks = blocks.drop(1)) else this
    }

    companion object {
        /** A document alone: a file in `test/fixtures/pages/normalized`. Throws for one that isn't an object. */
        fun decode(text: String): PageDoc = decode(Json.parseToJsonElement(text))

        fun decode(element: JsonElement): PageDoc {
            val o = element as? JsonObject ?: throw IllegalArgumentException("A page is a JSON object.")
            val cut = Cut()
            val title = PageCaps.cut(o.str("title") ?: "", PageCaps.TITLE, cut)
            val summary = PageCaps.cut(o.str("summary") ?: "", PageCaps.SUMMARY, cut)
            val glance = o.str("glance")?.let { PageCaps.cut(it, PageCaps.GLANCE, cut) }
            val blocks = mutableListOf<PageBlock>()
            // What's left of the document's 32 KiB, counted in the bytes of
            // the words drawn (never more than they serialize to, so a page
            // the runner took always fits), and of its 200 references.
            var bytes = PageCaps.DOCUMENT_BYTES
            val refs = intArrayOf(PageCaps.REFS)
            for (any in (o["blocks"] as? JsonArray).orEmpty()) {
                if (blocks.size >= PageCaps.BLOCKS) {
                    cut.happened = true
                    break
                }
                val block = PageBlock.decode(any, cut).limitingRefs(refs, cut)
                bytes -= block.textBytes
                if (bytes < 0) {
                    cut.happened = true
                    break
                }
                blocks += block
            }
            // More than the design allows (a runner that didn't check, or a
            // page edited by hand): what fits is drawn, then one line says so.
            if (cut.happened) blocks += PageBlock.Unknown(PageCaps.TOO_LARGE_TYPE, PageWords.TOO_LARGE)
            return PageDoc(
                v = o.num("v")?.toInt() ?: 1,
                title = title,
                summary = summary,
                glance = glance,
                staleAfterMin = o.num("stale_after_min")?.toInt(),
                blocks = blocks,
            )
        }
    }
}

/** A state with a glyph and a word: one fixed set for list items and steps. The word is always drawn or spoken. */
enum class PageState(val wire: String, val word: String?) {
    DONE("done", "Done"),
    ACTIVE("active", "Active"),
    WAITING("waiting", "Waiting"),
    BLOCKED("blocked", "Blocked"),
    FAILED("failed", "Failed"),
    TODO("todo", "To do"),
    NONE("none", null);

    companion object {
        fun of(word: String?): PageState = entries.firstOrNull { it.wire == word } ?: NONE
    }
}

/** Neutral, or the amber the app uses for "Needs you". There's no other color. */
enum class PageTone {
    NEUTRAL, ATTENTION;

    companion object {
        fun of(word: String?): PageTone = if (word == "attention") ATTENTION else NEUTRAL
    }
}

/** What a reference points at: something the app can already open. */
sealed interface PageTarget {
    /** The name it was written with: what an unresolved reference draws. */
    val rawName: String

    data class Task(val key: String) : PageTarget { override val rawName get() = key }
    data class Ask(val key: String) : PageTarget { override val rawName get() = key }
    data class Lane(val name: String) : PageTarget { override val rawName get() = name }
    data class Theme(val name: String) : PageTarget { override val rawName get() = name }
    data class Page(val slot: String) : PageTarget { override val rawName get() = slot }
    data class Worktree(val name: String) : PageTarget { override val rawName get() = name }
    data class Terminal(val worktree: String, val name: String) : PageTarget { override val rawName get() = "$worktree/$name" }
    data class Url(val raw: String) : PageTarget { override val rawName get() = raw }
    /** CI (ov-306): `main`, a commit's SHA, or `run:<id>`, read by the runner through `gh` while a page names it. */
    data class Ci(val subject: String) : PageTarget {
        override val rawName get() = subject
        /** The subject the runner reads it under: `main`, `run:<id>` or `sha:<sha>`. */
        val ciSubject: String get() = if (subject == "main" || subject.startsWith("run:")) subject else "sha:$subject"
    }
    /** How many of the board's cards are in a status (ov-306): a status's word, or `open`. */
    data class Cards(val status: String) : PageTarget { override val rawName get() = status }

    /** A target this build doesn't know: drawn as its label, as plain text. */
    data object Unknown : PageTarget { override val rawName get() = "" }
}

/** A reference: one target, and the words to draw for it when it has them. */
data class PageRef(val target: PageTarget, val label: String? = null) {
    companion object {
        fun of(element: JsonElement?): PageRef? {
            val o = element as? JsonObject ?: return null
            val terminal = o["terminal"] as? JsonObject
            val target = when {
                o.str("task") != null -> PageTarget.Task(o.str("task")!!)
                o.str("ask") != null -> PageTarget.Ask(o.str("ask")!!)
                o.str("lane") != null -> PageTarget.Lane(o.str("lane")!!)
                o.str("theme") != null -> PageTarget.Theme(o.str("theme")!!)
                o.str("page") != null -> PageTarget.Page(o.str("page")!!)
                o.str("worktree") != null -> PageTarget.Worktree(o.str("worktree")!!)
                terminal?.str("worktree") != null && terminal.str("name") != null ->
                    PageTarget.Terminal(terminal.str("worktree")!!, terminal.str("name")!!)
                o.str("url") != null -> PageTarget.Url(o.str("url")!!)
                o.str("ci") != null -> PageTarget.Ci(o.str("ci")!!.lowercase())
                o.str("cards") != null -> PageTarget.Cards(o.str("cards")!!)
                else -> PageTarget.Unknown
            }
            return PageRef(target, o.str("label"))
        }
    }
}

/** What a reference cell draws when it has no text of its own: a lane's state, or a lane's or a theme's spend (ov-306). */
enum class PageShow {
    NAME, STATE, SPEND;

    companion object {
        fun of(word: String?): PageShow = when (word) {
            "state" -> STATE
            "spend" -> SPEND
            else -> NAME
        }
    }
}

/** A table cell: text, a live reference, or text that links somewhere. */
data class PageCell(
    val text: String? = null,
    val ref: PageRef? = null,
    val show: PageShow = PageShow.NAME,
    val tone: PageTone = PageTone.NEUTRAL,
    val mono: Boolean = false,
) {
    companion object {
        fun of(element: JsonElement): PageCell {
            (element as? JsonPrimitive)?.takeIf { it.isString }?.let { return PageCell(text = it.content) }
            val o = element as? JsonObject ?: return PageCell()
            val show = PageShow.of(o.str("show"))
            return PageCell(o.str("text"), PageRef.of(o["ref"]), show, PageTone.of(o.str("tone")), o.bool("mono") ?: false)
        }
    }
}

/** A figure in a `stats` row: a value as written, or a reference drawn live (ov-306), when [value] is empty. */
data class PageStat(
    val label: String,
    val value: String,
    val detail: String? = null,
    val tone: PageTone = PageTone.NEUTRAL,
    val ref: PageRef? = null,
    val show: PageShow = PageShow.NAME,
)

/** A column of a table. */
data class PageColumn(val title: String, val align: Align = Align.START, val grow: Boolean = false) {
    enum class Align { START, CENTER, END }
}

/** A row of a `list` block. */
data class PageItem(
    val text: String,
    val state: PageState = PageState.NONE,
    val detail: String? = null,
    val ref: PageRef? = null,
    val tone: PageTone = PageTone.NEUTRAL,
)

/** An entry on a timeline: milliseconds since 1970, as the runner stores it. */
data class PageEntry(val at: Long, val text: String, val ref: PageRef? = null)

/** A step in a pipeline. */
data class PageStep(val label: String, val state: PageState)

/** A part of a progress bar. */
data class PagePart(val label: String, val count: Int)

/** The nine blocks, and a tenth for any this build doesn't know. */
sealed interface PageBlock {
    data class Heading(val text: String) : PageBlock
    data class Text(val md: String, val tone: PageTone = PageTone.NEUTRAL) : PageBlock
    data class Stats(val items: List<PageStat>) : PageBlock
    data class Progress(val label: String, val done: Int, val total: Int, val detail: String? = null, val parts: List<PagePart> = emptyList()) : PageBlock
    data class Table(val columns: List<PageColumn>, val rows: List<List<PageCell>>) : PageBlock
    data class ListBlock(val items: List<PageItem>) : PageBlock

    /** Newest first, unless the orchestrator asked for its own order. */
    data class Timeline(val entries: List<PageEntry>, val given: Boolean = false) : PageBlock
    data class Steps(val steps: List<PageStep>) : PageBlock
    data class Links(val refs: List<PageRef>) : PageBlock

    /** A block from a newer runner, or one that wouldn't decode: its `alt`, when it has one. */
    data class Unknown(val type: String, val alt: String?) : PageBlock

    /** The UTF-8 bytes of the words it draws: less than it takes written out, so a byte cap never over-counts. */
    val textBytes: Int
        get() {
            fun n(s: String?) = s?.toByteArray(Charsets.UTF_8)?.size ?: 0
            fun ref(r: PageRef?) = n(r?.label) + n(r?.target?.rawName)
            return when (this) {
                is Heading -> n(text)
                is Text -> n(md)
                is Stats -> items.sumOf { n(it.label) + n(it.value) + n(it.detail) + ref(it.ref) }
                is Progress -> n(label) + n(detail) + parts.sumOf { n(it.label) }
                is Table -> columns.sumOf { n(it.title) } + rows.flatten().sumOf { n(it.text) + ref(it.ref) }
                is ListBlock -> items.sumOf { n(it.text) + n(it.detail) + ref(it.ref) }
                is Timeline -> entries.sumOf { n(it.text) + ref(it.ref) }
                is Steps -> steps.sumOf { n(it.label) }
                is Links -> refs.sumOf { ref(it) }
                is Unknown -> n(alt)
            }
        }

    /**
     * This block with its references held to what's left of the page's
     * [budget]: past it, a reference is dropped and its words draw as plain
     * text, and [cut] says so.
     */
    fun limitingRefs(budget: IntArray, cut: Cut): PageBlock {
        fun keep(ref: PageRef?): PageRef? {
            if (ref == null) return null
            if (budget[0] <= 0) {
                cut.happened = true
                return null
            }
            budget[0] -= 1
            return ref
        }
        return when (this) {
            is Table -> copy(rows = rows.map { row ->
                row.map { cell ->
                    val ref = cell.ref
                    if (ref != null && keep(ref) == null) cell.copy(text = cell.text ?: (ref.label ?: ref.target.rawName), ref = null) else cell
                }
            })
            is ListBlock -> copy(items = items.map { it.copy(ref = keep(it.ref)) })
            is Timeline -> copy(entries = entries.map { it.copy(ref = keep(it.ref)) })
            is Links -> copy(refs = refs.mapNotNull { keep(it) })
            is Stats -> copy(items = items.map { stat ->
                val ref = stat.ref
                if (ref != null && keep(ref) == null) stat.copy(value = stat.value.ifEmpty { ref.label ?: ref.target.rawName }, ref = null) else stat
            })
            else -> this
        }
    }

    companion object {
        /** A block, held to the design's caps ([PageCaps]); [cut] is set when anything was left out or cut short. */
        fun decode(element: JsonElement, cut: Cut = Cut()): PageBlock {
            val o = element as? JsonObject ?: JsonObject(emptyMap())
            val type = o.str("type") ?: ""
            fun items(key: String): List<JsonElement> = PageCaps.prefix((o[key] as? JsonArray).orEmpty(), PageCaps.items(type, key), cut)
            fun text(value: String?, limit: Int = PageCaps.STRING): String? = value?.let { PageCaps.cut(it, limit, cut) }
            val alt = text(o.str("alt"), PageCaps.ALT)
            return when (type) {
                "heading" -> text(o.str("text"))?.let { Heading(it) }
                "text" -> text(o.str("md"), PageCaps.MD)?.let { Text(it, PageTone.of(o.str("tone"))) }
                "stats" -> Stats(items("items").mapNotNull { s ->
                    val s = s as? JsonObject ?: return@mapNotNull null
                    val label = s.str("label") ?: return@mapNotNull null
                    val ref = PageRef.of(s["ref"])
                    val value = s.str("value") ?: (if (ref != null) "" else return@mapNotNull null)
                    PageStat(label, value, s.str("detail"), PageTone.of(s.str("tone")), ref, PageShow.of(s.str("show")))
                })
                "progress" -> {
                    val label = o.str("label")
                    val done = o.num("done")
                    val total = o.num("total")
                    if (label == null || done == null || total == null) null
                    else Progress(
                        label, done.toInt(), total.toInt(), o.str("detail"),
                        items("parts").mapNotNull { p ->
                            val p = p as? JsonObject ?: return@mapNotNull null
                            val name = p.str("label") ?: return@mapNotNull null
                            val count = p.num("count") ?: return@mapNotNull null
                            PagePart(name, count.toInt())
                        },
                    )
                }
                "table" -> {
                    val columns = items("columns").map { c ->
                        val c = c as? JsonObject ?: JsonObject(emptyMap())
                        val align = when (c.str("align")) {
                            "end" -> PageColumn.Align.END
                            "center" -> PageColumn.Align.CENTER
                            else -> PageColumn.Align.START
                        }
                        PageColumn(c.str("title") ?: "", align, c.bool("grow") ?: false)
                    }
                    if (columns.isEmpty()) null
                    else Table(columns, items("rows").map { row ->
                        PageCaps.prefix((row as? JsonArray).orEmpty(), columns.size, cut).map { cell ->
                            val read = PageCell.of(cell)
                            read.copy(text = read.text?.let { PageCaps.cut(it, PageCaps.STRING, cut) })
                        }
                    })
                }
                "list" -> ListBlock(items("items").mapNotNull { i ->
                    val i = i as? JsonObject ?: return@mapNotNull null
                    val words = text(i.str("text")) ?: return@mapNotNull null
                    PageItem(words, PageState.of(i.str("state")), text(i.str("detail")), PageRef.of(i["ref"]), PageTone.of(i.str("tone")))
                })
                "timeline" -> Timeline(
                    items("entries").mapNotNull { e ->
                        val e = e as? JsonObject ?: return@mapNotNull null
                        val at = e.num("at") ?: return@mapNotNull null
                        val words = text(e.str("text")) ?: return@mapNotNull null
                        PageEntry(at, words, PageRef.of(e["ref"]))
                    },
                    given = o.str("order") == "given",
                )
                "steps" -> Steps(items("steps").mapNotNull { s ->
                    val s = s as? JsonObject ?: return@mapNotNull null
                    PageStep(s.str("label") ?: return@mapNotNull null, PageState.of(s.str("state")))
                })
                "links" -> Links(items("items").mapNotNull { PageRef.of(it) })
                else -> null
            } ?: Unknown(type, alt)
        }
    }
}

/** Whether anything was left out or cut short while a page was read. */
class Cut(var happened: Boolean = false)

/**
 * The design's limits (section 7), held by the reader as well as the runner,
 * so a page that breaks them draws what fits and says the rest didn't.
 */
object PageCaps {
    const val BLOCKS = 60
    const val ROWS = 50
    const val COLUMNS = 8
    const val ITEMS = 50
    const val MD = 2_000
    const val STRING = 200
    const val ALT = 500
    const val TITLE = 60
    const val SUMMARY = 120
    const val GLANCE = 60

    /** A document, serialized. */
    const val DOCUMENT_BYTES = 32 * 1024

    /** References on one page. */
    const val REFS = 200

    /** The block a clamped page ends with. */
    const val TOO_LARGE_TYPE = "too-large"

    /** The most of [key] a [type] block may hold. */
    fun items(type: String, key: String): Int = when {
        type == "stats" -> 6
        type == "steps" || type == "links" -> 12
        type == "progress" -> 6
        type == "table" && key == "columns" -> COLUMNS
        type == "table" -> ROWS
        else -> ITEMS
    }

    fun <T> prefix(values: List<T>, limit: Int, cut: Cut): List<T> {
        if (values.size <= limit) return values
        cut.happened = true
        return values.take(limit)
    }

    /** [text] at most [limit] characters, the last an ellipsis when it was cut. */
    fun cut(text: String, limit: Int, cut: Cut): String {
        val count = text.codePointCount(0, text.length)
        if (count <= limit) return text
        cut.happened = true
        return text.substring(0, text.offsetByCodePoints(0, limit - 1)) + "…"
    }
}

private fun JsonObject.str(key: String): String? = (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull

private fun JsonObject.bool(key: String): Boolean? = (this[key] as? JsonPrimitive)?.takeIf { !it.isString }?.booleanOrNull

/** A whole number, written as an integer or as a double with nothing after the point. */
private fun JsonObject.num(key: String): Long? {
    val p = this[key] as? JsonPrimitive ?: return null
    if (p.isString || p is JsonNull) return null
    p.longOrNull?.let { return it }
    val d = p.doubleOrNull ?: return null
    return if (d.isFinite() && d == Math.rint(d) && kotlin.math.abs(d) < 9.0e15) d.toLong() else null
}
