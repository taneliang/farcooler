# Channel Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make dev, beta and release three installations that coexist on one machine and cannot see each other's state.

**Architecture:** A compile-time `Channel` stamped beside `FARCOOLER_BUILD` selects the runtime directory. Everything else — socket, database, install-id, and therefore the tmux server, worktrees and pastes — already hangs off that directory, so one function carries most of the isolation. The one thing that does not follow is worktree adoption, which takes anything `git worktree list` reports; that gets an ownership marker in git's per-worktree admin directory.

**Tech Stack:** Rust (daemon, protocol, client, cli crates), `directories`, `tempfile`, `tokio::test`, bash (`scripts/version.sh`).

Implements `docs/superpowers/specs/2026-08-11-release-channels-design.md`, sequence steps 1–3.

## Global Constraints

- US English throughout, in code and copy. Never "authorise", "colour", "centre".
- Never run `cargo fmt` in this tree. It is hand-formatted and CI skips `fmt --check` deliberately.
- `cargo clippy --workspace --all-targets -- -D warnings` must pass. This is what CI runs.
- Cargo is not on `PATH` by default in this environment; use `~/.cargo/bin/cargo`.
- Release-channel behaviour must be **byte-identical to today**. Nothing already installed may move, or an existing user's fleet disappears on upgrade.
- An unstamped or unknown channel is `dev`, never `release`. `scripts/version.sh` states the rule: defaulting the other way "would let a hand-made build pass itself off as a release."
- A runtime root must stay short enough that a per-terminal agent socket beneath it fits in `sun_path` — see `crates/daemon/src/agent_supervisor.rs:529`.
- Error copy is a sentence a person reads, never a raw Rust error. Say "machine", not "host".

---

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/protocol/build.rs` | Modify: stamp `FARCOOLER_CHANNEL` beside `FARCOOLER_BUILD` |
| `crates/protocol/src/lib.rs` | Modify: `Channel` enum, `CHANNEL` const, `daemon_binary_name`, `cli_binary_name` |
| `crates/daemon/src/paths.rs` | Modify: `runtime_dir_for(Channel)`; `runtime_dir()` delegates |
| `crates/daemon/src/git.rs` | Modify: `admin_dir`, `mark_owner`, `owner_of` |
| `crates/daemon/src/service.rs` | Modify: hold `install_id`; mark worktrees on create |
| `crates/daemon/src/reconcile.rs` | Modify: skip worktrees owned by another install |
| `crates/daemon/src/test_support.rs` | Modify: `two_daemons()` fixture |
| `crates/daemon/tests/channel_isolation.rs` | Create: the two-daemon isolation test |
| `crates/client/src/session.rs` | Modify: derive the remote binary name from the channel |
| `crates/cli/src/host_install.rs` | Modify: channel-aware install paths and service units |

---

### Task 1: Stamp the channel at compile time

**Files:**
- Modify: `crates/protocol/build.rs`
- Modify: `crates/protocol/src/lib.rs`

**Interfaces:**
- Produces: `farcooler_protocol::Channel` (`Dev | Beta | Release`), `farcooler_protocol::CHANNEL: Channel`, `Channel::as_str() -> &'static str`, `Channel::from_str_or_dev(&str) -> Channel`.

- [x] **Step 1: Write the failing test**

