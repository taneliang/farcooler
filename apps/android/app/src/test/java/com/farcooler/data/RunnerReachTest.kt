package com.farcooler.data

import com.farcooler.ceremony.CeremonyRunner
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A runner is an address OR the tunnel, on disk and on the way to the core.
 *
 * Before this, [Runner] was an address and a port, so a ceremony could grant a
 * phone a tunneled runner and the phone had nowhere to put it — the reason no
 * ceremony had ever produced one. The three things that can go wrong now are
 * all silent, which is why each has a test:
 *
 * - **Losing every runner on the first launch after an update.** The list
 *   decodes in one call, so an entry written before `reach` existed that the
 *   decoder threw on would empty the list, not skip one row.
 * - **A token in a field meant for a hostname.** The two shapes carry different
 *   keys to the core, and `parse_destination` picks its path from which of them
 *   is present.
 * - **The wrong half of the node key.** Private and public are both 43
 *   characters of unpadded base64-URL, so nothing about either one's appearance
 *   says which it is.
 */
class RunnerReachTest {
    /** The same configuration [RunnerStore] persists with. */
    private val json = Json { ignoreUnknownKeys = true }

    /**
     * Exactly what every install on disk holds today.
     *
     * Transcribed from `RunnerStore.save` as it was before `reach` existed —
     * `id`, `label`, `address`, `port`, `user`, `fingerprint` — and NOT written
     * by encoding a `Runner`, because encoding one with today's code would
     * produce today's shape and the test would prove nothing about yesterday's.
     */
    private val legacy =
        """
        [{"id":"a1","label":"Studio","address":"10.0.0.4","port":2222,"user":"me",
          "fingerprint":"SHA256:abc"}]
        """.trimIndent()

    @Test
    fun aRunnerSavedBeforeReachExistedStillLoads() {
        val runners = json.decodeFromString(ListSerializer(Runner.serializer()), legacy)
        assertEquals("the list came back short: $runners", 1, runners.size)
        val runner = runners.single()
        assertEquals(Reach.Direct("10.0.0.4", 2222), runner.reach)
        assertEquals("me", runner.user)
        assertEquals("SHA256:abc", runner.fingerprint)
        assertEquals("a1", runner.id)
    }

    /**
     * The failure this protects against is losing the whole list, not one row,
     * so the fixture puts a legacy entry BESIDE a new one. A decoder that threw
     * on the legacy shape would take the tunneled runner down with it.
     */
    @Test
    fun oneLegacyEntryDoesNotTakeTheRestOfTheListWithIt() {
        val mixed =
            """
            [{"id":"a1","label":"Studio","address":"10.0.0.4","port":22,"user":"me"},
             {"id":"b2","label":"Attic","reach":{"kind":"tailcat","token":"tc-x"},"user":"me"}]
            """.trimIndent()
        val runners = json.decodeFromString(ListSerializer(Runner.serializer()), mixed)
        assertEquals("the list came back short: $runners", 2, runners.size)
        assertEquals(Reach.Direct("10.0.0.4", 22), runners[0].reach)
        assertEquals(Reach.Tailcat("tc-x"), runners[1].reach)
    }

    /**
     * A runner is written in the new shape and ONLY the new shape.
     *
     * `address` beside `reach` would be a second copy of where a runner lives,
     * and two facts about one thing diverge the moment anything edits one of
     * them — which is the mistake `Identity.publicKey` exists to explain.
     */
    @Test
    fun onlyTheNewShapeIsWritten() {
        val encoded = json.encodeToString(
            Runner.serializer(),
            Runner(id = "a1", label = "Studio", reach = Reach.Direct("10.0.0.4", 22), user = "me"),
        )
        // The TOP-LEVEL keys. `port` also appears inside the nested reach,
        // where it belongs, so a substring search would pass on any output.
        val keys = Json.parseToJsonElement(encoded).jsonObject.keys
        assertTrue("no reach was written: $encoded", keys.contains("reach"))
        assertFalse("a legacy address was written beside it: $encoded", keys.contains("address"))
        assertFalse("a legacy port was written beside it: $encoded", keys.contains("port"))
    }

    @Test
    fun aTunneledRunnerSurvivesBeingSavedAndReadBack() {
        val runner = Runner(id = "b2", label = "Attic", reach = Reach.Tailcat("tc-x"), user = "me")
        val back = json.decodeFromString(
            Runner.serializer(),
            json.encodeToString(Runner.serializer(), runner),
        )
        assertEquals(runner, back)
    }

    /**
     * A direct runner's config is byte-identical to what it always was.
     *
     * The common path must not move: this is the config every existing runner
     * on every phone connects with.
     */
    @Test
    fun aDirectRunnerNamesAHostAndAPortAndNoToken() {
        val config = Runner(label = "Studio", reach = Reach.Direct("10.0.0.4", 2222), user = "me")
            .config(privateKey = "PRIVATE", nodeKey = "node-private", derpMap = "")
        assertEquals(JsonPrimitive("10.0.0.4"), config["host"])
        assertEquals(JsonPrimitive(2222), config["port"])
        assertEquals(JsonPrimitive("me"), config["user"])
        assertEquals(JsonPrimitive("PRIVATE"), config["private_key"])
        // A config carrying both would leave two paths to one runner with
        // nothing choosing between them, and `parse_destination` takes the
        // token whenever one is present — so a token leaking into a direct
        // runner's config silently routes it through a tunnel.
        assertNull("a direct runner named a token: $config", config["token"])
        assertNull("a direct runner named a node key: $config", config["node_key"])
    }

