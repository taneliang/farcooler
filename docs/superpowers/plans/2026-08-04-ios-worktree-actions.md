# iOS worktree-row actions — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the iOS worktree sheet to parity with macOS's sidebar for four actions (new terminal, hide/unhide, remove worktree, add repository), and unify `crates/cli` and `crates/client` onto one shared Rust implementation for all five RPC calls involved, so macOS (which shells out to the CLI) and iOS (which calls `crates/client` via FFI) run identical logic instead of two independent copies.

**Architecture:** A new `crates/client/src/actions.rs` holds five free async functions, generic over `Client<R, W>` (the shared `farcooler_transport` type both `crates/cli`'s `Link` and `crates/client`'s `Session` already wrap around identically-shaped boxed reader/writer trait objects) — so both callers use the exact same request-building and result-unwrapping code without their connection-establishment layers (which genuinely differ — `Link` shells to the system `ssh` binary and can auto-start a local daemon; `Session` uses an in-process `russh` client and never auto-starts anything) needing to merge at all. `Session` gets thin delegating methods; `crates/cli`'s five call sites call the free functions directly through a new `Link::client_mut()` accessor. Two new FFI arms and five new `Connection.swift` wrappers expose all of it to iOS, behind a new ellipsis menu on each worktree row plus an addition to the New Workspace form.

**Tech Stack:** Rust (crates/client, crates/cli, crates/daemon unchanged), Swift/SwiftUI (iOS).

## Global Constraints

- Spec: `docs/superpowers/specs/2026-08-04-ios-worktree-actions-design.md`.
- Preserve `crates/cli`'s exact current stdout format at every refactored call site — macOS's `DaemonClient.swift` shells out to this CLI and, for remove-worktree, string-sniffs its error output for `"confirmation"` (`localizedCaseInsensitiveContains("confirmation")`). The refactor must keep that substring present in the confirmation-required error message.
- Do not touch `crates/cli`'s remote (`--host`) connection path or its local daemon auto-start behavior — only the five call sites' request-building/result-unwrapping code changes.
- Do not touch `terminal.create`/`workspace.create` in `crates/cli` — not in scope.
- Neither Swift app has a test target; Swift-side verification is manual, run against a real connected host. `crates/client` has a real integration test suite (`crates/client/tests/against_a_real_daemon.rs`, spins up a real `farcoolerd`) — new Rust logic gets tests there, following that file's existing patterns exactly.
- New iOS UI code lives in `apps/ios/FarCooler/FleetView.swift` (existing structures) and `apps/ios/FarCooler/Connection.swift` (new wrapper methods), plus new files only where a genuinely new, independently reusable view is created (the two new sheets).

---

### Task 1: Shared Rust actions — `hide`/`unhide`/`remove_worktree`

**Files:**
- Create: `crates/client/src/actions.rs`
- Modify: `crates/client/src/lib.rs` (declare the new module)
- Modify: `crates/client/src/session.rs:286-292` (`hide_workspace`/`unhide_workspace` become delegations; add `remove_worktree`)
- Modify: `crates/cli/src/daemon_link.rs:29-58` (add `Link::client_mut()`)
- Modify: `crates/cli/src/main.rs:1057-1084` (three call sites: `Hide`, `Unhide`, `RemoveWorktree`)
- Modify: `crates/cli/Cargo.toml` (add `farcooler-client` as a dependency)
- Test: `crates/client/tests/against_a_real_daemon.rs`

