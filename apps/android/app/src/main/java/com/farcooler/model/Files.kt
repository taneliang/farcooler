package com.farcooler.model

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import java.util.Locale

// A worktree's files, and a runner's extra read-only folders, as the phones'
// read-only Files browser reads them (ov-259).
//
// A port of `apps/shared/AgentKit/Sources/AgentKit/Files.swift` and
// `FilesBrowser.swift`, because Kotlin cannot import the Swift. The two are held
// together by `test/fixtures/files-lines.json`, which both suites read, and by
// the sentences, which are word for word.

/** One name in a directory, as the client core's `worktree.list_dir` says it. */
data class FileEntry(val name: String, val kind: Kind, val size: Long, val linkTarget: String) {
    enum class Kind {
        FILE, DIRECTORY, LINK, OTHER;

        companion object {
            /** A word a newer runner sends that this build hasn't heard of is listed, never opened. */
            fun parse(word: String?): Kind = when (word) {
                "file" -> FILE
                "directory" -> DIRECTORY
                "link" -> LINK
                else -> OTHER
            }
        }
    }
}

/** One directory's answer. The runner sends at most 5,000 entries for one directory. */
data class FileListing(val path: String, val entries: List<FileEntry>, val truncated: Boolean) {
    companion object {
        fun parse(json: JsonObject) = FileListing(
            path = json["path"]?.jsonPrimitive?.contentOrNull.orEmpty(),
            truncated = json["truncated"]?.jsonPrimitive?.booleanOrNull ?: false,
            entries = json["entries"]?.jsonArray.orEmpty().map { e ->
                val o = e.jsonObject
                FileEntry(
                    name = o["name"]?.jsonPrimitive?.contentOrNull.orEmpty(),
                    kind = FileEntry.Kind.parse(o["kind"]?.jsonPrimitive?.contentOrNull),
                    size = o["size"]?.jsonPrimitive?.longOrNull ?: 0L,
                    linkTarget = o["linkTarget"]?.jsonPrimitive?.contentOrNull.orEmpty(),
                )
            },
        )
    }
}

/** One file's answer. */
data class FileRead(
    val path: String,
    val state: State,
    val size: Long,
    val text: String,
    val linkTarget: String,
) {
    enum class State {
        TEXT, BINARY, TOO_LARGE, LINK, UNKNOWN;

        companion object {
            fun parse(word: String?): State = when (word) {
                "text" -> TEXT
                "binary" -> BINARY
                "too_large" -> TOO_LARGE
                "link" -> LINK
                else -> UNKNOWN
            }
        }
    }

    companion object {
        fun parse(json: JsonObject) = FileRead(
            path = json["path"]?.jsonPrimitive?.contentOrNull.orEmpty(),
            state = State.parse(json["state"]?.jsonPrimitive?.contentOrNull),
            size = json["size"]?.jsonPrimitive?.longOrNull ?: 0L,
            text = json["text"]?.jsonPrimitive?.contentOrNull.orEmpty(),
            linkTarget = json["linkTarget"]?.jsonPrimitive?.contentOrNull.orEmpty(),
        )
    }
}

/** Why a read came back with nothing, in a sentence for the screen. */
enum class FileReadFailure {
    /** The runner predates `worktree_files`. */
    RUNNER_TOO_OLD,

    /** Nothing at that path, or a link on the way to it. */
    MISSING,

    /** A directory, a FIFO, a socket. */
    NOT_A_FILE,

    /** Nothing at that path in an extra folder. */
    MISSING_IN_FOLDER,

    /** The runner won't show this extra folder at all anymore. */
    FOLDER_GONE,

    /** Anything else: the runner couldn't be reached, or said something new. */
    FAILED;

    /** For a file. */
    val sentence: String
        get() = when (this) {
            RUNNER_TOO_OLD -> "Update Far Cooler on this runner to see its files."
            MISSING -> "This file isn’t in the worktree anymore."
            MISSING_IN_FOLDER -> "This isn’t in the folder anymore."
            FOLDER_GONE -> "This runner doesn’t share this folder anymore."
            NOT_A_FILE -> "This isn’t a file Far Cooler can show."
            FAILED -> "Couldn’t read this file. Check that the runner is reachable, then try again."
        }

    /** For a directory. */
    val directorySentence: String
        get() = when (this) {
            MISSING -> "This folder isn’t in the worktree anymore."
            NOT_A_FILE -> "This isn’t a folder Far Cooler can show."
            FAILED -> "Couldn’t read this folder. Check that the runner is reachable, then try again."
            RUNNER_TOO_OLD, MISSING_IN_FOLDER, FOLDER_GONE -> sentence
        }

