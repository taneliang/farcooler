/*
 * Far Cooler review core — the C ABI.
 *
 * Small on purpose. The daemon does the git work and ships hunks over the
 * protocol, so a client never parses a patch. What a client does need locally
 * is the other kind of diff: an agent's tool call carries a file before and
 * after, with no git involved, and the transcript renders it as it streams.
 *
 * That used to be computed in Swift, which left one app holding two diff
 * implementations with two line models. It lives here now for the same reason
 * colour resolution lives in the terminal core: three renderers cannot be
 * trusted to agree, and these two diffs describe the same file.
 *
 * Lifetime: every char* returned is owned by the CALLER and must be passed to
 * farcooler_review_string_free exactly once.
 *
 * Null safety: a NULL text is read as empty — a new file is a real case, not an
 * error. A renderer bug must not take down the app.
 */

#ifndef FARCOOLER_REVIEW_H
#define FARCOOLER_REVIEW_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Diff two whole texts. Returns a JSON array, or NULL if either text is not
 * valid UTF-8:
 *
 *   [{"kind":"context"|"added"|"removed",
 *     "old_no":<number|null>,
 *     "new_no":<number|null>,
 *     "text":"...",
 *     "no_newline":<bool, omitted when false>}]
 *
 * JSON rather than a packed struct array, deliberately: unlike a terminal grid
 * this is computed once per tool call and then scrolled, so one parse costs
 * nothing and the shape stays free to grow a field without every renderer
 * having to agree on a layout the same day.
 */
char *farcooler_review_diff_texts(const char *old_text, const char *new_text);

/*
 * How many lines were added and removed, without building the JSON.
 *
 * For the "+N -M" summary above a collapsed diff, where allocating a whole
 * document to count it and throw it away would be the only cost.
 *
 * Either out-pointer may be NULL. Returns false only when a text is not valid
 * UTF-8.
 */
bool farcooler_review_count_changes(const char *old_text,
                                    const char *new_text,
                                    uint32_t *added_out,
                                    uint32_t *removed_out);

/* Free a string returned by this library. NULL is a no-op. */
void farcooler_review_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* FARCOOLER_REVIEW_H */
