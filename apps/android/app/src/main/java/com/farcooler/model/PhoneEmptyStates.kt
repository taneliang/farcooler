package com.farcooler.model

/**
 * Which Material icon a row of an empty state draws. A name here and the
 * drawing in `ui/PhoneEmptyRows.kt`, so the words and their shape can be tested
 * on the JVM without Compose.
 */
enum class EmptyIcon { BUBBLE, CHECKLIST, HAND, WORKSPACE, ADD_PERSON, BRANCH, MERGE }

/**
 * What an empty state says under its title, as a short lede and a few icon rows
 * rather than a paragraph (ov-245).
 *
 * AgentKit's `PhoneEmptyCopy` and the Mac's `EmptyStateCopy` (ov-205), word for
 * word, for the same reason: the owner read a five-line paragraph under "No
 * Workspace Selected" as "too many words", and nobody scans a paragraph. So an
 * empty state says what the thing is for in one short line, then two or three
 * rows, each an icon and a few words, then its button.
 */
data class PhoneEmptyCopy(val lede: String, val rows: List<Row>) {
    /** One row: an icon and a few words, with no period, since a row reads as a list item. */
    data class Row(val icon: EmptyIcon, val text: String)
}

/** Every phone empty state that explains something, by name; `PhoneEmptyStatesTest` reads them. */
object PhoneEmptyStates {
    /** No orchestrator, in a workspace. The Mac's words. */
    val NO_ORCHESTRATOR = PhoneEmptyCopy(
        "An orchestrator runs this workspace’s board.",
        listOf(
            PhoneEmptyCopy.Row(EmptyIcon.BUBBLE, "Tell it what you want done"),
            PhoneEmptyCopy.Row(EmptyIcon.CHECKLIST, "It plans tasks and puts agents on them"),
            PhoneEmptyCopy.Row(EmptyIcon.HAND, "It asks you when it needs a decision"),
        ),
    )

    /** Under an empty Needs You, while no orchestrator runs anywhere. */
    val NO_AGENTS_WORKING = PhoneEmptyCopy(
        "No agents are working yet.",
        listOf(
            PhoneEmptyCopy.Row(EmptyIcon.WORKSPACE, "Each workspace is one line of work"),
            PhoneEmptyCopy.Row(EmptyIcon.ADD_PERSON, "Start a workspace’s orchestrator to begin"),
        ),
    )

    /** A runner that lists no repository. The Mac's words. */
    val NO_REPOSITORIES = PhoneEmptyCopy(
        "Add the repository you want agents to work in.",
        listOf(
            PhoneEmptyCopy.Row(EmptyIcon.BRANCH, "Each agent gets its own folder and branch"),
            PhoneEmptyCopy.Row(EmptyIcon.MERGE, "Your checkout changes only when you merge"),
        ),
    )

    /** A workspace's worktrees with none. The Mac's words. */
    val NO_WORKTREES = PhoneEmptyCopy(
        "A worktree is where an agent works.",
        listOf(
            PhoneEmptyCopy.Row(EmptyIcon.BRANCH, "It has its own folder and branch"),
            PhoneEmptyCopy.Row(EmptyIcon.MERGE, "Your checkout changes only when you merge"),
        ),
    )

    /** An empty board its orchestrator leads, running. */
    val BOARD_WITH_ORCHESTRATOR = PhoneEmptyCopy(
        "The orchestrator fills this board.",
        listOf(
            PhoneEmptyCopy.Row(EmptyIcon.BUBBLE, "Tell it what you want done"),
            PhoneEmptyCopy.Row(EmptyIcon.CHECKLIST, "Each piece of work becomes a task"),
        ),
    )

    /** The same with none running: start it first. */
    val BOARD_NO_ORCHESTRATOR = PhoneEmptyCopy(
        "The orchestrator fills this board.",
        listOf(
            PhoneEmptyCopy.Row(EmptyIcon.ADD_PERSON, "Start the orchestrator first"),
            PhoneEmptyCopy.Row(EmptyIcon.BUBBLE, "Tell it what you want done"),
            PhoneEmptyCopy.Row(EmptyIcon.CHECKLIST, "Each piece of work becomes a task"),
        ),
    )

    /** A board on a runner too old for workspaces: one line short enough to need no rows. */
    val BOARD_IMPLICIT = PhoneEmptyCopy("Each piece of work on this board appears here as a task.", emptyList())

    val all: List<PhoneEmptyCopy> = listOf(
        NO_ORCHESTRATOR, NO_AGENTS_WORKING, NO_REPOSITORIES, NO_WORKTREES, BOARD_WITH_ORCHESTRATOR,
        BOARD_NO_ORCHESTRATOR, BOARD_IMPLICIT,
    )

    /** Every word above, for the voice check beside [FirstRunCopy.all]. */
    internal val allStrings: List<String> get() = all.flatMap { listOf(it.lede) + it.rows.map { row -> row.text } }
}
