package com.farcooler.model

/**
 * Where choosing an agent in the agents view goes (ov-641), decided apart from the screen that carries it out, so a JVM test
 * holds the rule: the iPhone's `AgentsOpening` and the Mac's `ContentView.destination`.
 *
 * - A subagent whose own conversation the runner serves opens it, in its parent's place: the orchestrator's beside the plan
 *   or on its tab, a lane pane's once that pane is up.
 * - Anything else opens its pane: a subagent known only by name opens its parent's.
 * - The orchestrator's own pane is the conversation already up: nothing to open on a phone, the chat to focus beside a plan.
 */
sealed interface AgentOpening {
    /** The chat beside the plan takes focus; the orchestrator's pane is already up. */
    data object FocusChat : AgentOpening

    /** Nothing to do: the orchestrator's conversation is the screen the agents view opened from. */
    data object Stay : AgentOpening

    /** Open the subagent [agentId] (row [row]) in [terminal]'s conversation, going to its pane first when [pane] is a worktree. */
    data class OpenOwn(val terminal: String, val row: String, val agentId: String, val pane: String?) : AgentOpening

    /** Go to [terminal]'s pane in [worktree]. */
    data class OpenPane(val worktree: String, val terminal: String) : AgentOpening

    companion object {
        /**
         * @param orchestrator the workspace's orchestrator's terminal, if one is up
         * @param wide a layout with the chat beside the plan
         * @param ownOffered the runner serves a subagent's own rows (`subagent_rows`)
         */
        fun of(agent: RunningAgent, orchestrator: String?, wide: Boolean, ownOffered: Boolean): AgentOpening {
            val own = ownOffered && agent.opensOwnConversation
            if (agent.terminal == orchestrator) {
                return when {
                    own -> OpenOwn(agent.terminal, agent.row!!, agent.agentId!!, pane = null)
                    wide -> FocusChat
                    else -> Stay
                }
            }
            return if (own) OpenOwn(agent.terminal, agent.row!!, agent.agentId!!, pane = agent.worktree) else OpenPane(agent.worktree, agent.terminal)
        }
    }
}
