//! A worktree's files as JSON (ov-189): what `farcooler files --json` prints
//! and the Mac decodes, and what a phone's Files screen will receive over the
//! FFI. One shape for every client, as `changes_json` is for diffs.
//!
//! Field names in camelCase, the apps' own, and every enum as a lowercase
//! word, never its number: a word an older app has never heard is one it can
//! show as unknown, where a number would decode as the wrong thing.

use farcooler_protocol::v1 as pb;
use serde_json::json;

/// `worktree.list_dir`'s answer.
pub fn dir_json(d: &pb::WorktreeDir) -> serde_json::Value {
    json!({
        "path": d.path,
        "truncated": d.truncated,
        "entries": d.entries.iter().map(|e| json!({
            "name": e.name,
            "kind": match pb::WorktreeEntryKind::try_from(e.kind) {
                Ok(pb::WorktreeEntryKind::File) => "file",
                Ok(pb::WorktreeEntryKind::Directory) => "directory",
                Ok(pb::WorktreeEntryKind::Link) => "link",
                _ => "other",
            },
            "size": e.size,
            "linkTarget": e.link_target,
        })).collect::<Vec<_>>(),
    })
}

/// `worktree.read_file`'s answer.
pub fn file_json(f: &pb::WorktreeFile) -> serde_json::Value {
    json!({
        "path": f.path,
        "state": match pb::WorktreeFileState::try_from(f.state) {
            Ok(pb::WorktreeFileState::Text) => "text",
            Ok(pb::WorktreeFileState::Binary) => "binary",
            Ok(pb::WorktreeFileState::TooLarge) => "too_large",
            Ok(pb::WorktreeFileState::Link) => "link",
            _ => "unknown",
        },
        "size": f.size,
        "text": f.text,
        "linkTarget": f.link_target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_and_state_is_a_word() {
        let d = pb::WorktreeDir {
            path: "src".into(),
            truncated: true,
            entries: vec![
                pb::WorktreeDirEntry { name: "a".into(), kind: pb::WorktreeEntryKind::Directory as i32, ..Default::default() },
                pb::WorktreeDirEntry { name: "b".into(), kind: pb::WorktreeEntryKind::File as i32, size: 3, ..Default::default() },
                pb::WorktreeDirEntry {
                    name: "c".into(), kind: pb::WorktreeEntryKind::Link as i32, link_target: "../x".into(), ..Default::default()
                },
                pb::WorktreeDirEntry { name: "d".into(), kind: 99, ..Default::default() },
            ],
        };
        let v = dir_json(&d);
        let kinds: Vec<&str> = v["entries"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["directory", "file", "link", "other"]);
        assert_eq!(v["entries"][2]["linkTarget"], "../x");
        assert_eq!(v["entries"][1]["size"], 3);
        assert_eq!(v["truncated"], true);

        let states = [
            (pb::WorktreeFileState::Text, "text"),
            (pb::WorktreeFileState::Binary, "binary"),
            (pb::WorktreeFileState::TooLarge, "too_large"),
            (pb::WorktreeFileState::Link, "link"),
            (pb::WorktreeFileState::Unspecified, "unknown"),
        ];
        for (state, word) in states {
            let f = pb::WorktreeFile { path: "p".into(), state: state as i32, text: "t".into(), ..Default::default() };
            assert_eq!(file_json(&f)["state"], word);
        }
    }
}
