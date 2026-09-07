package com.farcooler.model

/**
 * Why a runner said no to one request, and what this app says about it.
 *
 * **The runner sends a stable machine word; this file owns the sentence.** The
 * same rule [com.farcooler.ui.AgentFailure] follows for a pane with no agent in
 * it and [AdapterTestOutcome] follows for the Test button. The word is one of
 * the `ErrorCode` names in `proto/farcooler.proto`, spelled in kebab by
 * `farcooler_core::error::word`, carried across the FFI on the answer line as
 * `code`, and read off [com.farcooler.core.CoreException] here.
 *
 * **Why this is not `Connection.Failure`.** That type is a different axis and
 * its inputs prove it: it is built from a connect-time message by substring, it
 * needs a runner's name and port to say anything, and every next move it offers
 * is about keys and authorization. It answers "we could not reach this runner
 * at all". This answers "we reached it, it understood, and it refused" — a live
 * session, one control, and next moves that are mostly things to do on the
 * runner rather than buttons on a screen. Folding a code-keyed table into a
 * message-keyed enum would give one type two constructors that can never both
 * be right.
 *
 * **Only codes that can actually arrive are here.** Of the twenty-eight in the
 * proto, twelve cannot reach a phone at all: `host-offline` has no
 * `DomainError` variant to produce it, eight more have no production site
 * anywhere in the tree, `version-incompatible` is intercepted at the handshake,
 * `client-too-slow` is raised on the local send and never put in a response,
 * and `idempotency-mismatch` has no caller outside the store's own tests.
 * `confirmation-required` is real but the client core turns both of its remove
 * paths into structured outcomes first, and `operation-failed` is the generic —
 * it is the caller's own sentence, which is what [troubleFor] falls back to.
 *
 * Word for word with `apps/shared/AgentKit/Sources/AgentKit/RunnerRefusal.swift`,
 * because a runner refusing the same thing must not be described two ways
 * depending on which phone is in your hand. `RunnerRefusalTest` pins that from
 * this side; Kotlin cannot import the Swift.
 */
enum class RunnerRefusal(val word: String, val sentence: String) {
    /** This device's access was withdrawn while a request was in flight. */
    AUTH_REQUIRED(
        "auth-required",
        "This device’s access to this runner was withdrawn. Add it again to get it back."),
    /** This device was enrolled to look, not to change things. */
    SCOPE_DENIED(
        "scope-denied",
        "This device can only look at this runner. Changing anything needs control, which is " +
            "granted from a device that already has it."),
    /** The runner is older than the feature being asked for. */
    CAPABILITY_UNSUPPORTED(
        "capability-unsupported",
        "This runner’s Far Cooler is too old for this. Update it there, then try again."),
    /** Whatever was being acted on is not there anymore. */
    NOT_FOUND(
        "not-found",
        "It isn’t on the runner anymore. Something else removed it while this was open."),
    /** The runner has no tmux, which is what it runs everything in. */
    TMUX_UNAVAILABLE(
        "tmux-unavailable",
        "The runner can’t reach tmux. Far Cooler runs every pane inside it, so install tmux " +
            "there and try again."),
    /** A directory of that name is already on disk. */
    WORKTREE_EXISTS(
        "worktree-exists",
        "There’s already a folder with that name on the runner. Pick another name."),
    /** A branch of that name is already in the repository. */
    BRANCH_EXISTS(
        "branch-exists",
        "That branch already exists. Pick another name, or resume the branch you have."),
    /** Panes are still alive under the thing being removed. */
    RUNNING_PROCESSES(
        "running-processes",
        "Something is still running there. Stop it first, then try this again."),
    /** Nothing is running, but records would be orphaned. */
    WORKSPACES_EXIST(
        "workspaces-exist",
        "Its workspaces are still here. Remove those first, then remove the folder."),
    /** Outside every allowlisted root, or overlapping one. */
    PATH_NOT_ALLOWED(
        "path-not-allowed",
        "That folder isn’t inside one you’ve added, or it overlaps one you already have."),
    /** A whole home directory or a system path, which never becomes allowable. */
    SENSITIVE_ROOT(
        "sensitive-root",
        "That’s a whole home folder or a system folder, and it can never be added. Pick a " +
            "folder inside it."),
    /** No base to compare this branch against. */
    BASE_UNRESOLVABLE(
        "base-unresolvable",
        "Can’t find the branch this work is based on. Pick a base to compare against."),
    /** Somebody else moved it first. */
    RESOURCE_CONFLICT(
        "resource-conflict",
        "Something else changed this first. Take another look and try again."),
    /** The request itself was malformed. Ours to fix, not the reader's. */
    INVALID_ARGUMENT(
        "invalid-argument",
        "This runner couldn’t make sense of what Far Cooler asked for. That’s a problem in " +
            "the app, not in anything you typed.");

    companion object {
        /**
         * A word this build has a sentence for, or null for every other input —
         * absent, empty, the generic, a code that cannot reach a phone, and a
         * code from a runner newer than this build.
         */
        fun of(word: String?): RunnerRefusal? =
            if (word.isNullOrEmpty()) null else entries.firstOrNull { it.word == word }
    }
}

/**
 * The failure to put on a screen, given the word a runner sent.
 *
 * One function rather than a nullable every caller unwraps, because the
 * unwrapping is where the bug lives. A caller that writes `if (refusal != null)`
 * has, by construction, an `else` it must remember to fill — and the branch it
 * would forget is the one a runner NEWER than this build takes, which is the
 * failure that shipped in the agent-failure work and had to be fixed on macOS
 * afterwards. Here the unknown word is not a branch anybody writes: it is the
 * default, and it is [generic] with the runner's words in the box beneath it,
 * which is exactly what every one of these screens already showed.
 *
 * So an unknown code degrades to a generic failure and never to silence, and a
 * null word does too — that is a link that dropped or an argument this app
 * refused before sending, and no runner refused anything.
 *
 * The transcript is dropped wherever we have a diagnosis of our own. The core's
 * `Display` under one of these says strictly less than the sentence above it —
 * "workspaces still exist under this resource" under "Remove those first" — so
 * keeping it would be noise rather than diagnosis.
 */
fun troubleFor(word: String?, message: String?, generic: String): Trouble {
    val refusal = RunnerRefusal.of(word) ?: return Trouble(generic, message)
    return Trouble(refusal.sentence)
}

/**
 * The same, with a sentence of this screen's own about the step in front of it.
 *
 * For the sheets whose failure has a STEP as well as a reason — "Created the
 * worktree, but couldn't start Claude." — where dropping the step would lose
 * the half that says how much of the job got done. Two sentences, both this
 * app's, which is a different thing from splicing our prose onto a runner's:
 * nothing here is quoted from the core.
 */
fun troubleAfter(word: String?, message: String?, context: String): Trouble {
    val refusal = RunnerRefusal.of(word) ?: return Trouble(context, message)
    return Trouble(context + " " + refusal.sentence)
}
