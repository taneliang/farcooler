//! Which files are still LFS pointers (ov-199): read off the files, beneath
//! the worktree, never through a link.

use super::*;

const POINTER: &str = "version https://git-lfs.github.com/spec/v1\noid sha256:aa\nsize 9\n";

fn paths(of: &[&str]) -> Vec<String> {
    of.iter().map(|p| p.to_string()).collect()
}

#[tokio::test]
async fn only_a_small_regular_file_that_starts_like_a_pointer_is_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("pointer.bin"), POINTER).unwrap();
    std::fs::write(root.join("hawser.bin"), "version https://hawser.github.com/spec/v1\noid sha256:aa\nsize 9\n").unwrap();
    std::fs::write(root.join("content.bin"), vec![7u8; 4096]).unwrap();
    // Starts like a pointer, but is two kilobytes of something else.
    std::fs::write(root.join("long.bin"), format!("{POINTER}{}", "x".repeat(2048))).unwrap();
    std::fs::write(root.join("text.bin"), "just some words\n").unwrap();
    std::fs::create_dir(root.join("dir.bin")).unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/nested.bin"), POINTER).unwrap();
    // A link named like the path, to a real pointer: never read through.
    std::os::unix::fs::symlink(root.join("pointer.bin"), root.join("link.bin")).unwrap();
    // A directory that is a link, and a pointer behind it.
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("x.bin"), POINTER).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), root.join("out")).unwrap();

    let all = paths(&[
        "pointer.bin", "hawser.bin", "content.bin", "long.bin", "text.bin", "dir.bin", "sub/nested.bin", "gone.bin",
        "link.bin", "out/x.bin",
    ]);
    assert_eq!(pointers(&root, &all).await, paths(&["pointer.bin", "hawser.bin", "sub/nested.bin"]));
}

#[tokio::test]
async fn nothing_listed_is_nothing_left() {
    let dir = tempfile::tempdir().unwrap();
    assert!(pointers(dir.path(), &[]).await.is_empty());
}

#[test]
fn nul_separated_output_is_split_into_paths() {
    assert_eq!(paths_of(b"a.bin\0dir/b c.bin\0"), paths(&["a.bin", "dir/b c.bin"]));
    assert!(paths_of(b"").is_empty());
}
