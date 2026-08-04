package com.farcooler.model

// The diff a tool call's edit is rendered from.
//
// Ported from `apps/shared/AgentKit/Sources/AgentKit/DiffComputation.swift`,
// which exists because the algorithm was written once and then copied — same
// trimming, same LCS, same 160,000-cell cutoff — with only the doc comment
// drifting between the two Apple targets. Two copies of a diff algorithm is two
// places for an off-by-one to live and one place for it to be fixed. There is
// no way to share Swift with Kotlin, so this is a third copy; the unit tests
// are what hold it to the same answers.

/** The diffing itself, kept apart from the view so it has no Compose in it. */
object DiffComputation {
    enum class Kind { CONTEXT, ADDED, REMOVED }

    data class Line(
        val id: Int,
        val kind: Kind,
        val oldNumber: Int?,
        val newNumber: Int?,
        val text: String,
    )

    /**
     * Line-based diff of two whole file texts.
     *
     * A real Myers diff is O(n·d) and would be the right tool against two
     * arbitrary files; what a tool call actually hands over is usually one small
     * change inside two mostly-identical texts, so trimming the common prefix
     * and suffix first — O(n) — throws away nearly all of the work before the
     * expensive part ever runs. The expensive part, an LCS over whatever is left
     * in the middle, is only worth its O(n·m) on the leftover sliver; past a
     * size where that would be slow it falls back to "the whole middle changed",
     * which is still a correct diff, just not the minimal one.
     */
    fun compute(old: String, new: String): List<Line> {
        val oldLines = if (old.isEmpty()) emptyList() else old.split("\n")
        val newLines = if (new.isEmpty()) emptyList() else new.split("\n")

        var prefix = 0
        while (prefix < oldLines.size && prefix < newLines.size &&
            oldLines[prefix] == newLines[prefix]
        ) {
            prefix += 1
        }

        var suffix = 0
        while (suffix < oldLines.size - prefix && suffix < newLines.size - prefix &&
            oldLines[oldLines.size - 1 - suffix] == newLines[newLines.size - 1 - suffix]
        ) {
            suffix += 1
        }

        val oldMiddle = oldLines.subList(prefix, oldLines.size - suffix)
        val newMiddle = newLines.subList(prefix, newLines.size - suffix)

        val result = mutableListOf<Line>()
        var id = 0
        var oldNum = 1
        var newNum = 1

        fun push(kind: Kind, oldNumber: Int?, newNumber: Int?, text: String) {
            result.add(Line(id, kind, oldNumber, newNumber, text))
            id += 1
        }

        for (line in oldLines.subList(0, prefix)) {
            push(Kind.CONTEXT, oldNum, newNum, line)
            oldNum += 1
            newNum += 1
        }

        // 400*400 is a worst case of 160,000 cells, which is fast; past that
        // the middle is shown as a flat replace rather than paying for the
        // table.
        if (oldMiddle.size * newMiddle.size <= 160_000) {
            for (op in lcsDiff(oldMiddle, newMiddle)) {
                when (op) {
                    is Op.Equal -> {
                        push(Kind.CONTEXT, oldNum, newNum, op.line)
                        oldNum += 1
                        newNum += 1
                    }

                    is Op.Removed -> {
                        push(Kind.REMOVED, oldNum, null, op.line)
                        oldNum += 1
                    }

                    is Op.Added -> {
                        push(Kind.ADDED, null, newNum, op.line)
                        newNum += 1
                    }
                }
            }
        } else {
            for (line in oldMiddle) {
                push(Kind.REMOVED, oldNum, null, line)
                oldNum += 1
            }
            for (line in newMiddle) {
                push(Kind.ADDED, null, newNum, line)
                newNum += 1
            }
        }

        val suffixStart = oldLines.size - suffix
        for (offset in 0 until suffix) {
            push(Kind.CONTEXT, oldNum, newNum, oldLines[suffixStart + offset])
            oldNum += 1
            newNum += 1
        }

        return result
    }

    private sealed interface Op {
        data class Equal(val line: String) : Op
        data class Removed(val line: String) : Op
        data class Added(val line: String) : Op
    }

    /** Classic LCS table, walked backwards to recover the edit script. */
    private fun lcsDiff(a: List<String>, b: List<String>): List<Op> {
        if (a.isEmpty()) return b.map { Op.Added(it) }
        if (b.isEmpty()) return a.map { Op.Removed(it) }

        val table = Array(a.size + 1) { IntArray(b.size + 1) }
        for (i in a.indices.reversed()) {
            for (j in b.indices.reversed()) {
                table[i][j] =
                    if (a[i] == b[j]) table[i + 1][j + 1] + 1
                    else maxOf(table[i + 1][j], table[i][j + 1])
            }
        }

        val ops = mutableListOf<Op>()
        var i = 0
        var j = 0
        while (i < a.size && j < b.size) {
            when {
                a[i] == b[j] -> {
                    ops.add(Op.Equal(a[i]))
                    i += 1
                    j += 1
                }

                table[i + 1][j] >= table[i][j + 1] -> {
                    ops.add(Op.Removed(a[i]))
                    i += 1
                }

                else -> {
                    ops.add(Op.Added(b[j]))
                    j += 1
                }
            }
        }
        while (i < a.size) {
            ops.add(Op.Removed(a[i]))
            i += 1
        }
        while (j < b.size) {
            ops.add(Op.Added(b[j]))
            j += 1
        }
        return ops
    }
}
