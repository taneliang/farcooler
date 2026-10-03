package com.farcooler.account

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.ServerSocket
import kotlin.concurrent.thread

/**
 * [relayPost], the body of `Account.post`, against a real socket.
 *
 * It used to answer null for offline, a 5xx and a refusal alike.
 */
class RelayPostTest {

    /** Serve one request with this status and body, and return the URL. */
    private fun serving(status: Int, body: String?): Pair<String, () -> String> {
        val server = ServerSocket(0)
        var request = ""
        val worker = thread {
            server.use { s ->
                s.accept().use { socket ->
                    val input = socket.getInputStream().bufferedReader()
                    val head = generateSequence { input.readLine() }.takeWhile { it.isNotEmpty() }.toList()
                    val length = head.firstOrNull { it.lowercase().startsWith("content-length:") }
                        ?.substringAfter(':')?.trim()?.toInt() ?: 0
                    val chars = CharArray(length)
                    var read = 0
                    while (read < length) read += input.read(chars, read, length - read)
                    request = head.joinToString("\n") + "\n\n" + String(chars)
                    val bytes = body?.toByteArray() ?: ByteArray(0)
                    val out = socket.getOutputStream()
                    out.write(
                        ("HTTP/1.1 $status X\r\nContent-Type: application/json\r\n" +
                            "Content-Length: ${bytes.size}\r\nConnection: close\r\n\r\n").toByteArray()
                    )
                    out.write(bytes)
                    out.flush()
                }
            }
        }
        return "http://127.0.0.1:${server.localPort}/v1/auth/refresh" to { worker.join(); request }
    }

    private val sent = buildJsonObject { put("refreshToken", JsonPrimitive("rt-1")) }

    @Test
    fun aGoodAnswerIsAnswered() {
        val (url, request) = serving(200, """{"accessToken":"at-2"}""")
        val answer = relayPost(url, sent, bearer = "b")
        assertEquals(RelayAnswer.Answered(buildJsonObject { put("accessToken", JsonPrimitive("at-2")) }), answer)
        val seen = request()
        assertTrue(seen, seen.contains("Authorization: Bearer b"))
        assertTrue(seen, seen.endsWith("""{"refreshToken":"rt-1"}"""))
    }

    /** A refusal keeps its status and body, which is how a rejection is told apart. */
    @Test
    fun aRefusalKeepsItsStatusAndBody() {
        val (url, _) = serving(401, """{"error":"auth","status":400}""")
        val answer = relayPost(url, sent)
        assertEquals(
            RelayAnswer.Refused(
                401,
                buildJsonObject { put("error", JsonPrimitive("auth")); put("status", JsonPrimitive(400)) },
            ),
            answer,
        )
        assertTrue(SessionRefresher.rejectsRefreshToken(answer))
    }

    /** Mutation: a 5xx collapsed into Unreachable. Red. */
    @Test
    fun aServerFailureIsARefusalWithItsStatus() {
        val (url, _) = serving(503, null)
        assertEquals(RelayAnswer.Refused(503, null), relayPost(url, sent))
    }

    @Test
    fun aGoodStatusWithoutJSONIsNotAnAnswer() {
        val (url, _) = serving(200, "<html>")
        assertEquals(RelayAnswer.Refused(200, null), relayPost(url, sent))
    }

    @Test
    fun nobodyListeningIsUnreachable() {
        val port = ServerSocket(0).use { it.localPort }
        val answer = relayPost("http://127.0.0.1:$port/v1/auth/refresh", sent)
        assertTrue("$answer", answer is RelayAnswer.Unreachable)
    }

    @Test
    fun anUnusableAddressIsUnreachable() {
        val answer = relayPost("not a url", JsonObject(emptyMap()))
        assertEquals(RelayAnswer.Unreachable("relay address invalid"), answer)
    }
}
