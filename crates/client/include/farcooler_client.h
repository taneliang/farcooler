/*
 * Far Cooler client core — the C ABI.
 *
 * The whole of "talk to a Far Cooler host", for clients that are not the host:
 * SSH transport, protocol, and the shapes of the answers. iOS and Android
 * consume this; the Mac app and CLI can shell out to `ssh` instead, but a phone
 * has no `ssh` binary and no way to run one.
 *
 * ## Asynchronous underneath, synchronous at the boundary
 *
 * A UI toolkit's idea of async is not a Rust runtime's, and bridging them with
 * callbacks means one side is always wrong about which thread it is on. So:
 *
 *   1. Submit work. You get a ticket back immediately.
 *   2. The runtime does it on its own threads.
 *   3. Poll for finished results, oldest first.
 *
 * A UI already has a frame loop or a timer, so polling is free, and there is no
 * question about callback threading, re-entrancy, or a view disappearing while
 * a request is in flight.
 *
 * ## Everything is JSON
 *
 * The wire stays protobuf. This boundary is JSON because the alternative is a
 * protobuf runtime and generated types in Swift, and again in Kotlin, to
 * describe messages this library has already decoded.
 *
 * Every result has the same envelope:
 *
 *     {"ticket": 3, "ok": true,  "result": { ... }}
 *     {"ticket": 4, "ok": false, "error": "a sentence for a human"}
 *
 * ## Lifetime and threading
 *
 * A handle is not thread-safe; confine it to one thread. The pointer returned
 * by farcooler_client_poll belongs to the handle and is valid until the next
 * call on it — copy anything you keep.
 *
 * Every function tolerates a NULL handle. A UI bug must not take down the app.
 */

#ifndef FARCOOLER_CLIENT_H
#define FARCOOLER_CLIENT_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Create a client. Free with farcooler_client_free. NULL if a runtime could
 * not be started. */
void *farcooler_client_new(void);

/* Destroy a client, ending any SSH session it holds. Safe with NULL. */
void farcooler_client_free(void *handle);

/*
 * Connect to a host. `config` is JSON:
 *
 *     {"host":"box", "port":22, "user":"me",
 *      "private_key":"-----BEGIN OPENSSH PRIVATE KEY-----\n...",
 *      "passphrase":null,
 *      "host_fingerprint":"SHA256:..."}
 *
 * `host_fingerprint` decides how the host's identity is treated:
 *
 *   - present     the key must match exactly, or the call fails naming both
 *                 fingerprints. That is either a reinstalled server or an
 *                 interception, and software should not guess which.
 *   - absent      first contact. The call FAILS, with the host's fingerprint in
 *                 its message, so you can show it to a human. Silently trusting
 *                 an unknown key is what makes an interception invisible.
 *   - "accept-any"  connect without pinning. A deliberate choice, never a
 *                 default.
 *
 * Returns a ticket, or 0 if the arguments could not be read.
 */
uint64_t farcooler_client_connect(void *handle, const char *config);

