# ov-1458 · Browse worktree files and review changes on the Web

Open a Board ticket with a linked worktree and choose **Files** or **Changes**. Files lists runner-authorized directories and previews text. Changes shows the committed candidate and an uncommitted comparison. Select a changed file to see its patch, or a line number to post a comment. The runner stores inline comments on the ticket, so another browser reviewing the same candidate sees them.

Rust binds committed reviews to the worktree and the compared Git objects. The uncommitted comparison also uses the worktree digest. Rust resolves file and line anchors, projects reviewed markers, and checks the candidate again before a comment or approval. A replacement candidate keeps the file selection when that path still exists and clears the old line selection and reviewed markers. The browser stores reviewed marks for the tab and restores the file, diff page, selected line and scroll offset on reload.

Rust requires the full diff in one view to mark a file reviewed. Changes explains when a patch exceeds the byte limit or a hunk exceeds the line limit. Partial pages remain readable where the runner returns a forward cursor. File marks are separate from the existing workflow approval.

Source reads require the runner’s existing control grant. The review offers the workflow’s **Approve** or **Accept review** action when the authenticated principal can act on the displayed commit. Approval uses the existing workflow preparation, retained-command journal and runner validation.

## Browser evidence

The browser check runs the production ticket and review renderers against an isolated authenticated runner session with a real Git checkout. The fixture uses the [parity inventory](../ov-1439/parity.md) and [Web board report](../ov-1446/README.md). It checks folder navigation, a lengthy text preview, diff selection, shared inline comments, reviewed marks and scroll restoration on reload. It checks that byte-capped patches and line-capped hunks display the limit and offer neither a reviewed mark nor a next page. It then moves HEAD, checks that the old approval is refused, and restores and approves the exact workflow candidate.

| View | Phone | Desktop |
| --- | --- | --- |
| File preview | [320 × 900](files-320.png) | [1280 × 900](files-1280.png) |
| Restored inline review | [320 × 900](changes-320.png) | [1280 × 900](changes-1280.png) |

The Rust client suite covers stable anchors, current and stale candidates, read-scope refusal and rooted file reads. Against Git-generated patches, it also checks that both caps reject direct mark requests, including requests with a later hunk offset. The daemon diff suite checks that a pinned patch stays on its candidate after HEAD moves. The Web Rust, Web unit and workflow suites, Board and Work browser checks, both TypeScript checks, WASM build, and Canary wire and shared-core linters passed.

The socket review tests exercise pure and edited renames from a real Git candidate. A pure rename has no text hunks. An edited rename shows the original and replacement lines with their respective line numbers, and a comment on the original line reopens at the same anchor. The fixture includes other renames with similar names and nested paths to check that each patch stays on its selected file.

## Landing-base verification

All four ticket commits replayed onto `6570951376c01a51bb730b3fc528bc2dae9af094` with identical patches. The replay applied cleanly at both paths named in the handoff, `crates/daemon/src/workflow_runtime/native_messages.rs` and `services/web/src/client/wasm.d.ts`.

The review browser scenario passed again at 320 and 1280 pixels with the authenticated runner fixture. The captures above were verified against that run. The Rust client passed 415 tests, and Web Rust passed 83, including real-sshd passkey tests. All 47 targeted daemon tests passed across file diffs, change sets, review operations and native messages.

The Board, Work and Plan browser checks, 305 Web unit tests, workflow tests, both TypeScript checks, Tailcat Go WASM tests, and repository linters passed. The Canary Web bundle passed the size gate at 5,038,241 Brotli bytes.

## Reproduce the review browser check

To rerun the browser check with the Web dependencies and Playwright Chromium installed, start the fixture from the repository root.

```sh
export CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"
export REVIEW_BROWSER_BRIDGE="$FARCOOLER_SCRATCH/review-browser"
mkdir -p "$REVIEW_BROWSER_BRIDGE"
cargo test -p farcooler-client --lib web_review_browser_bridge -- --ignored
```

Once the fixture writes `ready.json`, run the browser check from another shell.

```sh
export REVIEW_BROWSER_BRIDGE="$FARCOOLER_SCRATCH/review-browser"
cd services/web
node test/browser/review-check.mjs ../../reports/ov-1458
```
