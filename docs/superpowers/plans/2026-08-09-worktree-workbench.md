# Worktree Workbench Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the review inspector with a composable tile layout, so a worktree opens with its agent on the left and its branch diff on the right — and delete the review buffer, which existed only because there was nowhere to put a diff.

**Architecture:** A worktree's main area becomes a client-owned tree of tiles. `TmuxCanvas` is one tile and tmux keeps its own tiling inside it, so `PaneGroup` stays a projection of tmux's window tree and no protocol changes are needed. The review buffer — entries, anchors, dispatch, the outbox, attachments — is removed; what survives is knowing that a worktree moved since you last read it.

**Tech Stack:** Rust (daemon, store, CLI), SwiftUI (macOS), protobuf over an SSH stdio control plane, SQLite.

**Scope:** This plan covers phases 1 and 2 of the spec's build order. The fleet view, the PR tile and the phone segments each get their own plan; stopping at the end of this one leaves a coherent product.

**Spec:** `docs/superpowers/specs/2026-08-09-worktree-workbench-design.md`

## Global Constraints

- **US English throughout**, in code and copy. Never "authorise", "colour", "centre".
- **Apple copy conventions**: title-case buttons, contractions, "machine" not "host", never a raw Rust error in the UI.
- **Never run `cargo fmt`.** The Rust tree is hand-formatted and CI skips `fmt --check` on purpose. Match surrounding style by hand.
- **`cargo` is not on the default PATH** in this shell. Every Rust command needs `export PATH="$HOME/.cargo/bin:$PATH"` first.
- **Work in the worktree**, not the main checkout: `/Users/e-liang/Library/Application Support/com.farcooler.FarCooler/worktrees/overnight-review`. Another agent is working on `main`.
- **A live Far Cooler is running.** Never `pkill` or match daemons by broad pattern; kill scratch daemons by recorded PID only.
- **Migrations are forward-only.** Migration 0007 is unreleased and on this branch alone, so it is edited in place rather than followed by an 0008 that drops what 0007 just created.
- **Tests are sentences.** `crates/<c>/tests/<prose_name>.rs` for integration, inline `#[cfg(test)] mod tests` for units, and test names read as claims: `two_terminals_do_not_share_a_socket`.
- **The macOS app needs its generated header staged** before it will build in a fresh worktree: `cd apps/macos && ./build-vt.sh`.

---

## File Structure

**Deleted outright**
- `crates/store/tests/a_dispatch_is_never_claimed_before_it_is_delivered.rs` — the buffer's integration tests.
- `apps/macos/Sources/FarCooler/Review.swift` — replaced by the tile files below.

**Heavily reduced**
- `crates/store/src/review.rs` — keeps only `review_base`/`set_review_base`, `mark_reviewed*`/`reviewed_mark`, and `stack_parent`/`set_stack_parent`. Everything about entries, dispatches and attachments goes.
- `crates/store/src/migrate.rs` — migration 0007 keeps `review_bases`, `review_reviewed`, `review_stack_parents`; drops the other four tables.
- `crates/daemon/src/review_ops.rs` — keeps change set, commit files, file diff, set base, inbox, stack, PR. Drops capture/update/delete/list/dispatch/mark_viewed/attachment_*.
- `crates/daemon/src/review.rs` — keeps the change-set cache and the cheap gate. Drops prompt composition, answer splitting, attachments and the snapshot budget.
- `crates/daemon/src/rpc.rs` — drops the deleted methods from `required_scope` and `dispatch`.
- `crates/cli/src/review.rs` → renamed `crates/cli/src/changes.rs`, dropping `note`/`list`/`drop`/`send`/`seen`.
- `proto/farcooler.proto` — drops the buffer messages; `review.*` methods become `changes.*`.

**Created**
- `apps/macos/Sources/FarCooler/Tiles.swift` — the tile model, the layout tree, and persistence.
- `apps/macos/Sources/FarCooler/TileContainer.swift` — the recursive split view that renders a layout.
- `apps/macos/Sources/FarCooler/DiffTile.swift` — the diff tile: scope selector, jump bar, long scroll, wide layout.
- `apps/macos/Sources/FarCooler/ChangesModel.swift` — the decoded change set and diff types the tile renders (what survives of `Review.swift`).

**Modified**
- `apps/macos/Sources/FarCooler/ContentView.swift` — the inspector goes; `detailWithReview` becomes the tile container.
- `apps/macos/Sources/FarCooler/DaemonClient.swift` — `review` calls become `changes` calls; buffer calls go.
- `apps/macos/Sources/FarCooler/SidebarViews.swift` — the needs-you badge goes; `+N −M` stays.

---

## Phase 1 — Delete the buffer, and rename

### Task 1: Reduce the store to what survives

