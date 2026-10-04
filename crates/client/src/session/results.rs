//! Naming a result that was not the one asked for.

use super::{SessionError, result};

pub(super) fn wrong(expected: &'static str, got: &result::Value) -> SessionError {
    SessionError::WrongResult { expected, got: variant_name(got) }
}

fn variant_name(value: &result::Value) -> &'static str {
    match value {
        result::Value::Host(_) => "host",
        result::Value::RepositoryRoot(_) => "repository_root",
        result::Value::RepositoryRootList(_) => "repository_root_list",
        result::Value::Repository(_) => "repository",
        result::Value::RepositoryList(_) => "repository_list",
        result::Value::Worktree(_) => "worktree",
        result::Value::WorktreeList(_) => "worktree_list",
        result::Value::Terminal(_) => "terminal",
        result::Value::TerminalList(_) => "terminal_list",
        result::Value::Operation(_) => "operation",
        result::Value::DaemonVersion(_) => "daemon_version",
        result::Value::TerminalAttach(_) => "terminal_attach",
        result::Value::BranchList(_) => "branch_list",
        result::Value::PaneGroupList(_) => "pane_group_list",
        result::Value::DiscoveredWorktreeList(_) => "discovered_worktree_list",
        result::Value::TerminalScreen(_) => "terminal_screen",
        result::Value::AgentEventBatch(_) => "agent_event_batch",
        result::Value::WorktreeFileList(_) => "worktree_file_list",
        result::Value::ThemeList(_) => "theme_list",
        result::Value::AdapterList(_) => "adapter_list",
        result::Value::AdapterTestResult(_) => "adapter_test_result",
        result::Value::Empty(_) => "empty",
        result::Value::TerminalFilePut(_) => "terminal_file_put",
        result::Value::ChangeSet(_) => "change_set",
        result::Value::FileChangeList(_) => "file_change_list",
        result::Value::FileDiff(_) => "file_diff",
        result::Value::StackLinkList(_) => "stack_link_list",
        result::Value::ChangesInbox(_) => "changes_inbox",
        result::Value::ClientList(_) => "client_list",
        result::Value::ClientEnroll(_) => "client_enroll",
        result::Value::ClientSetNodeKey(_) => "client_set_node_key",
        result::Value::Task(_) => "task",
        result::Value::TaskList(_) => "task_list",
        result::Value::TaskDetail(_) => "task_detail",
        result::Value::TaskNote(_) => "task_note",
        result::Value::TaskBlockList(_) => "task_block_list",
        result::Value::TaskNoteHitList(_) => "task_note_hit_list",
        result::Value::Workspace(_) => "workspace",
        result::Value::WorkspaceList(_) => "workspace_list",
        result::Value::NeedsYouList(_) => "needs_you_list",
        result::Value::Report(_) => "report",
        result::Value::UsageReport(_) => "usage_report",
        result::Value::TaskUsage(_) => "task_usage",
        result::Value::BoardReads(_) => "board_reads",
        result::Value::WorktreeDir(_) => "worktree_dir",
        result::Value::WorktreeFile(_) => "worktree_file",
        result::Value::BoardThemeView(_) => "board_theme_view",
        result::Value::Lane(_) => "lane",
        result::Value::Plan(_) => "plan",
        result::Value::PlanEventList(_) => "plan_event_list",
    }
}
