//! The Files tab's reads stay inside the worktree, whatever the path says and
//! whatever an agent planted in it.

use std::os::unix::fs::symlink;
use std::path::Path;

use super::*;

/// A worktree beside a secret, and the secret's text.
fn worktree_beside_a_secret() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("tree");
    std::fs::create_dir_all(root.join("src/deep")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("src/deep/a.txt"), "a\n").unwrap();
    std::fs::write(root.join(".env"), "TOKEN=plain\n").unwrap();
    std::fs::create_dir_all(dir.path().join("outside")).unwrap();
    std::fs::write(dir.path().join("outside/secret"), "SECRET\n").unwrap();
    (dir, root)
}

fn text(content: Content) -> String {
    match content {
        Content::Text { text, .. } => text,
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn a_file_in_the_worktree_is_read_whole() {
    let (_dir, root) = worktree_beside_a_secret();
    assert_eq!(text(read(&root, "src/main.rs").unwrap()), "fn main() {}\n");
    assert_eq!(text(read(&root, "src/deep/a.txt").unwrap()), "a\n");
}

#[test]
fn a_dot_env_is_shown_plainly() {
    let (_dir, root) = worktree_beside_a_secret();
    assert_eq!(text(read(&root, ".env").unwrap()), "TOKEN=plain\n");
    let names: Vec<String> = list(&root, "").unwrap().entries.into_iter().map(|e| e.name).collect();
    assert!(names.contains(&".env".to_string()), "{names:?}");
}

#[test]
fn dot_dot_and_absolute_paths_are_refused_before_anything_opens() {
    let (_dir, root) = worktree_beside_a_secret();
    for bad in ["../outside/secret", "src/../../outside/secret", "/etc/hosts", "./src/main.rs", "src/./main.rs"] {
        assert_eq!(read(&root, bad), Err(Refusal::NotRelative), "{bad}");
    }
    assert_eq!(list(&root, ".."), Err(Refusal::NotRelative));
    assert_eq!(list(&root, "/"), Err(Refusal::NotRelative));
    assert_eq!(read(&root, ""), Err(Refusal::NotRelative));
}

#[test]
fn a_linked_directory_on_the_way_is_refused_not_followed() {
    let (dir, root) = worktree_beside_a_secret();
    symlink(dir.path().join("outside"), root.join("escape")).unwrap();
    assert_eq!(read(&root, "escape/secret"), Err(Refusal::NotFound));
    assert_eq!(list(&root, "escape").map(|l| l.entries), Err(Refusal::NotFound));
    // Relative, and deeper: the same.
    symlink("../../../outside", root.join("src/deep/up")).unwrap();
    assert_eq!(read(&root, "src/deep/up/secret"), Err(Refusal::NotFound));
}

#[test]
fn a_link_at_the_end_is_answered_as_a_link_and_never_read() {
    let (dir, root) = worktree_beside_a_secret();
    let secret = dir.path().join("outside/secret");
    symlink(&secret, root.join("innocent.txt")).unwrap();
    assert_eq!(
        read(&root, "innocent.txt"),
        Ok(Content::Link { target: secret.to_string_lossy().into_owned() })
    );
    let entry = list(&root, "").unwrap().entries.into_iter().find(|e| e.name == "innocent.txt").unwrap();
    assert_eq!(entry.kind, EntryKind::Link);
    assert_eq!(entry.link_target, secret.to_string_lossy());
}

#[test]
fn a_fifo_is_listed_and_refused_without_hanging() {
    let (_dir, root) = worktree_beside_a_secret();
    let made = std::process::Command::new("mkfifo").arg(root.join("pipe")).status().expect("mkfifo");
    assert!(made.success());
    assert_eq!(read(&root, "pipe"), Err(Refusal::WrongKind));
    let entry = list(&root, "").unwrap().entries.into_iter().find(|e| e.name == "pipe").unwrap();
    assert_eq!(entry.kind, EntryKind::Other);
}

#[test]
fn a_directory_read_as_a_file_and_a_file_listed_are_the_wrong_kind_or_missing() {
    let (_dir, root) = worktree_beside_a_secret();
    assert_eq!(read(&root, "src"), Err(Refusal::WrongKind));
    assert_eq!(list(&root, "src/main.rs").map(|l| l.truncated), Err(Refusal::NotFound));
    assert_eq!(read(&root, "nope.txt"), Err(Refusal::NotFound));
}

#[test]
fn binary_and_too_large_files_say_so_and_send_nothing() {
    let (_dir, root) = worktree_beside_a_secret();
    std::fs::write(root.join("blob.bin"), b"PNG\0\x01\x02").unwrap();
    assert_eq!(read(&root, "blob.bin"), Ok(Content::Binary { size: 6 }));
    std::fs::write(root.join("latin1.txt"), b"caf\xe9\n").unwrap();
    assert_eq!(read(&root, "latin1.txt"), Ok(Content::Binary { size: 5 }));
    let big = vec![b'x'; MAX_READ_BYTES as usize + 1];
    std::fs::write(root.join("big.log"), &big).unwrap();
    assert_eq!(read(&root, "big.log"), Ok(Content::TooLarge { size: big.len() as u64 }));
    let edge = vec![b'y'; MAX_READ_BYTES as usize];
    std::fs::write(root.join("edge.log"), &edge).unwrap();
    assert!(matches!(read(&root, "edge.log"), Ok(Content::Text { size, .. }) if size == MAX_READ_BYTES));
}

#[test]
fn a_listing_puts_directories_first_hides_dot_git_and_sorts_by_name() {
    let (_dir, root) = worktree_beside_a_secret();
    std::fs::create_dir(root.join(".git")).unwrap();
    std::fs::write(root.join("B.md"), "b").unwrap();
    std::fs::write(root.join("a.md"), "aa").unwrap();
    std::fs::create_dir(root.join("Zeta")).unwrap();
    let listing = list(&root, "").unwrap();
    let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["src", "Zeta", ".env", "a.md", "B.md"]);
    assert_eq!(listing.entries[3].size, 2);
    assert!(!listing.truncated);
    // `.git` is hidden only at the root; a directory of that name deeper is
    // somebody's file.
    std::fs::create_dir(root.join("src/.git")).unwrap();
    assert!(list(&root, "src").unwrap().entries.iter().any(|e| e.name == ".git"));
}

#[test]
fn a_huge_directory_stops_at_the_cap_and_says_so() {
    let (_dir, root) = worktree_beside_a_secret();
    let many = root.join("many");
    std::fs::create_dir(&many).unwrap();
    for i in 0..=MAX_ENTRIES {
        std::fs::write(many.join(format!("f{i}")), "").unwrap();
    }
    let listing = list(&root, "many").unwrap();
    assert_eq!(listing.entries.len(), MAX_ENTRIES);
    assert!(listing.truncated);
}

#[test]
fn the_root_itself_may_be_reached_through_a_link() {
    // The worktree's own path is the daemon's, not a commit's: `/tmp` on a
    // Mac is a link to `/private/tmp`, and a root under it must still list.
    let (dir, root) = worktree_beside_a_secret();
    let alias = dir.path().join("alias");
    symlink(&root, &alias).unwrap();
    assert!(list(Path::new(&alias), "src").is_ok());
}
