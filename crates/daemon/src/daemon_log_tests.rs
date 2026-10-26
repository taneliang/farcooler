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
    let log = RotatingLog::open(dir.path(), 200, 3, false).unwrap();
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
    let log = RotatingLog::open(dir.path(), 200, 3, false).unwrap();
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
    let log = RotatingLog::open(dir.path(), 200, 3, false).unwrap();
    writeln!(&log, "after restart").unwrap();
    assert_eq!(std::fs::metadata(dir.path().join(FILE_NAME)).unwrap().len(), 14);
    assert_eq!(std::fs::metadata(dir.path().join("farcoolerd.log.1")).unwrap().len(), 500);
}
