# ov-1520: the chat files check at 320 wide

## Cause

`7e6ba7e4d` (ov-1510, the React shell). The check was wrong. The page wasn't.

- `chat-files-check.mjs` wrote its own copy of the old page: `.sidebar` beside
  `.pane`.
- `7e6ba7e4d` removed the `app.css` rules for it, including the 220 px sidebar
  width and the rule that stacked the two at 640 px and below.
- At 320 the sidebar then took the width of its text, and the chat got the
  rest. That width depends on the machine's fonts.
- In a narrow column the PDF's long name wraps to many lines. On Linux that
  evidently put the first image more than 300 px above the screen, where lazy
  loading doesn't fetch it, so the check's wait for it timed out.

On macOS the old page left a 209 px column and the check passed. Setting that
sidebar to 140 px reproduced CI's message exactly, with nothing fetched for the
first image. The Linux width itself wasn't measured.

## The signed-in page at 320

The real `index.html` and shell stack into one column. The image is drawn
294 px wide and opens by tap, and the file row's Download saves the file.

## Fix

- The check now serves `index.html` under the React shell
  (`test/browser/chat-files-harness.ts`).
- At 320 it also requires the image to be at least 272 px wide, and a tap on
  Download to save the file's bytes. No wait or timeout changed.
- With the chat held to 140 px, the check fails with CI's message.

## Failures behind it

CI stops a job at its first failing step, so the later browser checks didn't
run on `022667bfb`.

The workday walk (`csp-check.mjs --workday-runner`) called `selectOption` on
Switch Runner, which `7e6ba7e4d` had made a picker. ov-1499 fixed that on the
landing base, so this branch's copy of the fix was dropped in the rebase.

## The rebase onto 8d8a2f2b7

`chat-files-check.mjs` conflicted. It keeps both changes: ov-1499's web core
wasm module and policy, and this ticket's React shell page and 320 assertions.

After the rebase, the workday walk failed twice at its last step: no Landing
"Checking Here" on the Plan. Here the page was wrong. `29af4d78b` and
`447b9d461` (ov-1412) are on the base.

- The walk's runner has no workflow service, so it refuses `workflow.get`. The
  client then shows the plan without a workflow. A read-only client that's
  refused takes the same path.
- The plan still lists the engine's landing, as number 0.
- ov-1412 stopped drawing a card for number 0 and left the row to the workflow.
  Without one, the Plan showed no landing.

The shared Plan view (`plan_view/landing_queue.rs`) now draws those entries as
rows when there's no workflow. With a workflow, nothing changes. A new test
there failed before the fix. This is shared by every app, not only the web.

## Checks run on macOS, on the fix commit's code

- `chat-check`, `chat-files-check`, `plan-render-check`, `shell-check`,
  `work-check`, `board-check`, `csp-check` with the fake daemon, `sleep-check`
  and the workday walk passed.
- The stable and canary builds and their size gates passed.
- Both TypeScript checks, the web unit tests (268) and Needs You tests (50)
  passed. The size gate's self-test passed before the presenter fix.
- The `farcooler-client` library tests (427 passed, 2 ignored), its Clippy,
  the `crates/web` tests and `lane-gate.py --quick-only` passed.
- The Swift and Android tests weren't run.

The browser checks ran with the pane's `FARCOOLER_*` variables unset on the
last run.

Not yet seen: the `Web` and `Web bundles and browser checks` jobs on main.
They run after this lands. `chat-check.mjs` still writes its own copy of the
old page. It passes, and this ticket didn't change it.