**Interfaces:**
- Produces: `crates/client::actions::{hide_workspace, unhide_workspace, remove_worktree}`, all `pub async fn(client: &mut Client<R, W>, ...) -> Result<_, ClientError>` where `R: tokio::io::AsyncRead + Unpin + Send, W: tokio::io::AsyncWrite + Unpin + Send`. `remove_worktree` returns `Result<RemoveWorktreeOutcome, ClientError>` where `pub enum RemoveWorktreeOutcome { Removed, ConfirmationRequired }` (also defined in `actions.rs`, re-exported from `lib.rs`).
- Produces: `Session::remove_worktree(&mut self, workspace: Uuid, confirm: &str) -> Result<RemoveWorktreeOutcome, SessionError>` — Task 2 (FFI) consumes this directly.
- Produces: `Link::client_mut(&mut self) -> &mut Client<Reader, Writer>` (using `daemon_link.rs`'s existing `Reader`/`Writer` type aliases).

- [ ] **Step 1: Write `crates/client/src/actions.rs`**

```rust
//! The five worktree-row actions, shared between `crates/client` (which
//! iOS's FFI bridge calls through `Session`) and `crates/cli` (which macOS's
//! `DaemonClient.swift` shells out to). Both wrap the same
//! `farcooler_transport::Client` around a connection each side establishes
//! its own way — a system `ssh` subprocess and local-daemon auto-start for
//! the CLI, an in-process `russh` client with no auto-start for the mobile
//! core — so these functions are generic over the transport rather than over
//! either connection type, and never try to unify how the connection itself
//! was made.

use farcooler_protocol::v1::{Repository, RepositoryRoot, request, result};
use farcooler_transport::{Client, ClientError};
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

/// What asking the daemon to remove a worktree came back with.
///
/// Not folded into `ClientError`: a confirmation prompt is an expected
/// domain outcome a caller branches on, not a transport failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveWorktreeOutcome {
    Removed,
    ConfirmationRequired,
}

async fn call<R, W>(
    client: &mut Client<R, W>,
    method: &str,
    target: Uuid,
    payload: Option<request::Payload>,
) -> Result<Option<result::Value>, ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let mut request = farcooler_transport::request(method);
    request.target_resource_id = Some(bytes::Bytes::copy_from_slice(target.as_bytes()));
    if let Some(p) = payload {
        request.payload = Some(p);
    }
    Ok(client.call(request).await?.value)
}

pub async fn hide_workspace<R, W>(
    client: &mut Client<R, W>,
    workspace: Uuid,
) -> Result<(), ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    call(client, "workspace.hide", workspace, None).await.map(|_| ())
}

pub async fn unhide_workspace<R, W>(
    client: &mut Client<R, W>,
    workspace: Uuid,
) -> Result<(), ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    call(client, "workspace.unhide", workspace, None).await.map(|_| ())
}

/// Remove a worktree. `confirm` must be the workspace's exact task name,
/// unless the worktree is clean, in which case it may be empty.
///
/// Forwarded rather than checked here: the daemon refuses a mismatch itself,
/// so this is a courtesy and the daemon's own check is what actually
/// protects the files.
pub async fn remove_worktree<R, W>(
    client: &mut Client<R, W>,
    workspace: Uuid,
    confirm: &str,
) -> Result<RemoveWorktreeOutcome, ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let payload = request::Payload::TypedConfirmation(farcooler_protocol::v1::TypedConfirmation {
        typed_confirmation: confirm.to_string(),
    });
    match call(client, "workspace.remove_worktree", workspace, Some(payload)).await {
        Ok(_) => Ok(RemoveWorktreeOutcome::Removed),
        Err(ClientError::Daemon { code, .. })
            if code == farcooler_protocol::v1::ErrorCode::ConfirmationRequired as i32 =>
        {
            Ok(RemoveWorktreeOutcome::ConfirmationRequired)
        }
        Err(other) => Err(other),
    }
}
```

- [ ] **Step 2: Declare the module**

In `crates/client/src/lib.rs`, add near the other `pub mod` declarations:

```rust
pub mod actions;
```

- [ ] **Step 3: Delegate `Session::hide_workspace`/`unhide_workspace`, add `Session::remove_worktree`**

In `crates/client/src/session.rs`, replace lines 286-292:

```rust
    pub async fn hide_workspace(&mut self, workspace: Uuid) -> Result<(), SessionError> {
        self.value("workspace.hide", Some(workspace), None).await.map(|_| ())
    }

    pub async fn unhide_workspace(&mut self, workspace: Uuid) -> Result<(), SessionError> {
        self.value("workspace.unhide", Some(workspace), None).await.map(|_| ())
    }
```

with:

```rust
    pub async fn hide_workspace(&mut self, workspace: Uuid) -> Result<(), SessionError> {
        Ok(crate::actions::hide_workspace(&mut self.client, workspace).await?)
    }

    pub async fn unhide_workspace(&mut self, workspace: Uuid) -> Result<(), SessionError> {
        Ok(crate::actions::unhide_workspace(&mut self.client, workspace).await?)
    }

    /// Remove a worktree, or find out it needs the task name typed first —
    /// see `actions::remove_worktree` for what `confirm` means.
    pub async fn remove_worktree(
        &mut self,
        workspace: Uuid,
        confirm: &str,
    ) -> Result<crate::actions::RemoveWorktreeOutcome, SessionError> {
        Ok(crate::actions::remove_worktree(&mut self.client, workspace, confirm).await?)
    }
```

This compiles only if `Session.client`'s field type matches `actions.rs`'s generic bounds — confirm this now by running `cargo check -p farcooler-client` (next step). If the field is private with a different name, adjust the field access to whatever `session.rs`'s struct definition actually calls it (check the `struct Session` definition a few lines above `hide_workspace` if the build fails here).

- [ ] **Step 4: Build and fix any type mismatch**

Run: `cd /Users/e-liang/Dev/overnight && cargo check -p farcooler-client 2>&1 | tail -50`
Expected: clean build. If `self.client`'s concrete type doesn't satisfy `actions.rs`'s `R: AsyncRead + Unpin + Send, W: AsyncWrite + Unpin + Send` bounds, the compiler error names the actual bound that's missing — add it to `actions.rs`'s generic parameters (e.g. `+ Sync` if required) rather than changing `Session`.

- [ ] **Step 5: `Link::client_mut()` accessor**

In `crates/cli/src/daemon_link.rs`, inside `impl Link` (after the existing `call`/`next_event`/`daemon_build` methods, before the closing brace at line 661):

```rust
    /// The transport underneath, for callers reusing shared request-building
    /// code from `farcooler_client::actions` instead of building requests
    /// inline.
    pub fn client_mut(&mut self) -> &mut Client<Reader, Writer> {
        &mut self.client
    }
```

- [ ] **Step 6: Add the dependency**

In `crates/cli/Cargo.toml`, under `[dependencies]`, add:

```toml
farcooler-client.workspace = true
```

Run: `cd /Users/e-liang/Dev/overnight && cargo check -p farcooler-cli 2>&1 | tail -30`
Expected: clean build (the dependency resolves; nothing calls it yet).

- [ ] **Step 7: Refactor the CLI's three call sites**

In `crates/cli/src/main.rs`, replace lines 1057-1084:

```rust
        WorkspaceCmd::Hide { workspace } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            link.call(req_for("workspace.hide", uuid_of(&ws.id))).await?;
            println!("hidden {}  (git data untouched)", short_bytes(&ws.id));
        }

        WorkspaceCmd::Unhide { workspace } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            link.call(req_for("workspace.unhide", uuid_of(&ws.id))).await?;
            println!("unhidden {}", short_bytes(&ws.id));
        }

        WorkspaceCmd::RemoveWorktree { workspace, confirm } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            // The daemon checks this too, and its check is the one that counts:
            // a client that skips the prompt must still be refused.
            link.call(with(
                req_for("workspace.remove_worktree", uuid_of(&ws.id)),
                request::Payload::TypedConfirmation(farcooler_protocol::v1::TypedConfirmation {
                    typed_confirmation: confirm.unwrap_or_default(),
                }),
            ))
            .await?;
            println!("removed worktree for {} (branch kept)", short_bytes(&ws.id));
        }
```

with:

```rust
        WorkspaceCmd::Hide { workspace } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            farcooler_client::actions::hide_workspace(link.client_mut(), uuid_of(&ws.id)).await?;
            println!("hidden {}  (git data untouched)", short_bytes(&ws.id));
        }

        WorkspaceCmd::Unhide { workspace } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            farcooler_client::actions::unhide_workspace(link.client_mut(), uuid_of(&ws.id)).await?;
            println!("unhidden {}", short_bytes(&ws.id));
        }

        WorkspaceCmd::RemoveWorktree { workspace, confirm } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            // The daemon checks this too, and its check is the one that counts:
            // a client that skips the prompt must still be refused.
            use farcooler_client::actions::RemoveWorktreeOutcome;
            match farcooler_client::actions::remove_worktree(
                link.client_mut(),
                uuid_of(&ws.id),
                &confirm.unwrap_or_default(),
            )
            .await?
            {
                RemoveWorktreeOutcome::Removed => {
                    println!("removed worktree for {} (branch kept)", short_bytes(&ws.id));
                }
                // Same substring the CLI's own prior direct call produced in
                // its error text — DaemonClient.swift on macOS still sniffs
                // for "confirmation" in whatever this prints to stderr.
                RemoveWorktreeOutcome::ConfirmationRequired => {
                    return Err("exact typed confirmation required".into());
                }
            }
        }
```

- [ ] **Step 8: Build**

Run: `cd /Users/e-liang/Dev/overnight && cargo build -p farcooler-cli -p farcooler-client 2>&1 | tail -50`
Expected: clean build. If `request`/`with`/`req_for` imports in `main.rs` are now unused because these three call sites no longer reference them directly, check whether other call sites in the same file still use them (`RootCmd`/`RepoCmd` handlers do, per Task 2) — leave the imports if anything else uses them; remove only if the compiler warns them unused.

- [ ] **Step 9: Integration test — remove_worktree's three-way outcome**

In `crates/client/tests/against_a_real_daemon.rs`, add after `hiding_and_unhiding_round_trips_through_the_client` (following the file's existing patterns exactly — same `start()`/`register_root_and_repository()`/`create_workspace` setup):

```rust
#[tokio::test]
async fn removing_a_clean_worktree_needs_no_typed_name() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;

    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let workspace = session
        .create_workspace(repository, "clean removal", "feat/clean-removal", "HEAD")
        .await
        .expect("create");
    let id = farcooler_client::session::uuid_of(&workspace.id);

    use farcooler_client::actions::RemoveWorktreeOutcome;
    let outcome = session.remove_worktree(id, "").await.expect("remove");
    assert_eq!(outcome, RemoveWorktreeOutcome::Removed);
}

#[tokio::test]
async fn removing_a_dirty_worktree_needs_the_task_name_typed() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;

    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let workspace = session
        .create_workspace(repository, "dirty removal", "feat/dirty-removal", "HEAD")
        .await
        .expect("create");
    let id = farcooler_client::session::uuid_of(&workspace.id);

    // Find the worktree on disk and dirty it. The daemon derives "dirty" from
    // git status, so this has to be a real uncommitted change, not a flag.
    let fleet = session.fleet().await.expect("fleet");
    let workspaces = fleet["workspaces"].as_array().unwrap();
    let created =
        workspaces.iter().find(|w| w["task"] == "dirty removal").expect("workspace present");
    let worktree_path = created["worktree"].as_str().expect("worktree path");
    std::fs::write(std::path::Path::new(worktree_path).join("untracked.txt"), "uncommitted")
        .unwrap();

    use farcooler_client::actions::RemoveWorktreeOutcome;
    let outcome = session.remove_worktree(id, "").await.expect("first attempt");
    assert_eq!(outcome, RemoveWorktreeOutcome::ConfirmationRequired);

    let outcome =
        session.remove_worktree(id, "dirty removal").await.expect("confirmed attempt");
    assert_eq!(outcome, RemoveWorktreeOutcome::Removed);
}
```

If `created["worktree"]` isn't present in the fleet JSON shape (the earlier `the_fleet_shape_is_the_one_a_phone_decodes` test only asserts `runtime_healthy`/`live_panes`/`workspaces` exist, not a worktree path field), check `crates/daemon/src/wire.rs`'s `workspace()` function for the actual key name the daemon serializes the worktree path under, and use that key instead.

- [ ] **Step 10: Run the new tests**

Run: `cd /Users/e-liang/Dev/overnight && cargo build -p farcooler-daemon && cargo test -p farcooler-client --test against_a_real_daemon 2>&1 | tail -60`
Expected: all tests pass, including the two new ones.

- [ ] **Step 11: Commit**

```bash
git add crates/client/src/actions.rs crates/client/src/lib.rs crates/client/src/session.rs \
  crates/cli/src/daemon_link.rs crates/cli/src/main.rs crates/cli/Cargo.toml \
  crates/client/tests/against_a_real_daemon.rs
git commit -m "$(cat <<'EOF'
refactor(client,cli): share hide/unhide/remove-worktree between CLI and mobile

crates/cli built its own protobuf requests for these directly; iOS's
FFI bridge calls through crates/client::Session separately. Neither
reused the other, so they could silently drift. Extracts the actual
logic into crates/client::actions, generic over the transport rather
than either connection type (the CLI shells to system ssh and can
auto-start a local daemon; the mobile core uses an in-process russh
client and never does) -- Session delegates to these functions, and
the CLI's three call sites call them directly through a new
Link::client_mut() accessor. macOS needs no changes: DaemonClient.swift
already shells out to this same CLI.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Shared Rust actions — `add_repository_root`/`register_repository`

**Files:**
- Modify: `crates/transport/src/client.rs` (add one `ClientError` variant)
- Modify: `crates/client/src/actions.rs` (add two functions)
- Modify: `crates/client/src/session.rs` (add two `Session` methods)
- Modify: `crates/cli/src/main.rs:749-772,817-838` (`RootCmd::Add`, `RepoCmd::Register`)
- Test: `crates/client/tests/against_a_real_daemon.rs`

**Interfaces:**
- Consumes: nothing from Task 1 directly (independent RPCs), but follows the same `actions.rs` module and `Link::client_mut()` pattern Task 1 established.
- Produces: `actions::{add_repository_root, register_repository}`, `Session::{add_repository_root, register_repository}` — Task 6 (iOS UI) consumes these via new `Connection.swift` wrappers built on the FFI arms from Task 3.

- [ ] **Step 1: Add a `ClientError` variant for an unexpected result shape**

`ClientError` (`crates/transport/src/client.rs:25-40`) has no variant for "the daemon answered, but not with the resource this call asked for" — every existing `Session` method that needs this distinction (e.g. `create_workspace`) gets a plain `result::Value` back from `self.value(...)` and matches it locally into `SessionError::WrongResult`, never going through `ClientError` at all. `actions.rs`'s new functions, by contrast, need this to be part of `Result<T, ClientError>` itself, since `ClientError` is the only error type they can return without inventing a second one just for two functions.

In `crates/transport/src/client.rs`, replace lines 25-40:

```rust
pub enum ClientError {
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error("could not reach the daemon: {0}")]
    Connect(#[source] std::io::Error),
    #[error("the daemon closed the connection")]
    Closed,
    #[error("the daemon did not answer with a ServerHello")]
    NoHello,
    #[error("the daemon speaks protocol {daemon}, this client speaks {client}")]
    VersionMismatch { daemon: u32, client: u32 },
    #[error("{message}")]
    Daemon { code: i32, retryable: bool, message: String },
    #[error("the daemon returned an empty result")]
    EmptyResult,
}
```

with:

```rust
pub enum ClientError {
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error("could not reach the daemon: {0}")]
    Connect(#[source] std::io::Error),
    #[error("the daemon closed the connection")]
    Closed,
    #[error("the daemon did not answer with a ServerHello")]
    NoHello,
    #[error("the daemon speaks protocol {daemon}, this client speaks {client}")]
    VersionMismatch { daemon: u32, client: u32 },
    #[error("{message}")]
    Daemon { code: i32, retryable: bool, message: String },
    #[error("the daemon returned an empty result")]
    EmptyResult,
    #[error("the daemon returned {got} where {expected} was expected")]
    WrongResult { expected: &'static str, got: &'static str },
}
```

Run: `cd /Users/e-liang/Dev/overnight && cargo check -p farcooler-transport 2>&1 | tail -30`
Expected: clean build (nothing constructs this variant yet).

- [ ] **Step 2: Add the two functions to `actions.rs`**

Append to `crates/client/src/actions.rs` (extend the existing `use` line to add `RepositoryRootAdd`, `RepositoryRegister` — full updated import list, and the two new functions):

```rust
use farcooler_protocol::v1::{Repository, RepositoryRoot, request, result};
```
becomes
```rust
use farcooler_protocol::v1::{
    Repository, RepositoryRoot, RepositoryRegister, RepositoryRootAdd, request, result,
};
```

New functions, appended after `remove_worktree`:

```rust
/// Allowlist a folder Far Cooler may operate under. Returns the new root.
pub async fn add_repository_root<R, W>(
    client: &mut Client<R, W>,
    absolute_path: &str,
) -> Result<RepositoryRoot, ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let mut request = farcooler_transport::request("repository_root.add");
    request.payload = Some(request::Payload::RepositoryRootAdd(RepositoryRootAdd {
        absolute_path: absolute_path.to_string(),
        typed_confirmation: String::new(),
    }));
    match client.call(request).await?.value {
        Some(result::Value::RepositoryRoot(root)) => Ok(root),
        _ => Err(ClientError::WrongResult { expected: "repository_root", got: "something else" }),
    }
}

/// Register an existing repository inside an already-allowlisted root.
/// `relative_path` is the repository's absolute path — the daemon resolves
/// it against whichever registered root covers it, same as the CLI's own
/// `repo register` has always done.
pub async fn register_repository<R, W>(
    client: &mut Client<R, W>,
    relative_path: &str,
) -> Result<Repository, ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let mut request = farcooler_transport::request("repository.register");
    request.payload = Some(request::Payload::RepositoryRegister(RepositoryRegister {
        relative_path: relative_path.to_string(),
    }));
    match client.call(request).await?.value {
        Some(result::Value::Repository(repo)) => Ok(repo),
        _ => Err(ClientError::WrongResult { expected: "repository", got: "something else" }),
    }
}
```

- [ ] **Step 3: Add `Session` methods**

In `crates/client/src/session.rs`, add after the new `remove_worktree` method (from Task 1, Step 3):

```rust
    pub async fn add_repository_root(
        &mut self,
        absolute_path: &str,
    ) -> Result<RepositoryRoot, SessionError> {
        Ok(crate::actions::add_repository_root(&mut self.client, absolute_path).await?)
    }

    pub async fn register_repository(
        &mut self,
        relative_path: &str,
    ) -> Result<Repository, SessionError> {
        Ok(crate::actions::register_repository(&mut self.client, relative_path).await?)
    }
