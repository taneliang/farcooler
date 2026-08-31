package com.farcooler.model

import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

/*
 * The navigation shell, as arithmetic.
 *
 * A port of `apps/shared/AgentKit/Sources/AgentKit/ShellNavigation.swift` — the
 * PURE half of the gesture shell iOS shipped: which workspace and tab a swipe
 * lands on, where the axis locks, how far the column unfurls, what a release
 * means. There is deliberately not one Compose type in this file.
 *
 * ## Why this lands before any pixel of it
 *
 * Because the bugs live here, and they are invisible.
 *
 * iOS shipped its shell with the axis test written `abs(dx) > dy`, `dy`
 * up-positive. A downward drag has negative `dy`, so it lost to any horizontal
 * component at all: **every drag down the screen classified as horizontal**, and
 * a thumb's natural arc — about 79 points across for 511 down, some 9° — is well
 * past the 70-point commit. Vertical scrolling turned the page. It shipped, and a
 * real phone found it (`b192f17`).
 *
 * That is a pure function of two floats. It is a test that runs in a
 * millisecond, and no amount of looking at a screenshot would have caught it,
 * because the drawing was correct — the classification was not. The same commit
 * fixed two more of the same shape: a column row computed against a re-derived
 * edge instead of a measured one, and a pull-down gated on where a gesture ENDED
 * rather than where it began.
 *
 * So this file exists, and it exists first, and the fixed forms are what is
 * ported — not the ones in the brief, which predate the fix. Where
 * `.claude/agent/briefs/ios-shell-mechanics.md` and this disagree, the shipped
 * Swift wins; the brief has already been corrected once on `ROW_H`, which it
 * gave as 34 against a shipped 44.
 *
 * ## What is NOT here
 *
 * The flight — `ShellFlight.swift`'s cardness, scale, radius and the page's
 * journey into its tile. That is presentation, it is the last piece, and it
 * would be a large body of arithmetic serving nothing until there is an
 * overview to fly into.
 */

/**
 * The figures the shell is built from, in dp.
 *
 * **`pageWidth` is not among them, and its absence is the point.** iOS carries
 * `pageWidth = 393` as a default and calls it "device width". Android runs on a
 * range of widths wide enough that a constant would be wrong on most of them,
 * and Compose hands the real one to any layout that asks. So every function here
 * that needs a page width takes it as a parameter, and there is no default for a
 * caller to fall back to by accident.
 */
object ShellMetrics {
    /** One column row. One tab revealed per 44dp of upward travel. */
    const val ROW_HEIGHT = 44f

    /**
     * How far BELOW the fingertip the column row it selects sits.
     *
     * Thumb occlusion, and ONE constant rather than a second mapping: a finger
     * on a 44dp row covers most of it, so the row being chosen is the one that
     * cannot be seen. Shifting the hit-test down by this much draws the
     * highlight at or below the contact point instead of under it.
     *
     * **A quarter of a row, and the size is the argument.** The literal
     * reading of "the row below" is a whole [ROW_HEIGHT], and a whole row
     * breaks the mapping at both ends — the row nearest the bar would own 88dp
     * of travel while every other row owned 44, and a TAP, which goes through
     * [ShellGesture.columnRow] too, would choose the row under the one it
     * touched. Half a row is no better for the tap: a row's label is centred,
     * so aiming at it lands exactly on the boundary. Eleven leaves the middle
     * 33dp of every row still selecting itself.
     */
    const val ROW_BIAS = ROW_HEIGHT / 4

    /** The rail item's height inside the bar. */
    const val BAR_ROW = 44f

    /** Below this much travel, a release abandons and costs nothing. */
    const val OPEN_MIN = 16f

    /** Extra travel past the last row that reaches the overview. */
    const val OVER_RUN = 76f

    /** Horizontal distance that commits a page turn. */
    const val PAGE_COMMIT = 70f

    /** The bar's inset from each edge. */
    const val BAR_INSET = 16f

    /**
     * The first 6dp of movement decides horizontal versus vertical. On the
     * CONTENT that is the answer for the rest of the gesture; on the bar it is
     * only the first answer, and [ShellGesture.lean] is asked again every
     * frame after it.
     */
    const val AXIS_LOCK = 6f

    /**
     * How much further one axis has to have travelled than the other before a
     * BAR gesture already leaning one way changes its mind.
     *
     * **A ratio rather than a distance, because it is an angle.** 1.4 is
     * `tan 54.5°`: a gesture leaning horizontal keeps the gesture until the
     * finger is travelling more than 54.5° off horizontal, and one leaning
     * vertical keeps it until the finger is within 35.5° of horizontal.
     * Between them is a 19° band in which the answer is whatever it already
     * was — which is what a hysteresis is, and the answer to what happens on
     * the diagonal: nothing happens on the diagonal.
     *
     * Without it, a finger crawling the diagonal arrives as integer points and
     * lands either side of 1 from frame to frame: a 120-frame crawl changes
     * its answer 115 times, each of them a menu told to open and then to shut
     * over a surface that never finishes arriving.
     *
     * Scale-free, which is the second reason it is a ratio. Ten points up
     * takes fourteen across to redirect, which costs nothing; two hundred up
     * takes two hundred and eighty, by which point you have opened a menu and
     * are reading it, and leaving it should look like a decision.
     *
     * **The bar only.** The content keeps a true lock — see [ShellGesture.axis]
     * and `PaneTrack`'s header, where the same split is reached from Android's
     * own side.
     */
    const val REDIRECT = 1.4f