/*
 * Invoke a method. `args` is JSON; `{}` if it takes none.
 *
 *   fleet                  {}                     -> workspaces with derived state
 *   repositories           {}                     -> registered repositories
 *   host                   {}                     -> what that runner is, and
 *                                                    what this session may do
 *   workspace.create       {repository, task, branch, base?}
 *   workspace.hide         {workspace}
 *   workspace.unhide       {workspace}
 *   workspace.remove_worktree  {workspace, confirm}
 *                          -> {"ok": true} | {"confirmationRequired": true}
 *   repository_root.list   {}
 *   repository_root.add    {path}
 *   repository_root.remove {root, confirm}
 *                          -> {"ok": true} | {"confirmationRequired": true}
 *   terminal.create        {workspace, title, preset}
 *   terminal.stop          {terminal}
 *   terminal.restart       {terminal}
 *   terminal.dismiss_lost  {terminal}
 *   terminal.resize        {terminal, columns, rows}
 *
 * The two `confirm` arguments are a name a PERSON typed, and neither is
 * optional in the way an empty string is optional. They differ in when the
 * runner insists:
 *
 *   - workspace.remove_worktree only wants it when the worktree is dirty, so
 *     "" is a legitimate first attempt, and `confirmationRequired` back from it
 *     means "now ask the person" rather than "you got it wrong".
 *   - repository_root.remove ALWAYS wants it, because removing a root revokes
 *     Far Cooler's permission over a whole directory tree rather than deleting
 *     one worktree. There is no first attempt to make: collect the root
 *     directory's last path segment first, and read `confirmationRequired` as
 *     "that is not its name". An app cannot invent this on the user's behalf —
 *     that is the entire point of typing it.
 *
 * `host` answers `capabilities` and `grantedScope` together, and a control
 * usually needs both: the capability says this runner HAS the feature, the scope
 * says this connection may use it. `grantedScope` is one of "read", "control",
 * "host_admin" — sshd's own word for what it applied to this session, taken from
 * the handshake, not from `client.list`. It is "unspecified" from a runner too
 * old to send it, which means "no answer" and must not be drawn as "no
 * permission".
 *
 * Which devices may log in to that runner — the end of the enrollment ceremony,
 * and the only part of it that changes anything. Every one of these reads or
 * writes the runner's own ~/.ssh/authorized_keys, which is the authority on who
 * may log in; nothing is cached anywhere else.
 *
 *   client.list    {}
 *                  -> {"clients": [{"clientId": "phone-7",
 *                                   "fingerprint": "SHA256:...",
 *                                   "label": "farcooler-Ada-s-iPhone-tKvE1n0y",
 *                                   "scope": "read",
 *                                   "account": "you",
 *                                   "enrolledAt": 0,
 *                                   "foreign": false,
 *                                   "shellAccess": false}]}
 *   client.enroll  {publicKey, label, clientId, scope, shellAccess}
 *                  -> {"client": { ...as above... }, "alreadyEnrolled": false}
 *   client.revoke  {clientId}
 *                  -> {"clients": [ ...what is left... ]}
 *
 * The argument keys are camelCase, like every other multi-word key in this
 * table. `publicKey`, `clientId` and `scope` are required, and a missing one is
 * an error that names it — an app that sent `public_key` would otherwise be told
 * its key does not parse, which is true of the empty string and the wrong place
 * to start looking. `label` is optional: a device with no name still enrolls.
 *
 * `shellAccess` is optional and false by default, which is the RESTRICTED line:
 * `restrict,command="farcoolerd --stdio …"`, the shape this call has always
 * written. True asks for a PLAIN line instead — Key B, the ordinary SSH key that
 * Zed, git and Terminal use, which needs a shell behind it and so cannot carry a
 * forced command. It must be sent with `"scope": "host_admin"`; the runner
 * refuses the pair otherwise, because a request asking for a shell while saying
 * "read" does not agree with itself.
 *
 * A Mac therefore enrolls TWICE, with the same `clientId` and one call each way —
 * and the two calls must be SEQUENTIAL, not concurrent: the runner rebuilds the
 * block from a snapshot read outside the writer's lock, so two enrollments
 * landing together can lose one of the keys. A phone enrolls once, with this
 * false: there is no Zed on a phone. The same key in both shapes is refused
 * outright — sshd takes the first line that matches a key, so that would make
 * "does this device get a shell" a question about line order in a text file.
 *
 * `shellAccess` comes back on every listed line, and it is what tells a Mac's two
 * rows apart in a devices screen: both lines carry that Mac's one client id, so
 * without it "Far Cooler access" and "shell access" are the same object. Never
 * true for a `"foreign": true` line. `client.revoke` removes EVERY line under an
 * id, which for a Mac is both keys in one write.
 *
 * `scope` is one of "read", "control", "host_admin" — a word, because neither
 * Swift nor Kotlin has the wire enum. Anything else is refused rather than
 * defaulted: a key with no scope already means host_admin to sshd, so rounding a
 * typo up would turn a misspelling into the whole runner. A line Far Cooler did
 * not write comes back `"foreign": true` with an empty `clientId` and a scope of
 * "unspecified" — it is reported so a person can see it is there, and it is never
 * touched.
 *
 * `alreadyEnrolled` is not an error. It is the ordinary outcome of enrolling a
 * Mac on itself and of a ceremony offered a runner the device can already reach,
 * and the `client` beside it is then the grant that WAS there rather than the one
 * you asked for. `enrolledAt` is 0 for every device but one just enrolled:
 * authorized_keys records no time, and the file is the authority.
 *
 * `client.enroll` and `client.revoke` need host_admin on the connection; a
 * runner too old to serve any of the three advertises no "enrollment"
 * capability, which farcooler_client_connect already reports — dim the screen
 * from that rather than discovering it at the last step.
 *
 * Ids are UUID strings. An unknown method is an error rather than a no-op, so a
 * typo in a client is visible instead of silent.
 *
 * Returns a ticket, or 0 if the arguments could not be read.
 */