```

(`Repository`/`RepositoryRoot` are already imported at the top of `session.rs` per the existing `use farcooler_protocol::v1::{..., Repository, RepositoryRoot, ...}` import line.)

- [ ] **Step 4: Build**

Run: `cd /Users/e-liang/Dev/overnight && cargo build -p farcooler-client 2>&1 | tail -50`
Expected: clean build.

- [ ] **Step 5: Refactor the CLI's two call sites**

In `crates/cli/src/main.rs`, replace lines 749-772 (`RootCmd::Add`):

```rust
        RootCmd::Add { path } => {
            // Canonicalised here so the daemon is handed an absolute path
            // regardless of which directory the user ran this from.
            let absolute = path.canonicalize().unwrap_or(path);
            let r = link
                .call(with(
                    req("repository_root.add"),
                    request::Payload::RepositoryRootAdd(
                        farcooler_protocol::v1::RepositoryRootAdd {
                            absolute_path: absolute.to_string_lossy().into_owned(),
                            typed_confirmation: String::new(),
                        },
                    ),
                ))
                .await?;
            let result::Value::RepositoryRoot(root) = expect_value(r.value, "root")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            println!(
                "added root {}  {}",
                short_bytes(&root.id),
                root.display_path.unwrap_or_else(|| root.path_token.clone())
            );
        }