**Files:**
- Modify: `crates/store/src/review.rs`
- Modify: `crates/store/src/migrate.rs`
- Delete: `crates/store/tests/a_dispatch_is_never_claimed_before_it_is_delivered.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `Store::review_base(Uuid) -> Result<Option<String>>`, `Store::set_review_base(Uuid, &str) -> Result<()>`, `Store::mark_reviewed_with_gate(Uuid, &str, &str, &str, i64, i64, i64) -> Result<()>`, `Store::reviewed_mark(Uuid, &str) -> Result<Option<ReviewedMark>>`, `Store::stack_parent(Uuid, &str) -> Result<Option<String>>`, `Store::set_stack_parent(Uuid, &str, &str) -> Result<()>`. `ReviewedMark { head_commit: String, worktree_digest: String, gate_head: i64, gate_index: i64, marked_at: i64 }`.

- [x] **Step 1: Delete the buffer's integration tests**

```bash
cd "/Users/e-liang/Library/Application Support/com.farcooler.FarCooler/worktrees/overnight-review"
git rm crates/store/tests/a_dispatch_is_never_claimed_before_it_is_delivered.rs
```

- [x] **Step 2: Cut migration 0007 down to the three tables that survive**

Replace the whole body of `migration_0007_review` in `crates/store/src/migrate.rs` with:

```rust
/// What a worktree is compared against, and whether you have read it.
///
/// Edited in place rather than followed by an 0008 that drops what it just
/// created: 0007 is unreleased and lives on this branch alone, so no database
/// anywhere has ever run the version that made the buffer's tables.
///
/// The buffer itself — entries, anchors, dispatches, attachments — is gone. It
/// existed because there was nowhere to put a diff, so a comment had to be held
/// somewhere until you could type it at an agent. With a diff tile beside an
/// agent tile there is nothing to hold.
fn migration_0007_review(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        -- What a workspace is compared against, when the user pinned it.
        CREATE TABLE review_bases (
            workspace_id BLOB PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
            base_ref TEXT NOT NULL
        );

        -- "I have looked at this worktree." Stores the CHEAP gate as well as
        -- the digest, so the fleet can answer "changed since you looked" with
        -- two stats instead of a `git status` per worktree.
        CREATE TABLE review_reviewed (
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            branch TEXT NOT NULL,
            head_commit TEXT NOT NULL,
            worktree_digest TEXT NOT NULL,
            gate_head INTEGER NOT NULL DEFAULT 0,
            gate_index INTEGER NOT NULL DEFAULT 0,
            marked_at INTEGER NOT NULL,
            PRIMARY KEY (workspace_id, branch)
        );

        -- One branch's parent in a stack, when inference got it wrong and the
        -- user said so.
        CREATE TABLE review_stack_parents (
            repository_id BLOB NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            branch TEXT NOT NULL,
            parent_branch TEXT NOT NULL,
            PRIMARY KEY (repository_id, branch)
        );
        "#,
    )
}
```

- [x] **Step 3: Cut `crates/store/src/review.rs` to the surviving surface**

Delete these items entirely: `Disposition`, `EntryStatus`, `DispatchState`, `ReviewEntry`, `Dispatch`, `Attachment`, `WorkspaceCounts`, `row_to_entry`, `row_to_dispatch`, `row_to_attachment`, `ENTRY_COLUMNS`, and every method from `capture_review_entry` through `is_viewed`, plus `record_attachment`, `attach_to_entry`, `attachment`, `entry_attachments`, `workspace_attachment_bytes`, `review_counts_by_workspace`, `entries_with_snapshots`, `clear_snapshot`, and the inline `mod tests`.

Replace the module doc comment with:

```rust
//! What a worktree is compared against, and whether you have read it.
//!
//! This used to hold a review buffer: captured comments, their anchors, the
//! dispatch outbox, attachments. All of it is gone. The buffer existed because
//! there was nowhere to put a diff — you noticed something, held it in your
//! head, and typed it at an agent later — and with a diff tile beside an agent
//! tile there is nothing to hold.
//!
//! What is left was never about comments. `review_bases` is what a branch is
//! compared against when somebody pinned it, and `review_reviewed` is how the
//! fleet knows a worktree moved since you last looked at it.
```

Keep `ReviewedMark`, `review_base`, `set_review_base`, `mark_reviewed`, `mark_reviewed_with_gate`, `reviewed_mark`, `stack_parent`, `set_stack_parent` exactly as they are.

- [x] **Step 4: Add the guard test that the buffer stays gone**

Append to `crates/store/src/review.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The buffer is deleted, and the schema is where that has to be true.
    ///
    /// A table reappearing here means somebody rebuilt the thing the tile layout
    /// was supposed to make unnecessary — worth failing a build over, because it
    /// would come back one convenience method at a time.
    #[test]
    fn the_review_buffer_has_no_tables_left() {
        let s = Store::open_in_memory().unwrap();
        for gone in ["review_entries", "review_dispatches", "review_attachments", "review_viewed"]
        {
            assert!(
                s.column_names(gone).is_empty(),
                "`{gone}` is back: the review buffer was deleted deliberately"
            );
        }
    }

    #[test]
    fn what_survives_is_the_base_and_whether_you_have_read_it() {
        let s = Store::open_in_memory().unwrap();
        for kept in ["review_bases", "review_reviewed", "review_stack_parents"] {
            assert!(!s.column_names(kept).is_empty(), "`{kept}` should still exist");
        }
    }
}
```

- [x] **Step 5: Run the tests and verify they pass**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p farcooler-store 2>&1 | tail -20
```

Expected: PASS, and the two new tests named above appear.

- [x] **Step 6: Commit**

```bash
git add crates/store
git commit -m "refactor(store): delete the review buffer, keep what was never about comments"
```

---

### Task 2: Reduce the daemon to what survives

**Files:**
- Modify: `crates/daemon/src/review.rs`
- Modify: `crates/daemon/src/review_ops.rs`
- Modify: `crates/daemon/src/rpc.rs`
- Modify: `crates/daemon/src/service.rs`
- Modify: `proto/farcooler.proto`

**Interfaces:**
- Consumes: Task 1's store surface.
- Produces: `review_ops::{change_set, commit_files, file_diff, set_base, inbox, stack_get, stack_set_parent, pr_refresh}` with unchanged signatures. `pb::InboxWorkspace { workspace_id, task_name, branch, changed_since_reviewed, insertions, deletions }` — the four count fields are removed.

- [x] **Step 1: Delete the buffer messages from the protocol**

In `proto/farcooler.proto`, delete these messages: `Disposition`, `EntryStatus`, `AnchorState`, `ReviewEntry`, `ReviewEntryList`, `ReviewCapture`, `ReviewUpdate`, `ReviewDelete`, `ReviewList`, `DispatchEntry`, `ReviewDispatchRequest`, `DispatchState`, `ReviewDispatch`, `ReviewMarkViewed`, `Attachment`, `AttachmentPut`, `AttachmentGet`, `AttachmentBytes`.

Delete these `Request.payload` arms: `review_capture`, `review_update`, `review_delete`, `review_list`, `review_dispatch`, `review_mark_viewed`, `attachment_put`, `attachment_get`.

Delete these `Result.value` arms: `review_entry`, `review_entry_list`, `review_dispatch_result`, `attachment`, `attachment_bytes`.

Delete this `Event.payload` arm: `review_entry_changed`.

Do **not** renumber anything that remains. Tag numbers are permanent even when their message is gone, and reusing one would make an old client decode a new message as the deleted one.

In `InboxWorkspace`, delete the `open`, `dispatched`, `answered` and `dispatch_unknown` fields, leaving their tag numbers unused.

- [x] **Step 2: Rename the surviving methods from `review.*` to `changes.*`**

