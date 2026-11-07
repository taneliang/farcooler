package com.farcooler.ui

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*
import java.io.File
import java.security.MessageDigest
import java.util.UUID

/** Current Session getter + private durable intent journal. No native workflow policy. */
class WorkflowSessionPort(
    private val root: File,
    private val current: (String) -> Current?,
) : WorkflowPort {
    data class Current(val account: String, val host: String, val call: suspend (String, JsonObject) -> JsonObject)
    private data class Observation(val revision: String, val epoch: String, val run: String)
    private val observed = mutableMapOf<String, Observation>()
    private val mutex = Mutex()
    private fun key(vararg parts: String) = JsonArray(parts.map(::JsonPrimitive)).toString()
    private fun scope(host: String, workspace: String): Pair<Current, String> {
        val c = requireNotNull(current(host)) { "Workflow session unavailable" }
        require(c.account.isNotEmpty() && c.host.isNotEmpty())
        return c to key(c.account, c.host, workspace)
    }
    private fun file(key: String): File = File(root, MessageDigest.getInstance("SHA-256")
        .digest(key.toByteArray()).joinToString("") { "%02x".format(it) } + ".json")
    private fun checksum(payload: String) = MessageDigest.getInstance("SHA-256").digest(payload.toByteArray()).joinToString("") { "%02x".format(it) }
    private fun load(key: String): MutableList<JsonObject> {
        val path = file(key)
        if (!path.exists()) return mutableListOf()
        val envelope = Json.parseToJsonElement(path.readText()).jsonObject
        check(envelope.getValue("schema").jsonPrimitive.int == 1)
        val payload = envelope.getValue("payload").jsonPrimitive.content
        check(checksum(payload) == envelope.getValue("checksum").jsonPrimitive.content)
        val entries = Json.parseToJsonElement(payload).jsonArray.map { it.jsonObject }.toMutableList()
        for (entry in entries) {
            val phase = entry.getValue("phase").jsonPrimitive.content
            val attempts = entry.getValue("attempts").jsonPrimitive.long
            check(attempts >= 0)
            for (field in listOf("id", "item", "run", "label")) check(entry.getValue(field).jsonPrimitive.isString)
            when (phase) {
                "preparing", "abandoned" -> check(entry["prepared"] == null && entry["result"] == null && attempts == 0L)
                "prepared" -> check(entry["prepared"] != null && entry["result"] == null)
                "complete" -> check(entry["result"]?.jsonPrimitive?.isString == true)
                else -> error("Ambiguous workflow journal")
            }
        }
        return entries
    }
    private fun save(key: String, entries: List<JsonObject>) {
        require(root.isDirectory || root.mkdirs())
        val path = file(key)
        val temp = File(root, path.name + ".tmp")
        val payload = JsonArray(entries).toString()
        val envelope = buildJsonObject { put("schema", 1); put("payload", payload); put("checksum", checksum(payload)) }
        java.io.FileOutputStream(temp).use { stream ->
            stream.write(envelope.toString().toByteArray()); stream.fd.sync()
        }
        check(temp.renameTo(path)) { "Workflow journal write failed" }
    }
    override suspend fun read(host: String, workspace: String): String {
        val (c, scope) = scope(host, workspace)
        val reply = c.call("workflow.snapshot", buildJsonObject { put("workspace", workspace) })
        check(scope(host, workspace).second == scope)
        check(reply.getValue("host").jsonPrimitive.content == c.host)
        val view = WorkflowPresentation.decode(reply.getValue("presentation").toString(), workspace)
        for (run in view.runs) for (action in run.actions) observed[key(scope, run.item, action.descriptor.toString())] =
            Observation(view.value.getValue("revision").jsonPrimitive.content, view.value.getValue("epoch").jsonPrimitive.content, requireNotNull(run.text("run")))
        return view.value.toString()
    }
    override suspend fun pending(host: String, workspace: String): List<RetainedWorkflow> = mutex.withLock {
        load(scope(host, workspace).second).filter { it["phase"]?.jsonPrimitive?.content in listOf("prepared", "preparing") }
            .map { RetainedWorkflow(it.getValue("id").jsonPrimitive.content, it.getValue("item").jsonPrimitive.content,
                it.getValue("run").jsonPrimitive.content, it.getValue("label").jsonPrimitive.content, it["phase"]?.jsonPrimitive?.content == "preparing") }
    }
    override suspend fun dispatch(host: String, workspace: String, item: String, actionJson: String): String = mutex.withLock {
        val (c, scope) = scope(host, workspace)
        val entries = load(scope)
        val selection = Json.parseToJsonElement(actionJson).jsonObject
        val action = selection["action"]?.jsonObject ?: selection
        val reason = selection["reason"]
        entries.firstOrNull { it["phase"]?.jsonPrimitive?.content != "abandoned" && it["item"]?.jsonPrimitive?.content == item && it["action"] == action }?.let { prior ->
            check(prior["reason"] == reason) { "Inspect or retry the retained decision before changing its reason" }
            return@withLock prior["result"]?.jsonPrimitive?.content ?: error("Retry the retained action")
        }
        val observation = requireNotNull(observed[key(scope, item, action.toString())])
        val entry = buildJsonObject {
            put("phase", "preparing"); put("id", UUID.randomUUID().toString()); put("item", item); put("run", observation.run)
            put("label", action.getValue("label")); put("action", action); put("attempts", 0)
            if (reason != null) put("reason", reason)
        }
        entries.add(entry); val index = entries.lastIndex
        save(scope, entries) // preparing marker before prepare, no implicit retry
        val reply = c.call("workflow.prepare_selection", buildJsonObject {
            put("workspace", workspace); put("item", item); put("action", action)
            if (reason != null) put("reason", reason)
            put("revision", observation.revision); put("epoch", observation.epoch)
        })
        check(scope(host, workspace).second == scope)
        if (reply["kind"]?.jsonPrimitive?.content == "Validation") {
            val text = reply.getValue("error").jsonObject.getValue("text").jsonPrimitive.content
            entries[index] = JsonObject(entry + ("phase" to JsonPrimitive("abandoned"))); save(scope, entries)
            text
        } else if (reply["kind"]?.jsonPrimitive?.content == "Inspection") {
            val result = reply.getValue("intent").toString()
            entries[index] = JsonObject(entry + mapOf("phase" to JsonPrimitive("complete"), "result" to JsonPrimitive(result))); save(scope, entries)
            result
        } else {
            check(reply["kind"]?.jsonPrimitive?.content == "Mutation")
            entries[index] = JsonObject(entry + mapOf("phase" to JsonPrimitive("prepared"), "prepared" to reply.getValue("prepared")))
            save(scope, entries)
            submit(host, workspace, scope, entries, index)
        }
    }
    override suspend fun retry(host: String, workspace: String, id: String): String = mutex.withLock {
        val scope = scope(host, workspace).second
        val entries = load(scope)
        val index = entries.indexOfFirst { it["id"]?.jsonPrimitive?.content == id }
        check(index >= 0 && entries[index]["result"] == null)
        if (entries[index].getValue("phase").jsonPrimitive.content == "preparing") {
            read(host, workspace)
            check(scope(host, workspace).second == scope)
            entries[index] = JsonObject(entries[index] + ("phase" to JsonPrimitive("abandoned"))); save(scope, entries)
            "No action was submitted. Inspect the refreshed run and choose an action."
        } else {
            check(entries[index].getValue("phase").jsonPrimitive.content == "prepared")
            submit(host, workspace, scope, entries, index)
        }
    }
    private suspend fun submit(host: String, workspace: String, scope: String, entries: MutableList<JsonObject>, index: Int): String {
        val (c, now) = scope(host, workspace)
        check(now == scope)
        val entry = entries[index]; val attempts = entry.getValue("attempts").jsonPrimitive.long
        check(attempts < Long.MAX_VALUE)
        val prepared = entry.getValue("prepared")
        entries[index] = JsonObject(entry + ("attempts" to JsonPrimitive(attempts + 1))); save(scope, entries)
        val result = c.call("workflow.submit_action", buildJsonObject { put("prepared", prepared) }).toString()
        entries[index] = JsonObject(entries[index] + mapOf("phase" to JsonPrimitive("complete"), "result" to JsonPrimitive(result))); save(scope, entries)
        check(scope(host, workspace).second == scope)
        return result
    }
}