```

with:

```rust
        RootCmd::Add { path } => {
            // Canonicalised here so the daemon is handed an absolute path
            // regardless of which directory the user ran this from.
            let absolute = path.canonicalize().unwrap_or(path);
            let root = farcooler_client::actions::add_repository_root(
                link.client_mut(),
                &absolute.to_string_lossy(),
            )
            .await?;
            println!(
                "added root {}  {}",
                short_bytes(&root.id),
                root.display_path.unwrap_or_else(|| root.path_token.clone())
            );
        }
```

And replace lines 817-838 (`RepoCmd::Register`):

```rust
        RepoCmd::Register { path } => {
            let absolute = path.canonicalize().unwrap_or(path);
            let r = link
                .call(with(
                    req("repository.register"),
                    request::Payload::RepositoryRegister(
                        farcooler_protocol::v1::RepositoryRegister {
                            relative_path: absolute.to_string_lossy().into_owned(),
                        },
                    ),
                ))
                .await?;
            let result::Value::Repository(repo) = expect_value(r.value, "repository")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            println!(
                "registered {}  {}  ({})",
                short_bytes(&repo.id),
                repo.display_name,
                repo.remote_summary
            );
        }
```

with:

```rust
        RepoCmd::Register { path } => {
            let absolute = path.canonicalize().unwrap_or(path);
            let repo = farcooler_client::actions::register_repository(
                link.client_mut(),
                &absolute.to_string_lossy(),
            )
            .await?;
            println!(
                "registered {}  {}  ({})",
                short_bytes(&repo.id),
                repo.display_name,
                repo.remote_summary
            );
        }
```

- [ ] **Step 6: Build**

Run: `cd /Users/e-liang/Dev/overnight && cargo build -p farcooler-cli 2>&1 | tail -50`
Expected: clean build. If `expect_value`/`request`/`with`/`req` become unused in `main.rs`, check other call sites still using them (`RootCmd::Remove` at line 773-784 still builds a request inline and uses `with`/`req_for`/`request::Payload::TypedConfirmation` — leave those imports).

- [ ] **Step 7: Integration test**

In `crates/client/tests/against_a_real_daemon.rs`, add:

```rust
#[tokio::test]
async fn adding_a_root_and_registering_a_repository_round_trips() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }

    let root = session
        .add_repository_root(&dir.path().to_string_lossy())
        .await
        .expect("add_repository_root");
    assert_eq!(root.repository_count, 0);

    let registered = session
        .register_repository(&repo.to_string_lossy())
        .await
        .expect("register_repository");
    assert!(!registered.display_name.is_empty());

    let repositories = session.repositories().await.expect("repositories");
    assert_eq!(repositories.len(), 1);
}
```

- [ ] **Step 8: Run tests**

Run: `cd /Users/e-liang/Dev/overnight && cargo test -p farcooler-client --test against_a_real_daemon 2>&1 | tail -60`
Expected: all tests pass, including the new one.

- [ ] **Step 9: Commit**

```bash
git add crates/client/src/actions.rs crates/client/src/session.rs crates/cli/src/main.rs \
  crates/client/tests/against_a_real_daemon.rs
git commit -m "$(cat <<'EOF'
refactor(client,cli): share add-repository-root/register between CLI and mobile

Same deduplication as the previous commit, for the two calls Add
Repository needs. macOS needs no changes here either.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: FFI arms for all three new/changed methods

**Files:**
- Modify: `crates/client/src/ffi.rs:410-417` (replace the two existing hide/unhide arms — behavior unchanged, but confirm they still compile against the now-delegating `Session` methods) and add three new arms.

**Interfaces:**
- Consumes: `Session::{remove_worktree, add_repository_root, register_repository}` from Tasks 1-2.
- Produces: FFI methods `"workspace.remove_worktree"`, `"repository_root.add"`, `"repository.register"` — Task 4 (`Connection.swift`) consumes these by name via `core.call(_:_:)`.

- [ ] **Step 1: Add the three new arms**

In `crates/client/src/ffi.rs`, inside the `match method { ... }` block in `dispatch`, add after the existing `"workspace.unhide"` arm (ffi.rs:414-417):

```rust
        "workspace.remove_worktree" => {
            use farcooler_client::actions::RemoveWorktreeOutcome;
            match session
                .remove_worktree(id("workspace")?, &text("confirm"))
                .await
                .map_err(|e| e.to_string())?
            {
                RemoveWorktreeOutcome::Removed => Ok(json!({ "ok": true })),
                RemoveWorktreeOutcome::ConfirmationRequired => {
                    Ok(json!({ "confirmationRequired": true }))
                }
            }
        }

        "repository_root.add" => {
            let root = session
                .add_repository_root(&text("path"))
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "id": uuid_of(&root.id).to_string(),
                "displayPath": root.display_path,
            }))
        }

        "repository.register" => {
            let repo = session
                .register_repository(&text("path"))
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "id": uuid_of(&repo.id).to_string(),
                "displayName": repo.display_name,
            }))
        }
```