    /**
     * How much of a drag survives at the true ends of the fleet, where there is
     * nothing to bring in.
     */
    const val RUBBER_BAND = 0.34f

    /** The bar's rail, for a measured page width. */
    fun railWidth(pageWidth: Float): Float = pageWidth - 2 * BAR_INSET
}

/**
 * One tab in the shell, and what its mark says.
 *
 * **The mark is the product's mark**, not a fourth vocabulary. iOS carries a
 * separate `ShellMark` enum of four cases alongside `GlanceMark`, which is a
 * seam left over from the shell landing before the glance spec did. There is no
 * reason to reproduce the seam here: a tab holds a [GlanceMark] and an
 * [AgentOutcome] exactly as a fleet row does, decided by the same two functions,
 * so a ribbon dot and the row it names cannot disagree. Since `done` joined the
 * review tier the mark answers for all but one state, and [outcome] is a dead
 * turn and nothing else.
 *
 * Order is fixed and is **never re-sorted by activity** — §03, and the reason is
 * that a strip whose items move when an agent finishes is a strip you cannot
 * build muscle memory for.
 */
data class ShellTab(
    val id: String,
    val title: String,
    /** Null where [outcome] answers instead. Exactly one of the two is set. */
    val mark: GlanceMark?,
    /**
     * Null where [mark] answers instead, which is every tab but a turn that
     * DIED — see [AgentOutcome], which is down to one case.
     */
    val outcome: AgentOutcome? = null,
    /**
     * Whether this tab is asking for a person, by the app's own single
     * definition — `AgentActivity.wantsAttention`, blocked or done, shared with
     * the Mac since long before any of this.
     *
     * Carried rather than re-derived from [mark] because it is deliberately
     * BROADER than the amber ring: a finished turn wants you and draws the
     * middle-weight REVIEW ring, not the heavy amber one. See
     * [ShellWorkspace.precedence], which is the only thing that reads it and the
     * only place the distinction matters.
     */
    val wantsAttention: Boolean = false,
)

/**
 * Where a workspace sorts in the overview, and nowhere else.
 *
 * §03's order, one rung per row: needs you, then an unread diff, then a
 * workspace we have heard nothing from, then one getting on with it.
 */
enum class ShellPrecedence {
    NEEDS_YOU,
    UNREAD_DIFF,
    ALL_STALE,
    WORKING,
}

/** One workspace: its name, its tabs, and where to reopen it. */
data class ShellWorkspace(
    val id: String,
    val name: String,
    val tabs: List<ShellTab>,
    /**
     * Which runner this workspace is on.
     *
     * **iOS has this field and holds it nil**, with a comment saying "The field
     * earns its keep the day a fleet spans runners" — because a `Connection` is
     * one runner over there and the whole tree is keyed on it. On this platform
     * that day is today: `Route.Terminal` carries a `hostId` and this app is
     * connected to every runner at once. So it is populated, it is what
     * [ShellStep.crossesRunner] is computed from, and a bar that is about to
     * take you to another machine can say so before you commit.
     */
    val runnerId: String,
    /** The last few things this workspace's most active agent said. */
    val tail: List<String> = emptyList(),
    /** Which tab to reopen on, or null to fall back to the first. */
    val resume: Int? = null,
) {
    /** [resume], resolved against the tabs that exist right now. */
    val resumeTab: Int
        get() = resume?.takeIf { it in tabs.indices } ?: 0

    /**
     * This workspace's rung.
     *
     * **The top rung is `wantsAttention` and not the amber ring**, which is the
     * one place the shell's sort and the shell's DRAWING deliberately part
     * company. A finished turn draws the REVIEW ring rather than the amber one
     * — `GlanceMark.of` maps it there — but it is still a workspace you should
     * be shown first, and `AgentActivity.wantsAttention` has been this app's
     * single answer to "should this interrupt someone" since before the glance
     * vocabulary existed. So a done tab sorts on the needs-you rung while
     * drawing a rung below it, and the `TO_REVIEW` clause under this one is
     * therefore about a DIFF: a finished turn has already been claimed by the
     * line above. iOS reaches the same ordering by a shorter road, because over
     * there `wantsAttention` maps onto the amber ring directly.
     *
     * This is stated at length because it looks like an inconsistency and is
     * not: what a mark SAYS and what a list SORTS BY are different questions,
     * and folding them together is what made a finished agent and a failed one
     * the same dot.
     */
    val precedence: ShellPrecedence
        get() = when {
            tabs.any { it.wantsAttention } -> ShellPrecedence.NEEDS_YOU
            tabs.any { it.mark?.attention == GlanceMark.Attention.TO_REVIEW } ->
                ShellPrecedence.UNREAD_DIFF
            tabs.isNotEmpty() && tabs.all { it.mark?.link == GlanceMark.Link.BROKEN } ->
                ShellPrecedence.ALL_STALE
            else -> ShellPrecedence.WORKING
        }
}

/** Which way a swipe goes. */
enum class ShellDirection {
    PREVIOUS,
    NEXT;

    /** Which way the track slides. Next brings in the pane on the right. */
    val trackSign: Float get() = if (this == NEXT) -1f else 1f
}

/** A place in the fleet. */
data class ShellPosition(val workspace: Int, val tab: Int)

