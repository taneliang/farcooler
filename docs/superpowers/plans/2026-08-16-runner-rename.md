# Runner Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the thing a device connects to from "host"/"machine" to "runner" everywhere a person sees it, so the product has one word for one concept and "three machines on one machine" stops being a sentence it produces.

**Architecture:** Four passes, ordered so nothing is half-renamed at a commit boundary: the CLI surface with a compatibility alias, then the Rust internals, then the three apps, then the documentation. The wire protocol is deliberately excluded — `proto/farcooler.proto:471` defines `message Host`, and renaming a wire type is a versioned protocol change rather than a vocabulary one.

**Tech Stack:** Rust, Swift (SwiftUI, macOS + iOS), Kotlin (Compose), TypeScript, Markdown.

**Spec:** [`docs/superpowers/specs/2026-08-16-device-onboarding-design.md`](../specs/2026-08-16-device-onboarding-design.md) — "What a runner is".

**Depends on:** [`2026-08-16-onboarding-prerequisites.md`](2026-08-16-onboarding-prerequisites.md). Land that first — it touches `crates/cli/src/remote.rs` and `crates/daemon/src/main.rs`, and doing so after a rename means resolving the same conflicts twice.

## Global Constraints

- **US English throughout.** Never "authorise", "colour", "centre".
- **Never run `cargo fmt`.** Hand-formatted tree; CI skips `fmt --check` on purpose.
- **`cargo` is at `~/.cargo/bin/cargo`.**
- **Apple copy conventions in UI strings:** title-case buttons, contractions allowed, no raw Rust errors shown to a person.
- **A runner is one `farcoolerd`:** one Unix user, on one host, with its own worktrees and `~/.ssh/authorized_keys`. A host may carry several. This distinction is the reason for the rename and must survive it — anywhere the old text meant *the box*, "host" is still correct and stays.
- **`gradle` needs JDK 17**; `swift build` for macOS, `xcodebuild` for iOS.

---

### Task 1: The CLI surface, with `--host` kept working

**Files:**
- Modify: `crates/cli/src/main.rs` — the `--host` global flag and the `host` subcommand
- Modify: `crates/cli/src/host_install.rs` → rename file to `crates/cli/src/runner_install.rs`
- Modify: `crates/cli/src/remote.rs` — doc comments only
- Test: `crates/cli/tests/` — add `the_old_host_flag_still_works.rs`

**Interfaces:**
- Produces: `--runner` as the flag and `farcooler runner install|status|uninstall` as the subcommand. `--host` and `farcooler host …` remain as hidden aliases that behave identically.

- [ ] **Step 1: Write the failing test**

```rust
//! The old spelling keeps working, silently.
//!
//! Renaming a flag that lives in people's shell history and scripts is a
//! breaking change wearing a vocabulary change's clothes. The alias is hidden
//! from `--help` so the docs teach one word, and accepted forever so nothing
//! anyone already wrote stops working.

#[test]
fn the_old_host_flag_is_still_accepted() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .args(["--host", "nobody@nowhere.invalid", "status"])
        .output()
        .expect("run farcooler");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("unrecognized"),
        "--host was rejected: {stderr}"
    );
}

#[test]
fn the_new_runner_flag_is_accepted() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .args(["--runner", "nobody@nowhere.invalid", "status"])
        .output()
        .expect("run farcooler");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("unrecognized"),
        "--runner was rejected: {stderr}"
    );
}

/// `--help` teaches one word.
#[test]
fn help_names_only_the_runner() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .arg("--help")
        .output()
        .expect("run farcooler --help");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--runner"), "no --runner in help");
    assert!(!text.contains("--host"), "--host is still advertised");
}
```

- [ ] **Step 2: Run to verify failure**

```bash
~/.cargo/bin/cargo test -p farcooler-cli --test the_old_host_flag_still_works
```

Expected: FAIL — `--runner` is rejected.

- [ ] **Step 3: Rename the flag, keep the alias**

In `crates/cli/src/main.rs`, find the `--host` definition. With clap derive it becomes:

```rust
    /// The runner to talk to, as `user@host`.
    ///
    /// `--host` is accepted and hidden rather than removed: it lives in shell
    /// history and in scripts, and a vocabulary change is not a reason to break
    /// either. One word is taught; both are understood.
    #[arg(long = "runner", visible_alias = "runner", alias = "host", global = true)]
    runner: Option<String>,
```

Do the same for the subcommand: `#[command(name = "runner", alias = "host")]`. Check the actual attribute style in the file and match it — this repo may use `clap` builder rather than derive.

