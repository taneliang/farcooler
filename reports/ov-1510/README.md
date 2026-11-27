# ov-1510: React shell on the new design system

The signed-in web app now renders its shell with React 19.2, Base UI 1.9 and
React Router 7.18 (as a library), bundled by Vite 7.3. The shell is the
sidebar, the page header and the orchestrator pane beside the page. Board,
plan, chat and terminals stay DOM code, mounted into the shell through
`LegacyView` (`mount`, `update`, `dispose`). The shell renders `ShellStore`,
a view model that `app.ts` and `work-screen.ts` write, so feature logic stays
in the Rust core.

The ov-1483 tokens in `services/web/src/client/tokens.css` are the one source
of type scale, light and dark colors, spacing and corners. The old palette
names (`--bg`, `--card`, `--button-bg` and the rest) now point at the tokens,
so the views pick them up unchanged.

## 1. Size, build time and CSP

The size gate is `node scripts/size-gate.mjs dist`, with a limit of
5,500,000 B. "Before" is main at 29c41d4de, built from a clean copy in
scratch.

| | Before | After | Change |
|---|---:|---:|---:|
| Size gate, whole build (Brotli) | 5,095,327 B | 5,251,039 B | +155,712 B (+3.1%) |
| `app.js` raw | 848,078 B | 1,203,332 B | +355,254 B |
| `app.js` Brotli q11 | 197,671 B | 299,007 B | +101,336 B |
| `app.css` raw | 51,827 B | 63,783 B | +11,956 B |
| `app.css` Brotli q11 | 9,650 B | 11,998 B | +2,348 B |
| Icons and manifest (Brotli) | — | 51,841 B | new |

- **Headroom.** 248,961 B remains under the gate.
- **The script.** It is still one chunk, and the build asserts that, so a
  deploy can never strand a lazy chunk. React, React DOM, Base UI and the
  router account for the JS growth.
- **Build time.** Unchanged within noise. Load average was 23–26 throughout,
  and alternating pairs went both ways:

  | Run | Before | After |
  |---|---:|---:|
  | Pair 1 | 112.8 s | 80.2 s |
  | Pair 2 | 113.8 s | 130.1 s |

  Brotli of the Tailcat wasm dominates the build; the Vite step is a few
  seconds of it.
- **CSP.** `wrangler.toml` and `src/headers.ts` are unchanged, so the policy
  and Trusted Types are exactly as before. `src/assets.ts` only adds the
  `webp` and `webmanifest` content types for hashed files.
  - Base UI runs under `CSPProvider disableStyleElements`, and every script
    loads from `/a/`.
  - `csp-check.mjs --fake-daemon … --tunnel-delay 100` (the CI walk) passed
    end to end: "no CSP violations".

## 2. Every view in the shell

Each view was captured at 320, 768 and 1280 px, in light and dark:

- `shell/`: from `shell-check.mjs`, with long real names for the runner, the
  workspaces, the repository, the terminals and the account.
- `views/`: from the `csp-check.mjs` walk, which shows the views' real content
  over the fake daemon.

| View | Long names (shell chrome) | Real content |
|---|---|---|
| Terminals | [light 320](shell/shell-terminals-light-320.png) · [768](shell/shell-terminals-light-768.png) · [1280](shell/shell-terminals-light-1280.png) · [dark 320](shell/shell-terminals-dark-320.png) · [768](shell/shell-terminals-dark-768.png) · [1280](shell/shell-terminals-dark-1280.png) | terminal: [light 320](views/terminal-light-320.png) · [768](views/terminal-light-768.png) · [1280](views/terminal-light-1280.png) · [dark 320](views/terminal-dark-320.png) · [768](views/terminal-dark-768.png) · [1280](views/terminal-dark-1280.png) |
| Orchestrator chat | (same shell) | [light 320](views/chat-light-320.png) · [768](views/chat-light-768.png) · [1280](views/chat-light-1280.png) · [dark 320](views/chat-dark-320.png) · [768](views/chat-dark-768.png) · [1280](views/chat-dark-1280.png) |
| Needs You | [light 320](shell/shell-needs-light-320.png) · [768](shell/shell-needs-light-768.png) · [1280](shell/shell-needs-light-1280.png) · [dark 320](shell/shell-needs-dark-320.png) · [768](shell/shell-needs-dark-768.png) · [1280](shell/shell-needs-dark-1280.png) | [light 320](views/needs-light-320.png) · [768](views/needs-light-768.png) · [1280](views/needs-light-1280.png) · [dark 320](views/needs-dark-320.png) · [768](views/needs-dark-768.png) · [1280](views/needs-dark-1280.png) |
| Board | [light 320](shell/shell-board-light-320.png) · [768](shell/shell-board-light-768.png) · [1280](shell/shell-board-light-1280.png) · [dark 320](shell/shell-board-dark-320.png) · [768](shell/shell-board-dark-768.png) · [1280](shell/shell-board-dark-1280.png) | [light 320](views/board-light-320.png) · [768](views/board-light-768.png) · [1280](views/board-light-1280.png) · [dark 320](views/board-dark-320.png) · [768](views/board-dark-768.png) · [1280](views/board-dark-1280.png) |
| Ticket detail | (same shell) | [light 320](views/detail-light-320.png) · [768](views/detail-light-768.png) · [1280](views/detail-light-1280.png) · [dark 320](views/detail-dark-320.png) · [768](views/detail-dark-768.png) · [1280](views/detail-dark-1280.png) |
| Plan | [light 320](shell/shell-plan-light-320.png) · [768](shell/shell-plan-light-768.png) · [1280](shell/shell-plan-light-1280.png) · [dark 320](shell/shell-plan-dark-320.png) · [768](shell/shell-plan-dark-768.png) · [1280](shell/shell-plan-dark-1280.png) | [light 320](views/plan-light-320.png) · [768](views/plan-light-768.png) · [1280](views/plan-light-1280.png) · [dark 320](views/plan-dark-320.png) · [768](views/plan-dark-768.png) · [1280](views/plan-dark-1280.png) |