/**
 * A place to go, and what it costs to get there.
 *
 * [crossesWorkspace] and [crossesRunner] are what the incoming pane's title
 * shows before you commit — the crossing is made visible rather than discovered.
 */
data class ShellStep(
    val position: ShellPosition,
    val crossesWorkspace: Boolean,
    val crossesRunner: Boolean,
)

/**
 * Which surface a swipe started on.
 *
 * The bar moves by WORKSPACE; the content moves by TAB along one flat sequence.
 * Same thresholds, different destinations, which is why every function that
 * steps takes this.
 */
enum class ShellTrack {
    BAR,
    CONTENT,
}

/** The whole fleet, as the shell sees it. */
data class ShellFleet(val workspaces: List<ShellWorkspace>) {

    val isEmpty: Boolean get() = workspaces.isEmpty()

    fun tabCount(workspace: Int): Int =
        workspaces.getOrNull(workspace)?.tabs?.size ?: 0

    fun contains(position: ShellPosition): Boolean =
        workspaces.getOrNull(position.workspace)?.tabs?.indices?.contains(position.tab) == true

    fun tab(at: ShellPosition): ShellTab? =
        if (contains(at)) workspaces[at.workspace].tabs[at.tab] else null

    fun position(tabId: String): ShellPosition? {
        workspaces.forEachIndexed { w, workspace ->
            val t = workspace.tabs.indexOfFirst { it.id == tabId }
            if (t >= 0) return ShellPosition(w, t)
        }
        return null
    }

    /** The first tab that exists anywhere, or null for a fleet with none. */
    val first: ShellPosition?
        get() {
            workspaces.forEachIndexed { i, w -> if (w.tabs.isNotEmpty()) return ShellPosition(i, 0) }
            return null
        }

    /**
     * One step, along whichever track the gesture started on, or null when there
     * is nothing that way and the drag should rubber-band instead.
     */
    fun step(
        from: ShellPosition,
        direction: ShellDirection,
        along: ShellTrack,
    ): ShellStep? {
        if (from.workspace !in workspaces.indices) return null
        return when (along) {
            ShellTrack.BAR -> barStep(from.workspace, direction)
            ShellTrack.CONTENT -> contentStep(from, direction)
        }
    }

    /** The bar moves whole workspaces, landing on wherever that one was left. */
    private fun barStep(workspace: Int, direction: ShellDirection): ShellStep? {
        val next = if (direction == ShellDirection.NEXT) workspace + 1 else workspace - 1
        if (next !in workspaces.indices) return null
        return ShellStep(
            ShellPosition(next, workspaces[next].resumeTab),
            crossesWorkspace = true,
            crossesRunner = workspaces[next].runnerId != workspaces[workspace].runnerId,
        )
    }

    /**
     * The content walks ONE FLAT SEQUENCE across the whole fleet: the next tab
     * if there is one, else the next workspace's first, else nothing.
     *
     * **Flat across runners too, for now, and that is an open question rather
     * than a decision.** iOS is flat because a `Connection` is one runner and
     * the sequence cannot leave it; here it can, and swiping from a workspace on
     * a laptop into one on a build box is a bigger move than the gesture
     * suggests. The alternative — stop at a runner boundary and require the bar
     * or the overview to cross it — is a one-branch change confined to this
     * function, which is why the flag it would need ([ShellStep.crossesRunner])
     * is already computed and already carried. Written up for the owner rather
     * than settled here.
     */
    private fun contentStep(from: ShellPosition, direction: ShellDirection): ShellStep? {
        val here = workspaces[from.workspace]
        fun crossing(to: ShellPosition) = ShellStep(
            to,
            crossesWorkspace = true,
            crossesRunner = workspaces[to.workspace].runnerId != here.runnerId,
        )
        return when (direction) {
            ShellDirection.NEXT -> {
                if (from.tab + 1 in here.tabs.indices) {
                    return ShellStep(
                        ShellPosition(from.workspace, from.tab + 1),
                        crossesWorkspace = false,
                        crossesRunner = false,
                    )
                }
                // Skips empty workspaces rather than landing on one: a workspace
                // with no tabs is not somewhere a page turn can put you.
                var w = from.workspace + 1
                while (w in workspaces.indices) {
                    if (workspaces[w].tabs.isNotEmpty()) return crossing(ShellPosition(w, 0))
                    w += 1
                }
                null
            }
            ShellDirection.PREVIOUS -> {
                if (from.tab - 1 >= 0 && from.tab - 1 in here.tabs.indices) {
                    return ShellStep(
                        ShellPosition(from.workspace, from.tab - 1),
                        crossesWorkspace = false,
                        crossesRunner = false,
                    )
                }
                var w = from.workspace - 1
                while (w >= 0) {
                    val count = workspaces[w].tabs.size
                    // The previous workspace's LAST tab, so the flat sequence
                    // reverses exactly: stepping back and forward returns you.
                    if (count > 0) return crossing(ShellPosition(w, count - 1))
                    w -= 1
                }
                null
            }
        }
    }

    /** Whether a drag this way has nowhere to go and should resist instead. */
    fun rubberBands(at: ShellPosition, direction: ShellDirection, along: ShellTrack): Boolean =
        step(at, direction, along) == null

    /**
     * The overview's order: by rung, then by the fleet's own order within a rung.
     *
     * Stable on the index, so two workspaces at the same rung keep the order
     * they arrived in rather than swapping places on a poll.
     */
    fun overviewOrder(): List<Int> =
        workspaces.indices.sortedWith(
            compareBy({ workspaces[it].precedence.ordinal }, { it })
        )

