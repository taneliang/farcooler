/*
 * Overnight client core — the C ABI.
 *
 * The whole of "talk to an Overnight host", for clients that are not the host:
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
 * by overnight_client_poll belongs to the handle and is valid until the next
 * call on it — copy anything you keep.
 *
 * Every function tolerates a NULL handle. A UI bug must not take down the app.
 */

#ifndef OVERNIGHT_CLIENT_H
#define OVERNIGHT_CLIENT_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Create a client. Free with overnight_client_free. NULL if a runtime could
 * not be started. */
void *overnight_client_new(void);

/* Destroy a client, ending any SSH session it holds. Safe with NULL. */
void overnight_client_free(void *handle);

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
uint64_t overnight_client_connect(void *handle, const char *config);

/*
 * Invoke a method. `args` is JSON; `{}` if it takes none.
 *
 *   fleet                  {}                     -> workspaces with derived state
 *   repositories           {}                     -> registered repositories
 *   workspace.create       {repository, task, branch, base?}
 *   workspace.archive      {workspace}
 *   workspace.restore      {workspace}
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
uint64_t overnight_client_call(void *handle, const char *method, const char *args);

/*
 * Take the oldest finished result, or NULL if none is ready.
 *
 * Owned by the handle, valid until the next call on it. Each result is returned
 * exactly once.
 */
const char *overnight_client_poll(void *handle);

/* True once a session is established. */
bool overnight_client_connected(void *handle);

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
 * means "overnight".
 *
 * Returns the number of bytes NEEDED. If that exceeds `capacity` nothing is
 * written — a truncated private key would be worse than none — so call again
 * with a larger buffer. 4096 is ample.
 */
size_t overnight_client_generate_key(const char *comment, uint8_t *out, size_t capacity);

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
size_t overnight_client_public_key(const char *private_key, uint8_t *out, size_t capacity);

#ifdef __cplusplus
}
#endif

#endif /* OVERNIGHT_CLIENT_H */
