# ov-1458 · Browse worktree files and review changes on the Web

Open a Board ticket with a linked worktree and choose **Files** or **Changes**. Files lists runner-authorized directories and previews text. Changes shows the committed candidate and an uncommitted comparison. Select a changed file to see its patch, or a line number to post a comment. The runner stores inline comments on the ticket, so another browser reviewing the same candidate sees them.

Rust binds committed reviews to the worktree and the compared Git objects. The uncommitted comparison also uses the worktree digest. Rust resolves file and line anchors, projects reviewed markers, and checks the candidate again before a comment or approval. A replacement candidate keeps the file selection when that path still exists and clears the old line selection and reviewed markers. The browser stores reviewed marks for the tab and restores the file, diff page, selected line and scroll offset on reload.

Source reads require the runner’s existing control grant. The review offers the workflow’s **Approve** or **Accept review** action when the authenticated principal can act on the displayed commit. Approval uses the existing workflow preparation, retained-command journal and runner validation.

## Browser evidence

The browser check runs the production ticket and review renderers against an isolated authenticated runner session with a real Git checkout. The fixture uses the [parity inventory](../ov-1439/parity.md) and [Web board report](../ov-1446/README.md). It checks folder navigation, a lengthy text preview, diff selection, shared inline comments, reviewed marks and scroll restoration on reload. It then moves HEAD, checks that the old approval is refused, and restores and approves the exact workflow candidate.

| View | Phone | Desktop |
| --- | --- | --- |
| File preview | [320 × 900](files-320.png) | [1280 × 900](files-1280.png) |
| Restored inline review | [320 × 900](changes-320.png) | [1280 × 900](changes-1280.png) |

The Rust client suite covers stable anchors, current and stale candidates, read-scope refusal and rooted file reads. The daemon diff suite checks that a pinned patch stays on its candidate after HEAD moves. The Web Rust, Web unit and workflow suites, Board and Work browser checks, both TypeScript checks, WASM build, and Canary wire and shared-core linters passed.

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
