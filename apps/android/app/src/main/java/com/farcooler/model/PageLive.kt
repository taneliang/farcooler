package com.farcooler.model

import java.net.URI

// References that draw live (ov-269 design 3.4, the owner's ruling on Q4), and
// the rules a page's layout follows (3.6, 3.7): the Android twin of AgentKit's
// `PageLive.swift`, `PageLayout.swift` and `PageTable.swift`'s cell rules.
//
// A page names a card, a lane or a theme, and the app draws that thing's
// *current* words from what it already holds. A name nothing answers to (a
// dropped lane, a runner without the plan layer, a key on another board) draws
// its label or its name as plain text with no link: never an error.
//
// Navigation is the only action (Q3). A link outside the app opens only for
// `https`, with its domain drawn beside it (Q5), so a label can't hide where it
// goes, and it's checked again when it's opened.

/** Where a reference opens. */
sealed interface PageDestination {
    data class Task(val id: String) : PageDestination
    /** A task's open question, by the task's id: Needs You. */
    data class Ask(val id: String) : PageDestination
    data class Lane(val id: String) : PageDestination
    data class Theme(val id: String) : PageDestination
    data class Page(val slot: String) : PageDestination
    data class Worktree(val id: String) : PageDestination
    data class Terminal(val worktree: String, val name: String) : PageDestination
    /** A web page, in the browser. Only ever `https`. */
    data class Url(val url: String) : PageDestination

    val isExternal: Boolean get() = this is Url
}

/** A reference as it's drawn now. */
data class PageResolved(
    /** Its name: a key, a lane's name, a theme's, a page's title, a link's label; or its label or raw name when it resolves to nothing. */
    val name: String,
    /** Its live words: a card's status, a lane's state, a theme's progress, a link's domain. */
    val status: String? = null,
    /** Amber only for a question that's still open. */
    val statusTone: PageTone = PageTone.NEUTRAL,
    /** Where it opens; null when it resolves to nothing, so it draws as plain text. */
    val destination: PageDestination? = null,
    /** What TalkBack says for it. */
    val spoken: String = listOfNotNull(name, status).joinToString(", "),
)

/** A cell as it's drawn: its words, a link's domain after them, and where it opens. */
data class PageCellParts(val text: String, val domain: String? = null, val destination: PageDestination? = null) {
    /** What TalkBack hears for it: its words, and for a link whose words aren't its domain, where it goes. */
    val spoken: String get() = domain?.let { "$text, link to $it" } ?: text

    /** What TalkBack's action for it is called: "Open ov-274". */
    val action: String? get() = destination?.let { "Open $spoken" }
}

/** The words pages draw that aren't the orchestrator's, in Material's sentence case. */
object PageWords {
    const val NEEDS_YOU = "Needs you"
    const val ANSWERED = "Answered"
    const val NEWER_BLOCK = "This part needs a newer Far Cooler."
    const val TOO_LARGE = "This page is too large to show in full."
    const val UNREADABLE = "Far Cooler can’t draw this page. Update Far Cooler to see it."
    const val COULDNT_READ = "Far Cooler couldn’t read this board’s pages."
    const val FROM_THE_ORCHESTRATOR = "From the orchestrator"

    /** "Updated 12 min ago", or past the page's own limit, "Not updated for 3 hours": never amber. */
    fun updated(page: BoardPage, now: Long): String {
        val minutes = maxOf(0, now - page.updatedAtMs) / 60_000
        val limit = page.doc?.staleAfterMin
        if (limit != null && limit > 0 && minutes > limit) return "Not updated for ${span(minutes)}"
        return "Updated ${PlanWords.ago(page.updatedAtMs, now)}"
    }

    /** "40 minutes", "3 hours", "2 days". */
    fun span(minutes: Long): String {
        fun plural(n: Long, unit: String) = if (n == 1L) "1 $unit" else "$n ${unit}s"
        return when {
            minutes < 60 -> plural(minutes, "minute")
            minutes < 1440 -> plural(minutes / 60, "hour")
            else -> plural(minutes / 1440, "day")
        }
    }