Note `use farcooler_client::actions::RemoveWorktreeOutcome;` inside the arm — check whether `ffi.rs` is itself part of the `farcooler_client` crate (i.e. `crate::actions::RemoveWorktreeOutcome` rather than `farcooler_client::actions::...`, since `ffi.rs` and `actions.rs` are siblings in the same crate per Task 1's file layout) and use whichever path form the crate's existing internal `use crate::...` statements elsewhere in `ffi.rs` follow.

- [ ] **Step 2: Build**

Run: `cd /Users/e-liang/Dev/overnight && cargo build -p farcooler-client 2>&1 | tail -50`
Expected: clean build.

- [ ] **Step 3: Rebuild the iOS xcframeworks**

The iOS app links these as prebuilt `.xcframework`s — a Rust-only build does not update what Xcode sees.

Run: `cd /Users/e-liang/Dev/overnight && ./scripts/build-ios-frameworks.sh --device 2>&1 | tail -40`
Expected: `Built: farcooler_client.xcframework farcooler_vt.xcframework`, both with device+simulator slices (matching how these were built earlier tonight for the same reason).

- [ ] **Step 4: Commit**

```bash
git add crates/client/src/ffi.rs
git commit -m "$(cat <<'EOF'
feat(client): expose remove_worktree/add-repository-root/register over FFI

Three new dispatch arms so iOS can reach the shared actions added in
the last two commits. remove_worktree's confirmation-required outcome
arrives as {"confirmationRequired": true} inside a successful FFI
reply rather than as a transport error, since it's a domain outcome a
caller branches on, not a failure -- core.call's try/catch would
otherwise make it indistinguishable from every other refusal.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

(The rebuilt `.xcframework`s are build output, not source — do not commit `apps/ios/Frameworks/`; check `.gitignore` covers it, matching how it was treated earlier tonight.)

---

### Task 4: `Connection.swift` — five new wrapper methods

**Files:**
- Modify: `apps/ios/FarCooler/Connection.swift` (add methods after `createTerminal`, around line 347)

**Interfaces:**
- Consumes: FFI methods from Task 3.
- Produces: `Connection.createTerminal(workspace:) async`, `Connection.hideWorkspace(_:) async`, `Connection.unhideWorkspace(_:) async`, `Connection.removeWorktree(_:confirm:) async -> RemoveWorktreeResult`, `Connection.addRepositoryRoot(path:) async throws`, `Connection.registerRepository(path:) async throws -> String` (repository id) — Tasks 5-7 (UI) consume all of these.

- [ ] **Step 1: Add the methods**

In `apps/ios/FarCooler/Connection.swift`, insert after `createTerminal` (after line 347, the closing brace of the existing `createTerminal(workspace:title:preset:)` method):

```swift
    /// A second (or third) terminal in a worktree you already have open.
    /// Always a shell — matching macOS's own "New terminal", which has never
    /// offered a preset picker either.
    func createTerminal(workspace: Workspace) async {
        _ = try? await createTerminal(
            workspace: workspace.id,
            title: "Terminal \(workspace.terminals.count + 1)",
            preset: "shell")
        await refresh()
    }

    func hideWorkspace(_ workspace: Workspace) async {
        _ = try? await core.call("workspace.hide", ["workspace": workspace.id])
        await refresh()
    }

    func unhideWorkspace(_ workspace: Workspace) async {
        _ = try? await core.call("workspace.unhide", ["workspace": workspace.id])
        await refresh()
    }

    /// What asking to remove a worktree came back with — mirrors macOS's
    /// `DaemonClient.RemoveWorktreeResult` so both apps' UIs make the same
    /// three-way distinction.
    enum RemoveWorktreeResult {
        case ok
        case confirmationRequired
        case failed(String)
    }

    /// `confirm` must be the workspace's exact task name, unless the
    /// worktree is clean, in which case it may be empty.
    func removeWorktree(_ workspace: Workspace, confirm: String) async -> RemoveWorktreeResult {
        let data: Data
        do {
            data = try await core.call(
                "workspace.remove_worktree", ["workspace": workspace.id, "confirm": confirm])
        } catch {
            await refresh()
            return .failed(error.localizedDescription)
        }
        struct Reply: Decodable {
            var ok: Bool?
            var confirmationRequired: Bool?
        }
        let reply = (try? JSONDecoder().decode(Reply.self, from: data)) ?? Reply()
        await refresh()
        if reply.confirmationRequired == true { return .confirmationRequired }
        return .ok
    }

    /// Allowlist the folder a repository lives in. Always a remote host's
    /// path from this app — a phone has no filesystem of its own worth
    /// pointing at.
    private struct RootReply: Decodable { var id: String }
    func addRepositoryRoot(path: String) async throws {
        _ = try await core.call("repository_root.add", ["path": path])
    }

    /// Register the repository itself, once its parent folder is
    /// allowlisted. Hands back the new repository's id, so the caller can
    /// select it immediately.
    func registerRepository(path: String) async throws -> String {
        let data = try await core.call("repository.register", ["path": path])
        return try JSONDecoder().decode(IdentifiedReply.self, from: data).id
    }
```

`IdentifiedReply` is the existing `private struct IdentifiedReply: Decodable { var id: String }` already defined above `createWorkspace(4-arg)` — reused here, not redefined. `RootReply` above is unused by name (its shape is never decoded — `addRepositoryRoot` discards the reply entirely since the caller only needs to know it succeeded before calling `registerRepository`); remove the `RootReply` struct declaration if the compiler warns it dead, keeping only what's used.

- [ ] **Step 2: Build**

Run: `cd /Users/e-liang/Dev/overnight/apps/ios && xcodebuild -project FarCooler.xcodeproj -scheme FarCooler -configuration Debug -destination "generic/platform=iOS Simulator" build 2>&1 | tail -60`
Expected: `** BUILD SUCCEEDED **`. Fix any type errors against the real `ClientCore.call` signature before continuing — this task adds no UI, so nothing is manually testable yet; a clean build is the only available verification.

- [ ] **Step 3: Commit**

```bash
git add apps/ios/FarCooler/Connection.swift
git commit -m "$(cat <<'EOF'
feat(ios): Connection wrappers for the four new worktree actions

Thin wrappers over the FFI arms added in the previous commit, matching
the existing createWorkspace/createTerminal conventions in this file.
No UI yet -- wired up in the next few commits.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: New terminal + ellipsis menu scaffold + Hide/Unhide

**Files:**
- Modify: `apps/ios/FarCooler/Model.swift:21-28` (add `isHidden`/`worktreeMissing`/`isMainCheckout`)
- Modify: `apps/ios/FarCooler/FleetView.swift` (`FleetList`, `WorkspaceListView`, `TerminalRow` region)

**Interfaces:**
- Consumes: `Connection.createTerminal(workspace:)`, `.hideWorkspace(_:)`, `.unhideWorkspace(_:)` from Task 4.
- Produces: `FleetList` gains a `connection: Connection` parameter (replacing the narrower `onAction` plumbing with something the header menu can also call) and an ellipsis menu per worktree section — Tasks 6-7 add more items to this same menu.

- [ ] **Step 1: Extend `Workspace`**

In `apps/ios/FarCooler/Model.swift`, replace lines 21-28:

```swift
struct Workspace: Decodable, Identifiable, Hashable {
    var id: String
    var short: String
    var task: String
    var branch: String
    var worktree: String?
    var state: String
    var terminals: [Terminal]
```

with:

```swift
struct Workspace: Decodable, Identifiable, Hashable {
    var id: String
    var short: String
    var task: String
    var branch: String
    var worktree: String?
    var state: String
    var terminals: [Terminal]

    /// The user asked not to see this one.
    var isHidden: Bool { state == "hidden" }

    /// git no longer lists this worktree, but the row carries terminals.
    var worktreeMissing: Bool { state == "worktree_missing" }

    /// Whether this workspace IS the repository's own checkout — offering to
    /// remove it would offer to delete the directory the repository itself
    /// lives in. Optional because an older daemon never sent this key, and
    /// decoding must not fail the entire fleet over one absent field.
    var isMainCheckout: Bool { is_main_checkout ?? false }
    // swiftlint:disable:next identifier_name
    var is_main_checkout: Bool?
```

- [ ] **Step 2: Build to confirm the decode still works against a real daemon's JSON**

Run: `cd /Users/e-liang/Dev/overnight/apps/ios && xcodebuild -project FarCooler.xcodeproj -scheme FarCooler -configuration Debug -destination "generic/platform=iOS Simulator" build 2>&1 | tail -40`
Expected: `** BUILD SUCCEEDED **`.

- [ ] **Step 3: Thread `connection` into `FleetList` and add the ellipsis menu**

In `apps/ios/FarCooler/FleetView.swift`, replace the `FleetList` struct (lines 552-613):

```swift
struct FleetList: View {
    let fleet: Fleet
    let onSelect: (Terminal) -> Void
    let onAction: (Connection.Action, Terminal) -> Void

    var body: some View {
        List {
            if fleet.workspaces.isEmpty {
                Text("No workspaces on this machine.")
                    .foregroundStyle(.secondary)
            }

            ForEach(fleet.workspaces) { workspace in
                let numbering = workspace.ordinals()
                Section {
                    ForEach(workspace.terminals) { terminal in
                        Button { onSelect(terminal) } label: {
                            TerminalRow(terminal: terminal, ordinal: numbering[terminal.id]) { action in
                                onAction(action, terminal)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                    if workspace.terminals.isEmpty {
                        Text("No terminals").font(.callout).foregroundStyle(.secondary)
                    }
                } header: {
                    HStack {
                        Text(workspace.task)
                        Spacer()
                        Text(workspace.branch)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                    }
                }
            }

            Section {
                HStack {
                    Circle()
                        .fill(fleet.runtimeHealthy ? .green : .orange)
                        .frame(width: 8, height: 8)
                    Text(
                        fleet.runtimeHealthy
                            ? "\(fleet.livePanes) live"
                            : "tmux unavailable on this host"
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
        }
    }
}
```

with (shown workspaces here; hidden ones and the disclosure section are Task 6):

```swift
struct FleetList: View {
    let fleet: Fleet
    @ObservedObject var connection: Connection
    let onSelect: (Terminal) -> Void
    let onAction: (Connection.Action, Terminal) -> Void
    var onRemove: (Workspace) -> Void = { _ in }

    private var shown: [Workspace] { fleet.workspaces.filter { !$0.isHidden } }

    var body: some View {
        List {
            if fleet.workspaces.isEmpty {
                Text("No workspaces on this machine.")
                    .foregroundStyle(.secondary)
            }

            ForEach(shown) { workspace in
                let numbering = workspace.ordinals()
                Section {
                    ForEach(workspace.terminals) { terminal in
                        Button { onSelect(terminal) } label: {
                            TerminalRow(terminal: terminal, ordinal: numbering[terminal.id]) { action in
                                onAction(action, terminal)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                    if workspace.terminals.isEmpty {
                        Text("No terminals").font(.callout).foregroundStyle(.secondary)
                    }
                    Button {
                        Task { await connection.createTerminal(workspace: workspace) }
                    } label: {
                        Label("New terminal", systemImage: "plus")
                    }
                    .font(.callout)
                } header: {
                    HStack {
                        Text(workspace.task)
                        Spacer()
                        Text(workspace.branch)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                        Menu {
                            Button {
                                Task { await connection.createTerminal(workspace: workspace) }
                            } label: {
                                Label("New terminal", systemImage: "plus")
                            }
                            if workspace.isHidden {
                                Button {
                                    Task { await connection.unhideWorkspace(workspace) }
                                } label: {
                                    Label("Unhide", systemImage: "eye")
                                }
                            } else {
                                Button {
                                    Task { await connection.hideWorkspace(workspace) }
                                } label: {
                                    Label("Hide", systemImage: "eye.slash")
                                }
                            }
                            if !workspace.isMainCheckout {
                                Button(role: .destructive) {
                                    onRemove(workspace)
                                } label: {
                                    Label("Remove Worktree…", systemImage: "trash")
                                }
                            }
                        } label: {
                            Image(systemName: "ellipsis.circle")
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }

            Section {
                HStack {
                    Circle()
                        .fill(fleet.runtimeHealthy ? .green : .orange)
                        .frame(width: 8, height: 8)
                    Text(
                        fleet.runtimeHealthy
                            ? "\(fleet.livePanes) live"
                            : "tmux unavailable on this host"
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
        }
    }
}
```

The section body also gets its own plain-text "New terminal" row (not just the header menu item) — that mirrors macOS exactly, which offers the same action in both the row-list button (`SidebarViews.swift:184-197`) and the ellipsis menu (`:270`), and is a bigger, easier target than the small header menu on a touch screen.

`onRemove` defaults to a no-op closure so this compiles before Task 7 wires the real one in — that default is temporary scaffolding removed in Task 7's edit, not a permanent placeholder (Task 7 will change the default-less call site in `WorkspaceListView`, at which point the empty default here becomes dead code worth deleting at that point).

- [ ] **Step 4: Update `FleetList`'s one caller**

In `apps/ios/FarCooler/FleetView.swift`, `WorkspaceListView`'s `body` (around line 1145) currently:

```swift
        FleetList(fleet: connection.fleet, onSelect: onSelect) { action, terminal in
            Task { await connection.act(action, on: terminal) }
        }
```

becomes:

```swift
        FleetList(fleet: connection.fleet, connection: connection, onSelect: onSelect) { action, terminal in
            Task { await connection.act(action, on: terminal) }
        }
```

- [ ] **Step 5: Build**

Run: `cd /Users/e-liang/Dev/overnight/apps/ios && xcodebuild -project FarCooler.xcodeproj -scheme FarCooler -configuration Debug -destination "generic/platform=iOS Simulator" build 2>&1 | tail -60`
Expected: `** BUILD SUCCEEDED **`.

- [ ] **Step 6: Manual check against a real connected host**

Run the app on a simulator or device connected to a real host with at least one worktree. Open the worktree sheet, confirm: the ellipsis menu appears in each section header; "New terminal" (either the header menu item or the row button) adds a new terminal to that same worktree, visible in the list after it refreshes; "Hide" removes a workspace from the visible list (a workspace vanishing without a "Hidden" section to find it in again is expected here — that section is Task 6); manually re-run with `workspace.unhide` via `farcooler workspace unhide <id>` from a terminal to restore it before continuing, since there is no UI path back yet.

- [ ] **Step 7: Commit**

```bash
git add apps/ios/FarCooler/Model.swift apps/ios/FarCooler/FleetView.swift
git commit -m "$(cat <<'EOF'
feat(ios): new terminal + hide/unhide on the worktree sheet

Adds the ellipsis menu macOS's sidebar has always had, starting with
the two actions that need no new UI beyond the menu itself. Remove
Worktree's menu item is wired to a no-op default for now -- its real
sheet is a separate commit.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Hidden section

**Files:**
- Modify: `apps/ios/FarCooler/FleetView.swift` (`FleetList`)

**Interfaces:**
- Consumes: `Workspace.isHidden` from Task 5.
- Produces: nothing new consumed elsewhere — this is a pure UI addition.

- [ ] **Step 1: Add the collapsed "Hidden N" section**

In `apps/ios/FarCooler/FleetView.swift`, `FleetList` (from Task 5's edit), add a `@State` for expansion and a computed `hidden` list, then a new `Section` before the final "N live" status section:

```swift
struct FleetList: View {
    let fleet: Fleet
    @ObservedObject var connection: Connection
    let onSelect: (Terminal) -> Void
    let onAction: (Connection.Action, Terminal) -> Void
    var onRemove: (Workspace) -> Void = { _ in }

    @State private var hiddenExpanded = false

    private var shown: [Workspace] { fleet.workspaces.filter { !$0.isHidden } }
    private var hidden: [Workspace] { fleet.workspaces.filter(\.isHidden) }
    private var hiddenAttention: Int {
        hidden.flatMap(\.terminals).filter(\.status.wantsAttention).count
    }
```

Then, immediately before the final "N live" `Section` in `body`, insert:

```swift
            if !hidden.isEmpty {
                Section {
                    if hiddenExpanded {
                        ForEach(hidden) { workspace in
                            let numbering = workspace.ordinals()
                            ForEach(workspace.terminals) { terminal in
                                Button { onSelect(terminal) } label: {
                                    TerminalRow(
                                        terminal: terminal, ordinal: numbering[terminal.id]
                                    ) { action in
                                        onAction(action, terminal)
                                    }
                                }
                                .buttonStyle(.plain)
                            }
                            HStack {
                                Text(workspace.task).font(.caption).foregroundStyle(.secondary)
                                Spacer()
                                Button("Unhide") {
                                    Task { await connection.unhideWorkspace(workspace) }
                                }
                                .font(.caption)
                            }
                        }
                    }
                } header: {
                    Button {
                        hiddenExpanded.toggle()
                    } label: {
                        HStack {
                            Image(systemName: hiddenExpanded ? "chevron.down" : "chevron.right")
                                .font(.caption2)
                            Text("Hidden")
                            Text("\(hidden.count)")
                                .foregroundStyle(.tertiary)
                            if hiddenAttention > 0 {
                                Circle().fill(Color.orange).frame(width: 6, height: 6)
                            }
                            Spacer()
                        }
                    }
                    .buttonStyle(.plain)
                }
            }
```

`Terminal.status.wantsAttention` and `TerminalRow` are both already used identically in the shown section above — no new types needed. `hidden`'s terminals reuse `TerminalRow` directly rather than a stripped-down variant, so restart/stop/dismiss swipe actions keep working on a hidden workspace's terminals exactly as they do on a visible one — hiding only ever changes visibility, never capability.

- [ ] **Step 2: Build**

Run: `cd /Users/e-liang/Dev/overnight/apps/ios && xcodebuild -project FarCooler.xcodeproj -scheme FarCooler -configuration Debug -destination "generic/platform=iOS Simulator" build 2>&1 | tail -60`
Expected: `** BUILD SUCCEEDED **`.

- [ ] **Step 3: Manual check**

Against a real host: hide a workspace (from Task 5's menu), confirm it now appears under a collapsed "Hidden 1" row instead of disappearing entirely; tap to expand, confirm its terminals show and are still individually actionable (restart/stop); tap "Unhide," confirm it returns to the main list. If that workspace has a terminal an agent is actively running in and wanting attention, confirm the orange dot appears on the collapsed "Hidden" row without expanding it — this may need a real agent turn in flight to test, or can be skipped if none is available at test time (note in the commit message if skipped).

- [ ] **Step 4: Commit**

```bash
git add apps/ios/FarCooler/FleetView.swift
git commit -m "$(cat <<'EOF'
feat(ios): collapse hidden worktrees into a Hidden N section

Matches macOS's HiddenWorktrees exactly: hiding moves a worktree out
of the way, never silences it -- an attention dot on the collapsed
row still surfaces an agent waiting on you inside it.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Remove Worktree — two-phase confirmation

**Files:**
- Create: a new `RemoveWorktreeConfirmSheet` view (add to `apps/ios/FarCooler/FleetView.swift`, near `NewWorkspaceView` — this app has no per-view-file convention, every sheet in this file lives in `FleetView.swift` today)
- Modify: `apps/ios/FarCooler/FleetView.swift` (`WorkspaceListView`)

**Interfaces:**
- Consumes: `Connection.removeWorktree(_:confirm:) -> RemoveWorktreeResult` from Task 4; `FleetList.onRemove` from Task 5.

- [ ] **Step 1: Add state and the first-phase confirmation dialog to `WorkspaceListView`**

In `apps/ios/FarCooler/FleetView.swift`, `WorkspaceListView`'s state (from the struct shown in Task 5's context) currently:

```swift
    @State private var showNewWorkspace = false
    @State private var showQuickTask = false
```

becomes:

```swift
    @State private var showNewWorkspace = false
    @State private var showQuickTask = false
    @State private var removeCandidate: Workspace?
    @State private var confirmingRemove = false
    @State private var needsTypedConfirmation: Workspace?
```

And its `body`'s `FleetList(...)` construction (from Task 5's edit) gains the real `onRemove`:

