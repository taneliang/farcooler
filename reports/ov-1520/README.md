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
- With the narrow side-by-side column forced back, it fails with CI's message.
  With a much wider font, it passes.

## A second failure behind it

CI stops a job at its first failing step, so the later browser checks never ran
on `022667bfb`. The workday walk (`csp-check.mjs --workday-runner`) failed too:
it called `selectOption` on Switch Runner, which `7e6ba7e4d` had made a picker.
It now opens the picker and chooses the runner.

## Checks run on macOS

`chat-check`, `chat-files-check`, `plan-render-check`, `shell-check`,
`work-check`, `board-check`, `csp-check` with the fake daemon,
`sleep-check`, and the workday walk all passed. So did the stable and canary
builds, their size gates and the size gate's self-test.

The first `csp-check` run failed one timing limit (keystroke echo p95
106.8 ms, limit 100 ms) with the load average near 20. The rerun measured
56.9 ms and passed.

Not yet seen: the `Web` and `Web bundles and browser checks` jobs on main.
They run after this lands. `chat-check.mjs` still writes its own copy of the
old page. It passes, and this ticket didn't change it.
