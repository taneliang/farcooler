package com.farcooler.core

import android.content.Context
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.farcooler.account.Account
import com.farcooler.account.RelayAnswer
import com.farcooler.account.relayPost
import com.farcooler.data.Identity
import com.farcooler.data.NodeIdentity
import com.farcooler.data.Reach
import com.farcooler.data.Runner
import com.farcooler.data.RunnerStore
import java.io.File
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeNotNull
import org.junit.Test
import org.junit.runner.RunWith

/** Opt-in, real runner connection. The config belongs to a disposable test identity. */
@RunWith(AndroidJUnit4::class)
class RelayConnectionDeviceTest {
    /** Optional UI fixture: export only the emulator app's public enrollment keys. */
    @Test
    fun prepareThePhone() {
        assumeNotNull(InstrumentationRegistry.getArguments().getString("prepareRelay"))
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val offer = buildJsonObject {
            put("ssh_public", requireNotNull(Identity.publicKey))
            put("node_public", requireNotNull(NodeIdentity.mintIfNeeded()).second)
        }
        File(context.filesDir, "relay-offer.json").writeText(offer.toString())
        if (InstrumentationRegistry.getArguments().getString("preparePreview") == "true") {
            // Public channel settings only. Sign in through the app; never import or log tokens.
            check(context.getSharedPreferences("farcooler.account", Context.MODE_PRIVATE).edit()
                .putString("account.relay", PREVIEW_RELAY)
                .putString("account.clientID", "client_01M03MXE27CHCZJJ0TMY7KCMAH")
                .commit())
        }
    }

    @Test
    fun aTunnelCarriesTheAndroidClientToTheRunner() = runBlocking<Unit> {
        val path = InstrumentationRegistry.getArguments().getString("relayConfig")
        assumeNotNull(path)
        val config = Json.parseToJsonElement(File(path!!).readText()).jsonObject
        assertTrue("this test must dial a tunnel", "token" in config && "host" !in config)
        assertTrue("the tunnel library must load", tunnelIsAvailable)
        val arguments = InstrumentationRegistry.getArguments()
        check(!arguments.containsKey("previewProbe")) {
            "previewProbe only checked an unauthenticated 401; use previewAccount=true and previewMachineId"
        }
        if (arguments.getString("previewAccount") == "true") {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val account = Account(context)
            check(account.relay == PREVIEW_RELAY) { "Configure the disposable app for the preview relay first" }
            val session = checkNotNull(account.accessToken()) {
                "Sign the disposable emulator app into a preview test account before running preview verification"
            }
            val machine = checkNotNull(arguments.getString("previewMachineId")) {
                "previewMachineId must name the disposable Mac paired to the preview account"
            }
            val answer = relayPost("$PREVIEW_RELAY/v1/account", buildJsonObject {}, session)
            // The successful account response contains metadata, never session or daemon tokens.
            check(answer is RelayAnswer.Answered) {
                "Authenticated preview account request failed: ${if (answer is RelayAnswer.Refused) answer.status else "unreachable"}"
            }
            Log.i("RelayConnection", "authenticated preview relay: HTTP 200 ${answer.body}")
            assertTrue("the preview account must list the paired test Mac",
                answer.body.getValue("machines").jsonArray.any {
                    it.jsonObject.getValue("id").jsonPrimitive.content == machine
                })
        }
        val client = ClientCore()
        try {
            withTimeout(60_000) {
                Log.i("RelayConnection", "connect: ${client.connect(config)}")
                assertTrue(client.isConnected())
                Log.i("RelayConnection", "probe: ${client.call("probe")}")
                Log.i("RelayConnection", "fleet: ${client.call("fleet")}")
            }
            if (InstrumentationRegistry.getArguments().getString("showRunner") == "true") {
                val context = InstrumentationRegistry.getInstrumentation().targetContext
                val runners = RunnerStore(context)
                // A rerun creates a new endpoint. Duplicate IDs select the old token,
                // so replace only this test fixture before capturing the real UI.
                runners.hosts.value.firstOrNull { it.id == "ov-1350-relay" }?.let(runners::remove)
                runners.add(Runner(
                    id = "ov-1350-relay",
                    label = "ov-1350 Mac via relay",
                    reach = Reach.Tailcat(config.getValue("token").jsonPrimitive.content),
                    user = config.getValue("user").jsonPrimitive.content,
                    fingerprint = config.getValue("host_fingerprint").jsonPrimitive.content,
                ))
                // Instrumentation exits immediately; flush RunnerStore's asynchronous save first.
                assertTrue(context.getSharedPreferences("farcooler.hosts", Context.MODE_PRIVATE).edit().commit())
            }
        } catch (error: UnreachedException) {
            Log.e("RelayConnection", "connect stopped: trouble=${error.trouble} tunnel=${error.tunnel}", error)
            throw error
        } finally {
            client.close()
        }
    }

    private companion object {
        const val PREVIEW_RELAY = "https://relay-preview.farcooler.com"
    }
}