uint64_t farcooler_client_call(void *handle, const char *method, const char *args);

/*
 * Take the oldest finished result, or NULL if none is ready.
 *
 * Owned by the handle, valid until the next call on it. Each result is returned
 * exactly once.
 */
const char *farcooler_client_poll(void *handle);

/* True once a session is established. */
bool farcooler_client_connected(void *handle);

/*
 * Generate a new ed25519 key pair for this device.
 *
 * Done here rather than in Swift or Kotlin for the reason nothing else in this
 * project implements cryptography: there is one place that does it, and it is a
 * library maintained by people who do this for a living. Writing an OpenSSH
 * private-key encoder twice, in two languages, to save a function call would be
 * a poor trade.
 *
 * Writes JSON:
 *
 *     {"private_key": "-----BEGIN OPENSSH PRIVATE KEY-----\n...",
 *      "public_key": "ssh-ed25519 AAAA... comment"}
 *
 * `comment` labels the key in a host's authorized_keys, which is what makes it
 * possible to revoke one device without guessing which line is which. NULL
 * means "farcooler".
 *
 * Returns the number of bytes NEEDED. If that exceeds `capacity` nothing is
 * written — a truncated private key would be worse than none — so call again
 * with a larger buffer. 4096 is ample.
 */
size_t farcooler_client_generate_key(const char *comment, uint8_t *out, size_t capacity);

/**
 * The public key belonging to a private key, as one OpenSSH line.
 *
 * Derived rather than stored: a device has one identity, and keeping the public
 * half somewhere else means two facts that can disagree — which they do, because
 * a keychain and a preferences file do not have the same lifetime.
 *
 * Returns the number of bytes needed; nothing is written if that exceeds
 * `capacity`. Returns 0 if the private key could not be read.
 */
size_t farcooler_client_public_key(const char *private_key, uint8_t *out, size_t capacity);

/**
 * The SHA256 fingerprint of a public key, as a person reads it on screen:
 * `SHA256:tKvE1n0y…`, exactly what `ssh-keygen -lf` prints and what the
 * confirmation screen shows.
 *
 * Raw text, NOT JSON — the same contract farcooler_client_public_key above it
 * uses, because this is the same kind of thing: one derived line, or nothing.
 * Returns the bytes needed, writes nothing when that exceeds `capacity`, and
 * NULL asks for the size. 64 is ample. Returns 0 when the text is not a public
 * key; a device with no readable key has no fingerprint, and a guess would be a
 * string a person compares against another screen.
 *
 * The confirmation screen used to get this by calling
 * farcooler_client_ceremony_reply with its own offer and no runners, then
 * reading `target` out of the manifest. Both come from one computation in
 * `ceremony.rs`, so they cannot disagree — but the sentence telling a human
 * which device they are about to trust should not be a side effect of the
 * leg-two builder.
 */