The walk also captures the screens around sign-in in both themes at the same
widths. They are not copied here, to keep the repository small. Run it with
`--screenshots DIR` to get them. The screens are:

- every runner, an unreachable runner, passkey, enroll and restore;
- join and connect, and the terminal.

**Layout.**

| Width | Layout |
|---|---|
| 1080 px and up | sidebar, page, orchestrator pane |
| 681–1079 px | sidebar and page; the pane sits under the page |
| 680 px and under | one column: the sidebar is a compact header, the views a 2×2 grid; the page and pane are each a screen tall |

Every control is at least 44 px tall at phone width. The audited pages have
no sideways scroll at any of the three widths.

**At most one filled button.**

- A plain button is now outlined. `.primary` is the one filled accent button.
- `.prominent` (an accent outline) marks the main choice in a list, such as
  Allow and PDF in Needs You, so a list never shows a row of filled buttons.
- The page header's Start Work is the filled action on Terminals and Board.
  It drops to outlined while a lost terminal or link offers recovery, which
  then carries the fill.
- `filledButtons()` (`test/browser/shell-audit.mjs`) counts visible
  accent-filled controls:
  - `shell-check` asserts the exact list on every page, width and theme;
  - `board-check` asserts it at 320, 641 and 1280 px;
  - the `csp-check` walk fails any screen it captures that shows more than
    one. With `--screenshots`, the walk also checks each screen at every
    width and in both themes.

**Contrast (WCAG AA).** `shell-check` measures these pairs from the computed
token colors, in both themes, and fails under the minimum. Text needs 4.5:1;
control edges and the focus ring need 3:1.

| Pair | Light | Dark |
|---|---:|---:|
| Main text / canvas, card or rail | 13.43:1 | 13.49:1 |
| Muted text / canvas, card or rail | 5.65:1 | 7.61:1 |
| Primary action label / fill | 7.01:1 | 9.22:1 |
| Primary action label / hover fill | 9.36:1 | 11.11:1 |
| Link and accent text / canvas, card or rail | 5.98:1 | 8.88:1 |
| Selected text / selection | 7.99:1 | 9.19:1 |
| Connected text / status surface | 6.16:1 | 8.48:1 |
| Lost-session text / status surface | 6.04:1 | 8.36:1 |
| Danger text / canvas, card or rail | 6.28:1 | 9.05:1 |
| Control edge / canvas, card or rail | 3.55:1 | 4.40:1 |
| Focus ring / canvas, card, rail or selection | 5.83:1 | 6.72:1 |
| Terminal text / terminal surface | 14.07:1 | 14.07:1 |

Each row is the worst case over the surfaces listed.

## 3. App icon

`services/web/src/client/icons/app-icon-1024.png` is the master, copied from
the Mac and iOS app icon, so it survives the removal of `apps/` (ov-1489).
The build hashes the sizes made from it into `/a/`.

| Use | File |
|---|---|
| Favicon | 32 px PNG |
| Sidebar mark | 64 px PNG |
| Apple touch icon | 180 px PNG |
| Manifest | 192 px PNG and 512 px WebP, in `app.webmanifest` ("Far Cooler", standalone, the canvas color) |

## 4. Tests

All of these ran on this branch, in the foreground:

