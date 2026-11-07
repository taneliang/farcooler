# Final exact source API/caller/effect anchors

Product source is unchanged from signed e1203d8c6bbeb4bf7eaecb51a08fd0ab2c6884e2. No product main execution.

## crates/daemon/src/paths.rs

SHA256 `fe612b79719e637752737c20fc424fec882976ebca9f74176252844a574e3d4d`

- `crates/daemon/src/paths.rs:19` — `pub fn runtime_dir() -> Result<PathBuf> {`
- `crates/daemon/src/paths.rs:41` — `pub fn runtime_dir_for(channel: farcooler_protocol::Channel) -> Result<PathBuf> {`
- `crates/daemon/src/paths.rs:47` — `pub struct RuntimeSelection {`
- `crates/daemon/src/paths.rs:55` — `pub fn from_override(channel: farcooler_protocol::Channel, value: Option<std::ffi::OsString>) -> Result<Self> {`
- `crates/daemon/src/paths.rs:84` — `pub fn selection() -> Result<&'static RuntimeSelection> {`
- `crates/daemon/src/paths.rs:110` — `pub fn ensure_runtime_dir() -> Result<PathBuf> {`
- `crates/daemon/src/paths.rs:211` — `fn runtime_selection_explicit_default_and_pre_effect_refusal() {`
- `crates/daemon/src/paths.rs:235` — `fn runtime_selection_refuses_before_socket_directory_effects() {`

## crates/daemon/src/main.rs

SHA256 `4cbfa54c59db517e16d289df5f0eb1bf1db1760a7a1ae9c42cb38fdfdc4c5730`

- `crates/daemon/src/main.rs:36` — `async fn main() {`
- `crates/daemon/src/main.rs:47` — `match kept_log(selected) {`
- `crates/daemon/src/main.rs:84` — `async fn run(selected: &paths::RuntimeSelection) -> Result<(), i32> {`
- `crates/daemon/src/main.rs:169` — `let socket = selected.socket_path().map_err(|e| {`
- `crates/daemon/src/main.rs:230` — `let service = match Service::open_selected(selected).await {`
- `crates/daemon/src/main.rs:249` — `service.start_workflows().map_err(|error| {`
- `crates/daemon/src/main.rs:483` — `async fn stream_terminal(terminal: &str) -> Result<(), i32> {`
- `crates/daemon/src/main.rs:488` — `let service = Service::open().await.map_err(|e| {`
- `crates/daemon/src/main.rs:612` — `async fn serve_stdio_session() -> Result<(), i32> {`
- `crates/daemon/src/main.rs:662` — `let service = match Service::open().await {`

## crates/cli/src/main.rs

SHA256 `5db82399fb49c3fbcbbb19941b18a6d7f7fc73448f88d7d9e054aa483280eb03`

- `crates/cli/src/main.rs:1069` — `async fn main() {`
- `crates/cli/src/main.rs:1070` — `if let Err(e) = farcooler_daemon::paths::selection() {`

## crates/daemon/src/runtime_selection.rs

SHA256 `134284fdf9cf9980123c547bd8c1ed4820bb383dc89562179e4772d6dc209bce`

- `crates/daemon/src/runtime_selection.rs:6` — `pub async fn open_selected(selected: &paths::RuntimeSelection) -> Result<Self> {`
- `crates/daemon/src/runtime_selection.rs:16` — `pub(super) fn bind_runtime_paths(&mut self, selected: &paths::RuntimeSelection) {`
- `crates/daemon/src/runtime_selection.rs:28` — `pub(crate) fn log_roots(&self) -> Vec<PathBuf> {`
- `crates/daemon/src/runtime_selection.rs:36` — `pub(crate) fn log_home(&self) -> Option<PathBuf> {`

## crates/daemon/src/service.rs

SHA256 `e1366c4f42ef54a96876039d8c4210a04e1fda58156e0e535c0cde0162ffb4ae`

- `crates/daemon/src/service.rs:52` — `pub fn cli_binary(daemon_exe: Option<&std::path::Path>) -> String {`
- `crates/daemon/src/service.rs:2097` — `pub async fn open() -> Result<Self> {`
- `crates/daemon/src/service.rs:2117` — `pub async fn open_in(root: PathBuf) -> Result<Self> {`
- `crates/daemon/src/service.rs:2139` — `authorized_keys: scratch_authorized_keys(&root),`
- `crates/daemon/src/service.rs:2142` — `agent_home: Some(root.join("home")),`

