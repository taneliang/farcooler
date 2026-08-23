package com.farcooler.model

/**
 * How one file's patch is cut up on the screen.
 *
 * Ported from the `DiffLayout` in `apps/ios/FarCooler/ChangesView.swift`, where
 * it is private to the view. Here it is in `model/` instead, and that is not
 * tidiness: this is arithmetic over line numbers with three off-by-one hazards
 * in it, no device was available for this work, and a JVM test can prove every
 * case of it. Keeping it out of the composable is what makes that possible —
 * the same split `model/NeedsYou.kt` and `ui/Navigation.kt` already make.
 */
object DiffLayout {
    /**
     * One run of lines with no gap in it.
     *
     * Derived from the line NUMBERS rather than from a `@@` header, because
     * `FileDiffReply` is already structured hunks and the flattening into
     * [DiffComputation.Line] drops the boundaries — a jump in the new file's
     * numbering is what is left of them, and it is enough.
     */
    data class Hunk(val id: Int, val lines: List<DiffComputation.Line>) {
        val firstLine: Int? get() = lines.firstNotNullOfOrNull { it.newNumber }
        val lastLine: Int? get() = lines.asReversed().firstNotNullOfOrNull { it.newNumber }

        /**
         * `Lines 120-148`, or nothing at all for a hunk with no new-side
         * numbering — every line of a deleted file, for one.
         */
        val rangeLabel: String?
            get() {
                val first = firstLine ?: return null
                val last = lastLine
                if (last == null || last <= first) return "Line $first"
                return "Lines $first-$last"
            }
    }

    fun hunks(lines: List<DiffComputation.Line>): List<Hunk> {
        val out = mutableListOf<Hunk>()
        var current = mutableListOf<DiffComputation.Line>()
        for ((index, line) in lines.withIndex()) {
            if (index > 0 && gap(lines[index - 1], line) && current.isNotEmpty()) {
                out.add(Hunk(out.size, current))
                current = mutableListOf()
            }
            current.add(line)
        }
        if (current.isNotEmpty()) out.add(Hunk(out.size, current))
        return out
    }

    private fun gap(previous: DiffComputation.Line, current: DiffComputation.Line): Boolean {
        val before = previous.newNumber ?: return false
        val after = current.newNumber ?: return false
        return after > before + 1
    }

    /** A stretch of a hunk, either drawn or folded away. */
    data class Segment(
        val id: Int,
        val lines: List<DiffComputation.Line>,
        /** How many unchanged lines this stands in for, when it is a fold. */
        val folded: Int?,
    )

    /** Lines kept either side of a folded run. */
    private const val KEEP = 2

    /**
     * The shortest run of unchanged lines worth folding at all.
     *
     * Five, which given the daemon's default three lines of context means this
     * fires on hunks git MERGED — a run of six unchanged lines between two
     * changes is what two nearby edits look like after `-U3` joins them. That is
     * exactly the shape an LLM refactor produces, twenty small edits scattered
     * down one file, and folding each gap to a tappable line saves several
     * screens of dragging over the length of the file. It never fires on a new
     * file, which has no unchanged lines at all.
     */
    private const val FOLD_FROM = 5

    fun segments(hunk: Hunk): List<Segment> {
        val out = mutableListOf<Segment>()
        val lines = hunk.lines
        var index = 0

        fun append(slice: List<DiffComputation.Line>, folded: Int?) {
            if (slice.isEmpty() && folded == null) return
            out.add(Segment(out.size, slice, folded))
        }

        while (index < lines.size) {
            if (lines[index].kind != DiffComputation.Kind.CONTEXT) {
                val start = index
                while (index < lines.size && lines[index].kind != DiffComputation.Kind.CONTEXT) {
                    index += 1
                }
                append(lines.subList(start, index), null)
                continue
            }
            val start = index
            while (index < lines.size && lines[index].kind == DiffComputation.Kind.CONTEXT) {
                index += 1
            }
            val run = lines.subList(start, index)
            val atStart = start == 0
            val atEnd = index == lines.size
            // Head and tail are what stays: the lines actually touching a change
            // are the ones that give it its place, and the ones further out are
            // the ones nobody reads.
            val head = if (atStart) 0 else KEEP
            val tail = if (atEnd) 0 else KEEP
            val hidden = run.size - head - tail
            if (run.size < FOLD_FROM || hidden < 2) {
                append(run, null)
                continue
            }
            if (head > 0) append(run.subList(0, head), null)
            append(run.subList(head, run.size - tail), hidden)
            if (tail > 0) append(run.subList(run.size - tail, run.size), null)
        }
        return out
    }
}