In `crates/protocol/src/lib.rs`, at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_round_trips_through_its_name() {
        for c in [Channel::Dev, Channel::Beta, Channel::Release] {
            assert_eq!(Channel::from_str_or_dev(c.as_str()), c);
        }
    }

    #[test]
    fn an_unknown_channel_is_dev_not_release() {
        // Defaulting the other way would let a hand-made build pass itself off
        // as a release. scripts/version.sh makes the same choice for the same
        // reason.
        assert_eq!(Channel::from_str_or_dev(""), Channel::Dev);
        assert_eq!(Channel::from_str_or_dev("nonsense"), Channel::Dev);
        assert_eq!(Channel::from_str_or_dev("RELEASE"), Channel::Dev);
    }

    #[test]
    fn the_stamped_channel_is_one_of_the_three() {
        assert!(matches!(CHANNEL, Channel::Dev | Channel::Beta | Channel::Release));
    }
}
```

- [x] **Step 2: Run it and watch it fail**

Run: `~/.cargo/bin/cargo test -p farcooler-protocol`
Expected: FAIL — `cannot find type Channel in this scope`.

- [x] **Step 3: Stamp `FARCOOLER_CHANNEL` in the build script**

In `crates/protocol/build.rs`, immediately after the `FARCOOLER_BUILD` `println!` block (which ends with the closing `);` at line 43):

```rust
    // Which channel this build belongs to, from the one implementation of the
    // question.
    //
    // Shelling out to `scripts/version.sh` rather than re-deriving it here: a
    // second implementation of "what channel is this" is exactly the drift that
    // script exists to prevent, and this build script already shells out to git
    // three times above.
    //
    // A missing or failing script is `dev`, deliberately and for the reason
    // version.sh gives about an unstamped bundle: defaulting the other way
    // would let a build made outside the release path call itself a release.
    println!("cargo:rerun-if-changed=../../scripts/version.sh");
    println!("cargo:rerun-if-changed=../../.git/refs/tags");
    let channel = std::process::Command::new("../../scripts/version.sh")
        .arg("channel")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "dev".to_string());
    println!("cargo:rustc-env=FARCOOLER_CHANNEL={channel}");
```

- [x] **Step 4: Add the type and the constant**

In `crates/protocol/src/lib.rs`, after the `BUILD` constant (line 32):

```rust
/// Which installation this build belongs to.
///
/// Three channels are three separate installs that coexist on one machine —
/// separate runtime directory, database, tmux server and binary name — so this
/// is not a label. It decides where the daemon lives.
///
/// Compile time, never a flag or an environment variable. `scripts/version.sh`
/// makes the same call about the channel it derives: "a flag is a thing to
/// forget on the build that mattered." A daemon that could be told which
/// channel it is could be told wrong, and being wrong means a beta writing into
/// the release database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Dev,
    Beta,
    Release,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Dev => "dev",
            Channel::Beta => "beta",
            Channel::Release => "release",
        }
    }

    /// Anything unrecognized is `dev`.
    ///
    /// The safe direction: a dev build isolates itself from everything, so
    /// guessing wrong costs a separate empty fleet. Guessing `release` wrong
    /// costs someone else's.
    pub fn from_str_or_dev(s: &str) -> Self {
        match s {
            "release" => Channel::Release,
            "beta" => Channel::Beta,
            _ => Channel::Dev,
        }
    }
}

/// The channel this binary was built for, stamped by `build.rs`.
pub const CHANNEL: Channel = Channel::from_str_or_dev_const(env!("FARCOOLER_CHANNEL"));