    /** "2 of 4". */
    fun progress(done: Int, total: Int) = "$done of $total"

    /** A lane's tokens for a table cell: "470K", or "Not reported". */
    fun tokens(spend: PlanSpend): String = if (spend.totalTokens > 0) TaskUsageFormat.tokens(spend.totalTokens) else PlanWords.NOT_REPORTED
}

/** The one rule for links leaving the app. */
object PageLinks {
    /**
     * [raw] as a link the browser may open: `https`, with a host of plain
     * ASCII (letters, digits, dots and hyphens; punycode shows as `xn--`), and
     * no user name or password to dress one domain up as another. Anything
     * else is null, and draws as plain text.
     */
    fun https(raw: String): URI? {
        val uri = runCatching { URI(raw) }.getOrNull() ?: return null
        val host = uri.host ?: return null
        if (!uri.scheme.equals("https", ignoreCase = true) || host.isEmpty() || uri.rawUserInfo != null) return null
        if (!host.all { it.code < 128 && (it.isLetterOrDigit() || it == '.' || it == '-') }) return null
        return uri
    }

    /** The domain a link shows, or null for one that won't open. */
    fun domain(raw: String): String? = https(raw)?.host
}

/** What the app holds that a page's references are drawn from. Nothing here asks the runner for anything. */
data class PageWorld(
    /** The board's cards. */
    val tasks: List<TaskRow> = emptyList(),
    /** The plan, or null on a runner without `board_plan`: lane and theme references then draw as plain text. */
    val plan: Plan? = null,
    val pages: List<BoardPage> = emptyList(),
    /** Worktree ids by name. */
    val worktrees: Map<String, String> = emptyMap(),
    /** The terminals the app knows by name, as `worktree id/name`. */
    val terminals: Set<String> = emptySet(),
    /** Now, for "Updated 12 min ago". */
    val nowMs: Long = 0,
) {
    private val byKey = tasks.associateBy { it.key.lowercase() }
    private val bySlot = pages.associateBy { it.slot }

    /** [ref] as it's drawn now. */
    fun resolve(ref: PageRef): PageResolved {
        val label = ref.label?.takeIf { it.isNotEmpty() }
        val plain = PageResolved(label ?: ref.target.rawName)
        return when (val t = ref.target) {
            is PageTarget.Task -> byKey[t.key.lowercase()]?.let { row ->
                PageResolved(
                    label ?: row.key, row.status.title, destination = PageDestination.Task(row.id),
                    spoken = listOf(label ?: row.key, row.title, row.status.title).joinToString(", "),
                )
            } ?: plain
            is PageTarget.Ask -> byKey[t.key.lowercase()]?.let { row ->
                val open = row.status == TaskStatus.NEEDS_DECISION
                val word = if (open) PageWords.NEEDS_YOU else PageWords.ANSWERED
                PageResolved(
                    label ?: row.key, word, if (open) PageTone.ATTENTION else PageTone.NEUTRAL,
                    if (open) PageDestination.Ask(row.id) else PageDestination.Task(row.id),
                    listOf(label ?: row.key, row.title, word).joinToString(", "),
                )
            } ?: plain
            is PageTarget.Lane -> plan?.lanes?.firstOrNull { it.name == t.name }?.let {
                PageResolved(label ?: it.name, PlanWords.status(it), destination = PageDestination.Lane(it.id))
            } ?: plain
            is PageTarget.Theme -> plan?.themes?.firstOrNull { it.name == t.name }?.let {
                PageResolved(label ?: it.name, PlanWords.progress(it.counts), destination = PageDestination.Theme(it.id))
            } ?: plain
            is PageTarget.Page -> bySlot[t.slot]?.let { PageResolved(label ?: it.title, destination = PageDestination.Page(t.slot)) } ?: plain
            is PageTarget.Worktree -> worktrees[t.name]?.let { PageResolved(label ?: t.name, destination = PageDestination.Worktree(it)) } ?: plain
            is PageTarget.Terminal -> worktrees[t.worktree]?.takeIf { terminalKey(it, t.name) in terminals }?.let {
                PageResolved(label ?: t.name, destination = PageDestination.Terminal(it, t.name))
            } ?: plain
            is PageTarget.Url -> PageLinks.domain(t.raw)?.let { host ->
                PageResolved(
                    label ?: host, if (label == null) null else host, destination = PageDestination.Url(t.raw),
                    spoken = label?.let { "$it, link to $host" } ?: "Link to $host",
                )
            } ?: plain
            is PageTarget.Ci -> {
                val name = label ?: ciName(t)
                val read = plan?.ci(t.ciSubject) ?: return PageResolved(name)
                PageResolved(
                    name, listOfNotNull(TrainWords.ciSummary(read), TrainWords.ciStale(read, plan?.nowMs ?: nowMs)).joinToString(" · "),
                    if (read.needsAttention) PageTone.ATTENTION else PageTone.NEUTRAL,
                    PageLinks.https(read.url)?.let { PageDestination.Url(read.url) },
                )
            }
            is PageTarget.Cards -> {
                val name = label ?: statusName(t.status)
                val count = plan?.cardCount(t.status) ?: return PageResolved(name)
                PageResolved(name, "$count", spoken = "$name, $count")
            }
            PageTarget.Unknown -> plain
        }
    }

    private fun themeOf(ref: PageRef): PlanTheme? =
        (ref.target as? PageTarget.Theme)?.let { t -> plan?.themes?.firstOrNull { it.name == t.name } }

    private fun laneOf(ref: PageRef): PlanLane? =
        (ref.target as? PageTarget.Lane)?.let { t -> plan?.lanes?.firstOrNull { it.name == t.name } }

    /** A figure as it's drawn (ov-306): its value as written, or its reference's live value, with a detail and a tone. */
    data class StatShown(val value: String, val detail: String?, val tone: PageTone)

    fun statText(stat: PageStat): StatShown {
        val ref = stat.ref ?: return StatShown(stat.value, stat.detail, stat.tone)
        val resolved = resolve(ref)
        return when (val t = ref.target) {
            is PageTarget.Ci -> plan?.ci(t.ciSubject)?.let { read ->
                // A stale read says how old it is before anything else under it.
                val detail = TrainWords.ciStale(read, plan?.nowMs ?: nowMs) ?: stat.detail ?: TrainWords.ciJobs(read)
                StatShown(TrainWords.ciStatus(read.status), detail, if (read.needsAttention) PageTone.ATTENTION else stat.tone)
            } ?: StatShown(resolved.name, stat.detail, stat.tone)
            is PageTarget.Cards -> StatShown(resolved.status ?: resolved.name, stat.detail, stat.tone)
            is PageTarget.Lane -> StatShown(
                laneOf(ref)?.let { if (stat.show == PageShow.SPEND) PageWords.tokens(it.spend) else PlanWords.status(it) } ?: resolved.name,
                stat.detail, stat.tone,
            )
            is PageTarget.Theme -> StatShown(
                (if (stat.show == PageShow.SPEND) themeOf(ref)?.let { PageWords.tokens(it.spend ?: PlanSpend()) } else resolved.status) ?: resolved.name,
                stat.detail, stat.tone,
            )
            else -> StatShown(resolved.status ?: resolved.name, stat.detail, if (resolved.statusTone == PageTone.ATTENTION) PageTone.ATTENTION else stat.tone)
        }
    }

    /** What a reference cell draws: its own text when it has some, else the target's name, a lane's state words or tokens. */
    fun cellText(cell: PageCell): String {
        cell.text?.let { return it }
        val ref = cell.ref ?: return ""
        val lane = laneOf(ref)
        return when (cell.show) {
            // Live data draws its value (ov-306): CI's status, a count.
            PageShow.NAME -> resolve(ref).let { if (ref.target is PageTarget.Ci || ref.target is PageTarget.Cards) it.status ?: it.name else it.name }
            PageShow.STATE -> lane?.let(PlanWords::status) ?: resolve(ref).name
            PageShow.SPEND -> themeOf(ref)?.let { PageWords.tokens(it.spend ?: PlanSpend()) }
                ?: lane?.let { PageWords.tokens(it.spend) } ?: resolve(ref).name
        }
    }

    /**
     * [cell] as it's drawn: a reference drawn by name, or text the
     * orchestrator wrote over one, is a link; a lane's live state or tokens
     * are words. A web link's domain follows its words unless they're it.
     */
    fun parts(cell: PageCell): PageCellParts {
        val text = cellText(cell)
        val ref = cell.ref
        if (ref == null || (cell.text == null && cell.show != PageShow.NAME)) return PageCellParts(text)
        val destination = resolve(ref).destination ?: return PageCellParts(text)
        val domain = (destination as? PageDestination.Url)?.let { PageLinks.domain(it.url) }?.takeIf { !it.equals(text, ignoreCase = true) }
        return PageCellParts(text, domain, destination)
    }

    /** One "Open …" action per link in a row, so TalkBack reaches every link a row speaks for. */
    fun actions(cells: List<PageCell>): List<Pair<String, PageDestination>> =
        cells.mapNotNull { cell -> parts(cell).let { p -> p.action?.let { it to p.destination!! } } }

    companion object {
        fun terminalKey(worktree: String, name: String) = "$worktree/$name"

        /** What a CI reference is called without a label: "Main", "Run 812", or the commit's first eight digits. */
        fun ciName(t: PageTarget.Ci): String = when {
            t.subject == "main" -> "Main"
            t.subject.startsWith("run:") -> "Run ${t.subject.removePrefix("run:")}"
            else -> t.subject.take(8)
        }

        /** A card-count reference's status, as the board says it. */
        fun statusName(word: String): String =
            if (word == "open") "Not Done" else TaskStatus.entries.firstOrNull { it.wire == word }?.title ?: word
    }
}

