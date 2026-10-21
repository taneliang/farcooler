package com.farcooler.net

import com.farcooler.model.OutgoingImage
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/** ov-622: a composer's photos survive a restart, sit under a cap, and go when they are sent, taken out, or their pane goes. */
class DraftImagesTest {
    private lateinit var root: File
    private val running = TestScope()
    private val source = FakeRowSource()
    private var sent = mutableListOf<Pair<String, Int>>()

    @Before
    fun setUp() {
        root = java.nio.file.Files.createTempDirectory("draft-images").toFile()
    }

    @After
    fun tearDown() {
        running.close()
        root.deleteRecursively()
    }

    private fun files(paneCap: Long = 1_000, totalCap: Long = 10_000, evict: Boolean = false, ageMs: Long = 3_600_000) =
        PaneImageFiles(root, paneCap, totalCap, evict, ageMs)

    private fun drafts(files: PaneImageFiles = files()) = DraftImageFiles(files)

    private fun image(id: Int, bytes: Int, mime: String = "image/png") = OutgoingImage(id, mime, ByteArray(bytes) { 7 })

    @Test
    fun `a drafts photos come back in order after a restart`() {
        val before = drafts()
        before.keep("p1", image(3, 10))
        before.keep("p1", image(4, 20, "image/jpeg"))
        before.flush()
        // A new store over the same directory: what a relaunch has.
        val restored = drafts().restored("p1")
        assertEquals(listOf(3, 4), restored.map { it.id })
        assertEquals(listOf("image/png", "image/jpeg"), restored.map { it.mime })
        assertEquals(listOf(10, 20), restored.map { it.data.size })
        assertTrue(drafts().restored("p2").isEmpty())
    }

    @Test
    fun `a photo taken out or sent does not come back, and takes the panes folder with the last`() {
        val store = drafts()
        store.keep("p", image(1, 10))
        store.keep("p", image(2, 10))
        store.drop("p", 2)
        assertEquals(listOf(1), store.restored("p").map { it.id })
        store.dropAll("p")
        assertTrue(store.restored("p").isEmpty())
        assertFalse(File(root, "p").exists())
    }

    @Test
    fun `a pane is capped and the photos already kept stay`() {
        val store = drafts(files(paneCap = 100))
        store.keep("p", image(1, 60))
        store.keep("p", image(2, 60)) // 120 > 100: not kept
        store.keep("p", image(3, 40)) // 100: fits
        assertEquals(listOf(1, 3), store.restored("p").map { it.id })
    }

    @Test
    fun `the whole store is capped, and a photo bigger than a pane is never written`() {
        val store = drafts(files(paneCap = 100, totalCap = 150))
        store.keep("one", image(1, 100))
        store.keep("two", image(2, 100)) // 200 > 150
        store.keep("three", image(3, 101))
        assertEquals(1, store.restored("one").size)
        assertTrue(store.restored("two").isEmpty())
        assertTrue(store.restored("three").isEmpty())
    }

    @Test
    fun `gone and old panes are pruned at launch`() {
        val store = drafts(files(ageMs = 3_600_000))
        store.keep("live", image(1, 10))
        store.keep("closed", image(2, 10))
        store.keep("stale", image(3, 10))
        store.flush()
        File(root, "stale").listFiles()!!.forEach { it.setLastModified(System.currentTimeMillis() - 7_200_000) }
        store.prune(keeping = setOf("live", "stale"))
        assertEquals(1, store.restored("live").size)
        assertTrue("a closed pane's photos stayed", store.restored("closed").isEmpty())
        assertTrue("a photo past its age stayed", store.restored("stale").isEmpty())
    }

    @Test
    fun `a cache makes room by removing the oldest, and never the file just written`() {
        val cache = files(paneCap = 130, totalCap = 150, evict = true)
        assertTrue(cache.write(ByteArray(60), "a", "p"))
        File(root, "p/a").setLastModified(System.currentTimeMillis() - 60_000)
        assertTrue(cache.write(ByteArray(60), "b", "p"))
        assertTrue(cache.write(ByteArray(60), "c", "p")) // 180 > 130: a goes
        assertFalse(cache.contains("a", "p"))
        assertTrue(cache.write(ByteArray(60), "d", "q")) // 180 > 150 in all
        assertTrue(cache.totalBytes <= 150)
        assertTrue("the file just written was the one removed", cache.contains("d", "q"))
    }

