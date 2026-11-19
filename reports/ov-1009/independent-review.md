# ov-1009 independent review

Reviewed by a separate agent after implementation and again after fixes.
Final verdict: no remaining blockers. No landing or deployment performed.

## Findings resolved

- Remember authenticated shared runners locally so a later missing/junk-copy
  repair preserves every runner the browser has learned. Existing local host
  pins and tokens are never replaced simply by reading another browser's copy.
- Track local runner baselines so unrelated additions from a stale browser do
  not replace a newer shared grant. An old browser without a baseline refuses
  ambiguous conflicting grants instead of overwriting them.
- Capture each save's local snapshot before I/O. An enrollment arriving while
  its POST is pending remains dirty until a later save includes it.
- Use nonredirecting relay calls for copy deletion, so an expired session (401)
  reaches the Sign Out warning instead of navigating away silently.
- A fresh independent review found that a copy read/save already pending when
  Sign Out starts could still redirect on a 401. Copy synchronization, deletion,
  and the saved-copy UI now share the nonredirecting `relayCopy` helper. The
  reviewer checked the fix and reported no remaining blockers.

The reviewer verified atomic account-scoped ciphertext comparison, authenticated
runner imports, safe text rendering, pending-save-before-delete ordering,
bounded failure paths, and explicit retry / Sign Out Anyway behavior.

## Validation

- Full web Vitest suite: 301 tests passed on the final follow-up. The new
  regression starts a copy read, begins Sign Out, then expires the requests
  with 401 responses. It checks that no navigation hides the delete warning.
  With redirecting copy calls it failed (two `location.assign` calls); with
  the fix it passed.
- Full relay Vitest suite: 839 tests passed, including conditional creation,
  stale updates, deleted-copy races, capacity, and account isolation.
- Both web TypeScript configurations passed; workflow controller/session tests
  passed through `npm run test:workflow`.
- Local Chrome at 375px and 1280px: open/list, failed deletion, retry, clear,
  close, and no horizontal overflow. Relay responses were simulated.

The installed workerd falls back from compatibility date 2026-01-15 to its
supported 2025-09-06; both suites passed with that existing toolchain limitation.

## Compatibility limit

Older clients that omit `expected` retain unconditional writes. The new clients
compare the exact previous ciphertext and retry conflicts; older clients must
refresh to receive that protection. No migration or new authenticated route is
introduced.

## R-112 send-back review (2026-11-15)

Rebased onto origin/main and moved recovery presentation into
`crates/client/src/recovery_view.rs`. Saved Copies gets count and passkey
labels through the WASM bridge. Sign Out uses a bundled snapshot of that
shared model, checked for exact parity by a Rust test, so failed or stalled
WASM loading cannot prevent emergency sign-out cleanup. The baseline was
not changed and no platform-copy exemptions were added.

A separate agent reviewed the complete feature diff and the ratchet fix. It
identified an initial dependency of sign-out cleanup on WASM loading; that
was removed before the final review. The reviewer then found no remaining
security or regression blockers, checking account-scoped CAS, authenticated
merge, local pin preservation, safe text sinks, and nonredirecting copy calls.
Saved Copies also has a regression test for a closeable module-load failure.

Fresh validation after rebase:

- Shared-core lint passes; `--baseline-not-grown origin/main` passes.
- Foreground full web suite: 302 tests pass.
- Foreground full relay suite: 839 tests pass.
- Both web TypeScript configurations and workflow tests pass.
- `cargo check --manifest-path crates/web/Cargo.toml --locked` passes.
- `TMPDIR=/private/tmp cargo test -p farcooler-client recovery_view --lib`:
  both count/identity and bundled-view parity tests pass.

## ov-1377 carry review (2026-11-19)

The pinned base already includes the ov-1009 implementation and shared
recovery view. The independent reviewer verified that alternating touches from
two browsers keep the same readable copy, that raced enrollments retain every
runner, and that account-scoped listing and deletion let the owner clear junk
copies.

The reviewer found that a late connection update could replace the Sign Out
warning and that an unrelated in-flight relay request could redirect away on
401. Sign Out now keeps Retry Delete and Sign Out Anyway in its own dialog.
The relay client checks whether to redirect when the response arrives.
Saved Copies opens before waiting for its presenter, so its dialog cannot
appear over the Sign Out warning after a late module load. The independent
follow-up found no remaining blocker.

The web suites passed 255 worker tests and 56 Rust-backed adapter tests.
The relay suite passed 841 tests. The Rust web suite and the shared recovery
model tests passed. Web and relay type checks, web workflow tests, the web
dependency check, and the shared-core baseline check passed.