In `proto/farcooler.proto`, rename the remaining `Review*` messages: `ReviewSetBase` → `ChangesSetBase`, `ReviewMarkReviewed` → `ChangesMarkRead`, `ReviewInboxRequest` → `ChangesInboxRequest`, `ReviewInbox` → `ChangesInbox`, `ReviewCommit` → `ChangeCommit`. Rename their `Request`/`Result` arm names to match, keeping every tag number.

- [x] **Step 3: Cut `crates/daemon/src/review.rs`**

Delete `PromptEntry`, `compose_prompt`, `split_numbered_answer`, `leading_marker`, `attachments_dir`, `MAX_ATTACHMENT_BYTES`, `MAX_ATTACHMENTS_PER_ENTRY`, `MAX_ATTACHMENT_BYTES_PER_WORKSPACE`, `MAX_SNAPSHOT_BYTES_PER_WORKSPACE`, `put_attachment`, `read_attachment`, `image_dimensions`, `capture_manifest`, `resolve_entry`, and every test covering them.

Keep `now_millis`, `CachedChangeSet`, `ReviewCache` and all its methods, and `cheap_gate`. Keep the two `cheap_gate` tests.

- [x] **Step 4: Cut `crates/daemon/src/review_ops.rs`**

Delete `capture`, `update`, `delete`, `list`, `dispatch`, `mark_viewed`, `attachment_put`, `attachment_get`, `hydrate`, `fingerprint_now`, `read_anchor`, `read_manifest`, `enforce_snapshot_budget`, `pb_disposition`, `parse_disposition`, `pb_entry_status`, `pb_anchor_state`, and the whole inline `mod tests`.

Rewrite `inbox` so it no longer reads counts:

```rust
/// The fleet's changed-since-you-looked state.
///
/// Every worktree with changes, not only those with something waiting: the
/// counts this used to carry came from the review buffer, and the buffer is
/// gone. What is left is the question the sidebar actually asks — did this move
/// since I last read it, and by how much.
pub async fn inbox(svc: &Service) -> Result<pb::ChangesInbox> {
    let mut items = Vec::new();

    for ws in svc.store.list_all_workspaces()? {
        let worktree = Path::new(&ws.worktree_path);
        if !worktree.is_dir() {
            continue;
        }
        let gate = review::cheap_gate(worktree);
        let mark = svc.store.reviewed_mark(ws.id, &ws.branch)?;

        // Three cheap signals, none of which runs git: the gate, which catches
        // commits, rebases and checkouts; an agent write this daemon served
        // itself, which catches the ordinary case the gate cannot see; and the
        // digest of an already-cached change set, free when somebody has been
        // looking at this worktree anyway.
        let changed = match &mark {
            Some(m) => {
                m.gate_head != gate.0 as i64
                    || m.gate_index != gate.1 as i64
                    || svc.review_cache.touched_at(ws.id).is_some_and(|t| t > m.marked_at)
                    || svc
                        .review_cache
                        .cached_digest(ws.id, &ws.branch)
                        .is_some_and(|d| d != m.worktree_digest)
            }
            None => true,
        };

        let base = match svc.store.review_base(ws.id)? {
            Some(recorded) => recorded,
            None => default_base_for(worktree).await,
        };
        let (_, ins, del) =
            svc.review_cache.shortstat(ws.id, worktree, &base).await.unwrap_or((0, 0, 0));

        if ins == 0 && del == 0 && !changed {
            continue;
        }

        items.push(pb::InboxWorkspace {
            workspace_id: id_bytes(ws.id),
            task_name: ws.task_name,
            branch: ws.branch,
            changed_since_reviewed: changed,
            insertions: ins,
            deletions: del,
        });
    }

    Ok(pb::ChangesInbox { items, elsewhere: 0 })
}
```

- [x] **Step 5: Add `list_all_workspaces` to the store**

The old `inbox` walked `review_counts_by_workspace`, which is gone. Add to `crates/store/src/store.rs`, beside `list_workspaces_for_repository`:

```rust
    /// Every workspace on this machine, across repositories.
    ///
    /// For the fleet's own questions, which are not scoped to a project the way
    /// the sidebar's are.
    pub fn list_all_workspaces(&self) -> Result<Vec<Workspace>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, repository_id, task_name, branch, worktree_path, hidden,
                        creation_failed, resource_version, is_main_checkout, worktree_missing
                 FROM workspaces WHERE hidden = 0 ORDER BY task_name",
            )
            .map_err(map_err)?;
        let rows = stmt.query_map([], row_to_workspace).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }
```

- [x] **Step 6: Drop the deleted methods from the RPC table**

In `crates/daemon/src/rpc.rs`, remove `review.capture`, `review.update`, `review.delete`, `review.list`, `review.dispatch`, `review.mark_viewed`, `review.attachment_put`, `review.attachment_get` from both `required_scope` and `dispatch`, and from the method list in `every_method_has_a_declared_scope`.

Rename the survivors in all three places: `review.change_set` → `changes.change_set`, `review.commit_files` → `changes.commit_files`, `review.file_diff` → `changes.file_diff`, `review.set_base` → `changes.set_base`, `review.mark_reviewed` → `changes.mark_read`, `review.inbox` → `changes.inbox`.

Delete `Service::send_review_prompt` from `crates/daemon/src/service.rs`, and `Watcher::announce_review_entry` from `crates/daemon/src/watch.rs`.

- [x] **Step 7: Build and run the whole suite**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --workspace 2>&1 | grep -E "^error" -A5 | head -30
cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | awk -F'[ ;]' '{p+=$4; f+=$6} END {print "passed:", p, "failed:", f}'
```

Expected: builds clean, 0 failures. The count drops by roughly 60 as the buffer's tests go.

- [x] **Step 8: Commit**

```bash
git add crates proto
git commit -m "refactor(daemon): review becomes changes, and loses the buffer"
```

---

### Task 3: Rename the CLI surface

**Files:**
- Rename: `crates/cli/src/review.rs` → `crates/cli/src/changes.rs`
- Modify: `crates/cli/src/main.rs`

**Interfaces:**
- Consumes: Task 2's `changes.*` methods.
- Produces: `farcooler changes status|diff|files|read|inbox|stack`.

- [x] **Step 1: Rename the module and its command enum**

```bash
git mv crates/cli/src/review.rs crates/cli/src/changes.rs
```

In `crates/cli/src/changes.rs`: rename `ReviewCmd` → `ChangesCmd`, `pub async fn review` → `pub async fn changes`, and delete the `Note`, `List`, `Drop` and `Send` variants together with their match arms, `list_entries`, `entry_json`, `anchor_json`, `state_word` and `status_word`. Rename the `Seen` variant to `Read` and its method call to `changes.mark_read`.

Update every remaining `req("review.…")` to `req("changes.…")` and every payload type to its renamed counterpart.

In `crates/cli/src/main.rs`: `mod review;` → `mod changes;`, `Command::Review(review::ReviewCmd)` → `Command::Changes(changes::ChangesCmd)` with doc comment "What this worktree changed.", and the dispatch arm to `changes::changes(host, c, cli.json).await`.

- [x] **Step 2: Build and check the help output**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build -p farcooler-cli 2>&1 | grep -E "^error" -A4 | head -20
./target/debug/farcooler changes --help
```

