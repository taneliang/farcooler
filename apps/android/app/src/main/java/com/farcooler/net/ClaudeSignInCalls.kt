package com.farcooler.net

import com.farcooler.model.ClaudeLogin
import com.farcooler.model.ClaudeSignIn
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

/**
 * A member signing in to Claude from this phone (ov-584): the runner's four
 * calls behind `claude_sign_in`, each answered or failed with the sentence the
 * sheet shows (never the runner's own text).
 */
sealed interface SignInReply<out T> {
    data class Ok<T>(val value: T) : SignInReply<T>
    data class Failed(val failure: ClaudeSignIn.Failure) : SignInReply<Nothing>
}

/** The page and session a sign-in began with. */
data class StartedSignIn(val session: String, val url: String)

/** This member's own login, from `member.list`, or null if the runner didn't say. */
suspend fun Connection.claudeLogin(you: String): ClaudeLogin? {
    val reply = attempt { core.call("member.list", JsonObject(emptyMap())) }.getOrNull() ?: return null
    return ClaudeSignIn.loginIn(reply, you)
}

private suspend fun <T> Connection.signInCall(
    method: String,
    args: JsonObject,
    finishing: Boolean = false,
    read: (JsonObject) -> T?,
): SignInReply<T> {
    val answer = try {
        core.call(method, args)
    } catch (e: Exception) {
        e.rethrowIfCancellation()
        return SignInReply.Failed(ClaudeSignIn.failure(e, host.displayLabel, finishing))
    }
    val value = read(answer) ?: return SignInReply.Failed(ClaudeSignIn.Failure.Silent(host.displayLabel))
    return SignInReply.Ok(value)
}

/**
 * `claude.sign_in`: a page to open. One that isn't Claude's sign-in page is
 * not handed on: it reads as a runner whose Claude Code this phone can't use.
 */
suspend fun Connection.startClaudeSignIn(you: String): SignInReply<StartedSignIn> {
    val runner = host.displayLabel
    val started = when (val reply = signInCall("claude.sign_in", Connection.args("handle" to you)) { it }) {
        is SignInReply.Failed -> return reply
        is SignInReply.Ok -> reply.value
    }
    val session = (started["session"] as? JsonPrimitive)?.contentOrNull
    val url = (started["url"] as? JsonPrimitive)?.contentOrNull
    if (session == null || url == null || !ClaudeSignIn.isSignInPage(url)) {
        return SignInReply.Failed(ClaudeSignIn.Failure.TooOld(runner))
    }
    return SignInReply.Ok(StartedSignIn(session, url))
}

/** `claude.sign_in_finish`: the login it left. */
suspend fun Connection.finishClaudeSignIn(session: String, code: String): SignInReply<ClaudeLogin> =
    signInCall("claude.sign_in_finish", Connection.args("session" to session, "code" to code), finishing = true) { ClaudeLogin.of(it) }

/** `claude.sign_in_cancel`: the session ends. Whatever it answers, there is nothing to show. */
suspend fun Connection.cancelClaudeSignIn(session: String) {
    attempt { core.call("claude.sign_in_cancel", Connection.args("session" to session)) }
}

/** `claude.sign_out`: the login it left. */
suspend fun Connection.signOutOfClaude(you: String): SignInReply<ClaudeLogin> =
    signInCall("claude.sign_out", Connection.args("handle" to you)) { ClaudeLogin.of(it) }