    companion object {
        /**
         * From the runner's refusal word (null when the link itself failed). In an
         * extra folder the runner answers a name it no longer shares, and a path it
         * refuses, as "not found", so nothing at the folder's root means the folder
         * is gone.
         */
        fun from(refusal: String?, inFolder: Boolean, atRoot: Boolean): FileReadFailure = when (refusal) {
            "not-found" -> when {
                !inFolder -> MISSING
                atRoot -> FOLDER_GONE
                else -> MISSING_IN_FOLDER
            }
            "invalid-argument" -> NOT_A_FILE
            "capability-unsupported" -> RUNNER_TOO_OLD
            else -> FAILED
        }
    }
}

/** The path rules, as values. */
object FilesPaths {
    /** [parent]'s child named [name], as a path from the root. */
    fun join(parent: String, name: String): String = if (parent.isEmpty()) name else "$parent/$name"

    /**
     * [location], a path an agent's tool call named, as a path inside the
     * worktree at [root], or null when it's somewhere else.
     */
    fun relative(location: String, root: String): String? {
        val trimmed = trimmedRoot(root)
        if (trimmed.isEmpty()) return null
        val path = if (location.startsWith("/")) {
            val match = listOf(trimmed, "/private$trimmed").firstOrNull { location.startsWith("$it/") }
                ?: return null
            location.substring(match.length + 1)
        } else {
            location
        }
        return normalized(path)
    }

    /**
     * Where a link at [path] leads, as a path inside the worktree at [root], or
     * null when it leaves the worktree. A phone has no [root]: the runner never
     * sends it one, so an absolute target leads nowhere there.
     */
    fun linkDestination(path: String, target: String, root: String): String? {
        if (target.startsWith("/")) return relative(target, root)
        val parent = path.substringBeforeLast('/', "")
        return normalized(join(parent, target))
    }

    /** [path] with `.` and `..` worked out, or null when `..` climbs above the root or nothing is left. */
    fun normalized(path: String): String? {
        val parts = ArrayList<String>()
        for (part in path.split('/')) {
            when (part) {
                "", "." -> Unit
                ".." -> if (parts.isEmpty()) return null else parts.removeAt(parts.size - 1)
                else -> parts.add(part)
            }
        }
        return if (parts.isEmpty()) null else parts.joinToString("/")
    }

    private fun trimmedRoot(root: String): String {
        var r = root
        while (r.length > 1 && r.endsWith("/")) r = r.dropLast(1)
        return r
    }
}

/** A file's text, as a viewer numbers and draws it. */
object FilesText {
    /**
     * A file's lines, as the viewer numbers them: broken at `\n`, `\r\n` and a
     * bare `\r`, none of them shown, and no empty line after a final break.
     */
    fun lines(text: String): List<String> {
        val lines = ArrayList<String>()
        val current = StringBuilder()
        var afterReturn = false
        for (c in text) {
            if (afterReturn) {
                afterReturn = false
                if (c == '\n') continue
            }
            if (c == '\n' || c == '\r') {
                lines.add(current.toString())
                current.setLength(0)
                afterReturn = c == '\r'
            } else {
                current.append(c)
            }
        }
        if (current.isNotEmpty()) lines.add(current.toString())
        return lines
    }

    /**
     * The most a phone draws of one line, in UTF-16 code units, the unit both
     * platforms' strings count in. The runner caps a file's bytes and not a
     * line's length, so a minified 512 KiB file is one line, and one
     * half-megabyte `Text` stalls a phone.
     */
    const val LINE_LIMIT = 2_000

    /** A line as drawn: [text], and whether it was [cut]. */
    data class Shown(val text: String, val cut: Boolean)

    /**
     * [line] as drawn: whole, or cut at [limit] code units, never in the middle
     * of a surrogate pair, with a closing "…".
     */
    fun display(line: String, limit: Int = LINE_LIMIT): Shown {
        if (line.length <= limit) return Shown(line, false)
        var keep = limit
        if (Character.isLowSurrogate(line[keep])) keep -= 1
        return Shown(line.substring(0, keep) + "…", true)
    }

