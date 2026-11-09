package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import java.io.File

/**
 * Stands the golden in for the core's shared phrases (`test/fixtures/plan-words.json`, which the Rust test pins), so a
 * test of the shell's close question or the Agents card needs no native library and says what the core says.
 */
object CorePhrasesFixture {
    private val golden = Json.parseToJsonElement(repositoryFile("test/fixtures/plan-words.json")).jsonObject

    /** Answers the words request with the Material style's golden. */
    fun install() {
        CorePhrases.reset()
        CorePhrases.bridge = { request ->
            check("\"words\"" in request) { "not a words request: $request" }
            golden.getValue("material").toString()
        }
    }

    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }
}