    /** The same, filtered by name. A blank query filters nothing. */
    fun overviewOrder(query: String): List<Int> {
        val needle = query.trim()
        if (needle.isEmpty()) return overviewOrder()
        return overviewOrder().filter { workspaces[it].name.contains(needle, ignoreCase = true) }
    }
}

/** Which way a gesture was decided to be going. */
enum class ShellAxis {
    HORIZONTAL,
    VERTICAL,
}

/** The gesture arithmetic. Every function here is pure and every one is tested. */
object ShellGesture {

    /**
     * Which way this gesture is going, or null while it is still too small to
     * say.
     *
     * **`dy` is UP-POSITIVE**, matching the rest of this file, and the two
     * absolute values are not decoration — they are the fix for the bug iOS
     * shipped. It read `abs(dx) > dy`, so a DOWNWARD drag (negative `dy`) lost
     * to any horizontal component whatsoever and **every downward drag
     * classified as horizontal**. A thumb travelling 79dp across while going
     * 511dp down is about 9° off vertical and well past [ShellMetrics.PAGE_COMMIT],
     * so scrolling a transcript turned the page.
     *
     * **The FIRST answer, and on the CONTENT the only one.** A caller on the
     * content that recomputes this every frame has reintroduced the class of
     * bug this returns null to prevent: vertical there belongs to whatever
     * vertical scroller is under the finger. On the BAR nothing else is
     * listening — down does nothing, up is the column, sideways is the
     * workspace — so [lean] starts here and then keeps asking.
     *
     * The tie goes to VERTICAL, and that is the answer to what wins on the
     * diagonal for a gesture with no history. After the first six points there
     * IS a history, and [lean] hands the diagonal to it.
     */
    fun axis(dx: Float, up: Float): ShellAxis? {
        if (max(abs(dx), abs(up)) <= ShellMetrics.AXIS_LOCK) return null
        return if (abs(dx) > abs(up)) ShellAxis.HORIZONTAL else ShellAxis.VERTICAL
    }

    /**
     * Which axis a BAR gesture is leaning toward NOW, given what it was leaning
     * toward a frame ago.
     *
     * **This is the redirection.** The owner's ask: *"the user can start
     * swiping horizontally, then decide they want to swipe vertically instead,
     * or vice versa."* WWDC 2018 803 on why it is worth the machinery: *"when
     * it's redirectable, the thought and gesture happen in parallel. And you
     * sort of think it with the gesture, and it turns out this is way faster
     * than thinking before doing."*
     *
     * The first answer is [axis] unchanged, so a gesture that never redirects
     * means exactly what it always meant — every straight-line drag, since
     * along a straight line the ratio is constant and an incumbent can never be
     * beaten by [ShellMetrics.REDIRECT] times itself.
     *
     * **Null is not reachable a second time.** A gesture that has decided an
     * axis has moved, and a thing that has moved is not a tap however far back
     * toward the origin it comes; the [ShellMetrics.AXIS_LOCK] guard is asked
     * only while there is no incumbent. Reaching null again would resolve a
     * drag out and back to [ShellRelease.ToggleColumn].
     *
     * **[holdingPage] takes the gesture away from the lean entirely.** Once the
     * page has left the display it is in your hand and both directions are its
     * own — see [pageIsHeld], and [ShellFleet.barRelease]'s Carry arm, which is
     * that composition and predates all of this. Handing a held page to
     * HORIZONTAL would drop it back onto the display while the thumb was still
     * up in the air holding it. So the redirection is bounded by exactly the
     * line the page leaves the glass at: below it the two axes are alternatives
     * and this arbitrates; at and above it they compose and there is nothing to
     * arbitrate.
     */
    fun lean(dx: Float, up: Float, from: ShellAxis?, holdingPage: Boolean = false): ShellAxis? {
        if (holdingPage) return ShellAxis.VERTICAL
        val current = from ?: return axis(dx, up)
        val across = abs(dx)
        val along = abs(up)
        return when (current) {
            ShellAxis.HORIZONTAL ->
                if (along > across * ShellMetrics.REDIRECT) ShellAxis.VERTICAL
                else ShellAxis.HORIZONTAL
            ShellAxis.VERTICAL ->
                if (across > along * ShellMetrics.REDIRECT) ShellAxis.HORIZONTAL
                else ShellAxis.VERTICAL
        }
    }

    /**
     * How far the track actually moves, given how far the finger did.
     *
     * Resistance ONLY at the true ends of the fleet, where there is nothing to
     * bring in. Within the fleet the pane follows the finger exactly, because
     * anything else makes the neighbour look like a preview rather than a page.
     */
    fun translation(dx: Float, rubberBanding: Boolean): Float =
        if (rubberBanding) dx * ShellMetrics.RUBBER_BAND else dx

    /** Which way a horizontal drag is asking to go. Zero asks for nothing. */
    fun direction(dx: Float): ShellDirection? = when {
        dx < 0 -> ShellDirection.NEXT
        dx > 0 -> ShellDirection.PREVIOUS
        else -> null
    }

