package com.farcooler.model

import java.net.URI

/**
 * A destination as a notification carries it, and as every notification that
 * already shipped spells it (ov-183). AgentKit's `DestinationPayloads.swift`,
 * mirrored.
 *
 * New posts carry the encoding whole under [PAYLOAD_KEY]. Every older
 * spelling still reads: a task notice (`kind: "task"`, `task`, `runner`,
 * `noticeId`, `event`) whose runner can be read off its id
 * `t:<runner>:<key>`; a decision (`kind: "decision"`, with or without
 * `event`); an agent's push (`terminal`, and `runner` from a runner that says
 * it); a local post's `target` and `repository`; and the
 * `<scheme>://terminal/<id>` link.
 *
 * A task is read before a terminal, because the relay sends `terminal: ""`
 * beside a task, and Firebase copies it into the launch intent.
 */
object DestinationPayloads {
    /** The data or extra key the encoding rides under. */
    const val PAYLOAD_KEY = "destination"

    /** `Notifier.EXTRA_TERMINAL`, read as `terminal` is. */
    const val ANDROID_TERMINAL_KEY = "com.farcooler.terminal"

    /**
     * What a tapped notification asks for, or null. [extra] reads one key, as
     * `Intent.getStringExtra` does; [thread] is the notification's thread or
     * group, where there is one.
     */
    fun from(extra: (String) -> String?, thread: String = ""): Destination? {
        extra(PAYLOAD_KEY)?.let(Destination::decode)?.let { return it }
        val host = extra("target")
        val kind = extra("kind")
        val key = extra("task")?.ifEmpty { null }
        if ((kind == "task" || kind == "decision") && key != null) {
            val noticeId = extra("noticeId")?.ifEmpty { null } ?: thread.takeIf { it.startsWith("t:") }
            val runner = extra("runner")?.ifEmpty { null } ?: noticeId?.let(::parseNoticeId)?.first
            val event = extra("event")
            return Destination(
                runner = Destination.Runner(host = host, id = runner?.lowercase()),
                place = Destination.Place.Task(null, Destination.TaskRef(key = key, repository = extra("repository")?.ifEmpty { null })),
                // A legacy decision with no event is a decision all the same.
                question = event == "decision" || (kind == "decision" && event == null),
            )
        }
        val threaded = if (thread.startsWith("t:") || thread.startsWith("a:")) null else thread.ifEmpty { null }
        // Android's own agent banner puts its pane under EXTRA_TERMINAL, and has no thread.
        val terminal = extra("terminal")?.ifEmpty { null } ?: extra(ANDROID_TERMINAL_KEY)?.ifEmpty { null }
            ?: threaded ?: return null
        return Destination(
            runner = Destination.Runner(host = host, id = extra("runner")?.ifEmpty { null }?.lowercase()),
            place = Destination.Place.Terminal(terminal),
        )
    }

    /**
     * What a local post puts on its intent: the encoding, and the older
     * spelling beside it so today's readers still open it.
     */
    fun extras(destination: Destination): Map<String, String> {
        val out = linkedMapOf(PAYLOAD_KEY to destination.encoded())
        destination.runner.host?.let { out["target"] = it }
        destination.runner.id?.let { out["runner"] = it }
        when (val place = destination.place) {
            is Destination.Place.Task -> {
                place.task.key?.let { out["kind"] = "task"; out["task"] = it }
                place.task.repository?.let { out["repository"] = it }
            }
            is Destination.Place.Terminal -> out["terminal"] = place.id
            else -> Unit
        }
        return out
    }

    /**
     * `t:<runner id>:<task key>` as its parts, or null for anything else,
     * including the hashed form `t:<16 hex>`.
     */
    fun parseNoticeId(noticeId: String): Pair<String, String>? {
        val parts = noticeId.split(":", limit = 3)
        if (parts.size != 3 || parts[0] != "t" || parts[1].isEmpty() || parts[2].isEmpty()) return null
        return parts[1] to parts[2]
    }

    /**
     * A link's destination: `<scheme>://open?d=<encoding>`, or
     * `<scheme>://terminal/<id>[?runner=<id>]`. Any scheme.
     */
    fun fromUrl(url: String): Destination? {
        val uri = runCatching { URI(url) }.getOrNull() ?: return null
        val query = (uri.rawQuery ?: "").split("&").filter { it.isNotEmpty() }.associate {
            val name = it.substringBefore("=")
            percentDecode(name) to percentDecode(it.substringAfter("=", ""))
        }
        return when (uri.host) {
            "open" -> query["d"]?.let(Destination::decode)
            "terminal" -> {
                // A trailing slash is ignored, as `URL.lastPathComponent` ignores it.
                val id = percentDecode(uri.rawPath.orEmpty().trimEnd('/').substringAfterLast("/"))
                if (id.isEmpty()) null
                else Destination(
                    runner = Destination.Runner(id = query["runner"]?.ifEmpty { null }?.lowercase()),
                    place = Destination.Place.Terminal(id),
                )
            }
            else -> null
        }
    }

    /** `%XX` decoded as UTF-8, and `+` left a plus, as `URLComponents` reads a query. */
    private fun percentDecode(text: String): String {
        if (!text.contains('%')) return text
        val bytes = java.io.ByteArrayOutputStream()
        var index = 0
        while (index < text.length) {
            val char = text[index]
            if (char == '%' && index + 2 < text.length) {
                val value = text.substring(index + 1, index + 3).toIntOrNull(16)
                if (value != null) {
                    bytes.write(value)
                    index += 3
                    continue
                }
            }
            bytes.write(char.toString().toByteArray(Charsets.UTF_8))
            index += 1
        }
        return bytes.toString(Charsets.UTF_8.name())
    }
}
