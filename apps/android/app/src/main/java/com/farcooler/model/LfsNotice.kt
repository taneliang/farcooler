package com.farcooler.model

/**
 * What the Changes screen says about a worktree's pointer files, or nothing
 * (ov-199). AgentKit's `LfsNotice`: the same decisions, in Material's sentence
 * case.
 *
 * A worktree of a Git LFS repository hydrates its large files best effort when
 * the runner makes it. An object missing from the runner's LFS store, or a
 * hydration that ran out of time, leaves the pointer file; the runner counts
 * them (`Worktree.lfs_pointers`) and `worktree.hydrate_lfs` tries again.
 */
data class LfsNotice(
    /** How many large files are still pointers. Always above zero. */
    val pointers: Int,
    /** False on the `read` scope, which sees the sentence without the button. */
    val canRetry: Boolean,
) {
    companion object {
        /**
         * The notice for a worktree reporting [pointers], or null when it
         * reports none. A runner that predates the count sends nothing, which
         * reads as no notice rather than as zero files being fine.
         */
        fun make(pointers: Int?, mayAct: Boolean): LfsNotice? =
            if (pointers == null || pointers <= 0) null else LfsNotice(pointers, mayAct)

        const val TITLE = "Some large files weren't downloaded."
        const val DETAIL = "If they aren't on this runner yet, run \"git lfs pull\" in the repository, then try again."
        const val RETRY = "Try again"
        const val RETRYING = "Downloading…"
    }
}