- [ ] **Step 4: Rename the module**

```bash
git mv crates/cli/src/host_install.rs crates/cli/src/runner_install.rs
```

Update `mod host_install;` and every `host_install::` reference. Rename `daemon_name()`'s neighbors only where they say *host* meaning *runner*; `host` inside an SSH destination is still a host.

- [ ] **Step 5: Run**

```bash
~/.cargo/bin/cargo test -p farcooler-cli
~/.cargo/bin/cargo build --workspace
```

- [ ] **Step 6: Commit**

```bash
git add -A crates/cli
git commit -m "feat(cli): the thing you connect to is a runner

A host is the box, and three engineers sharing one box is three
farcoolerds with three fences and three sets of worktrees. Machine
implied one physical computer and made 'three machines on one machine' a
sentence this product kept producing.

--host is hidden rather than removed: it is in shell history and in
scripts, and a vocabulary change is not a reason to break either."
```

---

### Task 2: Rust internals

**Files:**
- Modify: `crates/cli/src/*.rs`, `crates/daemon/src/*.rs`, `crates/core/src/*.rs`, `crates/client/src/*.rs` — identifiers and comments where they mean *runner*
- **Do not modify:** `proto/farcooler.proto`, or the generated `Host` type's use in `crates/daemon/src/wire.rs`

**Interfaces:**
- Produces: internal names only. No public API change beyond Task 1's flag.

- [ ] **Step 1: Find the candidates, then read every one**

```bash
grep -rn "host\|Host\|machine\|Machine" crates/*/src/ | grep -v target > /tmp/rename-candidates.txt
wc -l /tmp/rename-candidates.txt
```

This is a review list, not a substitution list. Three categories, and only the first is renamed:

| Means | Example | Action |
| --- | --- | --- |
| The farcoolerd | `host_status`, "the host is offline" | → runner |
| The box or the network name | `host key`, `HostName`, `you@box`'s host part | **keep** |
| The wire type | `wire::host`, `v1::Host` | **keep**, comment why |

- [ ] **Step 2: Rename category one**

Work file by file, not with a global substitution. A blind `sed` produces "runner key" for `host key`, which is wrong and hard to spot later.

- [ ] **Step 3: Add the comment that stops someone finishing the job**

In `crates/daemon/src/wire.rs`, above the `Host` conversion:

```rust
/// Still `Host` on the wire, deliberately.
///
/// `proto/farcooler.proto` is a versioned interface with negotiation on both
/// sides — see `docs/superpowers/specs/2026-08-11-api-versioning-design.md`.
/// Renaming a message is a protocol change that needs a version and a
/// compatibility window, not a side effect of a vocabulary decision. The word
/// a person sees is "runner"; the word on the wire changes when someone plans
/// that change on purpose.
```

- [ ] **Step 4: Build and test**

```bash
~/.cargo/bin/cargo test --workspace
```

- [ ] **Step 5: Commit**

```bash
git add crates/
git commit -m "refactor: runner, in the Rust that meant runner

Three categories, one renamed. A host key is still a host key and
HostName is still HostName -- those mean the box. The wire type stays
Host with a comment saying why: proto is a versioned interface with
negotiation on both sides, so renaming a message needs a version and a
compatibility window rather than being a side effect of a vocabulary
decision."
```

---

### Task 3: The three apps

**Files:**
- Modify: `apps/macos/Sources/FarCooler/` — `Hosts.swift`, `HostsSettings.swift`, `MachineSettings.swift`, `MachineSettingsStore.swift` and every reference
- Modify: `apps/ios/FarCooler/` — `MachineSettings.swift`, `FarCoolerApp.swift`, `FleetView.swift`, `Settings.swift`
- Modify: `apps/android/app/src/main/java/com/farcooler/ui/` — `MachineEditorScreens.kt`, `MachineSettingsScreen.kt`, `Screens.kt`, `FleetScreen.kt`
- Modify: `apps/shared/AgentKit/Sources/AgentKit/` — any shared strings

**Interfaces:**
- Produces: user-visible strings and Swift/Kotlin type names. `HostStore` → `RunnerStore`, `MachineSettings` → `RunnerSettings`, `Host` → `Runner`.

- [ ] **Step 1: Rename the files**