    /** A file's size, the way Finder and the iPhone say it: decimal units, one decimal under 10. */
    fun size(bytes: Long): String {
        if (bytes < 1_000) return if (bytes == 1L) "1 byte" else "$bytes bytes"
        val units = listOf("KB", "MB", "GB", "TB")
        var value = bytes / 1_000.0
        var unit = 0
        while (value >= 999.5 && unit < units.size - 1) {
            value /= 1_000.0
            unit += 1
        }
        val shown = if (unit == 0 || value >= 10) String.format(Locale.US, "%.0f", value)
        else String.format(Locale.US, "%.1f", value)
        return "$shown ${units[unit]}"
    }
}

/** Where a path is read from: a worktree, or one of the runner's extra read-only folders. */
sealed interface FilesPlace {
    data class Worktree(val id: String) : FilesPlace
    data class Folder(val name: String) : FilesPlace

    val isFolder: Boolean get() = this is Folder
}

/** What a screen expects at its path, when the caller knows. A link's destination could be either. */
enum class FilesExpecting(val wire: String) {
    DIRECTORY("directory"), FILE("file"), EITHER("either");

    companion object {
        fun parse(wire: String?): FilesExpecting = entries.firstOrNull { it.wire == wire } ?: DIRECTORY
    }
}

/** One screen's place and path, and what to expect there. */
data class FilesLocation(
    val place: FilesPlace,
    /** From the place's root; empty is the root. */
    val path: String = "",
    val expecting: FilesExpecting = FilesExpecting.DIRECTORY,
) {
    /** The app bar's title: the last name, or the place at its root. */
    val title: String
        get() = path.split('/').lastOrNull { it.isNotEmpty() }
            ?: when (place) {
                is FilesPlace.Worktree -> "Files"
                is FilesPlace.Folder -> place.name
            }
}

/** How the runner is asked, as a screen sees it. The app fills these from its client core. */
interface FilesSource {
    suspend fun list(place: FilesPlace, path: String): Result<FileListing>
    suspend fun read(place: FilesPlace, path: String): Result<FileRead>
}

/** A failed read, carrying which of the sentences it is. */
class FilesFailure(val why: FileReadFailure) : Exception(why.sentence)

