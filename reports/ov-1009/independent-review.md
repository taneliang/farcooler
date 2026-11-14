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