## crates/daemon/src/daemon_log.rs

SHA256 `61ad81b6713d3e68f1b9c32fc2738bdaec1e444fae54118570419f94575adda1`

- `crates/daemon/src/daemon_log.rs:48` — `pub fn log_dir() -> Option<PathBuf> {`
- `crates/daemon/src/daemon_log.rs:52` — `pub fn log_dir_for(selected: &crate::paths::RuntimeSelection) -> Option<PathBuf> {`

## crates/daemon/src/watch.rs

SHA256 `adcf974d1c32f32bd012d514164d2d5466e1528fa17c077200ac3386be798d94`

- `crates/daemon/src/watch.rs:2782` — `let log_roots = service.log_roots();`
- `crates/daemon/src/watch.rs:2804` — `service.log_home().unwrap_or_default(),`
- `crates/daemon/src/watch.rs:2858` — `log_watcher: crate::log_watch::LogWatcher::start(log_roots),`
- `crates/daemon/src/watch.rs:4697` — `self.log_watcher.follow(&crate::claude_registry::global().member_config_dirs());`

## crates/daemon/src/watch/workers.rs

SHA256 `cbd8f91300915391ffae598981c09e43317bf8878ce23758275538c2f8802195`

- `crates/daemon/src/watch/workers.rs:49` — `Follower { follow: Some(WorkerFollow::new(home.clone())), home, pinned: false }`
- `crates/daemon/src/watch/workers.rs:94` — `let mut taken = taken.unwrap_or_else(|| WorkerFollow::new(home.clone()));`
- `crates/daemon/src/watch/workers.rs:114` — `self.worker_follow.lock().unwrap_or_else(|e| e.into_inner()).follow = Some(WorkerFollow::new(home));`

## crates/daemon/src/claude_registry.rs

SHA256 `c18376333e021876a752573c664ec33c61a03ca4aec4580e59eeca8bdc48a382`

- `crates/daemon/src/claude_registry.rs:246` — `pub fn config_dir() -> Option<PathBuf> {`
- `crates/daemon/src/claude_registry.rs:631` — `pub fn global() -> &'static Registry {`

## crates/daemon/src/read_only_folders.rs

SHA256 `4b783d4c272a0d4dafdcc3f882e78e496fecf85eba130042b6f7f29d9358e6dc`

- `crates/daemon/src/read_only_folders.rs:157` — `pub fn load(runtime: &Path) -> Vec<Folder> {`

## crates/cli/src/daemon_link.rs

SHA256 `a6496f669a2e821c8e20044a489084ad9e3e9958ef67f318f23349c422e3e3c5`

- `crates/cli/src/daemon_link.rs:92` — `let socket = farcooler_daemon::paths::socket_path()?;`
- `crates/cli/src/daemon_link.rs:165` — `let socket = farcooler_daemon::paths::socket_path()?;`
- `crates/cli/src/daemon_link.rs:217` — `let socket = farcooler_daemon::paths::socket_path()?;`

## crates/cli/src/runtime_selection_tests.rs

SHA256 `49b72b38413dfc037b85e3bc842b55d2aa37981a40e8abb9be2a1091a20bb281`

- `crates/cli/src/runtime_selection_tests.rs:3` — `fn runtime_selection_cli_uses_the_shared_validated_capture() {`

## crates/daemon/src/runtime_selection_tests.rs

SHA256 `3682f62083047e37e04c2f95111877a498fa0eb3b95f45d023de9591649e50b5`

- `crates/daemon/src/runtime_selection_tests.rs:12` — `async fn runtime_selection_normal_explicit_start_keeps_private_paths() {`
- `crates/daemon/src/runtime_selection_tests.rs:34` — `async fn runtime_selection_default_binding_keeps_installed_precedence_without_opening_it() {`
- `crates/daemon/src/runtime_selection_tests.rs:52` — `async fn runtime_selection_private_fence_migration_caps_allowlist_and_revoke() {`