    /**
     * A scroll view's own deceleration, as a plain number.
     *
     * 0.998 — `UIScrollView.DecelerationRate.normal`, which Android's own
     * `ScrollView` friction is within a per cent of — and the unit is
     * "fraction of the velocity surviving one millisecond", which is where the
     * 1000 in [project] comes from. The same constant as iOS, deliberately:
     * the two platforms have to throw the same distance for the same flick, or
     * the shell is two shells.
     */
    const val DECELERATION_RATE = 0.998f

    /**
     * Where content thrown at [velocity] would come to rest.
     *
     * The projection function from WWDC 2018 803, in the units a release
     * arrives in: units per second in, units out. The talk on the version of
     * this shell that shipped without it — *"the issue here is that we're only
     * looking at position, we're completely ignoring the momentum"* — and on
     * why the rate is a scroll view's rather than a tuned one: a flick here
     * travels exactly as far as a flick in every other scroller on the device,
     * so there is nothing new to learn about how far a throw goes.
     */
    fun project(velocity: Float, decelerationRate: Float = DECELERATION_RATE): Float {
        if (decelerationRate <= 0f || decelerationRate >= 1f) return 0f
        return (velocity / 1000f) * decelerationRate / (1f - decelerationRate)
    }

    /**
     * Where a drag that has travelled [travel] and is still moving at
     * [velocity] would end up.
     *
     * **Every ESCAPE decision is measured against this and not against the
     * translation** — turn the page, leave for the overview — because an
     * escape is a decision about where the gesture was GOING. A slow
     * deliberate drag projects almost nothing and lands where it was pointed;
     * a flick projects hundreds of units and escapes.
     *
     * **Which row is selected is NOT measured against it.** That is live
     * feedback with a highlight under the finger, and confirming a row other
     * than the lit one would be a worse defect than the one this fixes.
     */
    fun projected(travel: Float, velocity: Float): Float = travel + project(velocity)

    /**
     * Whether a horizontal THROW has gone far enough to turn the page.
     *
     * [dx] is a throw distance, not a translation: both release sites pass
     * [projected], and so does the [direction] read beside it.
     */
    fun commits(dx: Float): Boolean = abs(dx) >= ShellMetrics.PAGE_COMMIT

    /**
     * Which row a touch at [above] dp above the bar's top edge lands on, or
     * null for a touch outside the column.
     *
     * **The only mapping from a finger to a column row, for the tap and the
     * drag alike.** There used to be two: this one, and `columnSelection`,
     * which answered a DRAG off its travel alone — `tabCount - ceil(up / 44)`,
     * a pure delta with no idea where the finger went down. Write `d` for how
     * far below the column's bottom edge the touch landed and the two agree
     * only at `d == 0`; at `d == 44` the delta one sat a full row ABOVE the
     * finger for the whole gesture, and a 20dp lift from there highlighted the
     * last row while the thumb was still 24dp below the column entirely. iOS
     * shipped that and its owner reported it; the delta mapping is gone from
     * both platforms.
     *
     * **Three bugs meet in this function**, and all three shipped on iOS.
     *
     * The first is the inversion: the column unfurls upward, so the row nearest
     * the bar is the LAST tab, and the arithmetic has to count from the bottom
     * and then flip. Counting the other way put every tap one column away from
     * where it looked.
     *
     * The second is not in this function and cannot be fixed inside it, so it is
     * recorded here where whoever calls it will read it: **[above] must be
     * measured against the bar's real bottom edge, not re-derived** from a safe
     * area plus a gap. iOS re-derived it, and a padding change silently sent
     * every tap one row off. The Compose form of the same mistake is computing
     * this from `WindowInsets` instead of from the bar's own `onGloballyPositioned`
     * bounds.
     *
     * [bias] shifts the answer DOWN the column — see [ShellMetrics.ROW_BIAS],
     * where the eleven is argued. It charges two prices and they are both
     * here: the row nearest the bar owns `ROW_HEIGHT + bias` of travel rather
     * than 44, and the region reaches [bias] past the column's own top edge so
     * that the top row keeps a full row of its own with a margin above it
     * rather than being squeezed. Past that margin the finger has left the
     * menu, the page has begun to rise, and there is no column drawn to choose
     * from.
     */
    fun columnRow(
        above: Float,
        tabCount: Int,
        bias: Float = ShellMetrics.ROW_BIAS,
    ): Int? {
        if (tabCount <= 0 || above <= 0f || above > columnFull(tabCount) + bias) return null
        val fromBottom =
            min(tabCount - 1, max(0, ((above - bias) / ShellMetrics.ROW_HEIGHT).toInt()))
        return tabCount - 1 - fromBottom
    }

    /** How tall the column is when every row is out. */
    fun columnFull(tabCount: Int): Float = tabCount * ShellMetrics.ROW_HEIGHT

    /**
     * The column's height right now.
     *
     * **Pinned is a separate fact from the drag, and keeping them separate is
     * deliberate.** The prototype held `colOpen` apart from `dragY` because
     * deriving visibility from both produced a real bug where a tap toggled the
     * wrong way. One source of truth per thing.
     */
    fun columnHeight(up: Float, tabCount: Int, pinned: Boolean): Float {
        val full = columnFull(tabCount)
        if (pinned) return full
        return if (up >= ShellMetrics.OPEN_MIN) full else 0f
    }

    /** How far past the last row the drag has gone, in dp. */
    fun pageRise(up: Float, tabCount: Int): Float = max(0f, up - columnFull(tabCount))

