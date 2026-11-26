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

The ticket was rebased onto `b74589a669753bf050362dac8f402306c064aa62`. Both the base and the ticket had reduced the size of the Rust client session. The resolved budget gives the rebased 1,912-line file the standard 50 lines of headroom, lowering its ceiling from 2,086 to 1,962 lines.

The review browser scenario passed at 320 and 1280 pixels against the authenticated runner fixture. The captures above show the long file preview and the restored inline comment after reload. The Rust client passed 415 tests, including projection and socket coverage for stable anchors, rename diffs, truncated diffs, scope refusal, and stale candidates. Web Rust passed 83 tests, including passkey sign-in against a local SSH server. The 95 targeted daemon tests passed across change sets, file diffs, review operations, Git isolation, Git LFS, and native messages.

The Board, Work, and Plan browser checks passed, along with 305 Web unit tests, workflow tests, both TypeScript checks, Tailcat Go WASM tests, and the file-size, daemon-layer, shared-core, and Canary protocol checks. Both Web bundles passed the size gate at 5,037,775 Brotli bytes for Canary and 5,036,136 for Stable.

The Canary bundle passed the Chromium security and split-pane reconnect checks on rerun. The first runs stalled during teammate enrollment and missed terminal input after a simulated ten-minute background period with a dead tunnel.

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
