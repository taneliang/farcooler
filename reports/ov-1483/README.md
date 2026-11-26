# Web design system options · ov-1483

**Owner decision.** Approve client-rendered React with Base UI, Vite, and React Router v7 in library mode as the end state, or choose one of the two smaller commitments below. I recommend the React end state and a shell-only first release. The spike exercises the strict CSP, the controls that prompted the ticket, and xterm in the same screen. It leaves the runner and authentication code available for reuse.

## What changes on screen

The Mac [plan column](../../services/site/public/docs/img/mac-plan-column-dark.webp) and [WorkspaceStyle](../../apps/macos/Sources/FarCooler/Theme.swift) set the quality bar. The useful cues are distinct type roles, quiet dividers, densely spaced but legible rows, and pane headers that give the work an identity. The web design takes those cues into a responsive layout with larger hit areas, neutral surfaces, and one filled action per active view.

| Nov 27 feedback | Proposed treatment |
|---|---|
| Several choices read “Main” | Use the workspace name in the picker. Keep repository identity in the breadcrumb and runner identity beside the terminal. If two workspace names collide, add the repository or runner as a disambiguator. |
| The section selector and action stack compete | Use text and a quiet selection tint for navigation. Keep commands and workflow runs accessible in production through secondary affordances. |
| Four equally prominent buttons | Give the board one filled action, **Start work**. Its dialog has one submit action. Workspace changes and opening the full pane are secondary. |
| Six indistinguishable orchestrators | Give the active orchestrator a dedicated pane. When adding the production session selector, display workspace, run identity, and active or ended state, using stable IDs for selection. The mock covers a single stream. |
| A lost session has guidance without recovery | Put **Recover terminal** beside the explanation. Recovery is the primary action in a lost full pane. In the healthy full pane, **Focus terminal** is primary. The board retains **Start work** when the terminal is docked. |

**Before and after, same board and terminal transcript.** The before capture renders the checked-in HTML, CSS and board renderer against fixture data. Its sidebar is seeded to reproduce the owner's reported state. The after capture renders the React mock against the same ticket data and a live xterm canvas connected to a fixture session. It shows Board and Orchestrator at 1280 px. Below 1080 px the terminal has its own view, and at 320 px the sidebar becomes a compact header with stacked board lanes.

| Before | After |
|---|---|
| [Current board and split · 1280](captures/before-board-1280.png) | [Proposed board and split · 1280](captures/after-board-1280.png) |
| | [Board · 320](captures/after-board-320.png) · [Orchestrator · 320](captures/after-terminal-320.png) · [Lost session · 1280](captures/after-recover-1280.png) |

The source feedback was the five images in the supplied `web-ui-feedback-nov27` folder, covering the workspace dropdown, section control, primary button stack, orchestrator rows, and lost terminal.

## Stack options

| Option | Migration | Cost and tradeoff |
|---|---|---|
| **React + Base UI across the signed-in UI** · recommended | Introduce React and React DOM. Use Base UI for styled, accessible primitives, Vite for static assets, and React Router v7 in declarative mode for views. Reimplement the user-facing DOM and its event wiring incrementally, including the shell in `app.ts`, workspace navigation and the view renderers for Board, Needs You, Plan, Ticket, Workflow, Review, and Conversation. Keep the underlying WASM, Tailcat, relay, passkey/QR and enrollment logic, runner protocol, view-model decoding, xterm and pane management. The existing pre-auth Worker documents can continue serving their current HTML. | The largest rendering migration. One consistent component model once complete. The isolated spike adds at most 181,223 measured Brotli bytes if shipped alongside the old UI as built. The full integrated size and load cost need measurement before rollout. |
| **React + Base UI for the shell only** | Introduce React, React DOM and Base UI for the rail, workspace picker, commands, and layout. Give legacy renderers dedicated DOM mount nodes with explicit mount, update and dispose interfaces. Keep the existing routes or use a thin adapter around `WorkspaceHistory` during transition. | The smallest way to try Base UI in the product. Still incurs the React runtime. Focus, ownership of an element, and navigation synchronization need a contract at each React/legacy boundary. Useful as a release milestone and a viable permanent choice if a full renderer migration is too expensive. Size depends on selected primitives and whether xterm stays shared. |
| **Plain TypeScript + Web Awesome custom elements** | Keep the incumbent build and DOM renderers. Adopt individual packaged custom elements for picker, dialog and menus, use native HTML for ordinary controls, and map the same tokens to component CSS custom properties/parts. | Least rendering rewrite and no second UI framework. Includes the custom-element runtime and selected components. The package supplies styles and conventions that constrain the reskin more than Base UI. Prototype the actual chosen elements under Trusted Types and strict styles, especially shadow sheet fallback, dynamic icon loading, keyboard flow and form integration. This path has no measured size or CSP result in this ticket. |

