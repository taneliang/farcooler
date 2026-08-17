package com.farcooler.model

// Ported from the Mac's `QuickCreate.swift` (`Branch`) and `Composer.swift`
// (`Agents`), by way of the iOS app's `QuickTask.swift`. Pure logic, no UI and
// no host calls, so it is copied verbatim rather than re-derived: the whole
// point is that a phone, a tablet and a Mac describing the same task land on
// the same branch name and offer the same agents, and the only way to guarantee
// that is to make the rules identical rather than merely similar.

/**
 * Turning a sentence into a branch name and a worktree name.
 *
 * One sentence is both the worktree's name and, slugged, its branch. Keeping
 * the two derivations next to each other is what keeps them from drifting apart
 * the next time either one is tuned.
 */
object TaskSlug {
    /**
     * A git-safe slug, behind whatever the runner says branches start with.
     *
     * Conservative on purpose: git accepts far more than this, but a branch name
     * is something people type, paste into a PR title and see in a CI log, and
     * one carrying punctuation from a sentence is a small tax paid repeatedly.
     *
     * The prefix is applied HERE rather than by the daemon, because the composer
     * shows you the branch it is about to create — a prefix added on the far
     * side would make that preview a lie. The daemon still validates the
     * finished name.
     *
     * The 48-character budget is spent on the slug, not on the result: a long
     * prefix must not eat the part that says what the task was.
     */
    fun slug(text: String, prefix: String = ""): String {
        val out = StringBuilder()
        var lastWasDash = true // leading dashes are dropped

        for (character in text.lowercase()) {
            if (character.isLetter() || character.isDigit()) {
                out.append(character)
                lastWasDash = false
            } else if (!lastWasDash) {
                out.append('-')
                lastWasDash = true
            }
            // Long enough to stay readable, short enough for a terminal title.
            if (out.length >= 48) break
        }
        while (out.isNotEmpty() && out.last() == '-') out.deleteCharAt(out.lastIndex)
        return prefix + (if (out.isEmpty()) "task" else out.toString())
    }

    /**
     * The worktree's name, which is the directory it is created in.
     *
     * The slug again, minus the branch prefix, rather than the trimmed sentence
     * this used to hand over. A name is a path component now: the runner caps
     * it at 60 characters and refuses one with no letters or numbers left after
     * sanitizing, and a sentence can be either — "Ship it!!!" is the second.
     * Slugging is the one derivation that can be neither, and it costs nothing,
     * because the directory is read back as prose in the fleet list anyway.
     */
    fun name(text: String): String {
        // Through `sanitize` and not merely `slug`, because the two disagree
        // about what a letter is: Kotlin says yes to `é` and to 写, and the
        // runner — which keeps ASCII and dashes everything else — says no to
        // both. A description written in Chinese would otherwise slug to
        // something this thinks is a name and the daemon refuses outright.
        val name = sanitize(slug(text))
        return name.ifEmpty { "task" }
    }

    /**
     * The directory a typed name lands in.
     *
     * Duplicated from the runner for the same reason the branch prefix is
     * applied here rather than there: the form shows the folder it is about to
     * create, and a preview computed on the far side would be a preview that can
     * lie. The runner still has the last word, and refuses a name this leaves
     * empty.
     */
    fun sanitize(text: String): String {
        val out = StringBuilder()
        var lastWasDash = true // leading dashes are dropped
        for (character in text) {
            val kept = character == '_' || character in 'a'..'z' ||
                character in 'A'..'Z' || character in '0'..'9'
            if (kept) {
                out.append(character)
                lastWasDash = false
            } else if (!lastWasDash) {
                out.append('-')
                lastWasDash = true
            }
        }
        while (out.isNotEmpty() && out.last() == '-') out.deleteCharAt(out.lastIndex)
        return out.toString()
    }
}

/**
 * The agents Far Cooler knows how to launch, and the models worth offering.
 *
 * A short curated list plus "Default", not an exhaustive one. Copied verbatim
 * from the Mac so the pickers never disagree about what "codex" means.
 */
object QuickAgents {
    data class Agent(val id: String, val name: String, val models: List<String>)

    val all = listOf(
        Agent("claude", "Claude Code", listOf("opus", "sonnet", "haiku")),
        Agent("codex", "Codex", listOf("gpt-5.6-sol", "gpt-5.6-sol-high")),
        Agent("cursor", "Cursor", listOf("auto", "sonnet-4.5", "gpt-5")),
    )

    fun agent(id: String): Agent = all.firstOrNull { it.id == id } ?: all[0]

    /** The preset string the host expects: `agent` or `agent:model`. */
    fun preset(agent: String, model: String): String =
        if (model.isEmpty()) agent else "$agent:$model"
}

/**
 * The presets a terminal can be created with.
 *
 * The Mac offers this on every workspace; the iOS app only ever creates a
 * terminal as part of Quick Task, so a workspace that needed a second pane —
 * an agent and a shell to watch it, which is the ordinary layout on the Mac —
 * could not get one from a phone at all. `shell` is first because it is the one
 * a second pane is usually for.
 */
object TerminalPresets {
    data class Preset(val id: String, val name: String)

    val all = listOf(
        Preset("shell", "Shell"),
        Preset("claude", "Claude Code"),
        Preset("codex", "Codex"),
        Preset("cursor", "Cursor"),
    )
}
