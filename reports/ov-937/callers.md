# Exact caller and effect anchors

Generated from current source; review against signed scoped seal.

- `crates/daemon/src/paths.rs:19` — `pub fn runtime_dir() -> Result<PathBuf> {`
- `crates/daemon/src/paths.rs:47` — `pub struct RuntimeSelection {`
- `crates/daemon/src/paths.rs:55` — `pub fn from_override(channel: farcooler_protocol::Channel, value: Option<std::ffi::OsString>) -> Result<Self> {`
- `crates/daemon/src/paths.rs:84` — `pub fn selection() -> Result<&'static RuntimeSelection> {`
- `crates/daemon/src/paths.rs:110` — `pub fn ensure_runtime_dir() -> Result<PathBuf> {`
- `crates/daemon/src/paths.rs:211` — `fn runtime_selection_explicit_default_and_pre_effect_refusal() {`
- `crates/daemon/src/paths.rs:234` — `fn runtime_selection_refuses_before_socket_directory_effects() {`
- `crates/daemon/src/main.rs:36` — `async fn main() {`
- `crates/daemon/src/main.rs:47` — `match kept_log(selected) {`
- `crates/daemon/src/main.rs:84` — `async fn run(selected: &paths::RuntimeSelection) -> Result<(), i32> {`
- `crates/daemon/src/main.rs:169` — `let socket = selected.socket_path().map_err(|e| {`
- `crates/daemon/src/main.rs:230` — `let service = match Service::open_selected(selected).await {`
- `crates/daemon/src/main.rs:488` — `let service = Service::open().await.map_err(|e| {`
- `crates/daemon/src/main.rs:662` — `let service = match Service::open().await {`
- `crates/cli/src/main.rs:1069` — `async fn main() {`
- `crates/cli/src/main.rs:1070` — `if let Err(e) = farcooler_daemon::paths::selection() {`
- `crates/daemon/src/runtime_selection.rs:6` — `pub async fn open_selected(selected: &paths::RuntimeSelection) -> Result<Self> {`
- `crates/daemon/src/runtime_selection.rs:16` — `pub(super) fn bind_runtime_paths(&mut self, selected: &paths::RuntimeSelection) {`
- `crates/daemon/src/runtime_selection.rs:28` — `pub(crate) fn log_roots(&self) -> Vec<PathBuf> {`
- `crates/daemon/src/runtime_selection.rs:36` — `pub(crate) fn log_home(&self) -> Option<PathBuf> {`
- `crates/daemon/src/service.rs:2097` — `pub async fn open() -> Result<Self> {`
- `crates/daemon/src/service.rs:2117` — `pub async fn open_in(root: PathBuf) -> Result<Self> {`
- `crates/daemon/src/service.rs:2139` — `authorized_keys: scratch_authorized_keys(&root),`
- `crates/daemon/src/service.rs:2142` — `agent_home: Some(root.join("home")),`
- `crates/daemon/src/daemon_log.rs:48` — `pub fn log_dir() -> Option<PathBuf> {`
- `crates/daemon/src/daemon_log.rs:52` — `pub fn log_dir_for(selected: &crate::paths::RuntimeSelection) -> Option<PathBuf> {`
- `crates/daemon/src/watch.rs:2782` — `let log_roots = service.log_roots();`
- `crates/daemon/src/watch.rs:2804` — `service.log_home().unwrap_or_default(),`
- `crates/daemon/src/watch.rs:2858` — `log_watcher: crate::log_watch::LogWatcher::start(log_roots),`
- `crates/daemon/src/watch.rs:4697` — `self.log_watcher.follow(&crate::claude_registry::global().member_config_dirs());`
- `crates/daemon/src/watch/workers.rs:49` — `Follower { follow: Some(WorkerFollow::new(home.clone())), home, pinned: false }`
- `crates/daemon/src/watch/workers.rs:94` — `let mut taken = taken.unwrap_or_else(|| WorkerFollow::new(home.clone()));`
- `crates/daemon/src/watch/workers.rs:114` — `self.worker_follow.lock().unwrap_or_else(|e| e.into_inner()).follow = Some(WorkerFollow::new(home));`
- `crates/daemon/src/claude_registry.rs:246` — `pub fn config_dir() -> Option<PathBuf> {`
- `crates/daemon/src/claude_registry.rs:631` — `pub fn global() -> &'static Registry {`
- `crates/daemon/src/read_only_folders.rs:157` — `pub fn load(runtime: &Path) -> Vec<Folder> {`
- `crates/cli/src/daemon_link.rs:92` — `let socket = farcooler_daemon::paths::socket_path()?;`
- `crates/cli/src/daemon_link.rs:165` — `let socket = farcooler_daemon::paths::socket_path()?;`
- `crates/cli/src/daemon_link.rs:217` — `let socket = farcooler_daemon::paths::socket_path()?;`

Source positions are from the pre-fixture-correction read; product caller positions are unchanged. Tests may move; use seal.json hashes and source names for exact verification.
