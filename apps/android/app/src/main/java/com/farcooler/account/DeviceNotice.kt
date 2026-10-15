package com.farcooler.account

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * A device added to or removed from the signed-in account, or trusted on first
 * use again because none of its devices was confirmed (R-75).
 */
data class DeviceEvent(val kind: Kind, val row: DeviceKeyRow, val at: Long) {
    enum class Kind(val word: String) { ADDED("added"), REMOVED("removed"), TRUSTED("trusted") }

    val id: String get() = "${kind.word}-${row.id}"
}

/**
 * Telling the person, in every signed-in app, about each device added to or
 * removed from their account since that app last told them (ov-550 review
 * M1; the same as `DeviceNotice.swift` in AgentKit).
 *
 * The relay's push is only a hint: the stolen session it exists to expose can
 * delete every push install first. This reads `directory/me`, whose rows no
 * route deletes, and keeps, per account and per app, the time of the last
 * event the person dismissed. An app that has never shown one tells about
 * every device but itself, which is also how devices registered before the
 * notices shipped get announced.
 */
class DeviceNotice(
    /** Who is signed in and the account's keys, removed ones included; null when it can't say. */
    private val load: suspend () -> Pair<String, List<DeviceKeyRow>>?,
    private val ownKeyA: () -> String?,
    /** The last dismissed event's time, per account. */
    private val seen: Seen,
) {
    interface Seen {
        fun get(account: String): Long?
        fun set(account: String, at: Long)
    }

    private val _events = MutableStateFlow<List<DeviceEvent>>(emptyList())
    val events: StateFlow<List<DeviceEvent>> = _events.asStateFlow()
    private var account: String? = null

    /** Ask again: at launch, and each time the app comes forward. */
    suspend fun refresh() {
        val loaded = load()
        if (loaded == null) {
            _events.value = emptyList()
            return
        }
        account = loaded.first
        _events.value = unseen(loaded.second, seen.get(loaded.first), ownKeyA())
    }

    /** The person has read it: nothing up to the newest event shown is news again here. */
    fun dismiss() {
        val account = account ?: return
        val newest = _events.value.maxOfOrNull { it.at } ?: return
        seen.set(account, newest)
        _events.value = emptyList()
    }

    companion object {
        /** The events after [seen] (every one, with nothing seen yet), but this device's own arrival. */
        fun unseen(rows: List<DeviceKeyRow>, seen: Long?, ownKeyA: String?): List<DeviceEvent> = buildList {
            for (row in rows) {
                // A device trusted by the reset as it arrived is one event, the
                // trust, not an add as well (fix round 1, M1).
                val arrivedTrusted = row.wasTrustedAgain && row.trustedAt == row.createdAt
                row.createdAt?.let {
                    if (!arrivedTrusted && (seen == null || it > seen) && !row.isHeldBy(ownKeyA)) {
                        add(DeviceEvent(DeviceEvent.Kind.ADDED, row, it))
                    }
                }
                // Trusted by R-75's reset: the moment a stolen session that
                // removed every confirmed device would take over.
                row.trustedAt?.let {
                    if (row.wasTrustedAgain && (seen == null || it > seen) && !row.isHeldBy(ownKeyA)) {
                        add(DeviceEvent(DeviceEvent.Kind.TRUSTED, row, it))
                    }
                }
                row.revokedAt?.let { if (seen == null || it > seen) add(DeviceEvent(DeviceEvent.Kind.REMOVED, row, it)) }
            }
        }.sortedBy { it.at }

        fun title(events: List<DeviceEvent>): String = when {
            events.size != 1 -> "Your account’s devices changed"
            events[0].kind == DeviceEvent.Kind.ADDED -> "A device was added to your account"
            events[0].kind == DeviceEvent.Kind.TRUSTED -> "A device is now confirmed on your account"
            else -> "A device was removed from your account"
        }

        /** Each event with the device's quoted label, key words and day, then what to do. */
        fun detail(events: List<DeviceEvent>, day: (Long) -> String = DeviceKeyRow::dayOf): String =
            (events.map { event ->
                val words = event.row.fingerprint.takeIf { it.isNotEmpty() }
                    ?.let { " (${com.farcooler.ceremony.CeremonyStore.abbreviated(it)})" }.orEmpty() // not a count: key words
                val name = "“${event.row.shownLabel}”$words on ${day(event.at)}"
                when (event.kind) {
                    DeviceEvent.Kind.ADDED -> "Added $name."
                    DeviceEvent.Kind.REMOVED -> "Removed $name."
                    DeviceEvent.Kind.TRUSTED -> "Confirmed $name, since none of your devices was confirmed."
                }
            } + "If you don’t recognize a change, review your device keys in settings.").joinToString(" ")
    }
}