```bash
git mv apps/macos/Sources/FarCooler/Hosts.swift apps/macos/Sources/FarCooler/Runners.swift
git mv apps/macos/Sources/FarCooler/HostsSettings.swift apps/macos/Sources/FarCooler/RunnersSettings.swift
git mv apps/macos/Sources/FarCooler/MachineSettings.swift apps/macos/Sources/FarCooler/RunnerSettings.swift
git mv apps/macos/Sources/FarCooler/MachineSettingsStore.swift apps/macos/Sources/FarCooler/RunnerSettingsStore.swift
git mv apps/ios/FarCooler/MachineSettings.swift apps/ios/FarCooler/RunnerSettings.swift
git mv apps/android/app/src/main/java/com/farcooler/ui/MachineEditorScreens.kt apps/android/app/src/main/java/com/farcooler/ui/RunnerEditorScreens.kt
git mv apps/android/app/src/main/java/com/farcooler/ui/MachineSettingsScreen.kt apps/android/app/src/main/java/com/farcooler/ui/RunnerSettingsScreen.kt
```

iOS uses a generated Xcode project — after renaming, re-run `apps/ios/generate-project.py` so the new filenames are in the target.

- [ ] **Step 2: Rename the types and every reference**

Swift and Kotlin have no cross-file rename tool here, so compile-driven: rename the declaration, build, fix what the compiler names, repeat.

- [ ] **Step 3: Rewrite the user-visible strings**

Every one, in Apple copy voice. Examples of the exact substitutions:

| Was | Now |
| --- | --- |
| "Connect a Machine" | "Connect a Runner" |
| "Add a Machine" | "Add a Runner" |
| "Far Cooler runs coding agents on machines you already reach over SSH." | "Far Cooler runs coding agents on runners you already reach over SSH." |
| "Cannot open a worktree on \(host)" | "Cannot open a worktree on \(runner)" |
| "Machines" (settings tab) | "Runners" |

Search for stragglers:

```bash
grep -rn "Machine\|machine" apps/*/  --include=*.swift --include=*.kt | grep -v '\.build'
```

- [ ] **Step 4: Build all three**

```bash
cd apps/macos && swift build && cd ../..
cd apps/android && JAVA_HOME=$(/usr/libexec/java_home -v 17) ./gradlew assembleDebug && cd ../..
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler -destination 'generic/platform=iOS Simulator' build
```

- [ ] **Step 5: Look at it**

Launch the Mac app. Open Settings. Confirm the tab says Runners, the empty state says Connect a Runner, and nothing says machine.

- [ ] **Step 6: Commit**

```bash
git add -A apps/
git commit -m "feat(apps): runners, in every string a person reads"
```

---

### Task 4: Documentation

**Files:**
- Modify: `README.md`, `docs/remote-hosts.md` → `git mv` to `docs/runners.md`, `docs/farcooler-design.md`, `docs/adapters.md`, `docs/releasing.md`, `apps/android/README.md`, `services/relay/README.md`
- Modify: `docs/superpowers/specs/2026-08-16-device-onboarding-design.md` — already uses runner; check for stragglers

- [ ] **Step 1: Rename and rewrite**

```bash
git mv docs/remote-hosts.md docs/runners.md
grep -rn "remote-hosts.md" --exclude-dir=.git . | grep -v target
```

Fix every link. Then rewrite the prose, applying the same three-category rule as Task 2 — `docs/runners.md` talks about both runners and the hosts they sit on, and that distinction is now the point of the document rather than an accident.

- [ ] **Step 2: Add the definition where a reader meets the word first**

In `README.md`, near the top:

```markdown
A **runner** is one `farcoolerd`: one Unix user, on one host, with its own
worktrees and its own `~/.ssh/authorized_keys`. A host may carry several — three
engineers sharing a Linux box is three runners, sharing nothing.
```

- [ ] **Step 3: Check nothing dangles**

```bash
grep -rn "machine" README.md docs/*.md | grep -vi "state machine"
```

Expected: nothing, or only deliberate uses like "state machine".

- [ ] **Step 4: Commit**

```bash
git add -A README.md docs/
git commit -m "docs: runners, and a host is the box they sit on"
```

## Self-Review

**Spec coverage.** Implements "What a runner is" from the design. The three consequences in that section — daemon-generated id, label disambiguation, per-runner ssh_config aliases — are *not* here; they are behavior, and they belong to the enrollment and ceremony plans that introduce the code they apply to.

**Placeholders.** None. Task 2 deliberately refuses to give a substitution command, because the whole risk of this task is a blind substitution turning "host key" into "runner key".

**Type consistency.** `HostStore` → `RunnerStore` and `Host` → `Runner` in the apps; the Rust `v1::Host` wire type is explicitly unchanged and commented.