Expected: builds clean, and help lists `status`, `diff`, `files`, `read`, `inbox`, `stack` with no `note`/`list`/`drop`/`send`.

- [x] **Step 3: Commit**

```bash
git add crates/cli
git commit -m "refactor(cli): farcooler review becomes farcooler changes"
```

---

### Task 4: Strip the buffer out of the Mac app

**Files:**
- Create: `apps/macos/Sources/FarCooler/ChangesModel.swift`
- Delete: `apps/macos/Sources/FarCooler/Review.swift`
- Modify: `apps/macos/Sources/FarCooler/DaemonClient.swift`
- Modify: `apps/macos/Sources/FarCooler/SidebarViews.swift`
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift`

**Interfaces:**
- Consumes: Task 3's CLI.
- Produces: `ChangeSet`, `ChangedFile`, `ChangeCommit`, `WorkingTree`, `InboxRow`, and `ChangesStore` with `load(fresh:)`, `openFile(_:)`, `closeFile()`, `markRead()`, `@Published changeSet/diff/selectedFile/error`.

- [x] **Step 1: Create `ChangesModel.swift` with what survives**

Move `ChangeSet`, `ReviewCommit` (renamed `ChangeCommit`), `ChangedFile`, `WorkingTree` and `InboxRow` verbatim out of `Review.swift`, minus `needsYou` and the four count fields on `InboxRow`:

```swift
import AgentKit
import Foundation

/// One worktree's line in the fleet's changed-since-you-looked list.
struct InboxRow: Decodable, Equatable, Identifiable {
    var workspaceId: String
    var changedSinceReviewed: Bool
    var insertions: Int
    var deletions: Int

    var id: String { workspaceId }
    var hasDiff: Bool { insertions > 0 || deletions > 0 }

    enum CodingKeys: String, CodingKey {
        case workspaceId = "workspace_id"
        case insertions, deletions
        case changedSinceReviewed = "changed_since_reviewed"
    }
}
```

Then add the store, which is `ReviewStore` minus everything about entries:

```swift
/// What one worktree changed, and the file currently open.
@MainActor
final class ChangesStore: ObservableObject {
    @Published var changeSet: ChangeSet = .empty
    @Published var diff: [DiffComputation.Line] = []
    @Published var selectedFile: String?
    @Published var scope: DiffScope = .branch
    @Published var loading = false
    @Published var error: String?

    let client: DaemonClient
    let workspace: Workspace

    init(client: DaemonClient, workspace: Workspace) {
        self.client = client
        self.workspace = workspace
    }

    func load(fresh: Bool = false) async {
        loading = true
        defer { loading = false }
        var args = ["changes", "status", workspace.short, "--json"]
        if fresh { args.append("--fresh") }
        if let data = await client.changesJSON(args) {
            changeSet = (try? JSONDecoder().decode(ChangeSet.self, from: data)) ?? .empty
            error = nil
        } else {
            // A failure is not an empty diff. Saying so is what made a machine
            // running an older daemon look like a worktree with no changes.
            changeSet = .empty
            error = client.changesError
        }
    }

    func openFile(_ path: String) async {
        selectedFile = path
        diff = await client.changesDiff(workspace: workspace.short, path: path, scope: scope)
    }

    func closeFile() {
        selectedFile = nil
        diff = []
    }

    func markRead() async {
        await client.changesMarkRead(workspace: workspace.short)
        await load()
    }
}

/// Which comparison the diff tile is showing.
enum DiffScope: String, CaseIterable, Identifiable {
    case branch, local
    var id: String { rawValue }

    /// Title case, because these are controls.
    var label: String {
        switch self {
        case .branch: return "Branch"
        case .local: return "Local"
        }
    }
}
```

- [x] **Step 2: Delete `Review.swift` and rename the client calls**

```bash
git rm apps/macos/Sources/FarCooler/Review.swift
```

In `DaemonClient.swift`: rename `reviewJSON` → `changesJSON`, `reviewDiff` → `changesDiff`, `reviewSeen` → `changesMarkRead`, `refreshReviewInbox` → `refreshChangesInbox`, `reviewInbox` → `changesInbox`, `reviewSupported` → `changesSupported`, `reviewError` → `changesError`; delete `reviewNote`, `reviewDrop` and `reviewSend`. Change every argument array's leading `"review"` to `"changes"`, and `["review", "seen", …]` to `["changes", "read", …]`.

Give `changesDiff` a scope parameter:

```swift
    func changesDiff(
        workspace: String, path: String, scope: DiffScope
    ) async -> [DiffComputation.Line] {
        var args = ["changes", "diff", workspace, path]
        if scope == .local { args.append("--unstaged") }
        guard let data = await run(args, background: true),
            let text = String(data: data, encoding: .utf8)
        else { return [] }
        return Self.parseUnified(text)
    }
```

Move the existing body of `reviewDiff` that walks the unified output into a `static func parseUnified(_ text: String) -> [DiffComputation.Line]` on `DaemonClient`, unchanged apart from the signature.

- [x] **Step 3: Remove the needs-you badge from the sidebar**

In `SidebarViews.swift`, delete the `if let review, review.needsYou > 0 { … }` block entirely. Keep the `+N −M` cluster. Rename the `review` property to `changes` and its type stays `InboxRow?`.

In `ContentView.swift`, rename `reviewStatus(_:)` to `changesStatus(_:)` and the `review:` argument label to `changes:`.

- [x] **Step 4: Build**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd apps/macos && ./build-vt.sh >/dev/null && swift build 2>&1 | grep -E "error:" | head -10
```

