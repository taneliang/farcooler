package com.farcooler.model

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import com.farcooler.core.UnreachedException
import java.net.URI
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

/** Where a member's Claude login stands on a runner (`Member.claude_login.state`). */
enum class ClaudeLoginState { NONE, SIGNED_IN, STOPPED_WORKING }

/** A member's own Claude login, as `member.list` and the sign-in calls name it. */
data class ClaudeLogin(val state: ClaudeLoginState, val email: String) {
    companion object {
        val NONE = ClaudeLogin(ClaudeLoginState.NONE, "")

        /** `{state, email}`; anything unrecognized is "none", as the runner's own reading is. */
        fun of(json: JsonObject?): ClaudeLogin {
            json ?: return NONE
            val state = when ((json["state"] as? JsonPrimitive)?.contentOrNull) {
                "signed_in" -> ClaudeLoginState.SIGNED_IN
                "stopped_working" -> ClaudeLoginState.STOPPED_WORKING
                else -> ClaudeLoginState.NONE
            }
            return ClaudeLogin(state, (json["email"] as? JsonPrimitive)?.contentOrNull.orEmpty())
        }
    }
}

/**
 * A member signing in to Claude from this phone (ov-584): when the section is
 * offered, the page the runner hands back and the app's re-check of it, the
 * code, the sheet's steps, and every sentence. The Mac and the iPhone say the
 * same words.
 *
 * The runner runs `claude auth login` with no browser; this phone opens the
 * page it printed, the person approves with their own Claude account, copies
 * the code Claude shows, and pastes it back.
 */
object ClaudeSignIn {
    const val SECTION_TITLE = "Claude"
    const val BODY = "Sign in so your rep and the lanes you start use your own Claude account and its connectors."
    const val SIGN_IN = "Sign in to Claude…"
    const val SIGN_IN_AGAIN = "Sign in again…"
    const val SIGN_OUT = "Sign out"
    const val SHEET_TITLE = "Sign in to Claude"
    const val OPEN_PAGE_BODY = "Open the sign-in page and approve with the Claude account you want to use."
    const val OPEN_PAGE = "Open sign-in page"
    const val CODE_BODY = "Claude shows a code. Copy it, then paste it here."
    const val CODE_LABEL = "Code"
    const val FINISH = "Sign in"
    const val FINISHING = "Signing in…"
    const val CANCEL = "Cancel"
    const val DONE = "Done"
    const val TRY_AGAIN = "Try again"
    const val START_AGAIN = "Start again"
    const val SIGN_OUT_MESSAGE = "Your rep and lanes there stop until you sign in again."

    /**
     * The not-signed-in body. [sharedAccount] is non-null where this member's panes use the runner's own Claude
     * login until they sign in (ov-652): its email, or "" when this phone isn't told it. AgentKit's words.
     */
    fun body(sharedAccount: String?): String {
        if (sharedAccount == null) return BODY
        val whose = if (sharedAccount.isEmpty()) "this runner’s own Claude account" else "this runner’s own Claude account, $sharedAccount"
        return "Until you sign in, your rep and the lanes you start use $whose. Sign in to use your own account and its connectors."
    }

    /** The sign-out message: where members share the runner's Claude login, their panes move to it rather than stop. */
    fun signOutMessage(shared: Boolean): String =
        if (shared) "Your rep and lanes there use this runner’s own Claude account from the next time each starts." else SIGN_OUT_MESSAGE
    const val STOPPED_WORKING = "Claude login stopped working"
    const val NO_APP = "No browser on this phone can open the sign-in page."

    /** The most a pasted code can be; the runner refuses more. */
    const val MAX_CODE = 2048

    /** The most a sign-in page's address can be; the runner's cap too. */
    const val MAX_URL = 4096

    fun starting(runner: String) = "Getting a sign-in link from $runner…"
    const val SIGNED_IN_AS = "Signed in as"
    const val SIGN_OUT_FAILED = "Couldn’t sign out. Try again."
    const val CODE_TOO_LONG = "That’s longer than a sign-in code."
    fun signOutTitle(runner: String) = "Sign out of Claude on $runner?"

    /**
     * The trust copy, under the button and in the sheet. The orchestrator's
     * lead differs by workspace, and a runner has no single one to name, so it
     * says "the workspace lead’s" where the Mac, which reads a workspace,
     * can name them.
     */
    fun trust(runner: String) =
        "Your Claude login is kept on $runner. Its owner, and anyone with Control of it, could use it. " + // casing ok: the access level's name
            "Far Cooler uses it only for your rep and the lanes you start. " +
            "Lanes the orchestrator starts for your asks use the workspace lead’s login."

    /**
     * Whether the Claude section is drawn: the runner offers sign-in
     * (`claude_sign_in`) and this phone is a member's own device. Not the
     * owner's (empty [you]), not a released device's, and not before the
     * runner has said who this phone is (null).
     */
    fun offered(daemon: DaemonBuild?, you: String?): Boolean =
        daemon?.can(Capability.CLAUDE_SIGN_IN) == true &&
            you != null && you.isNotEmpty() && you != RELEASED