    /**
     * The node key in a config is this device's PRIVATE half.
     *
     * `parse_destination` reads it as `Reach::Tailcat { client_key }`, which is
     * what dials. The public half is the one that went into the offer and then
     * into the runner's allowlist. Both are 43 characters of unpadded
     * base64-URL, so nothing at the boundary can tell them apart.
     */
    @Test
    fun aTunneledRunnerNamesATokenAndThisDevicesNodeKeyAndNoHost() {
        val config = Runner(label = "Attic", reach = Reach.Tailcat("tc-x"), user = "me")
            .config(privateKey = "PRIVATE", nodeKey = "node-private", derpMap = "")
        assertEquals(JsonPrimitive("tc-x"), config["token"])
        assertEquals(JsonPrimitive("node-private"), config["node_key"])
        assertNull("a tunneled runner named a host: $config", config["host"])
        assertNull("a tunneled runner named a port: $config", config["port"])
    }

    /**
     * A granted tunneled runner keeps its token on the way into the list.
     *
     * This is the third gap, in one assertion. `asRunner` used to answer null
     * for anything that was not direct, so `JoinScreen.adopt` skipped it and a
     * phone that had just been granted a tunneled runner ended up with no
     * runner at all.
     */
    @Test
    fun aGrantedTunneledRunnerIsKeptRatherThanDropped() {
        val granted = CeremonyRunner(
            id = "b2",
            label = "Attic",
            alias = "attic",
            user = "me",
            hostKey = "",
            reach = Reach.Tailcat("tc-x"),
            pending = false,
        )
        val runner = granted.asRunner()
        assertEquals(Reach.Tailcat("tc-x"), runner.reach)
        assertEquals("me", runner.user)
        // A host key that came across empty stays null, which is what makes the
        // first connection report the fingerprint instead of trusting it.
        assertNull(runner.fingerprint)
    }

    /**
     * Private first, public second — the order the Go export writes them in.
     *
     * Getting this backwards stores the half that should have been offered and
     * offers the half that should have been stored. Nothing downstream can
     * detect it: both are 43 characters of unpadded base64-URL, the offer looks
     * filled in, and tailcat ignores an unrecognized client in silence, so the
     * symptom is a connection that times out saying nothing.
     */
    @Test
    fun theStoredPairIsPrivateThenPublic() {
        val pair = NodeIdentity.split("PRIV\nPUB")
        assertEquals("PRIV" to "PUB", pair)
    }

    /** A half-written pair is refused rather than returned half-empty. */
    @Test
    fun aHalfWrittenPairIsRefused() {
        assertNull("a pair with no public half was accepted", NodeIdentity.split("PRIV\n"))
        assertNull("a pair with no private half was accepted", NodeIdentity.split("\nPUB"))
        assertNull("a single line was accepted as a pair", NodeIdentity.split("PRIV"))
        assertNull("three lines were accepted as a pair", NodeIdentity.split("A\nB\nC"))
    }

    /**
     * The rendezvous crosses into the core under the name the core reads.
     *
     * `parse_destination` in `crates/client/src/ffi.rs` reads `derp_map`, and
     * `Session::connect_ssh` hands it to `farcooler_tailcat::set_derp_map_url`
     * before the dial that needs it. Nothing between here and there checks the
     * spelling — a JSON object is passed opaquely through
     * `Java_com_farcooler_core_NativeClient_nativeConnect` — so a key typed
     * `derpMap` here would be dropped in silence and the phone would keep
     * meeting at the rendezvous somebody just replaced. The failure would look
     * like a connection that times out saying nothing.
     *
     * Spelled out as a literal rather than referenced through a constant, on
     * purpose: a constant would rename with the field and leave this green.
     */
    @Test
    fun aTunneledRunnersConfigNamesTheRendezvousTheCoreReads() {
        val config = Runner(label = "Attic", reach = Reach.Tailcat("tc-x"), user = "me")
            .config(privateKey = "PRIVATE", nodeKey = "node-private", derpMap = "https://r.example/derpmap.json")
        assertEquals(JsonPrimitive("https://r.example/derpmap.json"), config["derp_map"])
    }

    /**
     * The default rendezvous is sent as an empty string, not left out.
     *
     * `parse_destination` promises that absent and blank land on the same
     * place, and they do — but only one of the two proves this side is passing
     * the setting at all. A config that omitted the field would be
     * indistinguishable from one whose plumbing was never connected, right up
     * until the day somebody typed a URL and nothing happened.
     */
    @Test
    fun theDefaultRendezvousIsSentAsEmptyRatherThanOmitted() {
        val config = Runner(label = "Attic", reach = Reach.Tailcat("tc-x"), user = "me")
            .config(privateKey = "PRIVATE", nodeKey = "node-private", derpMap = "")
        assertTrue("no rendezvous was sent at all: $config", config.containsKey("derp_map"))
        assertEquals(JsonPrimitive(""), config["derp_map"])
    }

    /** A direct runner carries the field too, so one shape serves both reaches. */
    @Test
    fun aDirectRunnersConfigCarriesTheRendezvousToo() {
        val config = Runner(label = "Studio", reach = Reach.Direct("10.0.0.4", 22), user = "me")
            .config(privateKey = "PRIVATE", nodeKey = null, derpMap = "https://r.example/derpmap.json")
        assertEquals(JsonPrimitive("https://r.example/derpmap.json"), config["derp_map"])
    }
}
