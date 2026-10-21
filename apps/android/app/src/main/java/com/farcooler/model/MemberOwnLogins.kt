package com.farcooler.model

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull

/**
 * Whether a runner's admins require members to sign in to each agent with
 * their own account (ov-652), as `member.list`'s `own_logins` says it. Off
 * by default: a member's pane of that agent may run on the runner's own
 * login, its owner's account, quota and history. AgentKit's
 * `MemberOwnLogins`, in this platform's sentence case.
 */
data class MemberOwnLogins(
    val claude: Boolean = false,
    val codex: Boolean = false,
    val cursor: Boolean = false,
    /** Whether this phone may change them: the runner's admins only. */
    val mayChange: Boolean = false,
) {
    /** An agent with a switch, by the name the wire gives it. */
    enum class Agent(val wire: String, val title: String) {
        CLAUDE("claude", "Claude"),
        CODEX("codex", "Codex"),
        CURSOR("cursor", "Cursor"),
    }

    fun requiresOwn(agent: Agent): Boolean = when (agent) {
        Agent.CLAUDE -> claude
        Agent.CODEX -> codex
        Agent.CURSOR -> cursor
    }

    companion object {
        /** The switches in a `member.list` answer, or null from a runner before them. */
        fun of(reply: JsonObject): MemberOwnLogins? {
            val own = reply["own_logins"] as? JsonObject ?: return null
            fun flag(key: String) = (own[key] as? JsonPrimitive)?.booleanOrNull ?: false
            return MemberOwnLogins(flag("claude"), flag("codex"), flag("cursor"), flag("may_change"))
        }

        const val SECTION_TITLE = "Member logins"

        fun toggleTitle(agent: Agent): String = "Require own ${agent.title} account"

        /** The line under a switch: what it means now. */
        fun stateLine(agent: Agent, on: Boolean): String = when {
            agent == Agent.CLAUDE && on -> "Each member signs in to Claude here with their own account."
            agent == Agent.CLAUDE -> "Members who haven’t signed in use this runner’s Claude login."
            on -> "Members can’t use ${agent.title} here until they can connect their own account."
            else -> "Members use this runner’s ${agent.title} login."
        }

        /** The section's footer. [owner]: this phone is the runner owner's. */
        fun footer(owner: Boolean, mayChange: Boolean): String {
            val whose = if (owner) "your account" else "the runner owner’s account"
            return "A member on this runner’s login uses $whose, its quota and its history, " +
                "including Claude’s memory, Codex sessions and Cursor chats. " +
                "A change applies the next time each pane starts." +
                (if (mayChange) "" else " Only this runner’s admins can change these.")
        }
    }
}