size_t farcooler_client_fingerprint(const char *public_key, uint8_t *out, size_t capacity);

// The client id to enroll a device under, derived from its own key.
//
// Nothing in the ceremony carries one, so without this each app invents its own
// format and the daemon's "already enrolled" check — which compares client ids —
// stops working across platforms. Raw text, same contract as the fingerprint
// above: bytes needed returned, nothing written when short, NULL asks the size,
// 0 when the text is not a public key.
size_t farcooler_client_client_id(const char *public_key, uint8_t *out, size_t capacity);

/*
 * ============================================================================
 * The enrollment ceremony: two QR codes, four moments.
 * ============================================================================
 *
 * A new device shows a code; a device you already trust scans it, picks
 * runners, and shows a code back. Every rule that decides whether a scan is
 * acceptable lives in Rust, so iOS, Android and macOS share one implementation
 * and none of them has a vote. An app encodes the string it is handed into a QR
 * code, points a camera at one, and shows the sentence that belongs to a
 * returned code. That is the whole of its part.
 *
 * NOTHING IN EITHER CODE IS A SECRET. The first carries public keys, a device
 * name, an opaque account id, the channel and a random ceremony id; the reply
 * carries addresses. Three drafts of this design put a secret in a QR code and
 * each was broken the same way — a symmetric secret on a screen is a bearer
 * token, and whoever films it holds what the scanner holds.
 *
 * A REFUSAL IS A CODE, NOT A SENTENCE. Every call below answers either the
 * payload it was asked for or an error object:
 *
 *     {"error": "wrong_ceremony"}
 *     {"error": "version", "version": 2}
 *     {"error": "channel", "channel": "canary"}
 *
 * The words are stable and the app owns the copy for each:
 *
 *     version         a newer Far Cooler made this code
 *     channel         another deployment's ceremony
 *     malformed       not a Far Cooler code
 *     wrong_ceremony  a reply to a different ceremony
 *     wrong_account   a reply for a different account
 *     wrong_target    a reply addressed to another device's key
 *     stale           scanned too long ago, by THIS device's clock
 *     already_taken   this ceremony has answered once already
 *     too_large       more runners than one code can carry
 *
 * Every one of these takes the buffer contract farcooler_client_generate_key
 * uses: JSON into `out`, returning the bytes NEEDED, writing nothing when that
 * exceeds `capacity`. 4096 is ample for an offer; a manifest is bounded by the
 * byte budget you pass it.
 */

/*
 * Leg one, the displaying side: build the code a new device shows.
 *
 * `key_b` may be NULL — a phone has one key, because there is no Zed on a
 * phone. The channel and the ceremony id are the library's to set: a device
 * that could be told which channel it is could be told wrong.
 *
 * The returned string is BOTH what goes in the QR code and what the device
 * keeps, to pass back as `expecting_json` when the reply arrives.
 */
size_t farcooler_client_ceremony_offer(const char *name, const char *account,
                                       const char *key_a, const char *key_b,
                                       uint8_t *out, size_t capacity);

/*
 * Leg one, the scanning side: read a scanned offer.
 *
 * `expecting_account` is the account signed in on THIS device. An offer naming
 * another one answers {"error":"wrong_account"} — the rule that stops a trusted
 * device granting your fleet to a stranger's phone held up in front of its
 * camera. It is an argument because this call had no way to know who was asking,
 * so the rule was written in Swift instead: one copy, in one of three apps.
 *
 * NULL is the empty account, and matches only an offer that names none. There is
 * deliberately no "skip the check" value: an account that may be omitted is a
 * security rule an app can switch off by passing nothing.
 *
 * `held_ms` is how long ago THIS device scanned, by its own clock, which is the
 * only clock that counts — a timestamp inside a code is controlled by the
 * device displaying it. Call this again at the moment of the confirmation, not
 * only at the scan, so a sheet left open past the window refuses. Freshness is
 * judged before the account, so a code held too long says "stale" rather than
 * sending someone to look for an account problem they do not have.
 */