| Check | Result |
|---|---|
| `npm test` (vitest) | 30 files, 259 tests passed, including the new `shell-store.test.ts` and asset types |
| `vitest.needs.config.ts` (the real Rust bridge) | 3 files, 50 tests passed |
| `tsc` for both configs | clean |
| `shell-check.mjs` (new) | passed 5 times in a row |
| `work-check.mjs` | 18 of 18 passed |
| `board-check.mjs` | passed |
| `chat-check.mjs` | passed |
| `plan-render-check.mjs` | passed |
| `review-check.mjs` (with the `web_review_browser_bridge` fixture) | passed |
| `needs-capture`, `plan-capture`, `workflow-cancelled-capture` | passed, no overflow |
| `csp-check.mjs --fake-daemon … --tunnel-delay 100` | passed, no CSP violations |
| `sleep-check.mjs --fake-daemon …` | every case passed in Chromium (CI's engine) and WebKit; Firefox won't start on this machine, even on a blank page |
| `copy-lint`, `doc-comment-check`, `web-client-tree-check`, `actionlint`, `file-size-budget` | passed |

`shell-check.mjs` now runs in CI's web job. It covers keyboard use of the
shell:

- **Tab order.** Tab walks the page in document order, and every stop shows
  a focus ring: runner picker, Add Runner, workspace picker, the four views,
  Orchestrator, Workflow Runs, Commands, Workspace Link, each terminal, Start
  Work, the page's content, then the account (attention count, Saved Copies,
  Sign Out).
- **Pickers.** Both open from the keyboard, move and choose with the arrow
  keys and Enter, and return focus to their trigger. Escape closes one without
  choosing. A runner this browser can't use is disabled and can't be chosen.
- **Dialogs.** Start Work and Commands take focus when they open, and Escape
  gives it back to the control that opened them.

Each check was mutated to make sure it can fail:

- removing every focus ring fails the order test;
- making a second button filled fails all six audits.

The old checks changed where the markup did:

- the Base UI picker in place of the `<select>`;
- links in place of buttons for the views;
- terminals in the sidebar;
- the board's own heading hidden under the shell's page title.

Four capture scripts spliced the bundle into HTML with `String.replace`. With
React in the bundle, its `$&` sequences were read as replacement patterns, so
the scripts now insert the bundle with a replacer function.

## 5. Typing and scrolling

`test/browser/measure.mjs` times each key or wheel event from its own
timestamp to a task queued from the next animation frame: the frame showing
its effect has been produced by then. It also records every frame's length
while scrolling. The same code ran against both builds:

- `csp-check.mjs --workstation-only --measure` covers the orchestrator chat
  composer and the main terminal, with 80 lines of output;
- `chat-check.mjs --measure` covers the long conversation.

Runs alternated before and after, five each, in headless Chromium on the same
machine. Each value is the median over the five runs, in ms:

| Measure (samples per run) | Before median | After median | Before p95 | After p95 | Before max | After max |
|---|---:|---:|---:|---:|---:|---:|
| Terminal: keydown to frame (54) | 6.9 | 7.8 | 16.5 | 16.2 | 17.2 | 18.1 |
| Chat composer: keydown to frame (108) | 9.1 | 9.4 | 16.1 | 16.4 | 17.0 | 17.2 |
| Terminal scroll: wheel to frame (60) | 31.9 | 31.7 | 33.3 | 40.4 | 48.7 | 47.7 |
| Terminal scroll: frame length (~197) | 16.7 | 16.7 | 16.7 | 16.7 | 16.8 | 16.8 |
| Chat scroll: wheel to frame (60) | 30.7 | 30.7 | 37.8 | 37.8 | | |
| Chat scroll: frame length | | | 16.8 | 16.8 | 33.3 | 33.3 |

**Typing.** No slower within the noise.

- Terminal medians ranged 5.8–9.0 ms before and 4.7–8.4 ms after.
- Chat medians ranged 8.3–9.4 ms before and 9.0–9.9 ms after. That is about
  0.3 ms later, under a fiftieth of a frame. Every p95 is within one 60 Hz
  frame in both builds.

**Scrolling.** Unchanged.

- Frames stay at 16.7 ms throughout.
- Wheel-to-frame medians match. The p95 moves by one frame in either
  direction from run to run: 48.8 ms in one before run, and 44.5 ms in one
  after run.

The shell re-renders only when its view model changes. A write that changes
nothing notifies no one, so terminal output and chat rows never re-render
React.

## Decisions taken without asking

- **Terminal surface.** The terminal stays dark in both themes, as the tokens
  define it. A pane showing an agent's conversation takes the page surface
  instead, so no dark frame shows around the chat in light.
- **Page titles.** The board's own "Board" heading is hidden: the shell's
  page header already names the view, and two `h1`s said the same thing.
- **Legacy colors.** Literal colors left in the legacy stylesheets now use
  status tokens:
  - chat.css's light-mode status colors;
  - the review diff's added and removed tints.
