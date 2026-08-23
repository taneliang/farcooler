package com.farcooler.model

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * `repository_root.list`, transcribed key for key from the producer.
 *
 * The payload below is copied out of `crates/client/src/ffi.rs` — the
 * `"repository_root.list"` arm — and not out of `wire::repository_root` or
 * `proto/farcooler.proto`, both of which describe a RICHER message than what
 * reaches this app. That distinction is the point of this file: the wire carries
 * `resource_version`, `host_id`, `path_token`, `created_at` and
 * `repository_count`, and the FFI's `json!` block passes on `id` and
 * `displayPath` and stops. A model written from the proto would declare five
 * keys that never arrive and — with `ignoreUnknownKeys = true` and a default on
 * every field — would say so by drawing zeroes rather than by failing.
 *
 * Fourth pass at this class of bug in one week. `isMainCheckout` (`07e75e8`),
 * `updatedAt` (`fb79a8c`) and `BranchRef.remote` (`22700b0`) were each a field
 * spelled or typed from something other than the producer the app actually
 * reads, and the third was covered the whole time by a test that had invented
 * its own payload with the same mistake in it. So this transcribes, and it
 * includes the null row, which is the arm that would otherwise never run.
 */
class RepositoryRootDecodeTest {
    /** The same configuration `Connection` decodes with. */
    private val json = Json { ignoreUnknownKeys = true }

    /**
     * Both arms of `display_path` in one reply.
     *
     * `wire::repository_root` writes `display_path: admin(scope).then(...)`, so
     * a client below `host_admin` gets `null` — and `serde_json` renders a
     * `None` as an explicit `null` rather than omitting the key, which is the
     * difference that matters: kotlinx will not fill a non-nullable property
     * from an explicit null even when the property has a default. That is
     * exactly how `BranchRef.remote` threw on every repository with a remote.
     */
    private val payload = """
        {
          "roots": [
            {
              "id": "8f14e45f-ce5b-4a5e-9c2b-000000000001",
              "displayPath": "/home/e/src"
            },
            {
              "id": "1c383cd3-0b0f-4a63-b8a1-000000000002",
              "displayPath": null
            }
          ]
        }
    """

    @Test
    fun `a root list decodes both a visible path and a hidden one`() {
        val list = json.decodeFromString(RepositoryRootList.serializer(), payload)
        assertEquals(2, list.roots.size)
        assertEquals("8f14e45f-ce5b-4a5e-9c2b-000000000001", list.roots[0].id)
        assertEquals("/home/e/src", list.roots[0].displayPath)
        assertEquals("1c383cd3-0b0f-4a63-b8a1-000000000002", list.roots[1].id)
        // Null, not empty. The row says "Hidden" off this, and an empty string
        // would draw a blank line that reads as a bug rather than as a rule.
        assertNull(list.roots[1].displayPath)
    }

    /**
     * A reply with no roots is a runner watching nothing, and decodes.
     *
     * The daemon sends `{"roots": []}` for a fresh install, which is the state
     * every runner starts in — and the state in which the Add button on the
     * settings screen matters most.
     */
    @Test
    fun `a runner watching nothing decodes to an empty list`() {
        val list = json.decodeFromString(RepositoryRootList.serializer(), """{"roots": []}""")
        assertEquals(0, list.roots.size)
    }

    /**
     * `repository.register` answers with `id` and `displayName`, and
     * `Connection.addRepository` reads the second.
     *
     * Transcribed from the same file's `"repository.register"` arm. The name is
     * the folder's own leaf as `Service::register_repository` computes it, which
     * is why the sheet can report it back without inventing one.
     */
    @Test
    fun `a registration answers with the name the runner chose`() {
        val reply = json.decodeFromString(
            Repository.serializer(),
            """{"id": "8f14e45f-ce5b-4a5e-9c2b-000000000003", "displayName": "far-cooler"}""",
        )
        assertEquals("far-cooler", reply.displayName)
    }
}
