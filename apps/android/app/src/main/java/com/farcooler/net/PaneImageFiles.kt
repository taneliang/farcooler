package com.farcooler.net

import com.farcooler.model.OutgoingImage
import java.io.File
import java.util.concurrent.Callable
import java.util.concurrent.Executors

/**
 * Image files kept on disk per pane, under a size cap and a maximum age (ov-622): a draft's photos waiting
 * to be sent ([DraftImageFiles]) and the originals of photos already sent (the prompt-image cache). AgentKit's
 * `PaneImageFiles`, in the same words.
 *
 * One directory per pane under [root], one file per image, all inside the app's own storage. A file that can't
 * be written or read is a miss, never an error to the person.
 *
 * The caps are bytes: [paneCap] bounds one pane and [totalCap] the whole root. A write that would pass either is
 * refused (a draft's photo stays in memory only) or, with [evictOldest], makes room by removing the oldest files
 * (a cache, which can always fetch again).
 */
class PaneImageFiles(
    private val root: File,
    private val paneCap: Long,
    private val totalCap: Long,
    private val evictOldest: Boolean,
    private val maxAgeMs: Long,
) {
    private fun directory(pane: String) = File(root, safe(pane))

    private class Entry(val file: File, val size: Long, val modified: Long)

    private fun entries(folder: File): List<Entry> =
        folder.listFiles { f -> f.isFile }.orEmpty().map { Entry(it, it.length(), it.lastModified()) }

    private fun allEntries(): List<Entry> = root.listFiles { f -> f.isDirectory }.orEmpty().flatMap { entries(it) }

    fun bytes(pane: String): Long = entries(directory(pane)).sumOf { it.size }

    val totalBytes: Long get() = allEntries().sumOf { it.size }

    /** The names of [pane]'s files, oldest first. */
    fun names(pane: String): List<String> =
        entries(directory(pane)).sortedWith(compareBy({ it.modified }, { it.file.name })).map { it.file.name }

    fun contains(name: String, pane: String): Boolean = File(directory(pane), name).isFile

    /** [data] as [name] in [pane]'s directory. False where it wasn't kept: bigger than a pane may hold, over a cap, or unwritable. */
    @Synchronized
    fun write(data: ByteArray, name: String, pane: String): Boolean {
        if (data.size > paneCap) return false
        val folder = directory(pane)
        val file = File(folder, name)
        val replaced = if (file.isFile) file.length() else 0L
        if (!evictOldest && (bytes(pane) - replaced + data.size > paneCap || totalBytes - replaced + data.size > totalCap)) return false
        return try {
            folder.mkdirs()
            val temporary = File(folder, "$name.part")
            temporary.writeBytes(data)
            if (!temporary.renameTo(file)) {
                file.delete()
                if (!temporary.renameTo(file)) return false
            }
            if (evictOldest) makeRoom(file, pane)
            true
        } catch (e: java.io.IOException) {
            false
        }
    }

    /** The bytes of [name] in [pane], or null. Reading marks the file recent, so a cache evicts what hasn't been looked at. */
    fun read(name: String, pane: String): ByteArray? {
        val file = File(directory(pane), name)
        if (!file.isFile) return null
        return try {
            file.readBytes().also { file.setLastModified(System.currentTimeMillis()) }
        } catch (e: java.io.IOException) {
            null
        }
    }

    fun remove(name: String, pane: String) {
        File(directory(pane), name).delete()
    }

    /** Everything kept for [pane]. */
    fun removePane(pane: String) {
        directory(pane).deleteRecursively()
    }

    /**
     * Files past the maximum age go, panes not in [keeping] go (null keeps every pane), and the oldest of what
     * is left go until the root is under its cap.
     */
    @Synchronized
    fun prune(keeping: Set<String>? = null, now: Long = System.currentTimeMillis()) {
        if (keeping != null) {
            val keep = keeping.map(::safe).toSet()
            root.listFiles { f -> f.isDirectory }.orEmpty().filter { it.name !in keep }.forEach { it.deleteRecursively() }
        }
        val left = ArrayList<Entry>()
        for (entry in allEntries()) {
            if (now - entry.modified > maxAgeMs) entry.file.delete() else left += entry
        }
        var total = left.sumOf { it.size }
        for (entry in left.sortedBy { it.modified }) {
            if (total <= totalCap) break
            entry.file.delete()
            total -= entry.size
        }
        root.listFiles { f -> f.isDirectory }.orEmpty().filter { it.list().isNullOrEmpty() }.forEach { it.delete() }
    }

    private fun makeRoom(keeping: File, pane: String) {
        var held = bytes(pane)
        for (entry in entries(directory(pane)).sortedBy { it.modified }) {
            if (held <= paneCap) break
            if (entry.file == keeping) continue
            entry.file.delete()
            held -= entry.size
        }
        var total = totalBytes
        for (entry in allEntries().sortedBy { it.modified }) {
            if (total <= totalCap) break
            if (entry.file == keeping) continue
            entry.file.delete()
            total -= entry.size
        }
    }

    companion object {
        /** A pane's directory name: its id where that is plain, else the plain part and a hash of the whole, so two ids never share one. */
        fun safe(name: String): String {
            val plain = name.map { if (it.isLetterOrDigit() || it == '-') it else '_' }.joinToString("")
            if (plain == name && name.isNotEmpty()) return name
            var hash = -0x340d631b7bdddcdbL
            for (byte in name.toByteArray(Charsets.UTF_8)) hash = (hash xor (byte.toLong() and 0xFF)) * 0x100000001b3L
            return "$plain-${java.lang.Long.toHexString(hash)}"
        }
    }
}

