# Design: Command-line tools on PATH

Date: 2026-08-04
Status: APPROVED (design)
Extends `docs/farcooler-design.md`.

## Problem

`docs/remote-hosts.md` says a macOS host is "set up by running the Far Cooler
app there once." That is only true for the login-item daemon. The app bundles
`farcooler` and `farcoolerd` inside `Contents/Resources/`, but never puts
either on `PATH`. An SSH session onto the Mac — which is exactly how the iOS
app reaches it — execs a login shell that has no idea either binary exists,
`farcoolerd --stdio` finds nothing, the pipe closes immediately, and the
client reports it as "did not answer": indistinguishable, from the wire, from
a hung daemon. `farcooler host install` does not fix this either — it is
explicit that macOS is out of scope for the copy-into-`~/.local/bin` step it
performs for Linux hosts.

So "run the app once" needs to become true.

## What we are building

A small `ObservableObject`, `CommandLineTools`, that symlinks the bundled
`farcooler` and `farcoolerd` into `~/.local/bin` — the same directory
`docs/remote-hosts.md` already documents as the expected location — plus a
toggle for it in Settings and a one-time launch prompt for people who have
never looked at Settings.

It is a sibling to `ServiceRegistration.swift`, not an extension of it. Both
answer "is this Mac reachable when nobody is sitting at it," but they are
different failure domains — one is an `SMAppService` registration that can
need System Settings approval, the other is two symlinks — and a person may
reasonably want one without the other (e.g. daemon-at-login on, but managing
their own dotfiles' `PATH` by hand). Keeping them separate rows, in the same
tab, keeps each independently understandable and testable.

## State

```swift
enum State: Equatable {
    case notInstalled
    case installed
    /// Something occupies `~/.local/bin/<name>` that this app did not put
    /// there — a real file, or a symlink pointing somewhere else.
    case conflict(String)
    case unavailable(String)
}
```

`farcooler` and `farcoolerd` are tracked as one unit, not two independent
toggles — `host_install.rs` already treats the pair this way for Linux hosts,
and there is no scenario where a user wants the CLI on `PATH` without the
daemon or vice versa. `refresh()` inspects both names under
`~/.local/bin` and folds them into a single `State`:

- Both missing → `.notInstalled`.
- Both symlinks whose destination equals this bundle's
  `Contents/Resources/<name>` → `.installed`.
- Either slot occupied by a real file, or a symlink pointing anywhere else →
  `.conflict("~/.local/bin/<name> already exists and isn't managed by Far Cooler")`,
  naming whichever slot is the problem. Conflict wins over not-installed: a
  half-broken state should never read as "nothing to see here."
- Running from a bare executable (no `.app` bundle) → `.unavailable(...)`,
  mirroring `ServiceRegistration`'s identical guard — there is no bundled
  binary to point a symlink at.

## Behavior

`install()`:
1. Refuses (no-op) if the current state is `.conflict` — this app never
   overwrites a path it did not create. The existing conflict message is what
   tells the user what to remove by hand.
2. `FileManager.createDirectory` for `~/.local/bin`, with intermediate
   directories, if it does not exist.
3. `createSymbolicLink` for both `farcooler` and `farcoolerd`, pointing at
   this bundle's `Contents/Resources/<name>`.
4. `refresh()`.

`uninstall()` removes both symlinks — but only the ones `refresh()` currently
sees as `.installed` (i.e. still pointing at this app). It never touches a
path in `.conflict` state, for the same reason `install()` never overwrites
one.

No shelling out, no new process, no entitlements needed — the macOS app
carries no `.entitlements` file today (confirmed: no App Sandbox), so a plain
`FileManager` write to `~/.local/bin` needs nothing special.

## Settings UI

`Preferences.swift`'s `host` section (the Startup tab, currently just the
login-item toggle) gets a second `Setting` row:

> Puts `farcooler` and `farcoolerd` on your PATH, so a terminal or an SSH
> session can find them.

with a toggle that mirrors the login-item row's own state-to-view mapping
(`.installed` → on, `.notInstalled` → off, `.conflict` → static text showing
the conflict, `.unavailable` → static text). Tapping on calls `install()`,
tapping off calls `uninstall()` — same shape as `ServiceRegistration`'s
`.registered` / `.notRegistered` cases in that same file today.

## One-time launch prompt

`FarCoolerApp.swift`'s `ContentView().onAppear` (already home to
`Appearance.apply`) also calls `CommandLineTools.refresh()`. If the result is
`.notInstalled` and `UserDefaults.standard.bool(forKey:
"hasPromptedCLIToolsInstall")` is false, a SwiftUI `.alert` on `ContentView`
offers:

> Install command-line tools?
> Lets your terminal and SSH sessions find `farcooler`. You can always do
> this later in Settings.
> [Install] [Not Now]

Either button sets `hasPromptedCLIToolsInstall` to true — this fires at most
once, ever, per machine. `.conflict` never triggers the prompt: there is
nothing to silently offer to fix, and popping an alert about a problem the
user has to go read Settings to understand anyway is worse than leaving the
Settings row to explain itself when found.

## Testing

- Unit tests for `CommandLineTools` against a temp `HOME` (existing tests in
  this app already redirect `HOME` for filesystem-touching code — follow the
  same pattern rather than touching the real `~/.local/bin` in CI):
  not-installed → install → installed; already-installed is idempotent;
  a real file in the slot produces `.conflict` and `install()` leaves it
  untouched; a foreign symlink produces `.conflict`; `uninstall()` removes
  only what it owns and leaves a `.conflict` slot alone.
- No UI test for the one-time alert beyond a manual check — it is one
  `UserDefaults` flag and a stock SwiftUI `.alert`.

## Out of scope

- Nothing changes for Linux/WSL hosts or `host_install.rs`; this is purely
  the macOS-app-as-local-host gap.
- No change to how the daemon itself starts — this only affects whether a
  *shell* (interactive or SSH-invoked) can find the binaries.