    /**
     * Whether the page has left the glass and is being carried.
     *
     * **The line where the bar stops arbitrating between its two axes and
     * starts composing them.** Below it they are alternatives: the vertical
     * draws a menu, the horizontal slides the whole track, and drawing both at
     * once slides an open column sideways off the bar it grew out of — so
     * exactly one is drawn and [ShellGesture.lean] says which. At and above it
     * they compose: the lift decides whether you stay up, sideways decides
     * which cell you land in, and `barRelease`'s Carry arm answers both off one
     * release.
     */
    fun pageIsHeld(up: Float, tabCount: Int): Boolean = pageRise(up, tabCount) > 0f

    /** How far into the overview this drag has got, 0…1. */
    fun overviewProgress(up: Float, tabCount: Int): Float =
        min(1f, max(0f, pageRise(up, tabCount) / ShellMetrics.OVER_RUN))

    /** How far the column has unfurled, 0…1. A fleet with no tabs is fully out. */
    fun columnProgress(up: Float, tabCount: Int): Float {
        val full = columnFull(tabCount)
        if (full <= 0f) return 1f
        return min(1f, max(0f, up / full))
    }

    /**
     * Whether a pull-down should leave the overview.
     *
     * **Gated on where the gesture BEGAN, not where it ended**, which is the
     * third bug iOS shipped. Scrolling back up through forty cards ends at the
     * top with a large downward translation — character for character the same
     * release a deliberate pull-down produces — so a check at release reopened
     * the last workspace every time somebody scrolled the grid back to the top.
     *
     * @param begunAtTop captured on the FIRST change of the gesture and not
     *   re-read afterwards.
     */
    fun overviewDismisses(begunAtTop: Boolean, down: Float): Boolean =
        begunAtTop && down >= ShellMetrics.PAGE_COMMIT
}

/**
 * The part of a BAR gesture that is not a pure function of where the finger is:
 * which axis it is leaning toward, and what each axis owes the other once it has
 * changed its mind.
 *
 * **The one stateful thing in this file, and it is here for the same reason
 * everything else is.** [ShellGesture] is deliberately stateless, which works
 * for as long as every question can be answered from the current translation
 * alone. The redirection cannot: whether this frame is a handover depends on
 * what the last frame answered, and what a handover charges depends on where
 * the finger was when it happened. Left in a view that is three pieces of
 * remembered state and a fifteen-line callback — the shape of thing no test can
 * reach, and the reason the axis stayed a lock for as long as it did.
 *
 * **It decides nothing on its own.** [ShellGesture.lean] for the axis,
 * [ShellGesture.pageIsHeld] for when the two axes stop competing,
 * [ShellGesture.pageRise] for what a handover charges the lift. This type is
 * their lifetime and nothing else.
 *
 * Android has no bar surface yet — `PaneTrack` is the content, and its header
 * is where the split between the two surfaces is argued from Android's own
 * side — so nothing calls this today. It is here because the model is the story
 * both platforms tell, and a bar built later against a shared model that had
 * quietly stopped sharing this would be a bar with a locked axis and no note
 * saying why.
 */
class ShellBarDrag {
    /** Which axis the gesture is answering, or null while it is still a tap. */
    var axis: ShellAxis? = null
        private set

    /**
     * Sideways travel this gesture spent on an axis it has since left.
     *
     * Both sideways channels measure from it — the track below the last row,
     * the carried card above it — because it is one fact about the gesture
     * rather than a property of either.
     */
    var spentSideways = 0f
        private set

    /**
     * Lift this gesture spent before the vertical claimed it, and NOT the same
     * rule as [spentSideways].
     *
     * **Positions are re-based; a pop is adopted whole.** The column is shut
     * and then whole, and the row it lights is read off the finger's absolute
     * place on the glass — no in-between to teleport through, and nothing
     * bought by making a finger already 200dp above the bar travel 16 more
     * before the menu it is plainly asking for appears. The PAGE past the last
     * row is a position and may not jump, so everything past the last row is
     * travel this gesture has not made yet.
     *
     * Which is [ShellGesture.pageRise] exactly, and deliberately: "how much of
     * this lift has moved the page" and "how much of it may not be handed to
     * whoever claims it next" are the same question.
     *
     * **It is a charge on TRAVEL, never on PLACE.** The row a column
     * highlights is read off the finger's absolute point on the glass, so this
     * never moves a highlight. But whatever decides whether the menu is DRAWN
     * must add this back first: iOS asked the charged lift whether the finger
     * was past the last row, and after a charge that is a different question
     * with a different answer — a gesture that swipes 150dp sideways and turns
     * upward hands over at 212, this puts the travel at exactly the column's
     * last row, and the menu sprang open with the thumb 80dp above every row
     * in it and shut on the next frame.
     */
    var spentLift = 0f
        private set

    /**
     * Whether the page has been off the display at any point in this gesture.
     *
     * Latched: a card brought back down out of the overview's run is still in
     * your hand, and one that fell out of it because you lowered your thumb
     * forty dp would be the drop the rule exists to prevent, reached more
     * slowly.
     */
    var holdingPage = false
        private set

    /** What one frame of a bar drag comes to. */
    data class Frame(
        val axis: ShellAxis?,
        /** The lift the column and the page are drawn at: floored, and net of any handover. */
        val lift: Float,
        /** The sideways travel the track — or the carried card — is drawn at, before banding. */
        val sideways: Float,
        /**
         * Which axis has just taken the gesture off the other, or null if
         * nothing changed hands this frame.
         *
         * A view needs this and only this to run the handover's other half:
         * putting back what the abandoned channel had drawn, eased, because it
         * is no longer the finger's position but an apology for having moved.
         * That is a transaction, not a number, so it cannot live here.
         */
        val claimed: ShellAxis?,
    )

