use super::*;

fn store_with_worktree() -> (Store, Uuid, u64) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let wt = store.create_worktree(repo, "feature", "/tmp/fc-lfs-test/feature", false).unwrap();
    (store, wt.id, wt.resource_version)
}

#[test]
fn recorded_paths_round_trip_and_count_per_worktree() {
    let (store, wt, _) = store_with_worktree();
    assert!(store.lfs_pointer_counts().unwrap().is_empty());
    assert!(store.set_lfs_pointers(wt, &["b.bin".into(), "a.bin".into(), "a.bin".into()]).unwrap());
    assert_eq!(store.lfs_pointer_paths(wt).unwrap(), ["a.bin", "b.bin"]);
    assert_eq!(store.lfs_pointer_counts().unwrap().get(&wt), Some(&2));
}

#[test]
fn setting_replaces_and_only_a_change_moves_the_version() {
    let (store, wt, v0) = store_with_worktree();
    let version = |s: &Store| s.get_worktree(wt).unwrap().resource_version;
    assert!(store.set_lfs_pointers(wt, &["a.bin".into()]).unwrap());
    let v1 = version(&store);
    assert!(v1 > v0, "a change moves the version");
    assert!(!store.set_lfs_pointers(wt, &["a.bin".into()]).unwrap(), "the same set is no change");
    assert_eq!(version(&store), v1);
    assert!(store.set_lfs_pointers(wt, &[]).unwrap());
    assert!(store.lfs_pointer_counts().unwrap().is_empty(), "an empty set removes the rows");
}

#[test]
fn deleting_the_worktree_takes_its_rows() {
    let (store, wt, _) = store_with_worktree();
    store.set_lfs_pointers(wt, &["a.bin".into()]).unwrap();
    let conn = store.conn();
    conn.execute("DELETE FROM worktrees WHERE id = ?1", params![uuid_blob(wt)]).unwrap();
    drop(conn);
    assert!(store.lfs_pointer_counts().unwrap().is_empty());
}
