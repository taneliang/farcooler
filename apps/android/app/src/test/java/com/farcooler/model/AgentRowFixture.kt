package com.farcooler.model

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject

/**
 * `test/fixtures/agent-rows.json`: what the client core's rows boundary writes
 * (`crates/client/src/ffi/rows_fixture_tests.rs` regenerates it and fails when it
 * drifts). The Mac's and the iPhone's decoders read the same file.
 */
object AgentRowFixture {
    private val root: JsonObject by lazy {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, "test/fixtures/agent-rows.json")
            if (candidate.isFile) return@lazy Json.parseToJsonElement(candidate.readText()).jsonObject
            directory = directory.parentFile
        }
        throw AssertionError("Could not find test/fixtures/agent-rows.json above ${System.getProperty("user.dir")}.")
    }

    val page: JsonObject get() = root.getValue("page").jsonObject
    val follow: JsonObject get() = root.getValue("follow").jsonObject

    /** A fresh session's page (ov-409): claude's `Try` example on its `Hint` row alone, `test/fixtures/agent-rows-hint.json`. */
    val hintPage: JsonObject by lazy {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, "test/fixtures/agent-rows-hint.json")
            if (candidate.isFile) return@lazy Json.parseToJsonElement(candidate.readText()).jsonObject
            directory = directory.parentFile
        }
        throw AssertionError("Could not find test/fixtures/agent-rows-hint.json above ${System.getProperty("user.dir")}.")
    }
}
