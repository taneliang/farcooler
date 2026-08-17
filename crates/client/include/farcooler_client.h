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
 *   workspace.create       {repository, task, branch, base?}
 *   workspace.hide         {workspace}
 *   workspace.unhide       {workspace}
 *   terminal.create        {workspace, title, preset}
 *   terminal.stop          {terminal}
 *   terminal.restart       {terminal}
 *   terminal.dismiss_lost  {terminal}
 *   terminal.resize        {terminal, columns, rows}
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
 * `held_ms` is how long ago THIS device scanned, by its own clock, which is the
 * only clock that counts — a timestamp inside a code is controlled by the
 * device displaying it. Call this again at the moment of the confirmation, not
 * only at the scan, so a sheet left open past the window refuses.
 */
size_t farcooler_client_ceremony_scan(const char *encoded, uint64_t held_ms,
                                      uint8_t *out, size_t capacity);

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
