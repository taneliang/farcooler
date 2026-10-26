package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

/**
 * The people on a shared runner and their roles (ov-784), as `member.list` answers: AgentKit's `RunnerPeople` and
 * `RunnerPeopleRoles`, in this platform's sentence case. The runner decides who may do what; this only keeps the
 * app from offering a control the runner would refuse.
 *
 * - The owner gives roles. A maintainer edits any charter, renames a workspace, changes its prefix, and sets the
 *   settings that don't act as the owner.
 * - A removed member's device reads nothing, and is told so ([isReleased]).
 */
data class RunnerTeam(val members: List<Member>, val you: String) {
    /** A person on the runner who isn't its owner. [role] is null from a runner before roles. */
    data class Member(val handle: String, val name: String, val role: Role?, val reps: Int = 0)

    /** What the owner has made a member: `member.list`'s `role`. */
    enum class Role(val wire: String, val title: String) {
        MEMBER("member", "Member"),
        MAINTAINER("maintainer", "Maintainer"),
        ;

        companion object {
            fun of(wire: String?): Role? = entries.firstOrNull { it.wire == wire }
        }
    }

    /** This phone is the runner owner's: the one that gives roles. */
    val isOwner: Boolean get() = you.isEmpty()

    /** This phone was a member's and the runner removed them. */
    val isReleased: Boolean get() = you == RELEASED

    /** Whether anyone but the owner works here: the only runner worth a People section. */
    val isShared: Boolean get() = members.isNotEmpty() || (!isOwner && !isReleased)

    /** Whether People offers a role picker on each member: to the owner alone, on a runner with roles. */
    fun offersRoles(runnerHasRoles: Boolean): Boolean = isOwner && runnerHasRoles

    companion object {
        const val RELEASED = "(released)"

        /** A `member.list` answer, or null when it isn't one. */
        fun of(reply: JsonObject): RunnerTeam? {
            val list = reply["members"] as? JsonArray ?: return null
            fun JsonObject.text(key: String) = (this[key] as? JsonPrimitive)?.contentOrNull
            val members = list.mapNotNull { it as? JsonObject }.map { m ->
                Member(
                    handle = m.text("handle") ?: "",
                    name = m.text("name") ?: "",
                    role = Role.of(m.text("role")),
                    reps = (m["rep_panes"] as? JsonArray)?.size ?: 0,
                )
            }
            return RunnerTeam(members, reply.text("you") ?: "")
        }

        const val SECTION_TITLE = "People"
        const val ROLE_LABEL = "Role"

        /** A member's second line: their handle, maintainer when they are one and no picker says so, their reps. */
        fun detail(member: Member, showsRole: Boolean = true): String {
            val reps = when (member.reps) {
                0 -> "no rep"
                1 -> "1 rep"
                else -> "${member.reps} reps"
            }
            val role = if (showsRole && member.role == Role.MAINTAINER) "maintainer" else ""
            return listOf(member.handle, role, reps).filter { it.isNotEmpty() }.joinToString(" · ")
        }

        /** What the role picker is for, under the People list. */
        const val ROLES_PURPOSE =
            "A maintainer keeps the team’s workspaces in order. They can edit any charter, rename a workspace, " +
                "change its task prefix, and set its pull request size, cost line, nudge hours and wake on answer. " +
                "Only you can make someone a maintainer."

        /** A role change the runner didn't take. */
        const val ROLE_NOT_SAVED =
            "The runner didn’t change that role. Check that this device is still the owner’s, then try again."

        /**
         * A device whose member was removed from the runner. It names nobody: the owner or any runner admin may have
         * done it. The owner can always add them back, so they're who to ask. AgentKit's words.
         */
        const val REMOVED_TITLE = "You were removed from this runner"
        const val REMOVED_BODY =
            "Someone who manages this runner removed you, so this device can’t see the runner’s work anymore. " +
                "To come back, ask the runner’s owner to add you again."

        /**
         * A device whose key the runner now refuses, after letting it in before (ov-818): the removed row's sibling.
         * It can't say "removed you", since the phone can't tell its member being removed from only this device being
         * taken off the member's list; both revoke its key (ov-783). AgentKit's words.
         */
        const val NO_ACCESS_TITLE = "You no longer have access to this runner"
        const val NO_ACCESS_BODY =
            "This runner now refuses this device’s key, though it let it in before. That happens when someone " +
                "who manages the runner removes you or this device. To come back, ask the runner’s owner to add " +
                "this device again, or if the runner is yours, add it yourself. Far Cooler checks every few " +
                "minutes and reconnects once it’s back."
    }
}