```swift
        FleetList(fleet: connection.fleet, connection: connection, onSelect: onSelect, onRemove: { ws in
            removeCandidate = ws
            confirmingRemove = true
        }) { action, terminal in
            Task { await connection.act(action, on: terminal) }
        }
```

Add, chained onto the same view (alongside the existing `.sheet(isPresented: $showNewWorkspace)`/`.sheet(isPresented: $showQuickTask)` modifiers):

```swift
        .confirmationDialog(
            "Remove worktree for \(removeCandidate?.task ?? "")?",
            isPresented: $confirmingRemove,
            titleVisibility: .visible
        ) {
            Button("Remove", role: .destructive) {
                guard let ws = removeCandidate else { return }
                Task {
                    switch await connection.removeWorktree(ws, confirm: "") {
                    case .ok:
                        break
                    case .confirmationRequired:
                        needsTypedConfirmation = ws
                    case .failed:
                        // The typed-name sheet also handles and displays a
                        // `.failed` result — route every non-.ok outcome
                        // there so there is one place this is shown, not two.
                        needsTypedConfirmation = ws
                    }
                }
            }
            Button("Cancel", role: .cancel) {}
        }
        .sheet(item: $needsTypedConfirmation) { ws in
            RemoveWorktreeConfirmSheet(workspace: ws) { typed in
                await connection.removeWorktree(ws, confirm: typed)
            }
        }
```