Expected: no errors. `ReviewPane` and `reviewOpen` will still be referenced from `ContentView`; delete those references now — the inspector goes in Task 5.

- [x] **Step 5: Commit**

```bash
git add -A apps/macos
git commit -m "refactor(macos): the review buffer leaves the app"
```

---

## Phase 2 — The tile system and the Diff tile

### Task 5: The tile model and its persistence

**Files:**
- Create: `apps/macos/Sources/FarCooler/Tiles.swift`
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift`

**Interfaces:**
- Consumes: nothing.
- Produces: `enum TileKind: Codable, Equatable { case tmux, diff, pr }`, `indirect enum TileNode: Codable, Equatable { case leaf(TileKind), split(Axis, [TileNode], [Double]) }`, `enum Axis: String, Codable { case horizontal, vertical }`, `TileLayout.default`, `TileLayout.load(workspace: String) -> TileNode`, `TileLayout.save(_ node: TileNode, workspace: String)`.

- [x] **Step 1: Write the failing test**

Swift package tests for the app target do not exist; put this in `apps/shared/AgentKit/Tests/AgentKitTests/` only if the types live in AgentKit. They do not — they are app-local. So this task's verification is a build plus a round-trip check written as an inline `#if DEBUG` assertion exercised by Step 4's manual run. Write `Tiles.swift` first:

```swift
import Foundation

/// Which way a split divides its children.
enum Axis: String, Codable, Equatable {
    case horizontal  // children side by side
    case vertical    // children stacked
}

/// What a tile shows.
///
/// `tmux` is one tile and tmux keeps its own tiling INSIDE it. That is the
/// constraint that makes this layout affordable: `PaneGroup` stays a projection
/// of tmux's window tree, `layout.split` still owns where panes go, and nothing
/// here needs a protocol change. A client-owned tree of terminals would be the
/// second pane tree migration 0003 deleted.
enum TileKind: String, Codable, Equatable, CaseIterable {
    case tmux
    case diff
    case pr

    var label: String {
        switch self {
        case .tmux: return "Agents"
        case .diff: return "Changes"
        case .pr: return "Pull Request"
        }
    }
}

/// A worktree's layout: a tree whose leaves are tiles.
indirect enum TileNode: Codable, Equatable {
    case leaf(TileKind)
    /// Children and their fractions, which always sum to 1 and always have the
    /// same count as `children`.
    case split(Axis, [TileNode], [Double])

    /// Every tile kind present, in reading order.
    var kinds: [TileKind] {
        switch self {
        case .leaf(let k): return [k]
        case .split(_, let children, _): return children.flatMap(\.kinds)
        }
    }

    /// Whether this tree contains a tile of the given kind.
    func contains(_ kind: TileKind) -> Bool { kinds.contains(kind) }
}

enum TileLayout {
    /// An agent on the left, the branch diff on the right.
    ///
    /// The default rather than something you arrange, because the diff beside
    /// the agent changing it is the most valuable adjacency in the product.
    static let `default`: TileNode = .split(.horizontal, [.leaf(.tmux), .leaf(.diff)], [0.5, 0.5])

    private static func key(_ workspace: String) -> String { "tiles.layout.\(workspace)" }

    /// Client-local, per workspace, per device.
    ///
    /// Not daemon-owned: this is view arrangement rather than intent about the
    /// work, and a phone shows one tile at a time regardless, so syncing it
    /// would buy almost nothing for the cost of a protocol surface.
    static func load(workspace: String) -> TileNode {
        guard let data = UserDefaults.standard.data(forKey: key(workspace)),
            let node = try? JSONDecoder().decode(TileNode.self, from: data)
        else { return `default` }
        return node
    }

    static func save(_ node: TileNode, workspace: String) {
        guard let data = try? JSONEncoder().encode(node) else { return }
        UserDefaults.standard.set(data, forKey: key(workspace))
    }

    static func forget(workspace: String) {
        UserDefaults.standard.removeObject(forKey: key(workspace))
    }
}
```

- [x] **Step 2: Verify the round trip in a scratch binary**

```bash
cd "/Users/e-liang/Library/Application Support/com.farcooler.FarCooler/worktrees/overnight-review"
cat > /tmp/tiles_check.swift <<'EOF'
// Mirrors Tiles.swift's types to prove Codable round-trips before wiring the UI.
EOF
export PATH="$HOME/.cargo/bin:$PATH"
cd apps/macos && swift build 2>&1 | grep -E "error:" | head -5
```

Expected: builds clean. A decode failure would surface as `TileLayout.load` returning `.default`, which is the safe direction.

- [x] **Step 3: Commit**

```bash
git add apps/macos/Sources/FarCooler/Tiles.swift
git commit -m "feat(macos): a worktree's layout is a tree of tiles"
```

---

### Task 6: The tile container

**Files:**
- Create: `apps/macos/Sources/FarCooler/TileContainer.swift`
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift`

**Interfaces:**
- Consumes: `TileNode`, `TileKind`, `TileLayout` from Task 5; `ChangesStore` from Task 4.
- Produces: `TileContainer(node: Binding<TileNode>, workspace: Workspace, changes: ChangesStore, tmux: () -> AnyView)`.

- [x] **Step 1: Write `TileContainer.swift`**

```swift
import SwiftUI

/// Renders a worktree's tile tree.
///
/// Recursive, and deliberately dumb: it draws splits and hands each leaf to the
/// view that owns that tile kind. The tmux leaf is handed back to the caller as
/// a closure, because the pane canvas needs the whole of `ContentView`'s state
/// and this view has no business knowing about it.
struct TileContainer: View {
    @Binding var node: TileNode
    let workspace: Workspace
    @ObservedObject var changes: ChangesStore
    let tmux: () -> AnyView

    var body: some View {
        render($node)
    }

    @ViewBuilder
    private func render(_ node: Binding<TileNode>) -> some View {
        switch node.wrappedValue {
        case .leaf(let kind):
            leaf(kind)
        case .split(let axis, let children, _):
            if axis == .horizontal {
                HSplitView {
                    ForEach(children.indices, id: \.self) { i in
                        render(childBinding(node, i))
                    }
                }
            } else {
                VSplitView {
                    ForEach(children.indices, id: \.self) { i in
                        render(childBinding(node, i))
                    }
                }
            }
        }
    }