/** Where a page degrades: the app owns layout, so it always can. */
object PageLayout {
    /** Narrower than this (in dp), a wide table stacks and steps go down the page. */
    const val NARROW = 480

    /** Whether a table with [columns] stacks at [width]: more than three columns on a narrow surface. */
    fun stacks(columns: Int, width: Int?): Boolean = width != null && width > 0 && columns > 3 && width < NARROW

    /** Whether steps go down the page rather than across. */
    fun stepsDown(width: Int?): Boolean = width != null && width > 0 && width < NARROW

    /** A table row as TalkBack reads it: "Lane, ov-274-phones. Cards, ov-274. State, In review." Empty cells left out. */
    fun spokenRow(columns: List<PageColumn>, cells: List<PageCell>, world: PageWorld): String =
        columns.zip(cells).mapNotNull { (column, cell) ->
            val text = world.parts(cell).spoken
            when {
                text.isEmpty() -> null
                column.title.isEmpty() -> "$text."
                else -> "${column.title}, $text."
            }
        }.joinToString(" ")

    /** The entries in the order they're drawn: newest first, unless the orchestrator gave its own order. */
    fun ordered(entries: List<PageEntry>, given: Boolean): List<PageEntry> =
        if (given) entries else entries.withIndex().sortedWith(compareByDescending<IndexedValue<PageEntry>> { it.value.at }.thenBy { it.index }).map { it.value }

