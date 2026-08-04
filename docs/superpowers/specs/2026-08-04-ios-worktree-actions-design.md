# Design: Worktree-row actions on iOS

Date: 2026-08-04
Status: APPROVED (design)
Extends `docs/farcooler-design.md`.

## Problem

The iOS worktree sheet (`WorkspaceListView`, built from `FleetList`/`TerminalRow`
in `apps/ios/FarCooler/FleetView.swift`) shows a worktree's terminals and lets
you act on each one — but the worktree itself has no actions at all. Its
section header is plain text. macOS's sidebar, by contrast, gives every
worktree row an ellipsis menu: New terminal, Hide/Unhide, Remove Worktree,
plus editor integration this spec excludes. The concrete complaint that
started this: there is no way to add a second terminal to a worktree you
already have open, on your phone, without going through Quick Task and
getting an entirely new workspace instead.

Editor integration is explicitly out of scope — iOS has no editor concept at
all today, and it is not obvious one should exist in the same shape as
macOS's SSH-remoting editors on a phone. That is a separate project.

## What we are building

Four worktree-row actions on iOS, reachable from a new ellipsis menu in each
worktree's section header (mirroring macOS's `SidebarViews.swift:262-297`
exactly) plus one addition to the New Workspace form:

1. **New terminal** — one tap, no picker, same as macOS's default.
2. **Hide / Unhide** — with hidden worktrees collapsing into a "Hidden N"
   section, same attention-preserving behavior macOS has.
3. **Remove Worktree** — a two-phase confirmation matching macOS's exactly:
   trivial for a clean worktree, requires typing the task name for a dirty
   one.
4. **Add Repository** — inside the New Workspace form's repository picker,
   simpler than macOS's version since a phone never has local repositories
   of its own.

## Architecture

One RPC path serves both platforms. Today it does not: `crates/cli` builds
its own protobuf requests directly for `workspace.remove_worktree`,
`repository_root.add`, `repository.register`, `workspace.hide`, and
`workspace.unhide`, entirely separately from `crates/client::Session` (the
library iOS calls through its FFI bridge). That split is accidental, not
intentional — `crates/client` predates none of this, it simply never grew
these five methods because nothing needed them from that side yet. This spec
closes it: every one of the five gets one implementation, in
`crates/client::Session`, and both callers — iOS via FFI, and `crates/cli`
(which is what macOS's `DaemonClient.swift` shells out to for all of this
today, and continues to) — call through it.

```
daemon RPC (all 5 methods already exist here)
  -> crates/client::Session   (2 of 5 exist: create_workspace, create_terminal
                                hide_workspace, unhide_workspace; remove_worktree,
                                add_repository_root, register_repository are new)
      -> crates/client FFI match arm (new arms for the 3 new Session methods)
      |    -> apps/ios/FarCooler/Connection.swift (new thin wrapper, all 5)
      |        -> FleetView.swift UI
      -> crates/cli (5 call sites switched from building requests inline to
           calling Session)
          -> apps/macos/Sources/FarCooler/DaemonClient.swift (unchanged —
               already shells out to this CLI)
```

`hide_workspace`/`unhide_workspace` on the `Session` side already exist and
are already exposed through the FFI (`ffi.rs:410-417`), proven by nothing
except that they compile and are covered by whatever `crates/client` test
harness already exists — nothing on either platform calls them in production
yet. `create_workspace`/`create_terminal` similarly already exist; this spec
does not touch either.

## Feature 1 — New terminal on an existing worktree

Add `func createTerminal(workspace: String) async` to `Connection.swift`,
wrapping the already-working `createTerminal(workspace:title:preset:)` with
`preset: "shell"` and `title: "Terminal \(workspace.terminals.count + 1)"` —
matching macOS's `openTerminalInNewLayout` (`ContentView.swift:1857-1860`)
exactly. No preset picker: macOS does not have one for this action, so
neither does iOS. Wired as the ellipsis menu's first item, "New terminal."

## Feature 2 — Hide / Unhide

1. `Workspace` (`apps/ios/FarCooler/Model.swift:21-28`) already decodes
   `state: String`. Add `isHidden`/`worktreeMissing` computed properties
   reading it, mirroring macOS's `Model.swift:65-68` exactly — no wire
   change.
2. `Connection.swift` gets `hideWorkspace(_:)`/`unhideWorkspace(_:)`, thin
   wrappers around the already-fully-wired `workspace.hide`/`workspace.unhide`
   FFI calls.
3. The ellipsis menu shows "Hide" or "Unhide" depending on
   `workspace.isHidden`.
4. `FleetList` splits workspaces into shown/hidden the way
   `ContentView.swift:496-498` does. Hidden ones collapse into a **"Hidden
   N"** disclosure section at the bottom of the list, collapsed by default,
   with an orange attention dot if any hidden workspace has a terminal
   wanting the user — mirroring `HiddenWorktrees`
   (`SidebarViews.swift:812-849`). Hiding a worktree never silences it; it
   only moves it out of the way.

## Feature 3 — Remove Worktree

