//! A stuck engine build is a Block in its own workspace, with what the pane asks.
use super::super::*;
use crate::engine_stuck::Stuck;

#[test]
fn a_stuck_build_is_a_block_naming_its_question_in_its_workspace() {
    let workspace = Uuid::now_v7();
    let mut inputs = Inputs::default();
    inputs.workspaces.insert(workspace, "Overnight".into());
    inputs.engine_stuck = vec![Stuck {
        workspace: workspace.to_string(),
        item: "ov-9".into(),
        since_ms: 1_000,
        question: "Do you trust the files in this folder?".into(),
    }];
    let items = assemble(&inputs, UNIX_EPOCH + std::time::Duration::from_secs(10));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind(), NeedsYouKind::Blocked);
    assert_eq!(items[0].question, "Do you trust the files in this folder?");
    assert_eq!(items[0].detail.as_deref(), Some("ov-9"));
    assert_eq!(items[0].workspace_name, "Overnight");
}