Note the `.failed` branch also routes to the typed-confirmation sheet rather than trying to show an error inline in the confirmation dialog (which has no room for one) — `RemoveWorktreeConfirmSheet` (next step) shows whatever the daemon actually said, which covers both "needs the name typed" and "refused for some other reason" in one place, matching how macOS's single `RemoveWorkspaceSheet` handles both.

- [ ] **Step 2: Write `RemoveWorktreeConfirmSheet`**

Add to `apps/ios/FarCooler/FleetView.swift`, near `NewWorkspaceView`:

```swift
/// The second phase: the worktree has uncommitted work, so removal needs the
/// task name typed exactly. Also where any other refusal surfaces, since
/// there is no room for an error message inside a confirmationDialog.
struct RemoveWorktreeConfirmSheet: View {
    let workspace: Workspace
    let onRemove: (String) async -> Connection.RemoveWorktreeResult

    @Environment(\.dismiss) private var dismiss
    @State private var typed = ""
    @State private var working = false
    @State private var errorMessage: String?

    private var matches: Bool { typed == workspace.task }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("This worktree has uncommitted work. Type its name to remove it anyway.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                TextField("Type \(workspace.task) to confirm", text: $typed)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                if let errorMessage {
                    Section {
                        Text(errorMessage).foregroundStyle(.red).font(.footnote)
                    }
                }
            }
            .navigationTitle("Remove worktree")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Remove", role: .destructive) {
                        working = true
                        Task {
                            switch await onRemove(typed) {
                            case .ok:
                                working = false
                                dismiss()
                            case .confirmationRequired:
                                working = false
                                errorMessage = "That name didn't match — try again."
                            case .failed(let message):
                                working = false
                                errorMessage = message
                            }
                        }
                    }
                    .disabled(!matches || working)
                }
            }
        }
    }
}
```

- [ ] **Step 3: Build**

Run: `cd /Users/e-liang/Dev/overnight/apps/ios && xcodebuild -project FarCooler.xcodeproj -scheme FarCooler -configuration Debug -destination "generic/platform=iOS Simulator" build 2>&1 | tail -60`
Expected: `** BUILD SUCCEEDED **`.

- [ ] **Step 4: Manual check against a real connected host**

Create a throwaway workspace (via Quick Task or New Workspace), leave it clean (no changes), remove it via the ellipsis menu → confirm via the dialog → confirm it disappears from the list with no typed-name sheet appearing. Create another, make an uncommitted change inside its worktree (from a terminal in the app, or externally on the host), remove it the same way → confirm the typed-name sheet appears this time → type the wrong name, confirm the Remove button stays disabled → type the exact task name → confirm it removes successfully. Also confirm the ellipsis menu has no "Remove Worktree…" item at all on the repository's main checkout.

- [ ] **Step 5: Commit**