    @Test
    fun `two pane ids that sanitize alike keep apart`() {
        val f = files()
        f.write(ByteArray(1), "a", "a/b")
        f.write(ByteArray(2), "a", "a:b")
        assertEquals(1L, f.bytes("a/b"))
        assertEquals(2L, f.bytes("a:b"))
    }

    // The model.

    private class PhotoMemory(override val draftImages: DraftImageFiles) : PaneViewMemory by InMemoryPaneViews()

    private fun model(memory: PaneViewMemory) = NativePaneModel(
        terminal = "t1",
        store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
        source = source,
        sink = ConversationSink { _, text, images ->
            sent += text to images.size
            false
        },
        memory = memory,
        scope = running.scope,
        draftDelayMs = 1,
    ).also { it.offer(rich = true, interrupts = false) }

    @Test
    fun `a photo in the draft is back after the model is made again, goes with the message, and is gone once sent`() = runBlocking {
        val store = drafts()
        val memory = PhotoMemory(store)
        val first = model(memory)
        first.attach(listOf(image(first.nextImageId(), 12)))
        store.flush()
        val second = model(memory)
        eventually("the photo came back") { second.images.size == 1 }
        assertEquals(12, second.images.single().data.size)
        assertTrue("a new photo's id is never a restored one", second.nextImageId() != second.images.single().id)
        second.onDraft("what is this")
        second.send()
        eventually("the send") { sent.size == 1 && second.images.isEmpty() }
        assertEquals("what is this" to 1, sent.single())
        store.flush()
        val third = model(memory)
        Thread.sleep(200)
        assertTrue("a sent photo came back", third.images.isEmpty())
    }

    @Test
    fun `a removed photo does not come back`() = runBlocking {
        val store = drafts()
        val memory = PhotoMemory(store)
        val first = model(memory)
        val id = first.nextImageId()
        first.attach(listOf(image(id, 12)))
        first.detach(id)
        store.flush()
        val second = model(memory)
        Thread.sleep(200)
        assertTrue(second.images.isEmpty())
    }

    @Test
    fun `without compose the photos wait on disk for a runner that takes them`() = runBlocking {
        val store = drafts()
        val memory = PhotoMemory(store)
        val first = model(memory)
        first.attach(listOf(image(first.nextImageId(), 12)))
        store.flush()
        first.offer(rich = false, interrupts = false)
        assertTrue(first.images.isEmpty())
        first.offer(rich = true, interrupts = false)
        eventually("the photo came back once the runner took photos again") { first.images.size == 1 }
    }

    @Test
    fun `a pane that is gone takes its photos with it`() = runBlocking {
        val store = drafts()
        val memory = PhotoMemory(store)
        val model = model(memory)
        model.attach(listOf(image(model.nextImageId(), 12)))
        store.flush()
        model.forgetPhotos()
        store.flush()
        assertTrue(store.restored("t1").isEmpty())
    }

    // The prompt images' cache.

    @Test
    fun `a sent messages image shows again after a restart with no runner to ask`() = runBlocking {
        val cache = files(paneCap = 1_000_000, totalCap = 10_000_000, evict = true)
        val fetched = mutableListOf<String>()
        val first = PromptImageStore("t1", cache).also {
            it.source = PromptImageSource { _, row, index ->
                fetched += "$row#$index"
                byteArrayOf(9, index.toByte())
            }
        }
        first.load("turn:0", 1)
        assertEquals(listOf<Byte>(9, 1), (first.state("turn:0", 1) as PromptImageStore.State.Shown).bytes.toList())
        // A new launch: a new store over the same disk, and no runner.
        val again = PromptImageStore("t1", cache)
        again.awaitIndex()
        assertTrue(again.isKept("turn:0", 1))
        assertFalse("an image never fetched is on disk", again.isKept("turn:0", 0))
        again.load("turn:0", 1)
        assertEquals(listOf<Byte>(9, 1), (again.state("turn:0", 1) as PromptImageStore.State.Shown).bytes.toList())
        assertEquals("the runner was asked again", listOf("turn:0#1"), fetched)
        again.load("turn:0", 0)
        assertTrue("nothing to show for an image that was never kept", again.state("turn:0", 0) is PromptImageStore.State.Loading)
    }