impl Channel {
    /// `from_str_or_dev` in a form a `const` can call.
    ///
    /// `match` on a `&str` is not permitted in a const context, so this
    /// compares bytes. Kept beside its runtime twin, and the round-trip test
    /// covers both.
    const fn from_str_or_dev_const(s: &str) -> Self {
        let b = s.as_bytes();
        match b {
            b"release" => Channel::Release,
            b"beta" => Channel::Beta,
            _ => Channel::Dev,
        }
    }
}
```

- [x] **Step 5: Run the tests**

Run: `~/.cargo/bin/cargo test -p farcooler-protocol`
Expected: PASS, all three.

- [x] **Step 6: Check clippy**

Run: `~/.cargo/bin/cargo clippy -p farcooler-protocol --all-targets -- -D warnings`
Expected: clean.

- [x] **Step 7: Commit**

```bash
git add crates/protocol/build.rs crates/protocol/src/lib.rs
git commit -m "feat(protocol): stamp the channel a build belongs to"
```

---

### Task 2: The runtime directory follows the channel

**Files:**
- Modify: `crates/daemon/src/paths.rs`

**Interfaces:**
- Consumes: `farcooler_protocol::{Channel, CHANNEL}` from Task 1.
- Produces: `paths::runtime_dir_for(Channel) -> Result<PathBuf>`. `runtime_dir()` keeps its signature and delegates to `runtime_dir_for(CHANNEL)`.

- [x] **Step 1: Write the failing tests**

In `crates/daemon/src/paths.rs`, inside the existing `mod tests`:

```rust
    #[test]
    fn each_channel_gets_its_own_runtime_directory() {
        // The whole isolation story rests on this: everything else the daemon
        // owns — socket, database, install-id and therefore the tmux server,
        // worktrees, pastes — is a path under this one.
        let dev = runtime_dir_for(Channel::Dev).unwrap();
        let beta = runtime_dir_for(Channel::Beta).unwrap();
        let release = runtime_dir_for(Channel::Release).unwrap();
        assert_ne!(dev, beta);
        assert_ne!(beta, release);
        assert_ne!(dev, release);
    }

    #[test]
    fn a_channel_directory_is_a_sibling_never_a_child() {
        // Nesting beta inside release would mean deleting a release install
        // deletes the beta's database with it.
        let beta = runtime_dir_for(Channel::Beta).unwrap();
        let release = runtime_dir_for(Channel::Release).unwrap();
        assert!(!beta.starts_with(&release), "{beta:?} must not sit inside {release:?}");
        assert!(!release.starts_with(&beta), "{release:?} must not sit inside {beta:?}");
    }

    #[test]
    fn release_keeps_the_directory_it_has_always_had() {
        // An existing install must not move. If this changes, every workspace
        // and terminal a user already has disappears on upgrade.
        let release = runtime_dir_for(Channel::Release).unwrap();
        let historic = directories::ProjectDirs::from("com", "farcooler", "FarCooler")
            .unwrap()
            .data_dir()
            .to_path_buf();
        assert_eq!(release, historic);
    }
```

- [x] **Step 2: Run them and watch them fail**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon paths::`
Expected: FAIL — `cannot find function runtime_dir_for`.

- [x] **Step 3: Implement**

Replace `runtime_dir` in `crates/daemon/src/paths.rs` (lines 11–19) with:

```rust
/// `~/Library/Application Support/com.farcooler.FarCooler` on macOS, for the
/// release channel.
///
/// `FARCOOLER_HOME` still overrides everything and is checked first: it is how
/// tests and scratch daemons get an isolated home, and a channel is only
/// consulted when nobody has named a directory outright.
pub fn runtime_dir() -> Result<PathBuf> {
    runtime_dir_for(farcooler_protocol::CHANNEL)
}

/// Where a given channel's install lives.
///
/// Three application names rather than one name with a suffix directory, so the
/// three are siblings: nesting a beta inside the release directory would mean
/// deleting a release install takes the beta's database with it.
///
/// Release's name is unchanged from before channels existed, and must stay that
/// way — an existing user's whole fleet is under it.
pub fn runtime_dir_for(channel: farcooler_protocol::Channel) -> Result<PathBuf> {
    use farcooler_protocol::Channel;
    if let Ok(over) = std::env::var("FARCOOLER_HOME") {
        return Ok(PathBuf::from(over));
    }
    let app = match channel {
        Channel::Release => "FarCooler",
        Channel::Beta => "FarCoolerBeta",
        Channel::Dev => "FarCoolerDev",
    };
    let dirs = directories::ProjectDirs::from("com", "farcooler", app)
        .ok_or(DomainError::OperationFailed)?;
    Ok(dirs.data_dir().to_path_buf())
}
```

Add to the test module's imports, at the top of `mod tests`:

```rust
    use farcooler_protocol::Channel;
```

