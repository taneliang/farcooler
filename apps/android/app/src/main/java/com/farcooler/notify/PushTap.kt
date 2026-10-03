package com.farcooler.notify

/**
 * Where a tapped notification should land, decided from its launch-intent
 * extras alone so the rule runs without an Activity.
 *
 * A push Firebase drew itself (app backgrounded or killed) copies the
 * message's `data` keys into the launch intent verbatim, and the relay always
 * sends `terminal`, as `""` when the push names a task and no pane. An empty
 * string is "absent" here: treating it as a terminal id skips the task route
 * and opens the app on whatever it showed last.
 */
sealed interface PushTap {
    /** Open the pane with this terminal id. */
    data class Terminal(val id: String) : PushTap

    /** Open the task with this key, on [runner] when the push carries one. */
    data class Task(val key: String, val runner: String?) : PushTap

    companion object {
        /** Reads [extra] by key, as `Intent.getStringExtra` does. */
        fun from(extra: (String) -> String?): PushTap? {
            val terminal = (extra(Notifier.EXTRA_TERMINAL) ?: extra(Notifier.PUSH_EXTRA_TERMINAL))
                ?.takeIf { it.isNotEmpty() }
            if (terminal != null) return Terminal(terminal)
            if (extra(Notifier.PUSH_EXTRA_KIND) != Notifier.KIND_DECISION) return null
            val task = extra(Notifier.PUSH_EXTRA_TASK)?.takeIf { it.isNotEmpty() } ?: return null
            return Task(task, extra(Notifier.PUSH_EXTRA_RUNNER)?.takeIf { it.isNotEmpty() })
        }
    }
}
