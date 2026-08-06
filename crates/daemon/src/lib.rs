//! Daemon composition: git worktree transactions, domain services, lifecycle.
pub mod agent_supervisor;
pub mod change_set;
pub mod fanout;
pub mod file_diff;
pub mod foreground;
pub mod git;
pub mod layout;
pub mod pastes;
pub mod paths;
pub mod push;
pub mod reconcile;
pub mod rpc;
pub mod runtime;
pub mod service;
pub mod session_discovery;
#[cfg(test)]
pub(crate) mod test_support;
pub mod watch;
pub mod wire;