- [x] **Step 4: Run the tests**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon paths::`
Expected: PASS, six tests (three new, three existing install-id ones).

- [x] **Step 5: Commit**

```bash
git add crates/daemon/src/paths.rs
git commit -m "feat(daemon): a channel chooses the runtime directory"
```

---

### Task 3: The isolation test, written against the bug

This task is expected to **end red**. The test it adds fails against current code, which is the point: it names the worktree-adoption hazard before Task 4 fixes it.

**Files:**
- Modify: `crates/daemon/src/test_support.rs`
- Create: `crates/daemon/tests/channel_isolation.rs`

**Interfaces:**
- Produces: `test_support::two_daemons() -> (TempDir, Arc<Service>, Arc<Service>, Uuid, Uuid)` — a temp dir, service A, service B, A's repository id, B's repository id, both registering the same repo on disk.

- [x] **Step 1: Add the two-daemon fixture**

Append to `crates/daemon/src/test_support.rs`:

```rust
/// Two services at two roots, both registering the SAME repository.
///
/// This is what two channels on one machine are. A channel's only job is to
/// choose a runtime directory, so two roots is two channels — and it is the
/// same shape `rpc_over_socket.rs` already relies on, where each test gets "a
/// daemon on a private socket with a private database" at an explicit
/// directory rather than through the process-global `FARCOOLER_HOME`.
pub(crate) async fn two_daemons()
-> (tempfile::TempDir, Arc<Service>, Arc<Service>, Uuid, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    for args in [
        vec!["init", "-q", "-b", "main", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        git::git(&repo, &args).await.unwrap();
    }

    let mut svcs = Vec::new();
    let mut repos = Vec::new();
    for tag in ["a", "b"] {
        let state = dir.path().join(tag);
        std::fs::create_dir_all(&state).unwrap();
        let svc = Arc::new(Service::open_in(state).await.unwrap());
        svc.add_root(dir.path()).await.unwrap();
        let registered = svc.register_repository(&repo).await.unwrap();
        svcs.push(svc);
        repos.push(registered.id);
    }

    let b = svcs.pop().unwrap();
    let a = svcs.pop().unwrap();
    (dir, a, b, repos[0], repos[1])
}
```

- [x] **Step 2: Write the isolation test**

Create `crates/daemon/tests/channel_isolation.rs`:

```rust
//! Two channels on one machine cannot see each other's state.
//!
//! Every claim in the channels design rests on this, and until now nothing
//! asserted it — while the whole test suite quietly depended on it.

mod support {
    include!("../src/test_support.rs");
}
```

That include trick does not work across the crate boundary. Instead, make the fixture reachable and put the test **inside** the crate. Create nothing in `tests/`; add this module at the bottom of `crates/daemon/src/reconcile.rs`, after the existing `mod tests`:

```rust
/// Two installs on one machine do not adopt each other's worktrees.
///
/// This is the property the channels design rests on, and the reason worktrees
/// need an owner at all: `list_worktrees` reports what GIT knows, and git knows
/// every worktree of a repository regardless of which daemon made it.
#[cfg(test)]
mod isolation_tests {
    use crate::test_support::two_daemons;

    #[tokio::test]
    async fn a_worktree_one_install_made_is_not_adopted_by_another() {
        let (_dir, a, b, repo_a, repo_b) = two_daemons().await;

        let ws = a.create_workspace(repo_a, "rate limiting", "feat/rate", "HEAD").await.unwrap();

        let outcome = super::repository(&b, repo_b).await.unwrap();
        assert_eq!(
            outcome.adopted, 0,
            "the other install's worktree must not be adopted: two daemons in one \
             directory means two agents writing the same files"
        );

        let seen = b.store.list_workspaces_for_repository(repo_b).unwrap();
        assert!(
            !seen.iter().any(|w| w.worktree_path == ws.worktree_path),
            "it must not appear in the other install's fleet either"
        );
    }

    #[tokio::test]
    async fn two_installs_have_different_identities() {
        let (_dir, a, b, _, _) = two_daemons().await;
        assert_ne!(
            a.install_id(),
            b.install_id(),
            "the tmux server is `tmux -L farcooler-<install-id>`, so equal ids \
             would mean one tmux server and therefore shared panes"
        );
        assert_ne!(a.host_id, b.host_id);
    }

    #[tokio::test]
    async fn a_hand_made_worktree_is_still_adopted() {
        // Adoption exists so a worktree someone made by hand is picked up.
        // Ownership must not cost that: an UNMARKED worktree belongs to nobody
        // and is fair game, and only a DIFFERENT install's mark is a refusal.
        let (dir, _a, b, _, repo_b) = two_daemons().await;
        let by_hand = dir.path().join("by-hand");
        crate::git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-b", "manual", &by_hand.to_string_lossy(), "HEAD"],
        )
        .await
        .unwrap();

        let outcome = super::repository(&b, repo_b).await.unwrap();
        assert_eq!(outcome.adopted, 1, "an unmarked worktree is still adopted");
    }
}
```

- [x] **Step 3: Expose `install_id` on `Service`**

The test needs it, and Task 4 needs it to compare owners. In `crates/daemon/src/service.rs`, add a field to `pub struct Service` beside `host_id`:

```rust
    /// This install's id, the same string the tmux server is named after.
    ///
    /// Held rather than re-read for the reason `root` is: the file it comes
    /// from is under a directory the environment can move.
    install_id: String,