size_t farcooler_client_ceremony_scan(const char *encoded, const char *expecting_account,
                                      uint64_t held_ms, uint8_t *out, size_t capacity);

/*
 * Leg two, the trusted device's side: build the reply.
 *
 * `offer_json` is what farcooler_client_ceremony_scan returned. `runners_json`
 * is an array of runner records:
 *
 *     [{"id": "...", "label": "box", "alias": "box",
 *       "address": "box.tail-1234.ts.net", "user": "you", "port": 22,
 *       "host_key": "SHA256:...", "pending": false}]
 *
 * `budget_bytes` is what YOUR QR encoder reports as fitting at the
 * error-correction level you chose; 0 takes the library's conservative default.
 * The cap is measured bytes and never an assumed runner count. Over budget
 * answers {"error":"too_large"}, and the remedy is to grant the rest by running
 * the ceremony again — there is no second code to reassemble.
 */
size_t farcooler_client_ceremony_reply(const char *offer_json, const char *runners_json,
                                       size_t budget_bytes, uint8_t *out, size_t capacity);

/*
 * Leg two, the new device's side: take a scanned reply, or refuse it.
 *
 * `expecting_json` is the offer this device is still showing.
 * `already_taken` is its own record that this ceremony has been answered once —
 * one reply per ceremony, so a forged one cannot follow a real one.
 * `held_ms` is again this device's own elapsed time.
 *
 * The reply must echo the ceremony id, the account, the channel and the
 * fingerprint of the Key A it answers, so it can be consumed only by the device
 * that asked, for the ceremony it asked in.
 */
size_t farcooler_client_ceremony_accept(const char *encoded, const char *expecting_json,
                                        bool already_taken, uint64_t held_ms,
                                        uint8_t *out, size_t capacity);

/// The themes compiled into this build, as JSON. No session needed — a phone
/// that has never connected still has to render something. Writes into `out`
/// and returns the byte count; if `out` is NULL or too small, writes nothing
/// and returns the size needed.
size_t farcooler_client_builtin_themes(uint8_t *out, size_t capacity);

/**
 * Start streaming a terminal's live output.
 *
 * Chunks arrive through `farcooler_client_poll` as
 * `{"stream": "<terminal>", "chunk": "<base64>"}` — no ticket, because a stream
 * is not an answer to anything. `{"stream": ..., "ended": true}` closes it, and
 * `{"stream": ..., "error": "..."}` reports why it never started.
 *
 * A separate ssh channel, not the control connection: that connection answers
 * one request at a time, so a stream sharing it would sit behind every fleet
 * refresh. The latency floor is the network round trip.
 *
 * Returns false when the client has no ssh session to open a channel on.
 */
bool farcooler_client_stream_start(void *handle, const char *terminal);

/** Stop streaming. Safe when nothing is running for that terminal. */
void farcooler_client_stream_stop(void *handle, const char *terminal);