```bash
git add apps/ios/FarCooler/FleetView.swift
git commit -m "$(cat <<'EOF'
feat(ios): remove worktree, with the same two-phase confirmation macOS has

A clean worktree removes with one tap-confirm. A dirty one escalates
to typing the exact task name, matching macOS's RemoveWorkspaceSheet --
a real data-loss guard, not UI ceremony, worth keeping identical
cross-platform.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: Add Repository

**Files:**
- Modify: `apps/ios/FarCooler/FleetView.swift` (`NewWorkspaceView`, plus a new `AddRepositorySheet` view)

**Interfaces:**
- Consumes: `Connection.addRepositoryRoot(path:)`, `.registerRepository(path:)` from Task 4.

- [ ] **Step 1: Write `AddRepositorySheet`**

Add to `apps/ios/FarCooler/FleetView.swift`, near `NewWorkspaceView`:

```swift
/// Registers a repository on a remote host. Always remote: this app has no
/// filesystem of its own worth pointing at, unlike macOS's version of this
/// sheet, which also offers a local file picker.
struct AddRepositorySheet: View {
    let connection: Connection
    /// Called with the new repository's id after a successful registration,
    /// so the caller can select it immediately.
    let onRegistered: (String) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var path = ""
    @State private var working = false
    @State private var errorMessage: String?

    private var canConfirm: Bool { !path.trimmingCharacters(in: .whitespaces).isEmpty && !working }

    var body: some View {
        NavigationStack {
            Form {
                TextField("Path on the host", text: $path)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Section {
                    Text("Far Cooler creates a worktree per task, so it needs an existing repository already on this host.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                if let errorMessage {
                    Section {
                        Text(errorMessage).foregroundStyle(.red).font(.footnote)
                    }
                }
            }
            .navigationTitle("Add repository")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") {
                        working = true
                        errorMessage = nil
                        Task {
                            do {
                                try await connection.addRepositoryRoot(path: path)
                                let id = try await connection.registerRepository(path: path)
                                working = false
                                onRegistered(id)
                                dismiss()
                            } catch {
                                working = false
                                errorMessage = error.localizedDescription
                            }
                        }
                    }
                    .disabled(!canConfirm)
                }
            }
        }
    }
}
```

Note this does not yet offer a host picker — per the spec, that's shown only when more than one machine is connected. `Connection` in this codebase is per-host already (one `Connection` instance per connected machine, per `WorkspaceListView`'s own `@ObservedObject var connection: Connection`), so "which host" is implicitly whichever `Connection` this sheet is constructed with — a host picker would only matter if `WorkspaceListView` itself is showing across multiple hosts at once. Check `HostStore`/`hosts` (referenced in `WorkspaceListView`'s `var hosts: HostStore?`) during this step to confirm whether the sheet is always scoped to one already-selected connection (in which case no picker is needed at all, and the spec's "host picker when >1 machine" was written before this was clear) or whether multiple hosts' repositories are meant to be reachable from one instance of this sheet — if the former, this step's implementation is already complete as written; if the latter, add a `Picker` at the top of the `Form` sourced from `hosts` before the `TextField`.

- [ ] **Step 2: Wire it into `NewWorkspaceView`**

In `apps/ios/FarCooler/FleetView.swift`, `NewWorkspaceView` currently:

```swift
struct NewWorkspaceView: View {
    let repositories: [Repository]
    let onCreate: (String, String, String) async -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var repository: String = ""
    @State private var task = ""
    @State private var branch = ""
    @State private var working = false
```

becomes:

```swift
struct NewWorkspaceView: View {
    let repositories: [Repository]
    let connection: Connection
    let onCreate: (String, String, String) async -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var repository: String = ""
    @State private var task = ""
    @State private var branch = ""
    @State private var working = false
    @State private var showAddRepository = false
```

and its `Form`'s `Picker` —

```swift
                Picker("Repository", selection: $repository) {
                    Text("Choose").tag("")
                    ForEach(repositories) { Text($0.displayName).tag($0.id) }
                }
```

— gains a sibling row right after it, inside the same `Form`:

```swift
                Picker("Repository", selection: $repository) {
                    Text("Choose").tag("")
                    ForEach(repositories) { Text($0.displayName).tag($0.id) }
                }
                Button("Add a repository…") { showAddRepository = true }
```

and the `Form` (or `NavigationStack`, whichever ends up the right scope — attach at the same level `.toolbar` is already attached, i.e. on the `Form` itself) gains:

```swift
                .sheet(isPresented: $showAddRepository) {
                    AddRepositorySheet(connection: connection) { newId in
                        repository = newId
                    }
                }
```

- [ ] **Step 3: Update `NewWorkspaceView`'s one call site**

In `WorkspaceListView`'s `body` (`apps/ios/FarCooler/FleetView.swift`, from Task 5's edit onward):

```swift
        .sheet(isPresented: $showNewWorkspace) {
            NewWorkspaceView(repositories: connection.repositories) { repository, task, branch in
                await connection.createWorkspace(repository: repository, task: task, branch: branch)
            }
        }
```

becomes:

```swift
        .sheet(isPresented: $showNewWorkspace) {
            NewWorkspaceView(repositories: connection.repositories, connection: connection) { repository, task, branch in
                await connection.createWorkspace(repository: repository, task: task, branch: branch)
            }
        }
```

- [ ] **Step 4: Build**

Run: `cd /Users/e-liang/Dev/overnight/apps/ios && xcodebuild -project FarCooler.xcodeproj -scheme FarCooler -configuration Debug -destination "generic/platform=iOS Simulator" build 2>&1 | tail -60`
Expected: `** BUILD SUCCEEDED **`.

- [ ] **Step 5: Manual check against a real connected host**

Open New Workspace, tap "Add a repository…", type the absolute path to a real git repository on the connected host that isn't registered yet, confirm it registers and the sheet dismisses back into New Workspace with that repository already selected in the picker. Confirm creating a workspace from it works normally afterward.

- [ ] **Step 6: Commit**

```bash
git add apps/ios/FarCooler/FleetView.swift
git commit -m "$(cat <<'EOF'
feat(ios): add a repository from the New Workspace form

Simpler than macOS's version of this sheet: a phone never has a local
repository of its own, so this only ever registers one on a remote
host -- a text field for the path, no file picker mode to build.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: Full-app end-to-end verification

**Files:** none.

- [ ] **Step 1: Full Rust build and test suite**

Run: `cd /Users/e-liang/Dev/overnight && cargo build --workspace 2>&1 | tail -60 && cargo test -p farcooler-client -p farcooler-cli -p farcooler-daemon 2>&1 | tail -100`
Expected: clean build, all tests pass across all three touched crates.

- [ ] **Step 2: Full iOS build**

Run: `cd /Users/e-liang/Dev/overnight/apps/ios && xcodebuild -project FarCooler.xcodeproj -scheme FarCooler -configuration Debug -destination "generic/platform=iOS Simulator" build 2>&1 | tail -60`
Expected: `** BUILD SUCCEEDED **`.

- [ ] **Step 3: macOS regression check**

Since `crates/cli` changed at five call sites that `DaemonClient.swift` shells out to, confirm macOS itself still works, not just the CLI in isolation: build and run the macOS app (`cd /Users/e-liang/Dev/overnight/apps/macos && export PATH="$HOME/.cargo/bin:$PATH" && ./build-app.sh`), and from its sidebar exercise all five: hide a worktree, unhide it, create a new terminal in it, remove a clean worktree, remove a dirty one (confirm the typed-name sheet still appears — this is the one that most directly exercises the refactored confirmation-required detection path).

- [ ] **Step 4: Full commit log review**

Run: `git log --oneline -10`
Expected: nine feature/refactor commits from this plan, in order, each with a clear message. No leftover uncommitted changes: `git status --short` shows nothing under `crates/client/`, `crates/cli/`, `apps/ios/FarCooler/`.
