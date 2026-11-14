# PR description: quarantine names resolve against the real test inventory (ov-1237)

## What changes

`scripts/quarantine.py run`, the command CI's Rust job and the lane gate use, now lists the workspace with
`cargo nextest list --workspace --message-format json` and refuses to run when a quarantine entry names a test that is not in
that list. The no-build `--lint` judges only form, duplicates and expiry, so it never rejects a valid inventory-only entry.
`--source` keeps the old source scan as a documented fallback. `ci.yml` and the lane-gate expansion are unchanged.

## Ways to declare a Rust test, and what the resolver does

Resolution uses nextest's own inventory, so anything nextest runs is accepted and anything it does not run is rejected.

| Declaration | Real inventory (`run`, `--inventory`, `--nextest`) | `--source` fallback |
|---|---|---|
| Inline `mod x { #[test] fn t() {} }` | Handled | Handled |
| File module (`mod x;`) | Handled | Handled |
| `#[path = "..."]` module | Handled | Handled |
| `#[path]` module inside an inline module | Handled | Not handled |
| Integration tests (`tests/*.rs`) | Handled | Handled |
| Custom `[[test]]` / `[[bin]]` targets | Handled | Not handled for `[[test]]` paths |
| Macro-generated tests | Handled | Not handled (macros are not expanded) |
| Doctests | Excluded: not in nextest's inventory, so they cannot be quarantined | Excluded |

## Tests

- `python3 scripts/quarantine-test.py`: all self-tests, including a `#[path]`-in-inline-mod entry accepted through `run` and an
  unknown name rejected.
- `HEAVY_LOCK_HELD=1 ./scripts/lane-gate.py --self-test`.