```

In `Service::open_in`, the local `install_id` already exists at line 359; add it to the struct literal, and add the accessor beside the other small ones:

```rust
    pub fn install_id(&self) -> &str {
        &self.install_id
    }
```

- [x] **Step 4: Run the tests and confirm the RIGHT one fails**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon isolation_tests`
Expected:
- `two_installs_have_different_identities` — PASS.
- `a_hand_made_worktree_is_still_adopted` — PASS.
- `a_worktree_one_install_made_is_not_adopted_by_another` — **FAIL**, `assertion (left == right) failed: left: 1, right: 0`.

That failure is the bug the channels design names. Do not fix it here.

- [x] **Step 5: Commit the red test**

```bash
git add crates/daemon/src/test_support.rs crates/daemon/src/service.rs crates/daemon/src/reconcile.rs
git commit -m "test(daemon): two installs must not adopt each other's worktrees

Fails against current code, which is the point: reconcile adopts whatever
git reports, and git reports every worktree of a repository no matter which
daemon made it."
```

---

### Task 4: Give a worktree an owner

**Files:**
- Modify: `crates/daemon/src/git.rs`
- Modify: `crates/daemon/src/service.rs`
- Modify: `crates/daemon/src/reconcile.rs`

**Interfaces:**
- Consumes: `Service::install_id()` from Task 3.
- Produces: `git::mark_owner(&Path, &str) -> Result<()>`, `git::owner_of(&Path) -> Option<String>`.

- [x] **Step 1: Add the git helpers**

Append to `crates/daemon/src/git.rs`:

```rust
/// The name of the marker file, inside git's own admin directory.
const OWNER_MARKER: &str = "farcooler-install-id";

/// Git's admin directory for a worktree.
///
/// `<repo>/.git/worktrees/<name>` for a linked worktree, `<repo>/.git` for the
/// main checkout. The right place for a marker: invisible to `git status`, so
/// it never appears in anyone's diff, and removed by `git worktree prune` along
/// with the worktree it describes.
///
/// Not `git config --worktree`, which needs `extensions.worktreeConfig` turned
/// on for the whole repository — a change with its own effects on how
/// `core.worktree` resolves, and not ours to make to someone's repo.
async fn admin_dir(worktree: &Path) -> Result<PathBuf> {
    let r = git(worktree, &["rev-parse", "--absolute-git-dir"]).await?;
    if !r.ok {
        return Err(DomainError::OperationFailed);
    }
    Ok(PathBuf::from(r.stdout.trim()))
}

/// Record which install owns a worktree.
///
/// Best effort by design: a marker that could not be written means the worktree
/// looks unowned, which degrades to the behaviour that existed before ownership
/// did. Failing workspace creation over it would be worse than the hazard.
pub async fn mark_owner(worktree: &Path, install_id: &str) {
    match admin_dir(worktree).await {
        Ok(dir) => {
            if let Err(e) = std::fs::write(dir.join(OWNER_MARKER), install_id) {
                tracing::warn!(error = %e, "could not mark worktree ownership");
            }
        }
        Err(e) => tracing::warn!(error = ?e, "could not locate the worktree admin dir"),
    }
}

/// Which install owns a worktree, if any claims it.
///
/// `None` means unowned — a worktree someone made by hand — and unowned is
/// adoptable. Only a mark naming a DIFFERENT install is a refusal.
pub async fn owner_of(worktree: &Path) -> Option<String> {
    let dir = admin_dir(worktree).await.ok()?;
    let s = std::fs::read_to_string(dir.join(OWNER_MARKER)).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}
```