/**
 * The photos in a composer's unsent message, kept on disk per terminal so a process death doesn't lose them
 * (ov-622). AgentKit's `DraftImages`: a photo is written when it's attached and removed when it's taken out, sent,
 * or its pane goes. Past 64 MB for a pane or 256 MB in all a photo is just not kept; anything untouched for 30 days
 * goes. Every change goes through one thread, in order, so a photo taken out right after it went in can't have its
 * write land afterwards.
 */
open class DraftImageFiles(private val files: PaneImageFiles) {
    private val queue = Executors.newSingleThreadExecutor { r -> Thread(r, "draft-images").apply { isDaemon = true } }
    private val clock = Any()
    private var lastMillis = 0L

    private fun nextMillis(): Long = synchronized(clock) {
        lastMillis = maxOf(System.currentTimeMillis(), lastMillis + 1)
        lastMillis
    }

    private fun extension(mime: String) = when (mime) {
        "image/jpeg" -> "jpg"
        "image/gif" -> "gif"
        "image/webp" -> "webp"
        else -> "png"
    }

    /** `<milliseconds>_<id>.<extension>`: files list in the order they were attached. */
    fun keep(terminal: String, image: OutgoingImage) {
        val name = "%013d_%d.%s".format(nextMillis(), image.id, extension(image.mime))
        queue.execute { files.write(image.data, name, terminal) }
    }

    fun drop(terminal: String, id: Int) {
        queue.execute { files.names(terminal).filter { it.substringAfter('_').substringBefore('.') == id.toString() }.forEach { files.remove(it, terminal) } }
    }

    fun dropAll(terminal: String) {
        queue.execute { files.removePane(terminal) }
    }

    fun prune(keeping: Set<String>? = null) {
        queue.execute { files.prune(keeping) }
    }

    /** What was kept for [terminal], in the order it was attached. */
    open fun restored(terminal: String): List<OutgoingImage> = queue.submit(Callable {
        files.names(terminal).sorted().mapNotNull { name ->
            val id = name.substringAfter('_').substringBefore('.').toIntOrNull() ?: return@mapNotNull null
            val mime = when (name.substringAfterLast('.')) {
                "png" -> "image/png"
                "jpg" -> "image/jpeg"
                "gif" -> "image/gif"
                "webp" -> "image/webp"
                else -> return@mapNotNull null
            }
            files.read(name, terminal)?.let { OutgoingImage(id, mime, it) }
        }
    }).get()

    /** Wait for the changes so far. For tests. */
    fun flush() {
        queue.submit {}.get()
    }
}

/**
 * Which panes of one runner have really gone, from the fleets it has told this app about (ov-622): a pane counts as gone
 * once it is missing from two reads at least [afterMs] apart, with no read between that had it, so one read racing a
 * restart doesn't take a draft's photos. AgentKit's `MissingPanes`.
 */
class MissingPanes(private val afterMs: Long = 30_000) {
    private var known: Set<String>? = null
    private val missingSince = HashMap<String, Long>()

    /** A fleet read listed [now]. The panes confirmed gone by it. */
    @Synchronized
    fun read(now: Set<String>, at: Long = System.currentTimeMillis()): Set<String> {
        val before = known
        if (before == null) {
            known = now
            return emptySet()
        }
        val gone = HashSet<String>()
        for (pane in before - now) {
            val since = missingSince[pane]
            if (since == null) missingSince[pane] = at else if (at - since >= afterMs) gone += pane
        }
        for (pane in now) missingSince.remove(pane)
        for (pane in gone) missingSince.remove(pane)
        known = (before + now) - gone
        return gone
    }
}

/** Where this process keeps its photos, set once at launch ([com.farcooler.data.Settings]); null in tests, which hand their own. */
object PaneImageDisk {
    @Volatile var drafts: DraftImageFiles? = null
    @Volatile var prompts: PaneImageFiles? = null

    /** A pane is really gone: its draft's photos and its cached images go with it. */
    fun forget(terminal: String) {
        drafts?.dropAll(terminal)
        prompts?.let { files -> Thread { files.removePane(terminal) }.apply { isDaemon = true }.start() }
    }

    fun install(filesDir: File, cacheDir: File) {
        if (drafts == null) {
            drafts = DraftImageFiles(
                PaneImageFiles(File(filesDir, "draft-images"), 64L shl 20, 256L shl 20, evictOldest = false, maxAgeMs = 30L * 24 * 3600 * 1000),
            ).also { it.prune() }
        }
        if (prompts == null) {
            prompts = PaneImageFiles(File(cacheDir, "prompt-images"), 64L shl 20, 200L shl 20, evictOldest = true, maxAgeMs = 60L * 24 * 3600 * 1000)
                .also { files -> Thread { files.prune() }.apply { isDaemon = true }.start() }
        }
    }
}
