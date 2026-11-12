//! A group's fold tree goes when the group ends, its build output goes to the next fold, and a tree under the name
//! from before ov-1081 goes unless a live run has it (ov-1084).
use super::*;
use super::coding_verbs_tests::{at_join, batched_push, command, lander, phase_of, until};

/// The group a ticket ran in, live or archived.
fn group_of(state: &State) -> String {
    state.items.iter().find(|(_, i)| i.group_of.is_some()).map(|(id, _)| id.clone())
        .or_else(|| state.archive.iter().find(|(_, a)| a.group.is_some()).map(|(id, _)| id.clone()))
        .expect("a group ran")
}
async fn branch_exists(runtime: &Runtime, workspace: &str, item: &str) -> bool {
    let branch = format!("refs/heads/wf-{}", &digest(&(workspace, item))[4..]);
    crate::git::git(&runtime.repository(), &["rev-parse", "--verify", "--quiet", &branch]).await.unwrap().ok
}
/// Git ignores `target/` in every tree of the repository, as a Rust repository's .gitignore does.
fn ignore_target(runtime: &Runtime) {
    let info = runtime.repository().join(".git/info");
    std::fs::create_dir_all(&info).unwrap();
    std::fs::write(info.join("exclude"), "target/\n").unwrap();
}
fn trees(runtime: &Runtime) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(&runtime.root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("tree-")).collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_landed_group_leaves_no_tree_and_the_next_fold_builds_on_its_output() {
    let (dir, mut runtime) = lander("build").await;
    ignore_target(&runtime);
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let first = group_of(&state);
    assert!(!runtime.item_tree("w", &first).exists(), "the group's tree went as it ended: {:?}", trees(&runtime));
    assert!(!branch_exists(&runtime, "w", &first).await, "and its branch");
    let listed = adapters::git(&runtime.repository(), &["worktree", "list", "--porcelain"]).await.unwrap();
    assert!(!listed.contains(&folder_name_of(&first)), "the repository forgot it: {listed}");

    at_join(&mut runtime, "two", &definition).await;
    command(&runtime, Command::Queue { item: "two".into() });
    until(&mut runtime, |s| phase_of(s, "two") == Some(Phase::Done)).await;
    assert_eq!(std::fs::read_to_string(dir.path().join("marker")).unwrap(), "cold\nwarm\n",
        "the second gate started from the first one's build output");
    // Only the tickets' own trees are left: neither group's fold tree.
    let mut tickets = vec![format!("tree-{}", folder_name_of("one")), format!("tree-{}", folder_name_of("two"))];
    tickets.sort();
    assert_eq!(trees(&runtime), tickets);
}

fn folder_name_of(item: &str) -> String { adapters::folder_name(&digest(&("w", item))) }

#[tokio::test]
async fn a_failed_group_leaves_no_tree() {
    let (_dir, mut runtime) = lander("gate-fail").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    let group = group_of(&state);
    assert_eq!(phase_of(&state, &group), Some(Phase::Failed), "the failed group stays on the board");
    assert!(!runtime.item_tree("w", &group).exists(), "but not its tree: {:?}", trees(&runtime));
}

/// The fold tree of a group that is still working stays, and so does a run's tree under the old name while the run
/// is live; an old-name tree no live run has goes when the daemon starts, as does a delete it didn't finish.
#[tokio::test]
async fn old_name_trees_nobody_runs_in_are_pruned_on_recovery() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    let repo = runtime.repository();
    let base = adapters::git(&repo, &["rev-parse", "HEAD"]).await.unwrap();
    let orphan = runtime.root.join(format!("tree-{}", digest(&("w", "long-gone"))));
    let live = runtime.root.join(format!("tree-{}", digest(&("w", "one"))));
    for tree in [&orphan, &live] {
        adapters::git(&repo, &["worktree", "add", "--detach", tree.to_str().unwrap(), &base]).await.unwrap();
    }
    let unknown = runtime.root.join(format!("tree-{}", folder_name_of("not-on-this-board")));
    adapters::git(&repo, &["worktree", "add", "--detach", unknown.to_str().unwrap(), &base]).await.unwrap();
    let trash = runtime.root.join("trash-left-by-a-crash");
    std::fs::create_dir_all(trash.join("deep")).unwrap();

    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(10).await.unwrap();
    assert!(!orphan.exists(), "the old-name tree nobody has is gone");
    assert!(live.exists(), "a live run's old-name tree stays");
    assert!(unknown.exists(), "a current-name tree no state knows of is left alone");
    let listed = adapters::git(&repo, &["worktree", "list", "--porcelain"]).await.unwrap();
    assert!(!listed.contains(orphan.to_str().unwrap()), "{listed}");
    for _ in 0..100 {
        if !trash.exists() { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!trash.exists(), "a delete a stopped daemon left is finished");
    assert!(std::fs::read_dir(&root).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with("trash-left")));
}
