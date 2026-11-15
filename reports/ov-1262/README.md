# ov-1262: passkey copy lands on the new main

Carries `ov-1009-passkey-one-copy` onto main (99b2a020f).

## What was carried

`git rev-list --reverse --no-merges origin/main..ov-1009-passkey-one-copy` lists four commits. Two were already on main:

- `65317555e` and `cc6dd7ffc` (preserve shared passkey recovery copies; keep sign-out warnings visible) are already in main's `87943bba1 wave A preview: ov-1009`.
- Cherry-picking them directly gave add/add conflicts in six files. Their content on main matches `cc6dd7ffc` for every touched file except `app.css`, `app.ts` and `saved-copies.ts`, where main has since added `// platform copy:` markers. Nothing was lost, so both commits were skipped.

Carried with `-x`:

- `bd2b7e0e4` (present recovery copy controls from shared client)
- `d5f99db85` (record ratchet fix review and validation)

No merge commits.

## Conflicts

Both in `bd2b7e0e4`:

- `services/web/src/client/app.ts` (sign-out failure copy, two hunks): main had literal strings with `// platform copy:` markers. ov-1009 replaces them with `words.deleteTitle`, `words.deleteDetail`, `words.retry` and `words.anyway` from the shared client. Resolved to ov-1009's side. The literals are gone, so the exemptions go with them.
- `services/web/src/client/saved-copies.ts` (Saved Copies dialog, ten hunks): same pattern for the heading, explanation, loading, failure, summary, passkey label, Clear Copy and cleared messages. Resolved to ov-1009's shared-client words. Main's other changes in these files are kept.
- `services/web/test/saved-copies.test.ts` merged without conflict.

No new behavior was added.

## ov-1009's acceptance

Still holds. Recovery copy text comes from the shared client (`crates/client/src/recovery_view.rs`). The web copy shows no new shared-core-lint entries, and the baseline is unchanged.

## Checks (all foreground)

- shared-core-lint: nothing new; baseline not raised
- copy-lint, file-size-budget: pass
- `tsc --noEmit`: clean; web tests: 31 files, 302 tests pass
- lane-gate `wire,doc-comments,web`: every step that ran passed. Steps that need the Linux x86 wasm toolchain were skipped and are left to CI.