    /** A progress block's fraction, held inside 0..1. */
    fun fraction(done: Int, total: Int): Float = if (total <= 0) 0f else (done.toFloat() / total).coerceIn(0f, 1f)
}

/** What TalkBack says for each part of a page: the same words the iPhone's VoiceOver does. */
object PageSpeech {
    /** "Blocked, Sidebar tint is undecided, Holds ov-222's last pass, ov-222, Sidebar tint follows the terminal theme, Needs you". */
    fun item(item: PageItem, resolved: PageResolved?): String =
        listOfNotNull(item.state.word, item.text, item.detail, resolved?.spoken).joinToString(", ")

    /** "Review, Active". */
    fun step(step: PageStep): String = listOfNotNull(step.label, step.state.word).joinToString(", ")

    /** "Cards closed": "2 of 4, since Monday, Done 3". */
    fun progress(block: PageBlock.Progress): String =
        (listOf(PageWords.progress(block.done, block.total)) + listOfNotNull(block.detail) + block.parts.map { "${it.label} ${it.count}" })
            .joinToString(", ")

    /** "Page, Train integ-10, In review · 3 of 4 lanes green, Updated 12 min ago". */
    fun row(page: BoardPage, now: Long): String =
        (listOf("Page", page.title) + listOfNotNull(page.summary.ifEmpty { null }) + PageWords.updated(page, now)).joinToString(", ")
}