**`crates/client::Session::remove_worktree`** — new. Matches the shape of
`search_worktree_files` (`session.rs:548-564`): builds a
`request::Payload::TypedConfirmation { typed_confirmation }` when given a
confirmation string (empty string when none), calls
`self.value("workspace.remove_worktree", Some(workspace), payload)`, and —
unlike the CLI's current bare `?`-propagation — distinguishes the daemon's
`DomainError::ConfirmationRequired` from every other failure in its return
type, so every caller gets the same clean three-way answer macOS's
`RemoveWorktreeResult` already models (`DaemonClient.swift:951-962`):
succeeded, needs a typed name, or failed for some other reason (with the
daemon's own message).

**`crates/cli`** — `WorkspaceCmd::RemoveWorktree` (`main.rs:1071-1084`)
switches from building the `TypedConfirmation` request inline to calling
`Session::remove_worktree`, preserving its exact current stdout format
(`"removed worktree for {} (branch kept)"`) and CLI exit-code-on-error
behavior, so macOS's existing error-text-sniffing in `DaemonClient.swift`
(`localizedCaseInsensitiveContains("confirmation")`) keeps working
unchanged. Whether `Session`'s connection lifecycle drops in for the CLI's
existing `Link`/`connect_to` setup, or needs adapting, is confirmed during
implementation — not assumed here.

**New FFI arm** (`ffi.rs`, alongside `workspace.hide`) —
`"workspace.remove_worktree"` reads `id("workspace")` and an optional
`text("confirm")`, calls the new `Session` method, and replies with a JSON
shape carrying all three outcomes (`{"ok": true}` /
`{"confirmationRequired": true}` / `{"error": "<message>"}`) — richer than
the existing arms' bare `Result<Value, String>`, because this is the first
FFI call that needs three outcomes instead of two.

**`Connection.swift`** — `removeWorktree(_:confirm:) async ->
RemoveWorktreeResult`, an iOS-side enum mirroring macOS's
(`.ok`/`.confirmationRequired`/`.failed(String)`).

**UI** — the ellipsis menu excludes this item entirely for the main checkout
(`!workspace.isMainCheckout`), same as macOS. Tapping "Remove Worktree…"
calls `removeWorktree(id, confirm: "")`. `.ok` dismisses nothing further
(the row disappears on the next refresh). `.confirmationRequired` presents a
new `RemoveWorktreeConfirmSheet` — a text field the user must fill with the
exact task name before a destructive confirm button enables, mirroring
`RemoveWorkspaceSheet`'s `matches` gate (`Sheets.swift:171,179`). `.failed`
shows the daemon's message inline.

## Feature 4 — Add Repository

**`crates/client::Session`** — two new methods:
`add_repository_root(absolute_path: &str) -> Result<RepositoryRoot,
SessionError>` wrapping `repository_root.add` with `RepositoryRootAdd {
absolute_path, typed_confirmation: String::new() }`, and
`register_repository(relative_path: &str) -> Result<Repository,
SessionError>` wrapping `repository.register` with `RepositoryRegister {
relative_path }` — matching the CLI's current payload construction
(`main.rs:753-762`, `:819-827`) exactly. Adding is not destructive, so
neither takes a confirmation.

**`crates/cli`** — `RootCmd::Add` and `RepoCmd::Register` switch to calling
these, preserving their current stdout format and the local-path
canonicalization behavior for a directly-run (non-`--host`) CLI.

**New FFI arms** — `"repository_root.add"` and `"repository.register"`,
following the plain-JSON-args pattern the existing arms use (no
typed-confirmation complexity here).

**`Connection.swift`** — two matching wrappers.

**UI** — since iOS only ever registers a repository on a *remote* host (a
phone has no filesystem worth pointing at), this is simpler than macOS's
dual-mode (`AddRepositorySheet`'s local-file-picker-or-typed-path) version:
a new "Add a repository…" row inside `NewWorkspaceView`'s existing
repository `Picker` (`FleetView.swift:707-709`) opens a small sheet with a
host picker (shown only when more than one machine is connected) and a text
field for the path. On success, both calls run in sequence
(`add_repository_root` then `register_repository`, same order
`AddRepositorySheet`'s `onAddRoot`/`onRegister` run in) and the new
repository is selected in the picker immediately, matching macOS's
`onRegistered` callback (`Sheets.swift:492`).

## A note on scope (`host_admin`)

`workspace.remove_worktree` and `repository_root.add` are gated at
`Scope::HostAdmin` on the daemon (`repository.register` is `Scope::Control`,
one tier down). Scope is granted per enrolled device key via
`client.set_scopes`, entirely daemon-side — no client code, on either
platform, controls or negotiates it. If the phone's enrolled key is not
already `host_admin`, these two calls will be refused by the daemon
regardless of how correctly this spec is built; that is a one-time
Settings → Machines change on macOS, not a code problem, and outside this
spec's scope to fix.

## Testing

`crates/client` has real Cargo unit tests today; the three new `Session`
methods get unit tests following whatever harness/mocking convention its
existing tests use (confirmed during implementation by reading
`session.rs`'s existing test module). The `crates/cli` refactor is verified
by running each of the five affected subcommands directly against a real
daemon and confirming identical output to before. Neither Swift app has a
test target (confirmed already for macOS in the CLI-tools-install spec;
iOS has none either) — UI verification is manual: build, run against a real
connected host, exercise all four iOS actions and their error paths (a
dirty worktree for remove, an already-registered path for add-repository).

## Out of scope

- Editor integration on iOS — a separate project, not attempted here.
- Any change to `terminal.create`/`workspace.create` in `crates/cli` — not
  flagged as a problem, left as they are.
- Pane tiling / move-to-layout — inherently desktop-only, no iOS
  equivalent makes sense.
