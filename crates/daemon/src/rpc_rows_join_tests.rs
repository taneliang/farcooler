//! The first page's join (ov-830 re-review, M1): a claude typed into a
//! shell is found by its live process in claude's registry, and a process
//! that isn't live (a crashed claude's file, a pid used again) finds none.

use super::*;
use crate::claude_registry::fake;

fn registry(config: &std::path::Path, alive: Vec<(i32, Option<&'static str>)>) -> Registry {
    Registry::unwatched(config.to_path_buf(), Box::new(fake::Alive(alive)))
}

/// Goes red with `by_pid` never matching (the join finding no typed claude).
#[test]
fn a_typed_claude_is_found_by_its_live_process_and_a_dead_one_isnt() {
    let config = tempfile::tempdir().unwrap();
    fake::write(config.path(), 4242, "s-typed", "/nonexistent/fc-rows-join");
    let project = config.path().join("projects/-nonexistent-fc-rows-join");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join("s-typed.jsonl");
    std::fs::write(&transcript, "").unwrap();

    let live = registry(config.path(), vec![(4242, None)]);
    assert_eq!(transcript_for("claude", 4242, &live, "/nonexistent/fc-rows-join"), Some(transcript), "a live claude's own");
    assert_eq!(transcript_for("shell", 4242, &live, "/nonexistent/fc-rows-join"), None, "not an agent");

    let dead = registry(config.path(), Vec::new());
    assert_eq!(transcript_for("claude", 4242, &dead, "/nonexistent/fc-rows-join"), None, "a file a gone claude left");
}
