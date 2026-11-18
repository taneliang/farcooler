package com.farcooler.core

import android.content.Context
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.farcooler.data.Identity
import com.farcooler.data.NodeIdentity
import com.farcooler.data.Reach
import com.farcooler.data.Runner
import com.farcooler.data.RunnerStore
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
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
    }

    @Test
    fun aTunnelCarriesTheAndroidClientToTheRunner() = runBlocking<Unit> {
        val path = InstrumentationRegistry.getArguments().getString("relayConfig")
        assumeNotNull(path)
        val config = Json.parseToJsonElement(File(path!!).readText()).jsonObject
        assertTrue("this test must dial a tunnel", "token" in config && "host" !in config)
        assertTrue("the tunnel library must load", tunnelIsAvailable)
        if (InstrumentationRegistry.getArguments().getString("previewProbe") == "true") {
            // A read-only, unauthenticated request: record real preview bytes without an account token.
            val probe = URL("https://relay-preview.farcooler.com/v1/account").openConnection() as HttpURLConnection
            try {
                probe.requestMethod = "POST"
                probe.connectTimeout = 10_000
                probe.readTimeout = 10_000
                probe.doOutput = true
                probe.setRequestProperty("Content-Type", "application/json")
                probe.outputStream.use { it.write("{}".toByteArray()) }
                val status = probe.responseCode
                val body = (if (status >= 400) probe.errorStream else probe.inputStream)?.bufferedReader()?.use { it.readText() }
                Log.i("RelayConnection", "preview relay: HTTP $status $body")
            } finally {
                probe.disconnect()
            }
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
                RunnerStore(context).add(Runner(
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
}