    /**
     * One frame of the finger.
     *
     * [up] is up-positive and RAW — the whole translation, not the lift —
     * because the lean is about which way the finger is going while the charges
     * are about how much of that each channel has been given. Mixing them would
     * make the ratio a function of its own history.
     */
    fun moved(dx: Float, up: Float, tabCount: Int): Frame {
        val lift = max(0f, up - spentLift)
        // Read before the lean and never unset: once the page is off the
        // display both directions are its own, and the lean stops being asked
        // for the rest of the gesture.
        if (axis == ShellAxis.VERTICAL && ShellGesture.pageIsHeld(lift, tabCount)) {
            holdingPage = true
        }
        val was = axis
        axis = ShellGesture.lean(dx, up, was, holdingPage)
        val claimed = axis
        // A first answer hands nothing over: both channels are at rest, so
        // there is nothing to put back and nothing to charge.
        if (claimed == null || claimed == was || was == null) {
            return Frame(axis, lift, dx - spentSideways, null)
        }
        // One line for both directions, because it is one fact: the sideways
        // travel this gesture has already spent.
        spentSideways = dx
        // The lift's charge belongs to whoever is holding the vertical, so it
        // is recomputed when the vertical takes the gesture and DROPPED when it
        // loses it. Left standing across a hand-back it is a number that no
        // longer describes anything.
        spentLift =
            if (claimed == ShellAxis.VERTICAL) ShellGesture.pageRise(up, tabCount) else 0f
        return Frame(axis, max(0f, up - spentLift), dx - spentSideways, claimed)
    }
}

/** What a release means. */
sealed interface ShellRelease {
    /** Turn the page to here. */
    data class Commit(val step: ShellStep) : ShellRelease

    /** Nothing was chosen; slide back. */
    data object SpringBack : ShellRelease

    /** Land on this tab of the current workspace. */
    data class Land(val tab: Int) : ShellRelease

    /** The page has flown; show the grid. */
    data object OpenOverview : ShellRelease

    /** Held the page AND swiped sideways: arrive at [step] still holding it. */
    data class Carry(val step: ShellStep) : ShellRelease

    /** Cost nothing; put everything back. */
    data object Abandon : ShellRelease

    /** A tap on the bar itself, which opens or shuts the column. */
    data object ToggleColumn : ShellRelease
}

/**
 * What releasing a drag that started on the BAR means.
 *
 * @param row which column row the finger came up over, from
 *   [ShellGesture.columnRow], or null when there was no open column under it.
 *   **This parameter is the whole of two shipped bugs.** The column draws rows
 *   and declares no target of its own, so the only recognizer on that surface
 *   is the bar's drag — and a tap on a row resolved to
 *   [ShellRelease.ToggleColumn], which SHUT the menu the person was trying to
 *   use. Opening had always worked because opening IS the toggle, which is why
 *   it took a real phone to find. A caller that passes null here has that bug
 *   back. It answers the DRAG as well as the tap now, because deriving a
 *   drag's row from its travel instead is the second bug, and
 *   [ShellGesture.columnRow] has the arithmetic.
 * @param dxVelocity horizontal speed at the instant of release, units per
 *   second.
 * @param upVelocity vertical speed at the instant of release, UP-positive the
 *   same way [up] is. **The two velocities decide the two escapes and nothing
 *   else** — far enough sideways to turn the page, far enough up to stay in the
 *   overview — because those are questions about where the gesture was going,
 *   while which row is lit is a highlight somebody is looking at. Both default
 *   to zero, which is the honest reading of a caller with no velocity to give.
 */
fun ShellFleet.barRelease(
    axis: ShellAxis?,
    dx: Float,
    up: Float,
    at: ShellPosition,
    row: Int? = null,
    dxVelocity: Float = 0f,
    upVelocity: Float = 0f,
): ShellRelease {
    if (axis == null) {
        return row?.let { ShellRelease.Land(it) } ?: ShellRelease.ToggleColumn
    }
    // Where the sideways half of this gesture was HEADED, which is what both of
    // its escapes are decided on.
    val thrownX = ShellGesture.projected(dx, dxVelocity)
    return when (axis) {
        ShellAxis.HORIZONTAL -> {
            val direction =
                if (ShellGesture.commits(thrownX)) ShellGesture.direction(thrownX) else null
            val step = direction?.let { step(at, it, ShellTrack.BAR) }
            step?.let { ShellRelease.Commit(it) } ?: ShellRelease.SpringBack
        }
        ShellAxis.VERTICAL -> {
            val tabs = tabCount(at.workspace)
            // A sideways component only counts once the page has left the glass:
            // below that the gesture is unfurling the column and a little
            // horizontal drift is a thumb, not an instruction. `pageIsHeld`
            // reads the REAL lift and not the thrown one — it asks whether
            // there is a page in your hand right now, which is a fact about the
            // screen rather than a prediction.
            val sideways =
                if (ShellGesture.pageIsHeld(up, tabs) && ShellGesture.commits(thrownX)) {
                    ShellGesture.direction(thrownX)?.let { step(at, it, ShellTrack.BAR) }
                } else null

            // The escape, and the one place the lift is projected. A flick up
            // from over a menu row is asking for the grid; only reading where
            // the thumb happened to be when it left the glass gave it the row.
            val thrownUp = ShellGesture.projected(up, upVelocity)
            when {
                thrownUp >= ShellGesture.columnFull(tabs) + ShellMetrics.OVER_RUN ->
                    sideways?.let { ShellRelease.Carry(it) } ?: ShellRelease.OpenOverview
                sideways != null -> ShellRelease.Commit(sideways)
                // Not an escape: the row under the finger, and the ACTUAL
                // finger. The OPEN_MIN gate is the one thing still read off the
                // travel, and it is not a mapping — it is the line between a
                // bar touched and moved a little, which must cost nothing, and
                // a tab chosen.
                up >= ShellMetrics.OPEN_MIN && row != null -> ShellRelease.Land(row)
                else -> ShellRelease.Abandon
            }
        }
    }
}