- [x] **Step 2: Mark worktrees the daemon creates**

In `crates/daemon/src/service.rs`, in `create_workspace`, immediately after the `git::create_worktree(...)` call succeeds:

```rust
        git::mark_owner(&dest, &self.install_id).await;
```

Do the same in `create_workspace_from_branch`, after its worktree is created. Find it with:

```bash
grep -n "create_worktree_from_branch(" crates/daemon/src/service.rs
```

- [x] **Step 3: Teach adoption to respect the owner**

In `crates/daemon/src/reconcile.rs`, inside `for worktree in &found`, after the `registered.contains(&path)` block's `continue` and before the `is_dir` check:

```rust
        // A worktree another install made is not ours to adopt.
        //
        // git reports every worktree of a repository regardless of which daemon
        // created it, so without this two installs each adopt the other's, show
        // it in their fleet, and can start agents in the same directory at the
        // same time on separate tmux servers. This is the rule
        // `@farcooler_daemon_id` already applies to panes, extended to the thing
        // that outlives them.
        if let Some(owner) = git::owner_of(Path::new(&worktree.path)).await
            && owner != svc.install_id()
        {
            tracing::debug!(path = %worktree.path, %owner, "worktree belongs to another install");
            continue;
        }
```

- [x] **Step 4: Run the isolation tests**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon isolation_tests`
Expected: all three PASS, including the one that failed in Task 3.

- [x] **Step 5: Run the whole daemon suite for regressions**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon`
Expected: PASS. Pay attention to `reconcile.rs`'s existing adoption tests — `registering_a_repository_adopts_its_main_checkout` in particular, which must still pass because the main checkout is unmarked.

- [x] **Step 6: Clippy**

Run: `~/.cargo/bin/cargo clippy -p farcooler-daemon --all-targets -- -D warnings`
Expected: clean.

- [x] **Step 7: Commit**

```bash
git add crates/daemon/src/git.rs crates/daemon/src/service.rs crates/daemon/src/reconcile.rs
git commit -m "fix(daemon): a worktree belongs to the install that made it"
```

---

### Task 5: Channel-aware binary names

**Files:**
- Modify: `crates/protocol/src/lib.rs`
- Modify: `crates/client/src/session.rs`

**Interfaces:**
- Consumes: `Channel` from Task 1.
- Produces: `Channel::daemon_binary_name() -> &'static str`, `Channel::cli_binary_name() -> &'static str`.

- [x] **Step 1: Write the failing test**

In `crates/protocol/src/lib.rs`'s `mod tests`:

```rust
    #[test]
    fn release_binaries_keep_their_bare_names() {
        // Every installed release client resolves `~/.local/bin/farcoolerd`.
        // A suffix here would strand all of them at once.
        assert_eq!(Channel::Release.daemon_binary_name(), "farcoolerd");
        assert_eq!(Channel::Release.cli_binary_name(), "farcooler");
    }

    #[test]
    fn each_channel_has_its_own_binary_name() {
        let names: Vec<_> = [Channel::Dev, Channel::Beta, Channel::Release]
            .iter()
            .map(|c| c.daemon_binary_name())
            .collect();
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), 3, "two channels cannot share one path: {names:?}");
    }
```

- [x] **Step 2: Run it and watch it fail**

Run: `~/.cargo/bin/cargo test -p farcooler-protocol`
Expected: FAIL — no method `daemon_binary_name`.

- [x] **Step 3: Implement**

In `impl Channel`:

```rust
    /// What this channel's daemon is called on disk.
    ///
    /// The scheme is frozen, not the literal: `farcoolerd[-<channel>]`. An App
    /// Store binary hardcodes the path it asks for, so the shape is as public
    /// as any proto message — and release's name has no suffix precisely so
    /// that every client already in the field keeps resolving what it always
    /// did.
    pub fn daemon_binary_name(self) -> &'static str {
        match self {
            Channel::Release => "farcoolerd",
            Channel::Beta => "farcoolerd-beta",
            Channel::Dev => "farcoolerd-dev",
        }
    }

    pub fn cli_binary_name(self) -> &'static str {
        match self {
            Channel::Release => "farcooler",
            Channel::Beta => "farcooler-beta",
            Channel::Dev => "farcooler-dev",
        }
    }
```