/*
 * Rewrite Far Cooler's fenced block in ~/.ssh/config.
 *
 * The end of enrollment on a desktop, and what makes the keys useful to anything
 * but Far Cooler: Zed, git, `scp` and every `ssh` anybody types reach a runner
 * through this file, or they do not reach it at all.
 *
 * `entries_json` is a JSON array of the lines that become the block, in order:
 *
 *     ["Host box", "  HostName box.tail-1234.ts.net", "  User you",
 *      "  Port 22", "  IdentityFile ~/.ssh/farcooler-macbook-air",
 *      "  IdentitiesOnly yes"]
 *
 * No newlines, no marker lines, no blank entries — each is refused, because one
 * entry in must never be two lines out. An empty array removes the block: a
 * machine with nothing enrolled should not carry two comment lines forever.
 *
 * Composing the block and choosing the aliases is the APP's — including checking
 * an alias against everything ssh already reads, which is what stops a runner
 * labeled `github.com` taking over every push. What is shared is the write.
 *
 * The block goes FIRST in the file, above any `Include` — ssh_config is
 * first-match-wins per keyword, so a block below an `Include ~/.ssh/config.d/*`
 * or a `Host *` is silently overridden and appending is the one placement that
 * reliably does nothing. An EXISTING block is rewritten where it already is,
 * because moving somebody's block would change which keywords win.
 *
 * Answers {"ok":true}, or a stable word — never a Rust error string:
 *
 *     damaged   the two marker lines are not one opening and one closing pair.
 *               Nothing was changed. This refuses rather than repairing: the way
 *               out is a person looking at the file, and a wrong guess about
 *               where the block ends rewrites lines Far Cooler did not write.
 *     missing   the directory that file lives in does not exist. A missing
 *               `.ssh` is NOT this — it is created at 0700, the mode sshd's
 *               StrictModes wants — so this means a path from a home directory
 *               that is not there. iOS and Android answer this always: there is
 *               no ~/.ssh on a phone, and no shell client to read one.
 *     io        the write did not happen. An entry carrying a newline or a
 *               marker, a file that is not UTF-8 or not a regular file, a
 *               directory another user can write, or a real I/O failure. Say
 *               that nothing was changed, not that a disk failed.
 *
 * The buffer contract is farcooler_client_generate_key's, with one addition this
 * call needs and the pure ones do not: WHEN IT CANNOT ANSWER IT DOES NOT WRITE
 * THE FILE EITHER. Passing NULL, or a capacity below the size it asks for,
 * touches nothing — so the "call, discover you were short, call again" dance
 * cannot perform the write twice. That matters because each write leaves a
 * checksummed backup beside the file, and a second write's backup would be a
 * copy of the first write's output rather than of the file you had. 64 bytes is
 * ample; the size asked for is a fixed maximum rather than the length of the
 * answer you are about to get.
 *
 * 0 means the library itself fell over — a panic caught at the boundary, which
 * would otherwise abort the app. Treat it as a refusal with no word: nothing was
 * written, and there is nothing in the buffer to parse.
 */
size_t farcooler_client_ssh_config_write(const char *path, const char *entries_json,
                                         uint8_t *out, size_t capacity);

/*
 * Paste a file into a terminal.
 *
 * The file is copied to the machine the terminal is on, written to a file the
 * daemon owns and expires, and its path is typed into the pane — which is how
 * an agent running there gets to look at it. Any type: an image, a PDF, a log.
 * Nothing about the transfer is ever written into the terminal; the path is the
 * only thing that reaches the pane.
 *
 * `name` is what the file was called where it came from, and may be empty. It
 * is sanitized on the far side but worth sending: a path ending in
 * `quarterly-report.pdf` tells the agent what it is looking at.
 *
 * Its own function rather than a `farcooler_client_call` method because the
 * payload is megabytes of binary, and the JSON boundary would mean base64 both
 * ways to describe something no client needs to look at.
 *
 * `data` is copied before this returns, so the caller may free it immediately.
 *
 * Progress arrives through `farcooler_client_poll` as
 * `{"ticket": 7, "progress": {"sent": 262144, "total": 1048576}}`, and the
 * answer once, as `{"ticket": 7, "ok": true, "result": {"path": "..."}}`.
 *
 * Returns a ticket, or 0 if the arguments could not be read.
 */
uint64_t farcooler_client_paste_file(void *handle, const char *terminal,
                                     const char *name, const char *mime,
                                     const uint8_t *data, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* FARCOOLER_CLIENT_H */
