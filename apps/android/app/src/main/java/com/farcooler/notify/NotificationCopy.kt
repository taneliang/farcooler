package com.farcooler.notify

import com.farcooler.model.AgentActivity
import com.farcooler.model.Terminal

/**
 * One notification about one pane: what it says, and where it says it.
 *
 * A value rather than three arguments threaded through a builder, so the whole
 * decision can be made — and asserted — without an Android `Context`. Everything
 * in [Notifier] that needs one is delivery; everything here is what gets
 * delivered.
 */
data class Announcement(val title: String, val body: String, val channel: String)

/**
 * What a notification about an agent says.
 *
 * **The sentences here are not this app's to choose.** The same runner sends the
 * same news twice by two roads — this app's own banner, drawn from a fleet it
 * just polled, and a push the daemon hands the relay when nobody is polling at
 * all — and one person reads whichever arrives. Two casings of one sentence is
 * two notifications about one pane, which is why iOS's `Notifications.swift`
 * capitalises a mid-sentence fragment and why this file does too. The producer
 * is `notification()` in `crates/daemon/src/watch.rs`; `NotificationCopyTest`
 * reads that function and fails when either side is reworded.
 *
 * Which is also why the Material sentence-case rule `cb13d31` settled does not
 * govern here. That rule is about labels this app writes — buttons, tabs,
 * headers. These are sentences a runner writes, quoted.
 *
 * ## Honesty
 *
 * `done` is not "it worked". The daemon reads the agent's own session log and
 * sends a turn that DIED and one that succeeded as the same `done`, saying which
 * in [Terminal.turnFailed] — so a notification that reads the activity alone
 * tells someone their agent finished when its last turn came back an error. That
 * is the green checkmark `8481657` took off the row, still on the lock screen
 * until now, and a lock screen is the worse of the two places to lie: it is what
 * decides whether somebody gets up.
 */
object NotificationCopy {

    /**
     * What a blocked agent's banner says when the daemon did not scrape a
     * question off the screen. `blocked` is derived from a tmux SCREEN, so
     * [Terminal.blockedQuestion] is a real line of text some of the time and
     * absent the rest of it.
     */
    const val WAITING = "Waiting for your answer"

    /**
     * What a turn that died says. Not the agent's error text: nothing in this
     * app has it, the daemon does not send it, and a Rust or Node backtrace on a
     * lock screen is worse than a sentence.
     */
    const val DID_NOT_FINISH = "Its last turn didn’t finish"

    /**
     * The banner for this terminal, or null when it is not worth one.
     *
     * Null for every state but the two that interrupt someone. A working agent
     * is the normal case, and a product that buzzes for the normal case is one
     * people turn off — after which it cannot tell them the thing that mattered.
     *
     * [runner] is the name of the runner this pane is on, and empty is a real
     * and ordinary answer: with one runner connected its name is on every
     * notification and says nothing, and with three it is the first thing you
     * need. That the fleet can be several runners at once is Android's, not a
     * port — see the do-not-delete list.
     */
    fun of(terminal: Terminal, worktree: String, runner: String = ""): Announcement? =
        when (terminal.agent) {
            AgentActivity.BLOCKED -> Announcement(
                title = "${terminal.label} needs you",
                // The question is what a person answers, and the worktree is
                // what tells them which of three blocked codexes is asking it.
                // Both, in that order, and the daemon's own reason for both:
                // "codex needs you / Do you want to create haiku.txt?" is
                // answerable and still not attributable.
                body = body(worktree, worthSaying(terminal.blockedQuestion) ?: WAITING, runner),
                channel = Notifier.CHANNEL_BLOCKED,
            )

            AgentActivity.DONE ->
                if (terminal.turnDidFail) {
                    Announcement(
                        title = "${terminal.label} failed",
                        body = body(worktree, DID_NOT_FINISH, runner),
                        channel = Notifier.CHANNEL_DONE,
                    )
                } else {
                    Announcement(
                        title = "${terminal.label} finished",
                        // What it finished, where there is an answer to that.
                        // The body was the worktree name alone, which the
                        // title's label had very nearly already said — so the
                        // whole notification was "claude finished / add auth", a
                        // sentence about Far Cooler rather than about the work.
                        //
                        // [Terminal.lastSaid], not `recentSteps.last()`: a feed
                        // entry is a wrapped ROW, so its last line is the END of
                        // the window, and a notification arrives after the fact
                        // and has to open where the sentence opens. The whole
                        // message is on the wire as `said`, cut from its start
                        // by the daemon.
                        body = body(worktree, worthSaying(terminal.lastSaid), runner),
                        channel = Notifier.CHANNEL_DONE,
                    )
                }

            else -> null
        }