/** The client core's `worktree.list_dir` and `worktree.read_file`, as an app passes and reads them. */
class CoreFilesSource(
    private val call: suspend (method: String, args: Map<String, String>) -> JsonObject,
    private val refusalWord: (Throwable) -> String?,
) : FilesSource {
    override suspend fun list(place: FilesPlace, path: String): Result<FileListing> =
        fetch(LIST_METHOD, place, path, FileListing::parse)

    override suspend fun read(place: FilesPlace, path: String): Result<FileRead> =
        fetch(READ_METHOD, place, path, FileRead::parse)

    private suspend fun <T> fetch(method: String, place: FilesPlace, path: String, parse: (JsonObject) -> T): Result<T> =
        try {
            Result.success(parse(call(method, arguments(place, path))))
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (e: Throwable) {
            Result.failure(FilesFailure(FileReadFailure.from(refusalWord(e), place.isFolder, path.isEmpty())))
        }

    companion object {
        const val LIST_METHOD = "worktree.list_dir"
        const val READ_METHOD = "worktree.read_file"

        /** The arguments: a worktree or a folder, never both. */
        fun arguments(place: FilesPlace, path: String): Map<String, String> = when (place) {
            is FilesPlace.Worktree -> mapOf("worktree" to place.id, "path" to path)
            is FilesPlace.Folder -> mapOf("folder" to place.name, "path" to path)
        }
    }
}

/** One row of a directory. */
data class FilesRow(
    /**
     * Unique within its directory, for the list's key: the runner sends names
     * through a lossy UTF-8 decode, so two names on disk can arrive as one
     * string, and a lazy list keyed by name throws on the repeat.
     */
    val key: String,
    val name: String,
    val kind: FileEntry.Kind,
    /** A file's size, a link's "→ target", nothing for a directory. */
    val detail: String,
    /** Where a tap goes: null for a link out of the place, and for what Far Cooler can't show. */
    val destination: FilesLocation?,
)

/** A directory as a screen draws it. */
data class FilesDirectory(
    val rows: List<FilesRow>,
    /** "Showing the first 5,000 items.", when the runner cut the list. */
    val footer: String?,
    /** "This folder is empty.", when there is nothing to list. */
    val empty: String?,
)

/** A file's text as a viewer draws it. */
data class FilesCode(
    /** Each line, cut at [FilesText.LINE_LIMIT]. */
    val lines: List<String>,
    val anyCut: Boolean,
    /** The gutter's width in digits. */
    val gutterDigits: Int,
) {
    companion object {
        const val CUT_NOTE = "Long lines are cut at 2,000 characters."
    }
}

/** What a screen shows. */
sealed interface FilesContent {
    data object Loading : FilesContent
    data class Directory(val directory: FilesDirectory) : FilesContent
    data class Code(val code: FilesCode) : FilesContent

    /** A file Far Cooler doesn't draw: binary, or past the runner's limit. */
    data class Message(val words: String) : FilesContent

    /** A link: where it points, and the place it leads when that's inside. */
    data class Link(val target: String, val destination: FilesLocation?) : FilesContent
    data class Failed(val sentence: String) : FilesContent
}

/** What one screen shows for [location]: read once, on demand, and never again until asked. */
object FilesLoader {
    /** A phone doesn't know the worktree's path on the runner, so a link with an absolute target leads nowhere. */
    suspend fun load(location: FilesLocation, source: FilesSource, root: String = ""): FilesContent =
        when (location.expecting) {
            FilesExpecting.DIRECTORY -> directory(location, source, root, null)
            FilesExpecting.FILE -> file(location, source, root, thenDirectory = false)
            FilesExpecting.EITHER -> file(location, source, root, thenDirectory = true)
        }

    private suspend fun directory(
        location: FilesLocation, source: FilesSource, root: String, fallback: FileReadFailure?,
    ): FilesContent {
        val listing = source.list(location.place, location.path).getOrElse {
            return FilesContent.Failed(((fallback ?: failureOf(it)).directorySentence))
        }
        return FilesContent.Directory(directory(listing, location, root))
    }

    private suspend fun file(
        location: FilesLocation, source: FilesSource, root: String, thenDirectory: Boolean,
    ): FilesContent {
        val read = source.read(location.place, location.path).getOrElse {
            val why = failureOf(it)
            return if (why == FileReadFailure.NOT_A_FILE && thenDirectory) {
                directory(location, source, root, FileReadFailure.NOT_A_FILE)
            } else {
                FilesContent.Failed(why.sentence)
            }
        }
        return file(read, location, root)
    }

    private fun failureOf(e: Throwable): FileReadFailure = (e as? FilesFailure)?.why ?: FileReadFailure.FAILED

    fun directory(listing: FileListing, here: FilesLocation, root: String): FilesDirectory {
        val rows = listing.entries.mapIndexed { index, entry ->
            val key = "$index/${entry.name}"
            val path = FilesPaths.join(here.path, entry.name)
            when (entry.kind) {
                FileEntry.Kind.DIRECTORY -> FilesRow(
                    key, entry.name, entry.kind, "", FilesLocation(here.place, path, FilesExpecting.DIRECTORY))
                FileEntry.Kind.FILE -> FilesRow(
                    key, entry.name, entry.kind, FilesText.size(entry.size),
                    FilesLocation(here.place, path, FilesExpecting.FILE))
                FileEntry.Kind.LINK -> FilesRow(
                    key, entry.name, entry.kind, "→ ${entry.linkTarget}",
                    FilesPaths.linkDestination(path, entry.linkTarget, root)
                        ?.let { FilesLocation(here.place, it, FilesExpecting.EITHER) })
                FileEntry.Kind.OTHER -> FilesRow(key, entry.name, entry.kind, "", null)
            }
        }
        return FilesDirectory(
            rows = rows,
            footer = if (listing.truncated) {
                "Showing the first ${String.format(Locale.US, "%,d", listing.entries.size)} items."
            } else null,
            empty = if (rows.isEmpty()) "This folder is empty." else null,
        )
    }

    fun file(read: FileRead, here: FilesLocation, root: String): FilesContent = when (read.state) {
        FileRead.State.TEXT -> {
            val shown = FilesText.lines(read.text).map { FilesText.display(it) }
            val lines = shown.map { it.text }
            FilesContent.Code(
                FilesCode(
                    lines = lines,
                    anyCut = shown.any { it.cut },
                    gutterDigits = maxOf(lines.size, 1).toString().length,
                ),
            )
        }
        FileRead.State.BINARY -> FilesContent.Message("This is a binary file. It’s ${FilesText.size(read.size)}.")
        FileRead.State.TOO_LARGE ->
            FilesContent.Message("This file is too large to show here. It’s ${FilesText.size(read.size)}.")
        FileRead.State.LINK -> FilesContent.Link(
            read.linkTarget,
            FilesPaths.linkDestination(here.path, read.linkTarget, root)
                ?.let { FilesLocation(here.place, it, FilesExpecting.EITHER) },
        )
        FileRead.State.UNKNOWN -> FilesContent.Message("Far Cooler can’t show this file yet. Update the app to see it.")
    }
}