/**
 * What releasing a drag that started on the CONTENT means.
 *
 * Same thresholds as the bar, one flat sequence instead of whole workspaces, and
 * no vertical behaviour at all: a vertical drag on a pane belongs to the pane,
 * which is scrolling.
 *
 * @param dxVelocity the finger's sideways speed at release, and the whole of
 *   what makes a short fast flick across a pane turn the page. Forty units
 *   thrown at 600 a second projects past 300 and commits; forty units placed
 *   deliberately projects almost nothing and springs back. The same forty
 *   units meaning two different things is why a threshold on translation alone
 *   was wrong — [ShellGesture.projected].
 */
fun ShellFleet.contentRelease(
    axis: ShellAxis?,
    dx: Float,
    at: ShellPosition,
    dxVelocity: Float = 0f,
): ShellRelease {
    val thrown = ShellGesture.projected(dx, dxVelocity)
    if (axis != ShellAxis.HORIZONTAL || !ShellGesture.commits(thrown)) {
        return ShellRelease.SpringBack
    }
    val direction = ShellGesture.direction(thrown) ?: return ShellRelease.SpringBack
    val step = step(at, direction, ShellTrack.CONTENT) ?: return ShellRelease.SpringBack
    return ShellRelease.Commit(step)
}

/**
 * Where the track's panes sit, and what a commit does to them.
 *
 * Three panes side by side — the one you are on, with a real previous and a real
 * next — each the width of a page, the whole track slid by however far your
 * thumb has gone. The neighbours are genuinely mounted and genuinely drawn,
 * which is the entire difference between an incoming terminal and a placeholder
 * that turns into one after you commit.
 *
 * ## Why the commit is arithmetic here rather than an animation trick
 *
 * The prototype animates the track to the neighbour, then re-centres it on the
 * new item with transitions disabled for one frame and restores them two
 * `requestAnimationFrame`s later. Skip that and the transform animates back to
 * zero and you watch the page jump back. iOS reaches for
 * `completionCriteria: .logicallyComplete` and a `Transaction` with
 * `disablesAnimations` to do the same thing properly.
 *
 * Both are working around the same thing: changing which pane is current and
 * resetting the offset are two writes, and a frame that lands between them
 * renders a lie.
 *
 * **[residual] removes the gap instead of racing it.** Moving the current index
 * one way and shifting the offset one page the other way are the SAME transform
 * — see [committingMovesNothing] in the tests, which asserts it for every slot
 * rather than for the incoming pane alone. So the commit does not animate to the
 * neighbour and then reset; it re-parameterises in place, at which point every
 * pane is drawn at exactly the pixel it was already at, and the settle animates
 * the residual to zero from there. There is no frame that can be wrong, because
 * the swap frame is pixel-identical to the one before it.
 *
 * That is not a cleverness the other platforms lack the ability to copy — it is
 * available to all three, and it is written down here because it is the version
 * that does not need a completion callback to be correct.
 */
object ShellTrackGeometry {

    /**
     * Where a pane sits, in pixels from the current pane's resting place.
     *
     * @param slot 0 for the pane you are on, −1 for the previous, +1 for the
     *   next. A mounted pane that is on neither side of you has no slot and is
     *   not drawn — see `PaneTrack`.
     * @param offset how far the track has been dragged, in pixels. Negative is
     *   dragging left, which brings the NEXT pane in from the right.
     */
    fun x(slot: Int, pageWidth: Float, offset: Float): Float = slot * pageWidth + offset

    /**
     * The offset that leaves every pane exactly where it already is, once the
     * current pane has moved one step in [direction].
     *
     * Committing to the next pane shifts every slot down by one, so the offset
     * has to grow by one page to compensate; committing to the previous does the
     * reverse. [ShellDirection.trackSign] is the same −1/+1 the track slides by,
     * which is why this is a subtraction rather than a `when`.
     */
    fun residual(offset: Float, direction: ShellDirection, pageWidth: Float): Float =
        offset - direction.trackSign * pageWidth

    /**
     * How much of an incoming neighbour is drawn while it is on its way.
     *
     * The prototype's 0.72, and it is doing a job: a pane at full strength
     * sliding under your thumb reads as already yours, and a page turn you have
     * not committed to should not. It resolves to 1 the instant the commit lands,
     * because the incoming pane becomes the current one and current panes are
     * never dimmed.
     */
    const val NEIGHBOUR_ALPHA = 0.72f
}
