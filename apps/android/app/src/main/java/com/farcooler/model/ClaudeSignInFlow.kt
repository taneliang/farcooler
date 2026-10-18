package com.farcooler.model

import com.farcooler.net.SignInReply
import com.farcooler.net.StartedSignIn
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** What opening the sign-in page came to. */
enum class PageOpen { OPENED, REFUSED, NO_BROWSER }

/** The runner's three sign-in calls, as the flow needs them. */
interface ClaudeSignInApi {
    suspend fun start(): SignInReply<StartedSignIn>
    suspend fun finish(session: String, code: String): SignInReply<ClaudeLogin>
    suspend fun cancel(session: String)
}

/**
 * The sign-in sheet's state (ov-584): its step, its code, and the one rule
 * that matters, that a reply landing after the sheet closed or restarted
 * changes nothing on screen.
 *
 * Every call carries the generation it began in; closing and starting bump it,
 * and each reply checks it before touching [step], the session or the login
 * row. A call can't be cancelled on the runner by dropping its coroutine (the
 * reply would be lost and, for a start, so would the session), so a start
 * that lands late sends its session's cancel itself, and a finish that lands
 * late still tells [onLogin], since the runner did sign them in.
 */
class ClaudeSignInFlow(
    private val scope: CoroutineScope,
    private val api: ClaudeSignInApi,
    private val runner: String,
    private val onLogin: (ClaudeLogin) -> Unit,
) {
    private val _step = MutableStateFlow<ClaudeSignInStep?>(null)
    val step: StateFlow<ClaudeSignInStep?> = _step.asStateFlow()

    private val _code = MutableStateFlow("")
    val code: StateFlow<String> = _code.asStateFlow()

    private val _codeNote = MutableStateFlow<String?>(null)

    /** "That's longer than a sign-in code." after a paste the field refused. */
    val codeNote: StateFlow<String?> = _codeNote.asStateFlow()

    private var generation = 0
    private var session: String? = null

    /** Begin (or begin again): any session already held is cancelled first. */
    fun start() {
        val old = session
        session = null
        val mine = ++generation
        _code.value = ""
        _codeNote.value = null
        _step.value = ClaudeSignInStep.Starting
        scope.launch {
            if (old != null) api.cancel(old)
            val reply = api.start()
            if (mine != generation) {
                // Closed or restarted while this was on its way.
                if (reply is SignInReply.Ok) api.cancel(reply.value.session)
                return@launch
            }
            _step.value = when (reply) {
                is SignInReply.Failed -> ClaudeSignInStep.Failed(reply.failure, null)
                is SignInReply.Ok -> {
                    session = reply.value.session
                    ClaudeSignInStep.OpenPage(reply.value.url)
                }
            }
        }
    }

    /** The page was opened, or wasn't, for [url]. */
    fun opened(url: String, result: PageOpen) {
        _step.value = when (result) {
            PageOpen.OPENED -> ClaudeSignInStep.Code(url)
            PageOpen.REFUSED -> ClaudeSignInStep.Failed(ClaudeSignIn.Failure.TooOld(runner), null)
            PageOpen.NO_BROWSER -> ClaudeSignInStep.Failed(ClaudeSignIn.Failure.NoBrowser, url)
        }
    }

    /** The field changed: past the cap it keeps what it had and says so; an error clears. */
    fun edit(text: String) {
        if (text.length > ClaudeSignIn.MAX_CODE) {
            _codeNote.value = ClaudeSignIn.CODE_TOO_LONG
            return
        }
        _codeNote.value = null
        _code.value = text
        val showing = _step.value
        if (showing is ClaudeSignInStep.Failed && showing.failure == ClaudeSignIn.Failure.Incomplete && showing.url != null) {
            _step.value = ClaudeSignInStep.Code(showing.url)
        }
    }

    /** Send the code: it must be whole, or the step says so without asking the runner. */
    fun submit(url: String) {
        val whole = ClaudeSignIn.code(_code.value)?.filterNot { it.isWhitespace() } ?: return
        if (!ClaudeSignIn.isWholeCode(whole)) {
            _step.value = ClaudeSignInStep.Failed(ClaudeSignIn.Failure.Incomplete, url)
            return
        }
        val live = session ?: return
        val mine = generation
        _step.value = ClaudeSignInStep.Finishing
        scope.launch {
            val reply = api.finish(live, whole)
            if (reply is SignInReply.Ok) onLogin(reply.value)
            if (mine != generation) return@launch
            _step.value = when (reply) {
                is SignInReply.Failed -> ClaudeSignInStep.Failed(reply.failure, url)
                is SignInReply.Ok -> {
                    session = null
                    ClaudeSignInStep.Done(reply.value.email)
                }
            }
        }
    }

    /** The sheet closed: whatever is on its way no longer shows, and a live session ends. */
    fun close() {
        val live = session
        val done = _step.value is ClaudeSignInStep.Done
        generation++
        session = null
        _step.value = null
        _code.value = ""
        _codeNote.value = null
        if (live != null && !done) scope.launch { api.cancel(live) }
    }
}
