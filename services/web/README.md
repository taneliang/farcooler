# Web client

The Worker behind app.farcooler.com (and app.canary.farcooler.com). Its shell's
Content-Security-Policy is built from `wrangler.toml` (`src/headers.ts`).

## The signed-in app

`scripts/build.mjs` bundles `src/client/app.ts` with Vite into one script and
one stylesheet, both served from `/a/` under the strict CSP.

- **Styling.** `src/client/tokens.css` is the one source of type, color (light,
  plus dark by `prefers-color-scheme` or `data-theme`), spacing and corners.
  Other stylesheets use its variables, not literal values (`scripts/styles.mjs`
  lists them in order).
- **Shell.** `src/client/shell/` renders the sidebar, page header and
  orchestrator pane with React, Base UI and React Router. It shows
  `ShellStore` (`store.ts`), which `app.ts` and `work-screen.ts` write; feature
  logic stays in the Rust core.
- **Views.** The board, plan, chat and terminals are still DOM code in
  `index.html`'s `#legacy` holder. The shell mounts each into place through a
  `LegacyView` (mount, update, dispose).
- **Actions.** A plain button is outlined, and `.primary` is the filled accent.
  Each view has at most one filled button; lists use `.prominent`.
- **Icon.** `src/client/icons/app-icon-1024.png` is the master of the app icon.
  The sizes beside it feed the favicon, the sidebar, the touch icon and
  `app.webmanifest`.

`test/browser/shell-check.mjs` covers keyboard focus order, the pickers and
dialogs, filled buttons and contrast at 320, 768 and 1280 px in both themes.

`test/browser/split-check.mjs` covers the message box as panes come back and
change width: a split closed and reopened, another terminal and back, and a
reply that lands while its pane is away.

## The Engine view

`engine-view.ts` draws the page the shared core makes
(`crates/client/src/engine_page.rs`): stations, landing groups, drafts waiting
for approval, and the actions on each row. It reads the runner's one-reply
`workflow.get`, and takes titles from the plan and the board. A press sends back
the row and kind, or the draft and the question read; the core makes the command
from a fresh read, and the runner decides who may send it.

`npm run test:engine` has a real runner read a 340-item board, then runs
`test/browser/engine-check.mjs` over the page the core made from that read. Give
the check a second argument, a folder, to write captures there.

## Adding a Tailcat relay host

The CSP names Tailcat's relays one by one, not as `*.ipn.dev`, so a relay that
appears in https://tailcat.dev/derpmap.json is blocked until its host is listed:
connects to runners homed there fail with a false "didn't answer".

1. Add the host to `DERP_HOSTS` (space separated) in every environment of
   `wrangler.toml`: the top-level `[vars]` and `[env.canary.vars]`.
2. Update the host list in `test/config.test.ts`.
3. Run `scripts/derp-hosts-check.py` from the repo root; it exits 0 when the
   map and both lists agree.

A daily workflow (`.github/workflows/derp-hosts.yml`) runs that check and opens
an issue when the map and `DERP_HOSTS` differ.

## Saved recovery copies

The header's **Saved Copies** lists the signed-in account's passkey credential
ids, marks this browser's passkey, and lets the owner clear any copy, including
unreadable junk. Clearing removes recovery for every browser sharing that
passkey; it does not revoke the passkey or remove local runners. An active
browser can save again after its next touch.

A passkey touch leaves a readable shared copy alone. Local changes merge the
copy's authenticated runners with local runners (only changed local entries
replace a shared grant), and compare the previous ciphertext at the relay before writing.
Conflicts re-read and merge again, up to three attempts. Failed reads never
trigger an overwrite. A failed or timed-out Sign Out deletion offers Retry
Delete or Sign Out Anyway and explains that recovery may remain.