/**
 * Where a board's pages are drawn (design 6.1): the Plan view's Pages section,
 * or inside the theme they're anchored to. A page whose theme is gone or
 * dropped falls back into the Pages section, so it's never lost.
 */
object PageShelf {
    fun liveThemes(plan: Plan?): Set<String> = plan?.themes.orEmpty().filter { it.state != "dropped" }.map { it.id }.toSet()

    fun listed(pages: List<BoardPage>, plan: Plan?): List<BoardPage> {
        val themes = liveThemes(plan)
        return pages.filter { page -> page.themeAnchor?.let { it !in themes } ?: true }
    }

    fun anchored(pages: List<BoardPage>, theme: String, plan: Plan?): List<BoardPage> =
        if (theme in liveThemes(plan)) pages.filter { it.themeAnchor == theme } else emptyList()
}

/**
 * The Markdown a text block draws (design 3.6): paragraphs, lists two levels
 * deep, bold, italic, code spans and `https` links. A heading, a table, a fence
 * or a quote draws as its plain words; any link but `https` keeps its words
 * and loses its link.
 */
object PageMarkdown {
    sealed interface Piece {
        data class Prose(val text: String) : Piece
        data class Item(val marker: String, val text: String, val depth: Int) : Piece
        data class Plain(val text: String) : Piece
    }

    /** A run of a line: [span]'s words and emphasis; [domain] when it's the domain after a web link. */
    data class Run(val span: Markdown.Span, val domain: Boolean = false)

    fun pieces(md: String): List<Piece> = Markdown.blocks(md).mapNotNull { block ->
        when (block) {
            is Markdown.Block.Paragraph -> Piece.Prose(block.text)
            is Markdown.Block.Bullet -> Piece.Item("•", block.text, minOf(block.depth, 1))
            is Markdown.Block.Numbered -> Piece.Item("${block.number}.", block.text, minOf(block.depth, 1))
            is Markdown.Block.Heading -> Piece.Plain(block.text)
            is Markdown.Block.Code -> Piece.Plain(block.text)
            is Markdown.Block.Quote -> Piece.Plain(block.text)
            Markdown.Block.Rule -> null
            is Markdown.Block.Table -> Piece.Plain((listOf(block.header) + block.rows).joinToString("\n") { it.joinToString("  ") })
        }
    }

    /**
     * Inline Markdown with only `https` links kept, each followed by its
     * domain, so a label can't hide where it goes (design 7, Q5). A label that
     * is already its domain isn't repeated.
     */
    fun inline(text: String): List<Run> = Markdown.inline(text).flatMap { span ->
        val link = span.link ?: return@flatMap listOf(Run(span))
        val host = PageLinks.domain(link) ?: return@flatMap listOf(Run(span.copy(link = null)))
        val label = span.text.trim().lowercase()
        if (label == host.lowercase() || label == link.lowercase()) listOf(Run(span))
        else listOf(Run(span), Run(Markdown.Span(" $host"), domain = true))
    }
}