    /// A binding to one child of a split.
    ///
    /// Written out rather than reached for with a subscript because `TileNode`
    /// is an enum: there is no stored property to project, so the setter has to
    /// rebuild the parent case.
    private func childBinding(_ parent: Binding<TileNode>, _ index: Int) -> Binding<TileNode> {
        Binding(
            get: {
                guard case .split(_, let children, _) = parent.wrappedValue,
                    index < children.count
                else { return .leaf(.diff) }
                return children[index]
            },
            set: { newValue in
                guard case .split(let axis, var children, let fractions) = parent.wrappedValue,
                    index < children.count
                else { return }
                children[index] = newValue
                parent.wrappedValue = .split(axis, children, fractions)
            }
        )
    }

    @ViewBuilder
    private func leaf(_ kind: TileKind) -> some View {
        switch kind {
        case .tmux:
            tmux()
        case .diff:
            DiffTile(changes: changes)
        case .pr:
            // The PR tile is a later plan. Saying so beats an empty pane that
            // looks like a worktree with no pull request.
            TilePlaceholder(
                title: "Pull requests aren't here yet",
                detail: "This tile arrives with the PR work.")
        }
    }
}

/// A tile that has nothing to show, and says which.
struct TilePlaceholder: View {
    let title: String
    let detail: String

