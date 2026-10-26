//! `daemon_log` (ov-798): the log is bounded however much is written, keeps
//! the newest lines, and lives where an owner can find it.

use std::io::Write;

use super::*;

fn names(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

#[test]
fn many_lines_leave_at_most_keep_files_each_within_the_bound() {
    let dir = tempfile::tempdir().unwrap();
    let log = RotatingLog::open(dir.path(), 200, 3, false, None).unwrap();
    for n in 0..500 {
        writeln!(&log, "line {n:04} padding padding padding").unwrap();
    }
    assert_eq!(names(dir.path()), ["farcoolerd.log", "farcoolerd.log.1", "farcoolerd.log.2"]);
    for name in names(dir.path()) {
        let len = std::fs::metadata(dir.path().join(name)).unwrap().len();
        assert!(len <= 200, "a file grew past its bound: {len}");
    }
}

#[test]
fn the_newest_lines_survive_and_the_oldest_go() {
    let dir = tempfile::tempdir().unwrap();
    let log = RotatingLog::open(dir.path(), 200, 3, false, None).unwrap();
    for n in 0..500 {
        writeln!(&log, "line {n:04} padding padding padding").unwrap();
    }
    let current = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    assert!(current.contains("line 0499"));
    let all: String = names(dir.path())
        .iter()
        .map(|n| std::fs::read_to_string(dir.path().join(n)).unwrap())
        .collect();
    assert!(!all.contains("line 0000"), "the oldest line should have rotated away");
}

#[test]
fn a_restart_over_a_full_file_rotates_instead_of_growing_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(FILE_NAME), vec![b'x'; 500]).unwrap();
    let log = RotatingLog::open(dir.path(), 200, 3, false, None).unwrap();
    writeln!(&log, "after restart").unwrap();
    assert_eq!(std::fs::metadata(dir.path().join(FILE_NAME)).unwrap().len(), 14);
    assert_eq!(std::fs::metadata(dir.path().join("farcoolerd.log.1")).unwrap().len(), 500);
}

#[test]
fn a_symlinked_folder_is_refused() {
    let real = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let link = parent.path().join("logs");
    std::os::unix::fs::symlink(real.path(), &link).unwrap();
    assert!(RotatingLog::open(&link, 200, 3, false, None).is_err());
}

#[test]
fn a_symlinked_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let target = tempfile::NamedTempFile::new().unwrap();
    std::os::unix::fs::symlink(target.path(), dir.path().join(FILE_NAME)).unwrap();
    assert!(RotatingLog::open(dir.path(), 200, 3, false, None).is_err());
}

#[test]
fn loose_modes_are_tightened() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE_NAME);
    std::fs::write(&file, "old\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let _log = RotatingLog::open(dir.path(), 200, 3, false, None).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&file), 0o600);
    assert_eq!(mode(dir.path()), 0o700);
}
