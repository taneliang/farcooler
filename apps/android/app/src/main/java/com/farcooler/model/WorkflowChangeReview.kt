package com.farcooler.model

/**
 * A workflow change, read back out of the question the daemon asks a person (ov-978).
 *
 * The daemon writes the question: a headline, the whole change in labeled parts, and last the words that
 * approve it, ``answer `approve <code>` ``. The Review screen shows every part, then Approve at the end,
 * which sends exactly those words. Anything without this shape returns null, so a question that isn't a
 * change (or one from a newer daemon) stays the plain Answer dialog and never gains an Approve button
 * from a guess.
 *
 * The shape is pinned by `crates/client/src/testdata/workflow-change-question.txt`, written by the daemon's
 * own producer; the Apple twin is `WorkflowChangeReview.swift` in AgentKit.
 */
class WorkflowChangeReview private constructor(
    /** The one sentence a row shows. */
    val headline: String,
    /** What comes before the parts: the orchestrator's reason, and what this replaces. */
    val intro: List<String>,
    /** Every part the daemon wrote, in its order. */
    val parts: List<Part>,
    private val code: String,
) {
    class Part(val title: String, val items: List<String>)

    /** What Approve sends. */
    val approval: String get() = "approve $code"

    companion object {
        /**
         * Whether a phone is shown Approve and Decline: the grant isn't Read ([mayAnswer]) and the phone is the
         * owner's. The daemon counts only the owner's answer, so a member's Approve is refused; they read the
         * change and are told who can approve it (ov-991). [viewer] is `member.list`'s `you`: empty for the
         * owner, null until the runner has said.
         */
        fun mayApprove(mayAnswer: Boolean, viewer: String?): Boolean = mayAnswer && (viewer?.isEmpty() ?: true)

        const val DECLINE = "decline"
        private val HEADINGS = listOf(
            "What runs as you:", "Checks that must pass:", "How code lands:", "Who approves:",
            "In full, exactly as it runs:",
        )
        private val APPROVE = Regex("answer `approve ([0-9a-f]{6})`")

        fun parse(question: String): WorkflowChangeReview? {
            val paragraphs = question.split("\n\n")
            if (paragraphs.size < 3) return null
            val headline = paragraphs.first()
            if (!headline.startsWith("Publish this workflow?")) return null
            val closing = paragraphs.last()
            val code = APPROVE.find(closing)?.groupValues?.get(1) ?: return null
            if (!closing.contains("answer `decline`")) return null
            val intro = mutableListOf<String>()
            val parts = mutableListOf<Part>()
            for (paragraph in paragraphs.subList(1, paragraphs.size - 1)) {
                val lines = paragraph.split("\n")
                if (lines.first() in HEADINGS) {
                    val rest = lines.drop(1)
                    if (rest.isEmpty() || rest.any { !it.startsWith("- ") }) return null
                    parts += Part(lines.first().dropLast(1), rest.map { it.removePrefix("- ") })
                } else if (parts.isEmpty()) {
                    intro += paragraph
                } else {
                    return null
                }
            }
            if (parts.map { it.title + ":" } != HEADINGS) return null
            return WorkflowChangeReview(headline, intro, parts, code)
        }
    }
}
