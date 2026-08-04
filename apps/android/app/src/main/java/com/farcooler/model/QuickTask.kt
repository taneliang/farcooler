package com.farcooler.model

// Ported from the Mac's `QuickCreate.swift` (`Branch`) and `Composer.swift`
// (`Agents`), by way of the iOS app's `QuickTask.swift`. Pure logic, no UI and
// no host calls, so it is copied verbatim rather than re-derived: the whole
// point is that a phone, a tablet and a Mac describing the same task land on
// the same branch name and offer the same agents, and the only way to guarantee
// that is to make the rules identical rather than merely similar.

/**
 * Turning a sentence into a branch name and a short title.
 *
 * One sentence is both the task's name and, slugged, its branch. Keeping the
 * two derivations next to each other is what keeps them from drifting apart the
 * next time either one is tuned.
 */
object TaskSlug {
    /**
     * A git-safe slug.
     *
     * Conservative on purpose: git accepts far more than this, but a branch name
     * is something people type, paste into a PR title and see in a CI log, and
     * one carrying punctuation from a sentence is a small tax paid repeatedly.
     */
    fun slug(text: String): String {
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
        return if (out.isEmpty()) "task" else out.toString()
    }

    /** A short human title, for the workspace's task name. */
    fun title(text: String): String {
        val trimmed = text.trim()
        if (trimmed.length <= 42) return trimmed
        // Cut on a word boundary rather than mid-word.
        val cut = trimmed.substring(0, 42)
        val space = cut.lastIndexOf(' ')
        return if (space >= 0) cut.substring(0, space) + "…" else "$cut…"
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