- [x] **Step 4: Make the client ask for its own channel's daemon**

In `crates/client/src/session.rs`, replace the hardcoded string at line 130:

```rust
        let streams = transport
            .exec(&format!(
                "~/.local/bin/{} --stdio",
                farcooler_protocol::CHANNEL.daemon_binary_name()
            ))
            .await?;
```

and at line 162:

```rust
        let streams = ssh
            .exec(&format!(
                "~/.local/bin/{} --stream {terminal}",
                farcooler_protocol::CHANNEL.daemon_binary_name()
            ))
            .await?;
```

Update the error copy at line 35 the same way, so it names the binary it actually tried.

- [x] **Step 5: Run the tests**

Run: `~/.cargo/bin/cargo test -p farcooler-protocol -p farcooler-client`
Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add crates/protocol/src/lib.rs crates/client/src/session.rs
git commit -m "feat: a client asks for its own channel's daemon"
```

---

### Task 6: Channel-aware install paths and service units

**Files:**
- Modify: `crates/cli/src/host_install.rs`

- [x] **Step 1: Make the unit templates take names**

`LAUNCH_AGENT` (line 35) and `UNIT` (line 57) are `const &str` with the label and `ExecStart` baked in. Turn each into a function taking the channel, substituting:

- launchd `Label`: `com.farcooler.daemon.remote` for release, `com.farcooler.daemon.remote.beta`, `com.farcooler.daemon.remote.dev`.
- systemd unit file name: `farcooler.service`, `farcooler-beta.service`, `farcooler-dev.service`.
- Both `ExecStart` / `ProgramArguments`: `~/.local/bin/<daemon_binary_name()>`.

- [x] **Step 2: Replace the hardcoded binary names**

Every `"farcoolerd"` / `"farcooler"` literal in this file becomes the channel's name. Find them:

```bash
grep -n 'farcoolerd\|"farcooler"' crates/cli/src/host_install.rs
```

Lines 111, 123, 127, 141–143, 292, 323–324, 509–510 all name a binary or a `~/.local/bin` path.

- [x] **Step 3: Build and check clippy**

Run: `~/.cargo/bin/cargo clippy -p farcooler-cli --all-targets -- -D warnings`
Expected: clean.

- [x] **Step 4: Run the whole workspace suite**

Run: `~/.cargo/bin/cargo test --workspace`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/cli/src/host_install.rs
git commit -m "feat(cli): install a channel's daemon under its own name"
```

---

## Self-Review

**Spec coverage.** Channels-design sequence step 1 → Tasks 1–2. Step 2, including "the isolation test written **first** — it fails against current code" → Tasks 3–4. Step 3 → Tasks 5–6. The "Testing the isolation" section's four assertions map to: mapping total and distinct → Task 2; two daemons do not see each other → Task 3; install-ids differ, therefore tmux servers differ → Task 3; binary names distinct with release unchanged → Task 5.

Steps 4–8 of the channels sequence (bundle identifiers, relay, `build_number`, `version.sh channel [tag]`, the promotion buttons, `releasing.md`) are **out of scope** for this plan and belong to Plan D.

**Known deviation from the spec.** The spec's test section describes an integration test; this plan puts it in `crates/daemon/src/reconcile.rs` as a `#[cfg(test)]` module instead of `crates/daemon/tests/`. `test_support` is `pub(crate)`, so an external test binary cannot reach the fixture without making it public API. An in-crate module is the smaller change and tests the same behaviour.

**Type consistency.** `Channel` is defined once in Task 1 and used by name in Tasks 2, 5, 6. `install_id()` returns `&str` in Task 3 and is compared against `String` in Task 4 — `owner != svc.install_id()` compares `String` to `&str`, which `PartialEq` covers.

**Placeholder scan.** No TBDs. Task 6 steps 1–2 describe substitutions rather than quoting the full templates, because both are long literals whose every occurrence is mechanical; the grep that finds them is given.