    /**
     * The request code and notification id a pushed notice is posted under:
     * its terminal, else — a decision, which has none — its task, else its
     * title. Distinct per subject, so a second decision's card doesn't
     * overwrite the first one's extras under `FLAG_UPDATE_CURRENT`, which a
     * shared `"".hashCode()` of 0 did.
     */
    fun postedAs(terminal: String?, task: String?, title: String): Int = when {
        !terminal.isNullOrEmpty() -> terminal.hashCode()
        !task.isNullOrEmpty() -> "task:$task".hashCode()
        else -> title.hashCode()
    }

    /**
     * Which channel a PUSHED notice belongs on, by the daemon's own word for
     * what it is announcing.
     *
     * `"blocked"` is `Notice.status` in `crates/daemon/src/watch.rs`, carried to
     * the relay as `Payload.status` in `services/relay/src/push.ts`. Anything
     * else the daemon chose to send is news that can wait for the next time the
     * phone is picked up.
     *
     * **This decides the foreground case only**, which is less than it sounds.
     * [FarCoolerMessagingService.onMessageReceived] is the one caller, and
     * Firebase calls that only while the app is in front of you — the case
     * [Notifier] already covers with a banner of its own. A phone in a pocket
     * never reaches this function at all: the tray card Firebase draws takes
     * the relay's own `android.notification.channel_id`, decided from the same
     * word by `androidChannel` in `push.ts`.
     *
     * This used to say the status was not on the FCM message at all — that
     * `sendFcm` built `data: { terminal }` and nothing else, and that the fix
     * was one key in that object. `33159fd` is that fix, and it was two keys
     * rather than one: `data.status`, which is what this reads, and the
     * `channel_id` above, which is the half that reaches a sleeping phone.
     * Neither substitutes for the other. A relay older than that commit sends
     * neither, and gets what it always got — this function answering
     * [Notifier.CHANNEL_DONE] for everything, and the manifest's default
     * channel doing the same for every card this process never sees.
     *
     * [kind] `"decision"` is the other thing that alerts (ruling 3): a task in
     * Needs Decision is waiting on you as surely as a blocked agent, and its
     * notice has no `status` at all. The relay's `androidChannel` makes the
     * same choice for the tray card.
     */
    fun channelFor(status: String?, kind: String? = null): String =
        if (status == "blocked" || kind == "decision") Notifier.CHANNEL_BLOCKED else Notifier.CHANNEL_DONE

    /**
     * [channelFor] over a push's `data` as [FarCoolerMessagingService] receives
     * it, reading the keys under this app's names for the relay's.
     */
    fun channelForPush(data: Map<String, String>): String {
        // A task notice goes on its class's own channel (ov-94), as the relay
        // names it for a card Firebase draws.
        val event = data[TaskNotices.EXTRA_EVENT]
        if (data[Notifier.PUSH_EXTRA_KIND] == TaskNotices.KIND_TASK && event in TaskNotices.EVENTS) {
            return TaskNotices.channelFor(event!!)
        }
        return channelFor(data[Notifier.PUSH_EXTRA_STATUS], data[Notifier.PUSH_EXTRA_KIND])
    }

    /**
     * One notification body: which pane this is about, what there is to say
     * about it, and — only when it adds something — which runner it is on.
     *
     * `add-auth — Both tests pass. · studio`. Ported from `Quoted::body` in
     * `crates/daemon/src/watch.rs`, including the part that matters: any half
     * can be missing and the result still reads as a sentence. A turn can be all
     * tool calls and say nothing, and a worktree name derived from a path can
     * be empty. Neither may produce a stray separator — `— Both tests pass.`
     * looks like a rendering bug, and a lock screen is not where to debug one.
     */
    private fun body(worktree: String, text: String?, runner: String): String {
        val name = worthSaying(worktree)
        val said = worthSaying(text)
        val head = when {
            name != null && said != null -> "$name — $said"
            name != null -> name
            said != null -> said
            else -> ""
        }
        val on = worthSaying(runner) ?: return head
        return if (head.isEmpty()) on else "$head · $on"
    }

    /**
     * The text, or nothing when it is absent or blank. Every field quoted here
     * arrives from somewhere that can legitimately hand over an empty string,
     * and an empty body is a blank second line on a lock screen.
     */
    private fun worthSaying(text: String?): String? = text?.trim()?.takeIf { it.isNotEmpty() }
}
