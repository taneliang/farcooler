//! A daemon's dispatch on a private socket, in this process, as
//! `rpc_over_socket.rs`'s `Harness` builds one, for the needs-you tests.
//!
//! In process so a test can reach the service and the watcher behind the
//! socket: the needs-you list reads a watcher's observations and a
//! supervisor's asks, and no sampling loop runs here to make either.

#![allow(dead_code)]

use std::sync::Arc;

use farcooler_daemon::{rpc::RpcFactory, service::Service, watch::Watcher};
use farcooler_protocol::v1::{Scope, TerminalIntent};
use farcooler_transport::{Client, HandshakeConfig, Peer, UnixListenerServer};
use uuid::Uuid;

pub type Link = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

pub struct Harness {
    _dir: tempfile::TempDir,
    pub socket: std::path::PathBuf,
    pub service: Arc<Service>,
    pub watcher: Arc<Watcher>,
    tmux_socket: String,
}

/// Take down any tmux server the service started, as `rpc_over_socket.rs`
/// does and for its reason.
impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux")
            .args(["-L", &self.tmux_socket, "kill-server"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// A runner whose every connection has `scope`.
///
/// The watcher is built, not run, and events are served as the daemon serves
/// them, so a connection can subscribe with `events`.
pub async fn start(scope: Scope) -> Harness {
    let dir = tempfile::Builder::new().prefix("ny").tempdir_in(short_tmp()).unwrap();
    let socket = dir.path().join("d.sock");
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    farcooler_daemon::service::stub_agents_in_this_process();
    let service = Arc::new(
        Service::open_in(dir.path().to_path_buf())
            .await
            .expect("service")
            .enrolling_into(home.join(".ssh").join("authorized_keys")),
    );
    let tmux_socket = service.tmux.socket().to_string();
    let server = UnixListenerServer::bind(&socket).expect("bind");
    let watcher = Watcher::new(service.clone());
    let (svc, w) = (service.clone(), watcher.clone());
    tokio::spawn(async move {
        let _ = server
            .serve(move |_| {
                Some((
                    HandshakeConfig { daemon_version: "test".into() },
                    RpcFactory::new(
                        svc.clone(),
                        w.clone(),
                        Arc::new(tokio::sync::Notify::new()),
                        Peer { client_id: None, scope },
                    ),
                ))
            })
            .await;
    });
    tokio::task::yield_now().await;
    Harness { _dir: dir, socket, service, watcher, tmux_socket }
}

/// A short temporary directory: macOS's own is long enough that a socket in
/// it passes `SUN_LEN`.
fn short_tmp() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from("/tmp/fc-ny");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub async fn connect(h: &Harness) -> Link {
    Client::connect(&h.socket, "test-client", "0.0.0").await.expect("connect")
}

/// A repository with its Main workspace and main checkout, made in the store
/// directly: nothing here needs git.
pub struct Repo {
    pub id: Uuid,
    pub workspace: Uuid,
    pub worktree: Uuid,
}

pub fn a_repository(h: &Harness) -> Repo {
    let store = &h.service.store;
    let host = Uuid::now_v7();
    let root = store.create_repository_root(host, "/repos/ny", 1_000).unwrap();
    let repo = store.create_repository(host, root.id, "repo", "/repos/ny/.git", "").unwrap();
    let main = store.ensure_main_workspace(repo.id).unwrap();
    let wt = store.create_worktree(repo.id, "main", "/repos/ny", true).unwrap();
    store.assign_worktree(wt.id, main.id).unwrap();
    Repo { id: repo.id, workspace: main.id, worktree: wt.id }
}

/// A claude pane in `worktree`, opened for `task` when there is one.
pub fn a_pane(h: &Harness, worktree: Uuid, task: Option<Uuid>) -> Uuid {
    h.service
        .store
        .create_terminal_for_task(worktree, "pane", "claude", TerminalIntent::Running, 80, 24, task)
        .unwrap()
        .id
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