Base UI is a React library. Its styles are authored here in CSS. A React-free path can still use this report's tokens and layout. All options should meet the same compressed asset gate and CSP. [Base UI styling](https://base-ui.com/react/handbook/styling) · [React Router modes](https://reactrouter.com/start/modes) · [Web Awesome installation](https://webawesome.com/docs)

## Built shell and hosting results

The spike uses React/React DOM **19.2.7**, Base UI **1.9.0**, Vite **7.3.8**, React Router **7.18.0** as a library, and the product's **xterm 5.5.0** with Canvas and Fit. Vite emits external modules and CSS under `/a/` with 16-character hexadecimal hashes, matching the Worker's [asset matcher](../../services/web/src/assets.ts). `HashRouter` keeps routes in the fragment, so refreshes use the same Worker document route `/`. Production still needs the existing version manifest and compatibility with legacy query links, browser history, stored selection, and passkey origin. [Vite build configuration](https://vite.dev/config/build-options) · [HashRouter](https://reactrouter.com/api/declarative-routers/HashRouter)

| Measure | Result |
|---|---:|
| Existing size gate, spike HTML + CSS + JS including xterm | **181,223 B** Brotli (3.30% of 5,500,000 B) |
| Archived Nov 15 canary dev build, full client including both WASMs | 5,076,025 B Brotli |
| Conservative coexistence estimate using that archive + the entire spike | 5,257,248 B Brotli, with 242,752 B remaining |
| Vite production build, dependencies installed, one local run | 1.18 s Vite, 1.69 s wall |

The archived baseline is the local `ov-779-web-keys/services/web/dist` artifact, stamped `canary/dev`. It contained 4,917,848 bytes of already-compressed WASM. The coexistence number deliberately double-counts xterm and includes the prototype sample route. It is a planning estimate against an archived baseline. The spike omits production WASM, ceremonies and the other signed-in views. A current complete dual-path build must pass the gate before opening the experiment to users. Vite also flagged the single 761 kB raw JS chunk. Measure startup on a phone and split secondary views by route before rollout. Chunking changes initial transfer and evaluation, while the existing gate still counts all files. Timing was measured on the local Mac with Node v26.13.1 using `/usr/bin/time -p npm run build` and excludes install, Rust/Go WASM build and the Brotli measurement.

### Content Security Policy

| Static host | What ran |
|---|---|
| Local Cloudflare Worker | Miniflare/workerd ran the checked-in `services/web/src/index.ts` and supplied the Vite directory through Static Assets. Fixture account and relay responses exercised the existing admitted-account path. Chromium loaded the document, JS and CSS from Worker responses and exercised workspace selection, history, modal focus, ticket action, terminal and recovery with the Worker's original CSP. |
| Rust loopback | The same directory was served over HTTP on `127.0.0.1` by the report's [Rust static fixture](spike/loopback.rs). There is no existing Rust static shell service in the ticket checkout that I could identify. The harness takes the exact Worker document CSP from the first run and Chromium checks header equality and exercises the same interactions. |

Both hosts passed with zero CSP violations or browser errors in Chromium 158. TypeScript and contrast checks passed, along with 52 existing Worker configuration, header, asset, reconnect and pane tests plus the web workflow fixture and session suites. Miniflare's local workerd is the repository test version, which predates the production compatibility date. This evidence covers the two local runtimes. The Worker has no production deployment in this ticket, and the runner and relay calls are fixtures.

`CSPProvider disableStyleElements` prevents injected Base UI styles. Keep styles on the same origin, use the Canvas addon before opening xterm, and use external CSS for component classes. The CSP still includes `style-src 'self'`, `require-trusted-types-for 'script'`, and `trusted-types fc`. Test any new component paths and chunk preloads under that header, including controls that opt into internal script tags. [Base UI CSP Provider](https://base-ui.com/react/utils/csp-provider) · [Existing Worker headers](../../services/web/src/headers.ts)

### Runner and streams

The mock deliberately puts [one runner session](spike/runner.ts) above the router. React context exposes that stable handle. `useSyncExternalStore` reads cached immutable board and connection snapshots. Terminal byte output goes straight from a subscription to `terminal.write`, and xterm's input goes back to the same handle. The workspace route keeps the same xterm host across Board and Orchestrator navigation, observing its box for fit and detaching when it unmounts. The verifier checks node identity across route changes and text output in xterm after sending commands and recovering. The fixture supports keyboard input, transcript replay to a newly mounted terminal, and a simulated lost session. It runs without account or runner access. [React external store](https://react.dev/reference/react/useSyncExternalStore)

In production the page's account-session owner should hold the registry of existing `Reconnector<WebRunner, TunnelConn>` instances by account and runner ID. The current `connectTo` already owns Tailcat dial, SSH sign-in and a reconnect sequence that resumes pane streams. Lift or expose that owner independently of React. Share low-frequency connection and board state through stable subscriptions, route commands through the existing runner/Workflow ports, and hand terminal output directly to existing `PaneView` and `PaneSet` instances keyed by runner and terminal. Preserve directed passkey prompts, generation fences, stream budgets, replay and scrollback, write/size leases and `pagehide` behavior. Switching UI views changes visibility and subscription, while account teardown owns closing the runner. Treat terminal process loss separately from tunnel loss, with **Recover terminal** for the former and **Reconnect** for the latter. [Runner boot and reconnect](../../services/web/src/client/app.ts) · [Existing terminal lifecycle](../../services/web/src/client/terminal.ts) · [Pane budget](../../services/web/src/client/panes.ts)

## Tokens and sample

[CSS tokens](spike/tokens.css) · [Rendered sample source](spike/main.tsx) · [Light 320](captures/sample-light-320.png) · [Dark 320](captures/sample-dark-320.png) · [Light 1280](captures/sample-light-1280.png) · [Dark 1280](captures/sample-dark-1280.png)

System sans uses the platform UI family, including SF on Apple platforms. Monospace uses the platform mono family with SF Mono, Menlo and Consolas fallbacks. Both use local system fonts, with zero font asset bytes. Use mono for ticket IDs, runner paths and terminal cells. Chrome and terminal colors are separate tokens, with the same dark terminal palette in both chrome themes for the spike.

| Role | Size / line height (px at default root size) | Weight |
|---|---:|---:|
| Page title | 32 / 36 | 700 |
| Section title | 18 / 24 | 600 |
| Body | 15 / 22 | 400 |
| Control label / card heading | 14 / 20 | 600 |
| Supporting text | 13 / 18 | 400 or 500 for breadcrumbs |
| Metadata and status | 12 / 16 | 400 mono, 600 for status |
| Terminal | 13 / 17.55 | 400 mono with ANSI emphasis |

Spacing starts at 4 px, with steps of 4, 8, 12, 16, 20, 24, 32, 40 and 48 px. Radius is 5 px for small marks, 8 px for controls, 12 px for panels, and fully rounded for status and stage dots. Desktop controls are at least 36 px high, with 44 px minimum for the primary controls and navigation at the phone breakpoint. The subtle rule token marks grouping. Controls that need a visible boundary use the stronger edge token.

Representative contrast ratios are below. Each row with multiple adjacent surfaces uses the lowest WCAG sRGB ratio from those surfaces. Status always has text. The [contrast script](spike/contrast.mjs) checks the underlying pairs individually against 4.5:1 for text and 3:1 for controls and focus. [WCAG text contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html) · [WCAG non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html)

| Pair | Light | Dark |
|---|---:|---:|
| Main text / canvas, card or rail | 13.43:1 | 13.49:1 |
| Muted text / canvas, card or rail | 5.65:1 | 7.61:1 |
| Primary action label / fill | 7.01:1 | 9.22:1 |
| Primary action label / hover fill | 9.36:1 | 11.11:1 |
| Selected text / selection | 7.99:1 | 9.19:1 |
| Connected text / status surface | 6.16:1 | 8.48:1 |
| Lost-session text / status surface | 6.04:1 | 8.36:1 |
| Control edge / canvas, card or rail | 3.55:1 | 4.40:1 |
| Primary fill / canvas, card or rail | 5.98:1 | 8.88:1 |
| Focus ring / canvas, card, rail or selection | 5.83:1 | 6.72:1 |
| Terminal text / terminal surface | 14.07:1 | 14.07:1 |

## Reproduce

From the repository root, with a current Node, Rust, and Chromium installed. Set `CHROMIUM_PATH` to a local binary or install the matching Playwright Chromium into the scratch directory. The verification command binds localhost for workerd and Rust, refreshes the PNGs, and closes its children on exit.

```sh
export OV1483_REPO="$PWD"
export TMPDIR="${FARCOOLER_SCRATCH:?Set the ticket scratch directory}"
export PLAYWRIGHT_BROWSERS_PATH="$FARCOOLER_SCRATCH/chromium"
mkdir -p "$FARCOOLER_SCRATCH/spike"
cp reports/ov-1483/spike/* "$FARCOOLER_SCRATCH/spike/"
cd "$FARCOOLER_SCRATCH/spike"
npm ci --cache "$FARCOOLER_SCRATCH/npm"
npm run typecheck
node contrast.mjs
/usr/bin/time -p npm run build
node "$OV1483_REPO/services/web/scripts/size-gate.mjs" dist
npm run verify
```

## Migration and reskin tickets, smallest first

1. **Introduce type, color and spacing tokens in legacy CSS.** Apply to control labels, metadata and pane headings. Keep current screen and transport behavior. Check both themes and text/control contrast.
2. **Fix workspace and action hierarchy in the current shell.** Show names that distinguish workspaces. Elevate the view's one action, make navigation quiet and give existing secondary actions a consistent location. Verify query links and keyboard and touch navigation.
3. **Show terminal state and recovery at the pane.** Disambiguate orchestrators by workspace and run identity. Put recovery next to a lost process and preserve the existing reconnect and writer-lease behavior. Verify end, recover, and reconnect independently.
4. **Introduce the framework and static build behind a canary opt-in.** Start with the shell and legacy mount nodes. Preserve sign-in, static routes, version checking and all existing views. Test the full integrated bundle under both policies, with size and phone startup measurements. Keep the legacy shell as the explicit fallback.
5. **Move the runner owner outside view rendering.** Adapt existing sessions to a stable subscription API and carry their pane instances through React navigation. Assert one connection per runner per page session and verify auth prompts, stream budgets, and replay across layout changes before switching ownership in canary.
6. **Ship the new board and orchestrator split.** Translate all current Board intents, dialogs and pane actions through existing contracts. Follow existing links and remember splits and selection. Release as a selectable shell once desktop and phone flows work against a real runner.
7. **Reskin remaining signed-in views one route at a time.** Start with Needs You, then Plan, Ticket and Review, Workflow, Conversation, and enrollment/settings. Reuse existing view models and commands. Keep a route on its current renderer until the replacement covers its states and actions.
8. **Make the new shell the default.** Roll out on canary after the route and runner tests pass. Retire legacy mount adapters and the old UI build only after the default has held up in daily use and there is a tested rollback.