    var body: some View {
        VStack(spacing: 4) {
            Text(title).font(.system(size: 12, weight: .medium))
            Text(detail).font(.system(size: 11)).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
```

- [x] **Step 2: Replace the inspector with the container in `ContentView`**

Delete `@State private var reviewOpen`, the whole `.inspector(isPresented:)` modifier and the toolbar button that toggled it. Add:

```swift
    /// One layout per worktree, loaded on first sight and saved on every change.
    @State private var tileLayouts: [String: TileNode] = [:]

    private func tileBinding(_ ws: Workspace) -> Binding<TileNode> {
        Binding(
            get: { tileLayouts[ws.id] ?? TileLayout.load(workspace: ws.id) },
            set: { newValue in
                tileLayouts[ws.id] = newValue
                TileLayout.save(newValue, workspace: ws.id)
            }
        )
    }
```

and make `detailWithReview` — rename it `workbench` — build the container:

```swift
    /// The worktree's workbench: its tiles.
    ///
    /// Not an inspector, and not a pane. `PaneGroup` is a projection of tmux's
    /// own window tree keyed by terminal id, so a view that is not a process has
    /// no pane identity to take — the tmux canvas is one leaf of THIS tree and
    /// keeps its own tiling inside it.
    @ViewBuilder
    private var workbench: some View {
        if let ws = detailWorkspace, let client = store.client(for: ws) {
            TileContainer(
                node: tileBinding(ws),
                workspace: ws,
                changes: changesStore(for: ws, client: client),
                tmux: { AnyView(detail) }
            )
            .id(ws.id)
        } else {
            detail
        }
    }
```

with one `ChangesStore` per worktree, held the same way the layouts are:

```swift
    @State private var changesStores: [String: ChangesStore] = [:]

    private func changesStore(for ws: Workspace, client: DaemonClient) -> ChangesStore {
        if let existing = changesStores[ws.id] { return existing }
        let made = ChangesStore(client: client, workspace: ws)
        // Assigned outside the view update: creating it IS a state change and
        // SwiftUI is reading that state right now. An earlier version wrote it
        // from a Task, which rebuilt the store on every render and threw away
        // each load before it finished.
        DispatchQueue.main.async { changesStores[ws.id] = made }
        return made
    }
```

Point the `NavigationSplitView`'s detail closure at `workbench`.

- [x] **Step 3: Build**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd apps/macos && swift build 2>&1 | grep -E "error:" | head -10
```

Expected: fails only on `DiffTile`, which Task 7 creates. Add a temporary stub if needed to see the rest compile, then delete it in Task 7.

- [x] **Step 4: Commit**

```bash
git add apps/macos
git commit -m "feat(macos): the worktree's main area is a tile tree, not an inspector"
```

---

### Task 7: The Diff tile

**Files:**
- Create: `apps/macos/Sources/FarCooler/DiffTile.swift`

**Interfaces:**
- Consumes: `ChangesStore`, `DiffScope`, `ChangedFile`, `DiffComputation.Line`.
- Produces: `DiffTile(changes: ChangesStore)`.

- [x] **Step 1: Write `DiffTile.swift`**

```swift
import AgentKit
import SwiftUI

/// What this worktree changed.
///
/// One concept at two widths rather than two modes: the jump bar along the top
/// is how you navigate when the tile is narrow, and when it is wide that same
/// list is promoted into a column beside the diff. The body is one long scroll
/// of every changed file, because skimming a branch is the first thing anybody
/// does and a viewer that opens files one at a time turns skimming into forty
/// clicks.
struct DiffTile: View {
    @ObservedObject var changes: ChangesStore

    /// Below this the file list is a bar; above it, a column.
    private static let wideEnough: CGFloat = 620

    var body: some View {
        GeometryReader { geo in
            VStack(spacing: 0) {
                header
                Divider()
                if let problem = changes.error {
                    failure(problem)
                }
                if geo.size.width >= Self.wideEnough {
                    HStack(spacing: 0) {
                        fileColumn.frame(width: 200)
                        Divider()
                        diffScroll
                    }
                } else {
                    VStack(spacing: 0) {
                        jumpBar
                        Divider()
                        diffScroll
                    }
                }
            }
        }
        .task { await changes.load() }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Picker("", selection: $changes.scope) {
                ForEach(DiffScope.allCases) { s in Text(s.label).tag(s) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()
            .onChange(of: changes.scope) { _, _ in
                Task { await changes.load(fresh: true) }
            }

            Spacer(minLength: 4)

            Text(changes.changeSet.branch)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .layoutPriority(-1)

            counts

            Button {
                Task { await changes.load(fresh: true) }
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Read the worktree again")

            Button {
                Task { await changes.markRead() }
            } label: {
                Image(systemName: "checkmark.circle")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Mark Read")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }

    private var counts: some View {
        HStack(spacing: 4) {
            if changes.changeSet.insertions > 0 {
                Text("+\(changes.changeSet.insertions)").foregroundStyle(.green)
            }
            if changes.changeSet.deletions > 0 {
                Text("−\(changes.changeSet.deletions)").foregroundStyle(.red)
            }
        }
        .font(.system(size: 10.5, design: .monospaced))
    }

    private var jumpBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(changes.changeSet.files) { f in
                    Button { Task { await changes.openFile(f.path) } } label: {
                        fileChip(f)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
        }
    }

    private var fileColumn: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(changes.changeSet.files) { f in
                    Button { Task { await changes.openFile(f.path) } } label: {
                        VStack(alignment: .leading, spacing: 1) {
                            Text(f.path)
                                .font(.system(size: 11, design: .monospaced))
                                .lineLimit(1)
                                .truncationMode(.head)
                            HStack(spacing: 4) {
                                if f.insertions > 0 {
                                    Text("+\(f.insertions)").foregroundStyle(.green)
                                }
                                if f.deletions > 0 {
                                    Text("−\(f.deletions)").foregroundStyle(.red)
                                }
                            }
                            .font(.system(size: 10, design: .monospaced))
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(
                            changes.selectedFile == f.path
                                ? Color.accentColor.opacity(0.15) : .clear,
                            in: RoundedRectangle(cornerRadius: 4))
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(6)
        }
    }

    private func fileChip(_ f: ChangedFile) -> some View {
        HStack(spacing: 4) {
            Text((f.path as NSString).lastPathComponent)
                .font(.system(size: 10.5, design: .monospaced))
                .lineLimit(1)
            if f.insertions > 0 { Text("+\(f.insertions)").foregroundStyle(.green) }
            if f.deletions > 0 { Text("−\(f.deletions)").foregroundStyle(.red) }
        }
        .font(.system(size: 10, design: .monospaced))
        .padding(.horizontal, 6)
        .padding(.vertical, 3)
        .background(
            changes.selectedFile == f.path ? Color.accentColor.opacity(0.18) : .quaternary.opacity(0.3),
            in: Capsule())
    }

    private var diffScroll: some View {
        ScrollView {
            if changes.selectedFile == nil {
                Text("Pick a file to read it.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 24)
            } else {
                ScrollView(.horizontal, showsIndicators: false) {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(changes.diff) { line in
                            row(line)
                        }
                    }
                }
            }
        }
        .scrollContentBackground(.hidden)
    }

    /// Code does not wrap, so the diff scrolls sideways on its own rather than
    /// making the whole tile do it.
    private func row(_ line: DiffComputation.Line) -> some View {
        HStack(spacing: 0) {
            Text(line.oldNumber.map(String.init) ?? "")
                .frame(width: 30, alignment: .trailing)
                .foregroundStyle(.tertiary)
            Text(line.newNumber.map(String.init) ?? "")
                .frame(width: 30, alignment: .trailing)
                .foregroundStyle(.tertiary)
            Text(marker(line.kind)).frame(width: 12)
            Text(line.text.isEmpty ? " " : line.text)
                .fixedSize(horizontal: true, vertical: false)
            Spacer(minLength: 0)
        }
        .font(.system(size: 10.5, design: .monospaced))
        .background(background(line.kind))
    }

    // No syntax highlighting, still. What earns the pixels is which lines
    // changed, and colouring keywords on top of an add/remove background fights
    // the one signal that matters.
    private func marker(_ kind: DiffComputation.Kind) -> String {
        switch kind {
        case .added: return "+"
        case .removed: return "−"
        case .context: return " "
        }
    }

    private func background(_ kind: DiffComputation.Kind) -> Color {
        switch kind {
        case .added: return .green.opacity(0.13)
        case .removed: return .red.opacity(0.13)
        case .context: return .clear
        }
    }

    private func failure(_ message: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(
                changes.client.changesSupported == false
                    ? "This machine can't show changes yet" : "Couldn't read this worktree"
            )
            .font(.system(size: 11.5, weight: .medium))
            Text(
                changes.client.changesSupported == false
                    ? "Its copy of Far Cooler is older than this. Update it in Settings › Machines."
                    : message
            )
            .font(.system(size: 11))
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(9)
        .background(.orange.opacity(0.12))
    }
}
```

- [x] **Step 2: Build**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd apps/macos && swift build 2>&1 | grep -E "error:" | head -10
```

Expected: no errors.

- [x] **Step 3: Run it against a scratch daemon and read a real diff**

```bash
cd "/Users/e-liang/Library/Application Support/com.farcooler.FarCooler/worktrees/overnight-review"
export PATH="$HOME/.cargo/bin:$PATH"
SCRATCH=/Users/e-liang/.claude/jobs/32495d6e/tmp/wb-scratch
rm -rf "$SCRATCH"; mkdir -p "$SCRATCH/home" "$SCRATCH/dev"
(cd "$SCRATCH/dev" && git init -q --initial-branch=main demo && cd demo \
  && git config user.email t@e.com && git config user.name T \
  && git config commit.gpgsign false \
  && printf 'fn main() {\n    println!("hi");\n}\n' > main.rs \
  && git add . && git commit -q -m initial)
cargo build --release --bin farcooler --bin farcoolerd
export FARCOOLER_HOME="$SCRATCH/home"
nohup ./target/release/farcoolerd > "$SCRATCH/d.log" 2>&1 & echo $! > "$SCRATCH/d.pid"
sleep 2
./target/release/farcooler root add "$SCRATCH/dev"
REPO=$(./target/release/farcooler repo register "$SCRATCH/dev/demo" | grep -oE '[0-9a-f]{8}' | head -1)
./target/release/farcooler workspace create "$REPO" "tiles" --branch feat/tiles
```

Then edit a file inside the created worktree, commit it, and run `./target/release/farcooler changes status tiles` to confirm the daemon sees it before opening the app.

- [x] **Step 4: Stop the scratch daemon by PID**

```bash
kill "$(cat "$SCRATCH/d.pid")"
```

- [x] **Step 5: Commit**

```bash
git add apps/macos/Sources/FarCooler/DiffTile.swift
git commit -m "feat(macos): the diff tile, one concept at two widths"
```

---

### Task 8: Add and remove tiles

**Files:**
- Modify: `apps/macos/Sources/FarCooler/TileContainer.swift`
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift`

**Interfaces:**
- Consumes: everything above.
- Produces: a per-worktree tile menu in the toolbar that toggles a tile kind in and out of the layout.

- [x] **Step 1: Add toggling to `TileNode`**

Append to `Tiles.swift`:

```swift
extension TileNode {
    /// Add a tile of this kind beside what is already there, or take it away.
    ///
    /// Toggling rather than free-form arrangement, deliberately: the useful
    /// layouts here are "agent and diff", "agent, diff and PR", and one of them
    /// alone. A general splitter UI would be more powerful and would mostly be
    /// used to rebuild the default by hand.
    func toggling(_ kind: TileKind) -> TileNode {
        if contains(kind) {
            return removing(kind) ?? .leaf(kind == .tmux ? .diff : .tmux)
        }
        return .split(.horizontal, [self, .leaf(kind)], [0.6, 0.4])
    }

    /// The tree without that kind, or nil when nothing would be left.
    private func removing(_ kind: TileKind) -> TileNode? {
        switch self {
        case .leaf(let k):
            return k == kind ? nil : self
        case .split(let axis, let children, _):
            let kept = children.compactMap { $0.removing(kind) }
            if kept.isEmpty { return nil }
            if kept.count == 1 { return kept[0] }
            let share = 1.0 / Double(kept.count)
            return .split(axis, kept, Array(repeating: share, count: kept.count))
        }
    }
}
```

- [x] **Step 2: Add the toolbar menu in `ContentView`**

Beside the existing `openInEditorToolbar`:

```swift
                .toolbar {
                    if let ws = detailWorkspace {
                        ToolbarItem(placement: .primaryAction) {
                            Menu {
                                ForEach(TileKind.allCases, id: \.self) { kind in
                                    Toggle(
                                        kind.label,
                                        isOn: Binding(
                                            get: { tileBinding(ws).wrappedValue.contains(kind) },
                                            set: { _ in
                                                let b = tileBinding(ws)
                                                b.wrappedValue = b.wrappedValue.toggling(kind)
                                            }
                                        ))
                                }
                                Divider()
                                Button("Reset Layout") {
                                    TileLayout.forget(workspace: ws.id)
                                    tileLayouts[ws.id] = TileLayout.default
                                }
                            } label: {
                                Label("Tiles", systemImage: "rectangle.split.2x1")
                            }
                            .help("Choose what this worktree shows")
                        }
                    }
                }
```

- [x] **Step 3: Build**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd apps/macos && swift build 2>&1 | grep -E "error:" | head -10
```

Expected: no errors.

- [x] **Step 4: Run the whole suite and build the app bundle**

```bash
cd "/Users/e-liang/Library/Application Support/com.farcooler.FarCooler/worktrees/overnight-review"
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" \
  | awk -F'[ ;]' '{p+=$4; f+=$6} END {print "passed:", p, "failed:", f}'
cd apps/shared/AgentKit && swift test 2>&1 | tail -2
```

Expected: 0 failures on both.

- [x] **Step 5: Commit**

```bash
git add apps/macos
git commit -m "feat(macos): choose what a worktree shows"
```

---

## Self-Review

**Spec coverage.** Tile model and client-local persistence — Task 5. `TmuxCanvas` as one tile with tmux inside — Task 6. Default agent-left/diff-right — Task 5's `TileLayout.default`. Diff tile scope selector, long scroll, jump bar, wide column — Task 7. Buffer deletion — Tasks 1, 2, 4. Rename — Tasks 2, 3, 4. Sidebar keeps `+N −M`, loses the badge — Task 4. Fleet view, PR tile and phone segments are explicitly out of this plan's scope and named as follow-on plans.

**Known gap, stated rather than hidden:** the spec's Diff tile has three scopes — Branch, Commit, Local — and `DiffScope` here has two. Commit needs a commit picker, and the spec's own open question asks whether the PR tile should own commit selection instead. Rather than build a picker this plan would have to guess at, Commit is deferred to the PR tile's plan, where the commit list already exists.

**Type consistency.** `ChangesStore` is used with that name in Tasks 4, 6 and 7. `changesJSON`/`changesDiff`/`changesMarkRead`/`changesSupported`/`changesError` are named identically in Tasks 4 and 7. `TileNode`/`TileKind`/`TileLayout` match across Tasks 5, 6 and 8. `InboxRow` loses `needsYou` in Task 4 and nothing later references it.

---

## What the plan did not predict

Kept here rather than dropped, because each one was found by running the thing
rather than by reading it, and each cost more than it should have.

**The fractions were decorative.** Task 6 rendered splits with `HSplitView`,
which sizes children by what they ask for. `TileNode` stored fractions, saved
them and reloaded them, and the layout engine ignored every one — a 50/50
default drew the Changes tile as a sliver wide enough for its scope picker and
nothing else. Splits are sized from the fractions now, in `TileSizing`, which is
also what lets a dragged divider be written down.

**`build-app.sh` swallowed compile errors.** It sent `swift build`'s stdout to
`/dev/null`, and that is where the Swift compiler writes diagnostics. A release
build that failed left the previous bundle in place looking current, so two
rounds of "the UI hasn't changed" went into finding a one-line type error that
the debug build had not caught. Fixed in the same branch.

**Task 5's verification step was a placeholder** — it said to write a scratch
binary and then wrote a comment where the binary should be. `Tiles.swift` has no
UI imports, so it compiles standalone: the real check exercises the Codable round
trip, `toggling`, the collapse-a-one-child-split rule, the corrupt-payload
fallback, and all of `TileSizing`.

**Three things only a real worktree showed.** A layout with the Agents tile off
lost the window's name, because all four `navigationTitle` calls live inside
terminal views. An empty change set claimed "Nothing changed here" directly under
the banner saying why it could not read anything. And with no base to name, it
said "This branch matches ." — a sentence with a hole in it.

**One test fails for reasons outside the code.** `rpc_over_socket`'s
`removing_a_root_survives_a_stopped_terminal_in_the_main_checkout` fails under
full parallelism on a machine with ~170 live tmux servers and ~1400 socket files
in `/private/tmp/tmux-502`, which is exactly the exhaustion the harness comments
at the top of that file describe. It passes alone and the whole suite passes with
`--test-threads=2`. Nothing in this branch touches that path.
