//! Normal startup path binding, distinct from private fixture construction.
use super::*;

impl Service {
    /// Normal startup with the selection already validated before logging/socket effects.
    pub async fn open_selected(selected: &paths::RuntimeSelection) -> Result<Self> {
        crate::cutover_gate::admit(selected.root())?;
        let mut service = Self::open_in(selected.ensure_dir()?).await?;
        // Here and not in `open_in`, so a test's service reads no folders
        // from the config of whoever runs the suite, and never reads, creates
        // or rewrites their `~/.ssh` (ov-506).
        service.read_only_folders = crate::read_only_folders::load(&service.root);
        service.bind_runtime_paths(selected);
        Ok(service)
    }

    pub(super) fn bind_runtime_paths(&mut self, selected: &paths::RuntimeSelection) {
        if !selected.is_explicit() {
            self.private_runtime = false;
            self.authorized_keys = default_authorized_keys();
            self.claude_config = crate::people::runner_claude_config();
            self.agent_home = user_home();
            self.claude_logins = crate::claude_logins::State::new(crate::claude_logins::runner_dir(std::env::var_os("CLAUDE_CONFIG_DIR"), self.agent_home.as_deref()));
        }
        self.rep_cli = Some(PathBuf::from(cli_binary(std::env::current_exe().ok().as_deref())));
    }

    /// Bound log roots: explicit runtimes never merge ambient HOME/CODEX_HOME.
    pub(crate) fn log_roots(&self) -> Vec<PathBuf> {
        if self.private_runtime {
            crate::log_watch::roots_in(self.agent_home.clone(), None, self.claude_logins.runner.as_deref())
        } else {
            crate::log_watch::roots(self.claude_logins.runner.as_deref())
        }
    }

    pub(crate) fn log_home(&self) -> Option<PathBuf> {
        if self.private_runtime { self.agent_home.clone() }
        else { std::env::var_os("HOME").map(PathBuf::from) }
    }

}
