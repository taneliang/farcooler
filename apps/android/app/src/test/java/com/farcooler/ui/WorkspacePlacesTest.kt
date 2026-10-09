package com.farcooler.ui

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Switching workspaces goes back to where each was left (ov-442): the pure
 * halves of it, [WorkspacePlaces] and [Backstack.goToWorkspace]. What
 * `AppModel` does with them is two calls, one in `install` and one in
 * `openWorkspace`.
 */
class WorkspacePlacesTest {
    private class Memory : WorkspacePlaces.Store {
        val kept = mutableMapOf<Pair<String, String>, String>()

        override fun get(hostId: String, workspaceId: String) = kept[hostId to workspaceId]

        override fun set(hostId: String, workspaceId: String, stack: String) {
            kept[hostId to workspaceId] = stack
        }
    }

    private val billing = Route.Workspace("h", "billing", WorkspaceTab.BOARD)
    private val shop = Route.Workspace("h", "shop", WorkspaceTab.BOARD)
    private val task = Route.BoardTask("h", "billing", "t1")

    /** The model's two moves, in order: open from [stack], then remember. */
    private fun open(places: WorkspacePlaces, stack: List<Route>, target: Route.Workspace): List<Route> {
        val next = Backstack.goToWorkspace(stack, target, places.over(target.hostId, target.workspaceId))
        places.remember(next)
        return next
    }

    @Test
    fun `switching away from a workspace and back reopens its task`() {
        val places = WorkspacePlaces(Memory())
        var stack = open(places, listOf(Route.NeedsYou), billing)
        stack = stack + task
        places.remember(stack)
        stack = open(places, stack, shop)
        assertEquals(listOf(Route.NeedsYou, shop), stack)
        stack = open(places, stack, billing)
        assertEquals(listOf(Route.NeedsYou, billing, task), stack)
    }

    @Test
    fun `closing the task is where the workspace was left`() {
        val places = WorkspacePlaces(Memory())
        places.remember(listOf(Route.NeedsYou, billing, task))
        places.remember(listOf(Route.NeedsYou, billing))
        assertEquals(emptyList<Route>(), places.over("h", "billing"))
    }

    @Test
    fun `going back to the front door keeps the place`() {
        val places = WorkspacePlaces(Memory())
        places.remember(listOf(Route.NeedsYou, billing, task))
        places.remember(listOf(Route.NeedsYou))
        assertEquals(listOf<Route>(task), places.over("h", "billing"))
    }

    @Test
    fun `a pane and another workspace's screens are not the place`() {
        val places = WorkspacePlaces(Memory())
        places.remember(listOf(Route.NeedsYou, billing, task, Route.Terminal("h", "w")))
        assertEquals(listOf<Route>(task), places.over("h", "billing"))
        places.remember(listOf(Route.NeedsYou, shop, task))
        assertEquals(emptyList<Route>(), places.over("h", "shop"))
    }

    @Test
    fun `tapping the workspace already open closes what is over it`() {
        val places = WorkspacePlaces(Memory())
        val stack = listOf(Route.NeedsYou, billing, task)
        places.remember(stack)
        assertEquals(
            listOf(Route.NeedsYou, billing),
            Backstack.goToWorkspace(stack, billing, places.over("h", "billing")),
        )
    }
}