    /** The row for [you] in a `member.list` answer, or null if they aren't in it. */
    fun loginIn(reply: JsonObject, you: String): ClaudeLogin? {
        val members = reply["members"] as? JsonArray ?: return null
        val mine = members.firstNotNullOfOrNull { m ->
            val member = m as? JsonObject
            member?.takeIf { (it["handle"] as? JsonPrimitive)?.contentOrNull == you }
        } ?: return null
        return ClaudeLogin.of(mine["claude_login"] as? JsonObject)
    }

    /**
     * Whether [raw] is a sign-in page this phone may open: https, no
     * credentials or port, one of the two hosts and paths Claude uses, no
     * control characters, spaces or backslashes, and a length cap. The runner
     * checks the same before handing it over; this is the phone's own check on
     * what it is about to open, since a runner is not the phone's to trust
     * with the browser.
     */
    fun isSignInPage(raw: String): Boolean {
        if (raw.isEmpty() || raw.length > MAX_URL) return false
        if (raw.any { it.isISOControl() || it.isWhitespace() || it == '\\' }) return false
        val uri = try {
            URI(raw)
        } catch (_: Exception) {
            return false
        }
        if (uri.scheme != "https" || uri.rawUserInfo != null || uri.port != -1) return false
        val host = uri.host ?: return false
        val path = uri.rawPath ?: return false
        return PAGES.any { (h, p) -> host == h && path == p }
    }

    /** What the code field holds, trimmed, or null if it holds none. */
    fun code(field: String): String? = field.trim().ifEmpty { null }

    /** Claude's code is `code#state`; one with no `#` was cut short in the copy. */
    fun isWholeCode(code: String): Boolean = '#' in code

    /** What a failed step says, and what the button beside it does. */
    sealed interface Failure {
        val sentence: String
        val action: String?

        data object Incomplete : Failure {
            override val sentence = "That code isn’t complete. Copy the whole code and try again."
            override val action: String? = null
        }

        data object Refused : Failure {
            override val sentence = "That code didn’t work. Open the sign-in page again to get a new one."
            override val action: String? = TRY_AGAIN
        }

        data object Expired : Failure {
            override val sentence = "The sign-in link expired."
            override val action: String? = START_AGAIN
        }

        data class TooOld(val runner: String) : Failure {
            override val sentence = "Couldn’t start sign-in on $runner. Claude Code there needs to be updated."
            override val action: String? = null
        }

        /** Five starts a minute is the runner's limit. */
        data object TooOften : Failure {
            override val sentence = "Too many sign-in attempts. Wait a minute, then try again."
            override val action: String? = null
        }

        /** The runner refused for a reason a person can't act on. */
        data object Generic : Failure {
            override val sentence = "Couldn’t sign in. Try again."
            override val action: String? = TRY_AGAIN
        }

        /** No browser on the phone takes the page. */
        data object NoBrowser : Failure {
            override val sentence = NO_APP
            override val action: String? = null
        }

        /** The runner couldn't be reached: the link, not a refusal. */
        data class Silent(val runner: String) : Failure {
            override val sentence = "$runner isn’t answering. Check your connection and try again."
            override val action: String? = TRY_AGAIN
        }
    }

    /**
     * The failure for what a call raised, by the word the runner refused with
     * and the conflict it named. The runner's own sentence is never shown.
     * "Isn't answering" is for the link failing; a refusal that isn't one of
     * the known ones is [Failure.Generic]. A missing session ("not-found")
     * is an expired link only when [finishing].
     */
    fun failure(error: Throwable, runner: String, finishing: Boolean = false): Failure {
        val word = (error as? CoreException)?.word
        val what = (error as? CoreException)?.what
        return when {
            what == "sign_in_code" -> Failure.Incomplete
            what == "code_refused" -> Failure.Refused
            what == "sign_in_too_often" -> Failure.TooOften
            what == "sign_in_too_old" || what == "sign_in_unreadable" -> Failure.TooOld(runner)
            finishing && word == RunnerRefusal.NOT_FOUND.word -> Failure.Expired
            error !is CoreException || error is DisconnectedException || error is UnreachedException ||
                word == RunnerRefusal.TIMED_OUT_WORD -> Failure.Silent(runner)
            else -> Failure.Generic
        }
    }

    private const val RELEASED = "(released)"
    private val PAGES = listOf("claude.com" to "/cai/oauth/authorize", "platform.claude.com" to "/oauth/authorize")
}

/** The sheet's steps. [Failed] keeps what step it came from, for what Try Again does. */
sealed interface ClaudeSignInStep {
    data object Starting : ClaudeSignInStep
    data class OpenPage(val url: String) : ClaudeSignInStep
    data class Code(val url: String) : ClaudeSignInStep
    data object Finishing : ClaudeSignInStep
    data class Done(val email: String) : ClaudeSignInStep
    data class Failed(val failure: ClaudeSignIn.Failure, val url: String?) : ClaudeSignInStep
}
