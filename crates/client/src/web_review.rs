//! Web worktree presentation. A comparison is identified by Git objects, with
//! the worktree digest added only for the mutable, uncommitted comparison.
use crate::{changes_json, web_board, workflow_presentation::Action};
use farcooler_protocol::v1 as pb;
use farcooler_workflow::digest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    #[default]
    Branch,
    Local,
}

impl Comparison {
    pub fn scope(self) -> &'static str {
        match self {
            Self::Branch => "branch",
            Self::Local => "local",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Position {
    pub comparison: Comparison,
    pub candidate: String,
    pub file: String,
    pub from: u32,
    pub line: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Marks {
    candidate: String,
    files: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum Request {
    Directory {
        #[serde(default)]
        path: String,
    },
    File {
        path: String,
    },
    Changes {
        #[serde(default)]
        position: Position,
        #[serde(default)]
        marks: Marks,
        edit: Option<Edit>,
    },
    Verify {
        candidate: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Edit {
    Review { reviewed: bool },
    Comment { body: String },
}

pub struct Candidate<'a> {
    pub id: String,
    pub position: Position,
    pub stale: bool,
    files: BTreeMap<&'a str, &'a pb::FileChange>,
    change_set: &'a pb::ChangeSet,
}

#[derive(Deserialize, Serialize)]
struct NoteAnchor {
    candidate: String,
    file: String,
    line: String,
}

impl<'a> Candidate<'a> {
    pub fn new(tree: Uuid, changes: &'a pb::ChangeSet, mut position: Position) -> Self {
        let scope = position.comparison.scope();
        let local = matches!(position.comparison, Comparison::Local);
        let id = digest(&(
            "web-review/v1",
            tree,
            scope,
            &changes.base_commit,
            &changes.head_commit,
            local.then_some(&changes.worktree_digest),
        ));
        let stale = !position.candidate.is_empty() && position.candidate != id;
        let mut files = BTreeMap::new();
        if local {
            if let Some(working) = &changes.working_tree {
                for file in working
                    .staged
                    .iter()
                    .chain(&working.unstaged)
                    .chain(&working.untracked_files)
                {
                    files.insert(file.path.as_str(), file);
                }
            }
        } else {
            files.extend(changes.files.iter().map(|file| (file.path.as_str(), file)));
        }
        if !files.contains_key(position.file.as_str()) {
            position.file = files.keys().next().copied().unwrap_or_default().to_owned();
            position.from = 0;
            position.line.clear();
        }
        if stale || position.candidate.is_empty() {
            position.from = 0;
            position.line.clear();
        }
        position.candidate.clone_from(&id);
        Self {
            id,
            position,
            stale,
            files,
            change_set: changes,
        }
    }

    pub fn file_id(&self, path: &str) -> String {
        digest(&("web-review-file/v1", &self.id, path))
    }

    fn line_id(file_id: &str, line: &pb::DiffLine) -> String {
        digest(&(
            "web-review-line/v1",
            file_id,
            line.old_no,
            line.new_no,
            line.kind,
            &line.text,
        ))
    }

    pub fn matches_patch(&self, patch: &pb::FileDiff) -> bool {
        if patch.path != self.position.file {
            return false;
        }
        // Older runners identify the selected range by cache generation.
        if patch.head_commit.is_empty() {
            return patch.change_set_version == self.change_set.version;
        }
        patch.head_commit == self.change_set.head_commit
            && patch.base_commit == self.change_set.base_commit
            && (matches!(self.position.comparison, Comparison::Branch)
                || patch.worktree_digest == self.change_set.worktree_digest)
    }

    pub fn selected_line(&self, patch: &pb::FileDiff) -> bool {
        let file_id = self.file_id(&self.position.file);
        !self.position.line.is_empty()
            && patch
                .hunks
                .iter()
                .flat_map(|h| &h.lines)
                .any(|line| Self::line_id(&file_id, line) == self.position.line)
    }

    pub fn comment_extra(&self) -> String {
        json!({"web_review": NoteAnchor {
            candidate: self.id.clone(),
            file: self.file_id(&self.position.file),
            line: self.position.line.clone(),
        }})
        .to_string()
    }

    pub fn update_marks(&self, marks: &mut Marks, reviewed: bool) {
        if marks.candidate != self.id {
            marks.candidate.clone_from(&self.id);
            marks.files.clear();
        }
        let id = self.file_id(&self.position.file);
        if reviewed {
            marks.files.insert(id);
        } else {
            marks.files.remove(&id);
        }
    }

    pub fn view(
        &self,
        patch: Option<&pb::FileDiff>,
        notes: &[pb::TaskNote],
        scope: &str,
        marks: &Marks,
        approval: Option<(&str, &Action)>,
    ) -> Value {
        let reviewed = |id: &str| marks.candidate == self.id && marks.files.contains(id);
        let file_id = self.file_id(&self.position.file);
        let mut comments = BTreeMap::<String, Vec<Value>>::new();
        for note in notes
            .iter()
            .filter(|n| n.kind == pb::TaskNoteKind::Comment as i32)
        {
            let Ok(extra) = serde_json::from_str::<Value>(&note.extra_json) else {
                continue;
            };
            let Ok(anchor) = serde_json::from_value::<NoteAnchor>(extra["web_review"].clone())
            else {
                continue;
            };
            if anchor.candidate == self.id && anchor.file == file_id {
                comments
                    .entry(anchor.line)
                    .or_default()
                    .push(json!({"actor": note.actor, "body": note.body}));
            }
        }
        let mut position = self.position.clone();
        if !patch.is_some_and(|p| self.selected_line(p)) {
            position.line.clear();
        }
        let diff = patch.map(|p| {
            let mut diff = changes_json::file_diff_json(p);
            for (hunk, view) in p
                .hunks
                .iter()
                .zip(diff["hunks"].as_array_mut().expect("diff hunks"))
            {
                for (line, view) in hunk
                    .lines
                    .iter()
                    .zip(view["lines"].as_array_mut().expect("diff lines"))
                {
                    let id = Self::line_id(&file_id, line);
                    view["id"] = json!(id);
                    view["label"] = json!(format!(
                        "Line {}",
                        line.new_no.or(line.old_no).unwrap_or_default()
                    ));
                    view["sign"] = json!(match pb::DiffLineKind::try_from(line.kind) {
                        Ok(pb::DiffLineKind::Added) => "+",
                        Ok(pb::DiffLineKind::Removed) => "−",
                        _ => " ",
                    });
                    view["position"] = json!(Position {
                        line: id.clone(),
                        ..position.clone()
                    });
                    view["comments"] = json!(comments.remove(&id).unwrap_or_default());
                }
            }
            diff["notice"] = json!(match p
                .unsupported
                .and_then(|u| pb::DiffUnsupported::try_from(u).ok())
            {
                Some(pb::DiffUnsupported::Binary) => Some("Binary file"),
                Some(pb::DiffUnsupported::Submodule) => Some("Submodule"),
                Some(_) => Some("Text diff is unavailable"),
                None if p.truncated.is_some() => Some("This page shows part of the diff."),
                None if p.hunks.is_empty() => Some("No text changes in this file."),
                None => None,
            });
            diff["next"] = json!(p.next_hunk.map(|from| (
                "Next diff page",
                Position {
                    from,
                    line: String::new(),
                    ..position.clone()
                }
            )));
            diff
        });
        let files: Vec<_> = self.files.iter().map(|(path, file)| {
            let id = self.file_id(path);
            let stats = (!matches!(position.comparison, Comparison::Local))
                .then(|| format!("+{} −{}", file.insertions, file.deletions));
            json!({
                "id": id, "path": path, "oldPath": file.old_path, "stats": stats,
                "reviewed": reviewed(&id).then_some("Reviewed"),
                "position": Position { file: path.to_string(), line: String::new(), from: 0, ..position.clone() },
            })
        }).collect();
        json!({
            "title": "Changes", "position": position, "marks": marks, "files": files, "diff": diff,
            "branch": self.change_set.branch,
            "baseWarning": (self.change_set.base_source == pb::BaseSource::Guessed as i32)
                .then(|| format!("The runner inferred the base from {}.", self.change_set.base_ref)),
            "summary": match position.comparison {
                Comparison::Branch => format!("{} → {}", abbrev(&self.change_set.base_commit), abbrev(&self.change_set.head_commit)),
                Comparison::Local => format!("Uncommitted changes against {}", abbrev(&self.change_set.head_commit)),
            },
            "comparisons": [
                {"label": "Candidate", "position": Position::default(), "selected": matches!(position.comparison, Comparison::Branch)},
                {"label": "Uncommitted", "position": Position {comparison: Comparison::Local, ..Position::default()},
                    "selected": matches!(position.comparison, Comparison::Local)},
            ],
            "notice": self.stale.then_some("The candidate changed. Review the current diff before continuing."),
            "empty": "No changed files", "refresh": "Refresh",
            "mark": patch.map(|_| {
                    let reviewed = reviewed(&file_id);
                    json!({"label":if reviewed {"Mark unreviewed"} else {"Mark reviewed"},
                        "edit":{"action":"review", "reviewed":!reviewed}})
                }),
            "editor": (web_board::can_write(scope) && !position.line.is_empty())
                .then_some(json!({"label":"Inline comment", "submit":"Post comment"})),
            "workflow": "Workflow Runs",
            "approval": approval.map(|(item, action)| json!({"item":item, "action":action})),
        })
    }
}

fn abbrev(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

pub fn files(raw: &Value, is_dir: bool) -> Value {
    let path = raw["path"].as_str().unwrap_or_default();
    let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
    let entries = raw["entries"].as_array().into_iter().flatten().map(|entry| {
        let name = entry["name"].as_str().unwrap_or_default();
        let kind = entry["kind"].as_str().unwrap_or_default();
        let path = if path.is_empty() { name.to_owned() } else { format!("{path}/{name}") };
        json!({
            "label": name, "kind": match kind {
                "directory" => "Folder", "file" => "File", "link" => "Link", _ => "Unavailable",
            },
            "view": match kind {"directory" => Some("directory"), "file" | "link" => Some("file"), _ => None},
            "path": path,
        })
    }).collect::<Vec<_>>();
    json!({
        "title": "Files", "path": path, "root": "Worktree root", "refresh": "Refresh",
        "parent": (!path.is_empty()).then_some(json!({"label":"Up one folder", "path":parent})),
        "entries": entries,
        "notice": if is_dir {
            if raw["truncated"] == true { Some("This folder listing was truncated.") }
            else if entries.is_empty() { Some("Empty folder") } else { None }
        } else {
            match raw["state"].as_str() {
                Some("text" | "link") => None,
                Some("binary") => Some("Binary file"),
                Some("too_large") => Some("This file is too large to preview."),
                _ => Some("File preview unavailable"),
            }
        },
        "text": match raw["state"].as_str() {
            Some("text") => raw["text"].as_str(),
            Some("link") => raw["linkTarget"].as_str(),
            _ => None,
        },
    })
}

pub fn links(scope: &str, worktree: bool, files: bool, changes: bool) -> Value {
    let source = web_board::can_write(scope) && worktree;
    json!({
        "files": (source && files).then_some("Files"),
        "changes": (source && changes).then_some("Changes"),
        "back": "Back to Ticket",
        "loading": "Loading worktree…",
    })
}

pub fn failure(word: &str, code: &str, what: &str) -> &'static str {
    match (word, code, what) {
        (_, _, "review_stale") => "The candidate or diff changed. Refresh before continuing.",
        (_, _, "review_worktree") => "The ticket’s worktree is no longer available.",
        (_, _, "review_comment") => "Enter an inline comment.",
        (_, "not-found", _) => "The ticket or path is no longer available.",
        (_, "capability-unsupported", _) | ("runner_too_old", _, _) => {
            "Update Far Cooler on the runner to use this view."
        }
        _ => web_board::failure(word, code, what),
    }
}

pub fn approval_result(raw: &str) -> &'static str {
    if serde_json::from_str::<Value>(raw).is_ok_and(|v| v.pointer("/receipt/result/Ok").is_some()) {
        "The workflow accepted the decision."
    } else {
        "The workflow refused the decision. Refresh to see the current candidate."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_survive_recomputation_and_do_not_follow_a_replacement_candidate() {
        let tree = Uuid::now_v7();
        let mut changes = pb::ChangeSet {
            base_commit: "base".into(),
            head_commit: "head".into(),
            worktree_digest: "dirty".into(),
            files: vec![pb::FileChange {
                path: "src/main.rs".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let first = Candidate::new(tree, &changes, Position::default());
        let patch = pb::FileDiff {
            path: first.position.file.clone(),
            hunks: vec![pb::Hunk {
                lines: vec![
                    pb::DiffLine {
                        old_no: Some(10),
                        text: "same text".into(),
                        kind: pb::DiffLineKind::Removed as i32,
                        ..Default::default()
                    },
                    pb::DiffLine {
                        new_no: Some(10),
                        text: "same text".into(),
                        kind: pb::DiffLineKind::Added as i32,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let first_view = first.view(Some(&patch), &[], "control", &Marks::default(), None);
        let [old, new] = [
            &first_view["diff"]["hunks"][0]["lines"][0],
            &first_view["diff"]["hunks"][0]["lines"][1],
        ];
        assert_ne!(old["id"], new["id"]);
        let position: Position = serde_json::from_value(new["position"].clone()).unwrap();

        changes.version += 1;
        changes.worktree_digest = "other dirty state".into();
        let current = Candidate::new(tree, &changes, position.clone());
        assert!(current.selected_line(&patch));
        let mut marks = Marks::default();
        current.update_marks(&mut marks, true);
        let notes = [pb::TaskNote {
            kind: pb::TaskNoteKind::Comment as i32,
            extra_json: current.comment_extra(),
            body: "Keep this check.".into(),
            ..Default::default()
        }];
        let view = current.view(Some(&patch), &notes, "read", &marks, None);
        assert_eq!(view["files"][0]["reviewed"], "Reviewed");
        assert_eq!(
            view["diff"]["hunks"][0]["lines"][1]["comments"][0]["body"],
            "Keep this check."
        );
        assert!(view["editor"].is_null());

        changes.head_commit = "replacement".into();
        let replacement = Candidate::new(tree, &changes, position);
        assert!(replacement.stale);
        assert_eq!(replacement.position.file, "src/main.rs");
        let view = replacement.view(Some(&patch), &notes, "control", &marks, None);
        assert_eq!(view["position"]["line"], "");
        assert_eq!(view["diff"]["hunks"][0]["lines"][1]["comments"], json!([]));
        assert!(view["files"][0]["reviewed"].is_null());
    }

    #[test]
    fn uncommitted_candidate_moves_with_the_working_tree() {
        let tree = Uuid::now_v7();
        let mut changes = pb::ChangeSet::default();
        let position = Position {
            comparison: Comparison::Local,
            ..Position::default()
        };
        let saved = Candidate::new(tree, &changes, position).position;
        changes.worktree_digest = "edit".into();
        assert!(Candidate::new(tree, &changes, saved).stale);
    }

    #[test]
    fn files_use_runner_paths_and_project_preview_refusals() {
        let directory = files(
            &json!({"path":"docs/review", "truncated":true, "entries":[
                {"name":"entry", "kind":"directory"}, {"name":"alias", "kind":"link"},
                {"name":"special", "kind":"other"}
            ]}),
            true,
        );
        assert_eq!(directory["parent"]["path"], "docs");
        assert_eq!(directory["entries"][0]["path"], "docs/review/entry");
        assert_eq!(directory["entries"][1]["view"], "file");
        assert!(directory["entries"][2]["view"].is_null());
        let preview = files(&json!({"path":"assets/chart.png", "state":"binary"}), false);
        assert_eq!(preview["notice"], "Binary file");
        assert!(preview["text"].is_null());
    }
}