    @Test
    fun `the prompt cache is capped and goes with the pane`() = runBlocking {
        val cache = files(paneCap = 25, totalCap = 10_000, evict = true)
        val store = PromptImageStore("t1", cache).also { it.source = PromptImageSource { _, _, _ -> ByteArray(10) } }
        for (index in 0 until 4) store.load("turn:0", index)
        assertTrue("the pane's cache grew past its cap", cache.bytes("t1") <= 25)
        cache.removePane("t1")
        assertTrue(cache.names("t1").isEmpty())
    }

    @Test
    fun `a resumed sessions turns are never cached, their ids name another turn in another session`() = runBlocking {
        val cache = files(paneCap = 1_000_000, totalCap = 10_000_000, evict = true)
        val first = PromptImageStore("t1", cache).also { it.source = PromptImageSource { _, _, _ -> byteArrayOf(5) } }
        first.load("turn:resumed:3", 0)
        assertTrue(first.state("turn:resumed:3", 0) is PromptImageStore.State.Shown)
        assertTrue("a resumed turn's image went in the cache", cache.names("t1").isEmpty())
        // Another session, the same id: nothing from the first.
        val again = PromptImageStore("t1", cache)
        again.awaitIndex()
        assertFalse(again.isKept("turn:resumed:3", 0))
        again.load("turn:resumed:3", 0)
        assertTrue(again.state("turn:resumed:3", 0) is PromptImageStore.State.Loading)
    }

    // A pane's photos go only once it has really gone.

    @Test
    fun `one read without a pane does not make it gone, two reads 30 seconds apart do`() {
        val panes = MissingPanes()
        assertTrue(panes.read(setOf("a", "b"), at = 0).isEmpty())
        assertTrue("a single read that lacked b", panes.read(setOf("a"), at = 5_000).isEmpty())
        assertTrue("a second read, but not 30 s on", panes.read(setOf("a"), at = 20_000).isEmpty())
        assertEquals(setOf("b"), panes.read(setOf("a"), at = 36_000))
        assertTrue("told twice", panes.read(setOf("a"), at = 90_000).isEmpty())
    }

    @Test
    fun `a pane back in between starts the count again`() {
        val panes = MissingPanes()
        panes.read(setOf("a", "b"), at = 0)
        assertTrue(panes.read(setOf("a"), at = 5_000).isEmpty())
        assertTrue(panes.read(setOf("a", "b"), at = 20_000).isEmpty())
        assertTrue(panes.read(setOf("a"), at = 40_000).isEmpty())
        assertEquals(setOf("b"), panes.read(setOf("a"), at = 80_000))
    }

    @Test
    fun `the panes of a runner keep their photos through one missing read`() = runBlocking {
        val store = drafts()
        val memory = PhotoMemory(store)
        val gone = mutableListOf<String>()
        var clock = 0L
        val panes = NativePanes(
            running.scope, ClientCall { _, _ -> throw AssertionError("no call goes to a core here") },
            sourceFor = { FakeRowSource() }, onGone = { gone += it }, now = { clock },
        )
        panes.model("t1", memory).also { it.offer(rich = true, interrupts = false) }.attach(listOf(image(1, 12)))
        panes.model("t2", memory)
        store.flush()
        panes.retain(setOf("t1", "t2"))
        panes.retain(setOf("t2"))
        assertTrue("one read without t1 took its photos", gone.isEmpty())
        // The model stays through one missing read (ov-660), and so do the photos.
        assertNotNull(panes.ifMade("t1"))
        assertEquals(1, store.restored("t1").size)
        // A second read without it, 30 s on: the pane is surely gone, model and photos.
        clock = 31_000
        panes.retain(setOf("t2"))
        assertEquals(listOf("t1"), gone)
        assertNull(panes.ifMade("t1"))
    }

    @Test
    fun `photos are not restored into a composer that has sent since`() = runBlocking {
        val store = drafts()
        val first = model(PhotoMemory(store))
        first.attach(listOf(image(first.nextImageId(), 12)))
        store.flush()
        val release = java.util.concurrent.CountDownLatch(1)
        val slow = object : DraftImageFiles(files()) {
            override fun restored(terminal: String): List<OutgoingImage> {
                release.await()
                return super.restored(terminal)
            }
        }
        val second = model(PhotoMemory(slow))
        second.onDraft("a new message")
        second.send()
        eventually("the send") { second.sent == 1 }
        release.countDown()
        Thread.sleep(300)
        assertTrue("a photo was restored into a composer that had sent", second.images.isEmpty())
    }
}
