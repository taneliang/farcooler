//! A disposable runner for the browser workday check. The browser's SSH
//! command is the ordinary stdio relay to the fixture’s private daemon socket.
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use farcooler_daemon::{rpc::RpcFactory, service::Service, watch::Watcher};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_store::{Actor, NoteKind, TaskStatus, testing};
use farcooler_transport::{Client, HandshakeConfig, UnixListenerServer, request};
use farcooler_workflow::{
    Command as WorkflowCommand, Definition, Grant, GroupOf, Input, Phase, StageKind,
};
use tokio::sync::Notify;
use uuid::Uuid;

type Link = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

fn git(repo: &Path, args: &[&str]) {
    let result = Command::new("git")
        .current_dir(repo)
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

async fn call(
    link: &mut Link,
    method: &str,
    target: Option<bytes::Bytes>,
    payload: request::Payload,
) -> result::Value {
    let mut req = request(method);
    req.target_resource_id = target;
    req.payload = Some(payload);
    link.call(req).await.unwrap().value.unwrap()
}

async fn prepare(link: &mut Link, service: &Service, root: &Path) -> Uuid {
    let repo = root.join("workday");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@localhost"]);
    git(&repo, &["config", "user.name", "Workday Fixture"]);
    git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "Base",
            "--allow-empty",
        ],
    );
    call(
        link,
        "repository_root.add",
        None,
        request::Payload::RepositoryRootAdd(pb::RepositoryRootAdd {
            absolute_path: root.display().to_string(),
            ..Default::default()
        }),
    )
    .await;
    let result::Value::Repository(repo) = call(
        link,
        "repository.register",
        None,
        request::Payload::RepositoryRegister(pb::RepositoryRegister {
            relative_path: repo.display().to_string(),
        }),
    )
    .await
    else {
        panic!("repository")
    };
    let repository = Uuid::from_slice(&repo.id).unwrap();
    let workspace = service.store.ensure_main_workspace(repository).unwrap().id;
    let result::Value::Worktree(worktree) = call(
        link,
        "worktree.create",
        Some(repo.id),
        request::Payload::WorktreeCreate(pb::WorktreeCreate {
            task_name: "Review checkout".into(),
            branch: "workday-review".into(),
            base_revision: "main".into(),
            workspace_id: Some(bytes::Bytes::copy_from_slice(workspace.as_bytes())),
            ..Default::default()
        }),
    )
    .await
    else {
        panic!("worktree")
    };
    let checkout = PathBuf::from(worktree.worktree_path.unwrap());
    std::fs::write(checkout.join("review.txt"), "Verified deployment plan\n").unwrap();
    git(&checkout, &["add", "review.txt"]);
    git(
        &checkout,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "Prepare deployment",
        ],
    );

    let result::Value::Terminal(lost) = call(
        link,
        "terminal.create",
        Some(worktree.id),
        request::Payload::TerminalCreate(pb::TerminalCreate {
            title: "Lost workday agent".into(),
            command_preset: "codex".into(),
            ..Default::default()
        }),
    )
    .await
    else {
        panic!("terminal")
    };
    let lost = Uuid::from_slice(&lost.id).unwrap();
    assert!(service.tmux.kill_terminal_window(lost).await.unwrap());
    service.backstop_reconcile().await;

    let task = service
        .store
        .create_task(workspace, "Deployment decision", Actor::User)
        .unwrap();
    service
        .store
        .add_note(
            task.id,
            NoteKind::Question,
            Actor::User,
            "Run the deployment checks?",
            serde_json::json!({"options": ["Run checks", "Hold"]}),
        )
        .unwrap();
    service
        .store
        .set_task_status(task.id, TaskStatus::NeedsDecision, Actor::User)
        .unwrap();
    workspace
}

async fn marker(root: &Path, name: &str) {
    while !root.join(name).exists() {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Seed a workflow group, then change its stage when the browser is watching.
async fn landing_fixture(service: &Service, root: &Path, workspace: Uuid) {
    marker(root, "open-landing").await;
    let workspace = workspace.to_string();
    let store = &service.store;
    let mut definition = Definition::coding();
    definition.stages.truncate(2);
    definition.stages[0].kind = StageKind::Run;
    definition.stages[1].kind = StageKind::AwaitCi;
    let id = "group-00000000000000000001-landing";
    let now = testing::now_millis();
    // Enqueue a normal item to obtain a valid run shape and a workspace row.
    let receipt = store
        .workflow_apply(&Input {
            schema: 1,
            id: "fixture-landing".into(),
            workspace: workspace.clone(),
            expected_revision: 0,
            epoch: 1,
            source: "workday".into(),
            source_sequence: 1,
            now_ms: now,
            grant: Grant::Controller,
            busy_resources: Default::default(),
            command: WorkflowCommand::Enqueue {
                item: "fixture-landing".into(),
                definition: Definition::coding(),
                resource: "workday".into(),
                policy_pin: "fixture".into(),
                dependencies: Default::default(),
                initial_candidate: None,
            },
        })
        .unwrap();
    assert!(receipt.result.is_ok(), "{receipt:?}");
    let mut state = store.workflow_state(&workspace).unwrap().unwrap();
    let mut item = state.items.remove("fixture-landing").unwrap();
    item.group_of = Some(GroupOf {
        name: "landing".into(),
        pin: definition.pin(),
    });
    item.definition = definition;
    item.phase = Phase::Running;
    item.since_ms = now;
    state.items.insert(id.into(), item);
    store.workflow_state_for_tests(&state).unwrap();

    marker(root, "advance-landing").await;
    let item = state.items.get_mut(id).unwrap();
    item.stage = 1;
    item.since_ms = testing::now_millis();
    store.workflow_state_for_tests(&state).unwrap();
    std::future::pending::<()>().await;
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().expect("fixture directory"));
    let state = PathBuf::from(args.next().expect("runtime directory"));
    let keys = PathBuf::from(args.next().expect("fixture authorized_keys"));
    assert!(root.is_dir());
    farcooler_daemon::service::stub_agents_in_this_process();
    farcooler_daemon::sshd_version::offer_passkey_keys().await;
    let service = Arc::new(
        Service::open_in(state.clone())
            .await
            .unwrap()
            .enrolling_into(keys),
    );
    let watcher = Watcher::new(service.clone());
    let socket = state.join("farcoolerd.sock");
    let admin = UnixListenerServer::bind(&socket).unwrap();
    let (local, local_watcher) = (service.clone(), watcher.clone());
    let local_server = tokio::spawn(async move {
        admin
            .serve(move |preamble| {
                let peer = farcooler_daemon::sessions::peer_from_preamble(preamble.as_ref())?;
                Some((
                    HandshakeConfig {
                        daemon_version: "workday".into(),
                    },
                    RpcFactory::new(
                        local.clone(),
                        local_watcher.clone(),
                        Arc::new(Notify::new()),
                        peer,
                    ),
                ))
            })
            .await
            .unwrap();
    });
    let mut link = Client::connect(&socket, "workday-fixture", "1")
        .await
        .unwrap();
    let workspace = prepare(&mut link, &service, &root).await;
    println!("ready");
    io::stdout().flush().unwrap();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = landing_fixture(&service, &root, workspace) => {},
    }
    local_server.abort();
    let _ = service.tmux.kill_server().await;
}
