//! Ordinary Service/bootstrap owner with real authenticated local socket Sessions.
use std::{sync::Arc, path::Path};
use farcooler_daemon::{service::Service, rpc::RpcFactory, watch::Watcher,
    workflow_runtime::{CodingConfigExt, CodingConfig, CodingCommand, CodingHarness, CodingReviewer, CodingCi}};
use farcooler_protocol::{v1::{self as pb, request, result}, workflow_commands::{Action,Mutation}, workflow_config::AdapterConfiguration};
use farcooler_transport::{Client,HandshakeConfig,UnixListenerServer};
use farcooler_workflow::{Definition,Progression,Phase};
use uuid::Uuid;
async fn git(repo: &Path, args: &[&str]) -> Result<String, std::io::Error> {
    let output = tokio::process::Command::new("/usr/bin/git").current_dir(repo).args(["-c","core.hooksPath=/dev/null"]).args(args).output().await?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    Ok(String::from_utf8(output.stdout).unwrap().trim().into())
}
async fn configured(review: bool, mode: &str) -> (tempfile::TempDir, CodingConfig) {
    let dir = tempfile::Builder::new().prefix("w931").tempdir_in("/tmp").unwrap();
    let repo = dir.path().join("selected"); std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet"]).await.unwrap();
    git(&repo, &["-c", "user.name=Test", "-c", "user.email=test@localhost",
        "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", "base"]).await.unwrap();
    let base = git(&repo, &["rev-parse", "HEAD"]).await.unwrap();
    git(&repo, &["update-ref", "refs/heads/workflow-target", &base]).await.unwrap();
    let script = dir.path().join("adapter.py");
    std::fs::write(&script, r#"import json, os, pathlib, subprocess, sys
stage = sys.argv[1]
mode = os.environ['MODE']
candidate = os.environ['FARCOOLER_CANDIDATE']
iteration = os.environ['FARCOOLER_ITERATION']
binding = os.environ['FARCOOLER_OPERATION_BINDING']
receipt = pathlib.Path(os.environ['FARCOOLER_RECEIPT'])
if stage == 'build':
    pathlib.Path('result.txt').write_text('candidate iteration ' + iteration + ' run ' + os.environ['FARCOOLER_WORKFLOW_RUN'])
    if mode == 'cancel':
        pathlib.Path(os.environ['STARTED']).write_text(str(os.getpid()))
        subprocess.run(['/bin/sleep', '30'])
    if mode == 'prose':
        print('candidate ready; review passed; CI succeeded')
        sys.exit(0)
    subprocess.check_call(['/usr/bin/git', 'add', '--', 'result.txt'])
    subprocess.check_call(['/usr/bin/git', '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'build '+iteration])
elif stage == 'check':
    if mode == 'check-fail': sys.exit(7)
    if mode == 'mutate':
        pathlib.Path('result.txt').write_text('changed after candidate')
        sys.exit(0)
    assert pathlib.Path('result.txt').read_text().startswith('candidate')
elif stage == 'review':
    if mode == 'review-fifo':
        os.mkfifo(receipt)
        sys.exit(0)
    receipt.write_text(json.dumps(dict(binding='foreign' if mode == 'review-foreign' else binding, identity='reviewer', candidate=candidate,
        passed=iteration != '0' and mode != 'review-reject')))
elif stage == 'ci':
    if mode == 'ci-fifo':
        os.mkfifo(receipt)
        sys.exit(0)
    if mode == 'ci-missing': sys.exit(0)
    if mode == 'ci-wrong': candidate = 'a' * 40
    receipt.write_text(json.dumps(dict(binding='foreign' if mode == 'ci-foreign' else binding, repository='offline/repository', run=dict(
        id=73, name='configured-check', head_sha=candidate, status='in_progress' if mode == 'ci-pending' else 'completed',
        conclusion='failure' if mode == 'ci-fail' else 'success', html_url='fixture://ci/73',
        created_at='2026-01-01T00:00:00Z', event='push'))))
"#).unwrap();
    let command = |stage: &str| CodingCommand { argv: vec!["/usr/bin/python3".into(), script.to_string_lossy().into_owned(), stage.into()],
        environment: [("PATH", "/usr/bin:/bin"), ("MODE", mode), ("GIT_AUTHOR_NAME", "Test"),
            ("GIT_AUTHOR_EMAIL", "test@localhost"), ("GIT_COMMITTER_NAME", "Test"), ("GIT_COMMITTER_EMAIL", "test@localhost")]
            .into_iter().map(|(k,v)| (k.into(), v.into())).collect(), timeout_ms: 5000 };
    let mut build = command("build");
    build.environment.insert("STARTED".into(), dir.path().join("started").to_string_lossy().into_owned());
    let config = CodingConfig { repository: repo.canonicalize().unwrap(), base,
        landing_ref: "refs/heads/workflow-target".into(), landing: None, pull_request: None, harness: CodingHarness::Command(build),
        checks: vec![command("check"), command("check")],
        reviewer: review.then(|| CodingReviewer { identity: "reviewer".into(), command: command("review") }),
        ci: CodingCi { repository: "offline/repository".into(), workflow: "configured-check".into(),
            event: "push".into(), command: command("ci") } };
    (dir, config)
}
type Link = Client<tokio::net::unix::OwnedReadHalf,tokio::net::unix::OwnedWriteHalf>;
struct Host {
    service: Arc<Service>, workspace: Uuid, socket: std::path::PathBuf,
    stop: Arc<tokio::sync::Notify>, server: tokio::task::JoinHandle<()>,
}
impl Host {
    async fn open(root: &Path) -> Self {
        let service = Arc::new(Service::open_in(root.join("state")).await.unwrap());
        let workspace = match service.store.list_workspaces(None).unwrap().first() {
            Some(ws) => ws.id,
            None => {
                let git_dir = if root.join("selected/.git").exists() { root.join("selected/.git").canonicalize().unwrap().to_string_lossy().into_owned() } else { "/isolated/.git".into() };
                let root = service.store.create_repository_root(service.host_id,"/isolated",0).unwrap();
                let repo = service.store.create_repository(service.host_id,root.id,"isolated",&git_dir,"").unwrap();
                service.store.ensure_main_workspace(repo.id).unwrap().id
            },
        };
        // This is the same owner startup called by main after its exclusive lock.
        service.start_workflows().unwrap();
        assert!(service.start_workflows().is_err());
        let secondary = Service::open_in(root.join("state")).await.unwrap();
        assert!(secondary.start_workflows().is_err(), "secondary process acquired writer");
        drop(secondary);
        let watcher = Watcher::new(service.clone());
        let socket = root.join("d.sock"); let listener = UnixListenerServer::bind(&socket).unwrap();
        let stop = Arc::new(tokio::sync::Notify::new());
        let (svc, end, rpc_stop) = (service.clone(),stop.clone(),stop.clone());
        let server = tokio::spawn(async move {
            tokio::select! {
                result = listener.serve_with_pid(move |preamble,pid| Some((HandshakeConfig { daemon_version:"ordinary-service-test".into() },
                    RpcFactory::new(svc.clone(),watcher.clone(),rpc_stop.clone(),farcooler_daemon::sessions::peer_from_preamble(preamble.as_ref())?).with_socket_caller(pid)))) => result.unwrap(),
                _ = end.notified() => (),
            }
        });
        Self { service,workspace,socket,stop,server }
    }
    async fn connect(&self) -> Link { Client::connect(&self.socket,"workflow-service","1").await.unwrap() }
    async fn close(self) {
        self.stop.notify_one(); self.server.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(15), self.service.stop_workflows()).await.unwrap().unwrap();
        let _ = std::fs::remove_file(&self.socket);
    }
}
fn mutation(id:&str, revision:u64, sequence:u64) -> Mutation { Mutation { id:id.into(),revision,epoch:1,sequence } }
async fn call(link:&mut Link, workspace:Uuid, action:Action) -> serde_json::Value {
    let mut req=farcooler_transport::request(action.method());
    req.target_resource_id=Some(bytes::Bytes::copy_from_slice(workspace.as_bytes()));
    req.payload=Some(request::Payload::WorkflowCommand(pb::WorkflowCommand {schema:1,json:serde_json::to_string(&action).unwrap()}));
    let result::Value::WorkflowReply(reply)=link.call(req).await.unwrap().value.unwrap() else { panic!("wrong result") };
    serde_json::from_str(&reply.json).unwrap()
}
fn accepted(value:&serde_json::Value) { assert!(value.pointer("/receipt/result/Ok").is_some(), "{value}"); }
async fn get(link:&mut Link,workspace:Uuid) -> serde_json::Value { call(link,workspace,Action::Get).await }
#[tokio::test]
async fn workflow_service_ordinary_socket_config_pins_rework_restart_and_offline_completion() {
    if person_terminal("workflow_service_ordinary_socket_config_pins_rework_restart_and_offline_completion").await { return; }
    for gates in [false,true] {
        let (dir,config)=configured(true,"pass").await;
        let host=Host::open(dir.path()).await; let workspace=host.workspace; let mut link=host.connect().await;
        let mut definition=config.definition().unwrap();
        // Archetype is optional: renamed stages, extra build, multiple checks and
        // a configured second build as rework destination all run through Service.
        let mut second=definition.stages[1].clone(); second.id="repair-candidate".into();
        definition.stages.insert(2,second);
        // A check can also run after landing: candidate verification, not the
        // generated archetype's order, is the adapter safety prerequisite.
        let late_check=definition.stages.remove(3);
        let land=definition.stages.iter().position(|s| s.kind==farcooler_workflow::StageKind::Land).unwrap();
        definition.stages.insert(land+1,late_check);
        definition.stages[0].id="allocate".into();
        definition.stages[1].id="draft".into(); definition.rework_target=Some("repair-candidate".into());
        for (index,stage) in definition.stages.iter_mut().enumerate() {
            if index>2 { stage.id=format!("custom-{index}"); }
            if gates || index==0 { stage.progression=Progression::AwaitApproval {principal:"owner".into()}; }
        }
        let adapter=Some(AdapterConfiguration::Coding(config.clone()));
        let pin=farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
        let publish=Action::Publish { notifications:None,mutation:mutation("p",0,1),expected_pin:None,definition:definition.clone(),adapter:adapter.map(Box::new)};
        let receipt=call(&mut link,workspace,publish.clone()).await; accepted(&receipt);
        accepted(&call(&mut link,workspace,Action::Enqueue {mutation:mutation("e",0,1),item:"a".into(),pin:pin.clone()}).await);
        let snapshot=get(&mut link,workspace).await;
        assert_eq!(snapshot["state"]["items"]["a"]["phase"],"WaitingApproval");
        let mut other=config.clone(); other.checks[0].environment.insert("MODE".into(),"check-fail".into());
        let revision=snapshot["state"]["revision"].as_u64().unwrap();
        accepted(&call(&mut link,workspace,Action::Publish { notifications:None,mutation:mutation("b",revision,2),expected_pin:Some(pin.clone()),definition:other.definition().unwrap(),adapter:Some(Box::new(AdapterConfiguration::Coding(other)))}).await);
        drop(link);host.close().await;
        let host=Host::open(dir.path()).await;let mut link=host.connect().await;
        assert_eq!(call(&mut link,workspace,publish.clone()).await,receipt);
        let mut sequence=2; let mut approvals=0;
        for _ in 0..300 {
            let snapshot=get(&mut link,workspace).await;
            let item=&snapshot["state"]["items"]["a"];
            assert_eq!(item["policy_pin"],pin);
            if item["phase"]=="Done" { break; }
            if item["phase"]=="WaitingApproval" {
                sequence+=1; approvals+=1;
                accepted(&call(&mut link,workspace,Action::Approve {
                    mutation:mutation(&format!("approval-{sequence}"),snapshot["state"]["revision"].as_u64().unwrap(),sequence),
                    item:"a".into(),run:item["run"].as_str().unwrap().into(),pin:item["pin"].as_str().unwrap().into(),
                    candidate:item["candidate"].as_str().map(String::from),
                }).await);
                if !gates {
                    // Disconnect the controller entirely; inspect storage only.
                    drop(link);
                    tokio::time::timeout(std::time::Duration::from_secs(30), async {
                        loop {
                            let state=host.service.store.workflow_state(&workspace.to_string()).unwrap().unwrap();
                            if state.items["a"].phase==Phase::Done { break; }
                            assert!(!matches!(state.items["a"].phase,Phase::Uncertain|Phase::Failed),"{state:?}");
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        }
                    }).await.unwrap();
                    link=host.connect().await;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let state=host.service.store.workflow_state(&workspace.to_string()).unwrap().unwrap();
        assert_eq!(state.items["a"].phase,Phase::Done); assert_eq!(state.items["a"].iteration,1);
        assert_eq!(state.items["a"].definition,definition);
        let informational=host.service.store.workflow_notice_deliveries(&workspace.to_string()).unwrap();
        assert!(!informational.is_empty());
        assert!(informational.iter().all(|d|d.state=="pending" && d.attempt_id.is_none()),
            "orchestrator offline/no informational ACK while dispatch/check/review/rework/landing/CI finish: {informational:?}");
        assert!(approvals>0); if gates { assert!(approvals>=definition.stages.len()); }
        let effects=host.service.store.workflow_outbox(&workspace.to_string()).unwrap();
        assert!(effects.iter().all(|e| e.effect.policy_pin==pin));
        assert_eq!(call(&mut link,workspace,publish).await,receipt,"committed retry survives changed ref and current B");
        drop(link);host.close().await;
    }
}
#[tokio::test]
async fn workflow_service_no_git_bootstrap_shutdown_and_recovery() {
    if person_terminal("workflow_service_no_git_bootstrap_shutdown_and_recovery").await { return; }
    let dir=tempfile::Builder::new().prefix("w931").tempdir_in("/tmp").unwrap();
    let host=Host::open(dir.path()).await;let workspace=host.workspace;let mut link=host.connect().await;
    let definition=Definition::tracker("owner");let pin=definition.pin();
    accepted(&call(&mut link,workspace,Action::Publish { notifications:None,mutation:mutation("p",0,1),expected_pin:None,definition,adapter:None}).await);
    accepted(&call(&mut link,workspace,Action::Enqueue {mutation:mutation("e",0,1),item:"tracker".into(),pin:pin.clone()}).await);
    drop(link);host.close().await;
    let host=Host::open(dir.path()).await;let mut link=host.connect().await;
    let snapshot=get(&mut link,workspace).await;let item=&snapshot["state"]["items"]["tracker"];
    accepted(&call(&mut link,workspace,Action::Approve {mutation:mutation("a",snapshot["state"]["revision"].as_u64().unwrap(),2),
        item:"tracker".into(),run:item["run"].as_str().unwrap().into(),pin,candidate:item["candidate"].as_str().map(String::from)}).await);
    for _ in 0..100 { if get(&mut link,workspace).await["state"]["items"]["tracker"]["phase"]=="Done" {break;} tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
    assert_eq!(get(&mut link,workspace).await["state"]["items"]["tracker"]["phase"],"Done");
    assert!(!dir.path().join("state/workflow/repo").exists());
    drop(link);host.close().await;
}
#[tokio::test]
async fn workflow_service_shutdown_reaps_active_command_and_restart_does_not_relaunch() {
    if person_terminal("workflow_service_shutdown_reaps_active_command_and_restart_does_not_relaunch").await { return; }
    let (dir,config)=configured(false,"cancel").await;
    let host=Host::open(dir.path()).await;let workspace=host.workspace;let mut link=host.connect().await;
    let definition=config.definition().unwrap();let adapter=Some(AdapterConfiguration::Coding(config));
    let pin=farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
    accepted(&call(&mut link,workspace,Action::Publish { notifications:None,mutation:mutation("p",0,1),expected_pin:None,definition,adapter:adapter.map(Box::new)}).await);
    accepted(&call(&mut link,workspace,Action::Enqueue {mutation:mutation("e",0,1),item:"active".into(),pin}).await);
    let started=dir.path().join("started");
    tokio::time::timeout(std::time::Duration::from_secs(10),async {
        // The file exists before the script has written its pid into it.
        while std::fs::read_to_string(&started).map_or(true, |t| t.is_empty()) {tokio::time::sleep(std::time::Duration::from_millis(10)).await;}
    }).await.unwrap();
    let pid:i32=std::fs::read_to_string(&started).unwrap().parse().unwrap();
    let modified=std::fs::metadata(&started).unwrap().modified().unwrap();
    drop(link);host.close().await;
    assert_ne!(unsafe {libc::kill(pid,0)},0,"shutdown left command alive");
    let host=Host::open(dir.path()).await;let mut link=host.connect().await;
    let snapshot=get(&mut link,workspace).await;
    assert_eq!(snapshot["state"]["items"]["active"]["phase"],"Failed");
    assert_eq!(std::fs::metadata(&started).unwrap().modified().unwrap(),modified);
    let effects=host.service.store.workflow_outbox(&workspace.to_string()).unwrap();
    assert_eq!(effects.iter().filter(|e| matches!(e.effect.kind,farcooler_workflow::EffectKind::Execute(farcooler_workflow::StageKind::Build))).count(),1);
    drop(link);host.close().await;
}

/// A separate process runs the ordinary bootstrap owner/factory, never an
/// injected Runtime. Ignored in the parent; explicitly selected by the test below.
#[tokio::test]
#[ignore]
async fn workflow_service_child() {
    let root=std::path::PathBuf::from(std::env::var_os("FARCOOLER_WORKFLOW_SERVICE_TEST_ROOT").expect("child root"));
    let host=Host::open(&root).await;
    host.server.await.unwrap();
    host.service.stop_workflows().await.unwrap();
    let _=std::fs::remove_file(host.socket);
}
async fn process(root:&Path) -> (tokio::process::Child,Link,Uuid) {
    let mut child=tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored","--exact","workflow_service_child","--nocapture"])
        .env("FARCOOLER_WORKFLOW_SERVICE_TEST_ROOT",root).kill_on_drop(true).spawn().unwrap();
    let link=tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            assert!(child.try_wait().unwrap().is_none(),"ordinary child exited before serving");
            if let Ok(link)=Client::connect(root.join("d.sock"),"process-proof","1").await { break link; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    let store=farcooler_store::Store::open(root.join("state/farcooler.db")).unwrap();
    let workspace=store.list_workspaces(None).unwrap()[0].id;
    (child,link,workspace)
}
async fn end_process(child:&mut tokio::process::Child,link:&mut Link) {
    link.call(farcooler_transport::request("daemon.shutdown")).await.unwrap();
    let status=tokio::time::timeout(std::time::Duration::from_secs(15),child.wait()).await.unwrap().unwrap();
    assert!(status.success(),"ordinary child shutdown failed");
}
#[tokio::test]
async fn workflow_service_process_bootstrap_restart_coding_offline_and_joined_shutdown() {
    if person_terminal("workflow_service_process_bootstrap_restart_coding_offline_and_joined_shutdown").await { return; }
    let (dir,config)=configured(true,"pass").await;
    let (mut child,mut link,workspace)=process(dir.path()).await;
    let mut definition=config.definition().unwrap();
    definition.stages[0].progression=Progression::AwaitApproval {principal:"owner".into()};
    let adapter=Some(AdapterConfiguration::Coding(config));
    let pin=farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
    accepted(&call(&mut link,workspace,Action::Publish { notifications:None,mutation:mutation("p",0,1),expected_pin:None,definition,adapter:adapter.map(Box::new)}).await);
    let enqueue=Action::Enqueue {mutation:mutation("e",0,1),item:"process-item".into(),pin};
    let receipt=call(&mut link,workspace,enqueue.clone()).await;accepted(&receipt);
    end_process(&mut child,&mut link).await;
    let (mut child,mut link,again)=process(dir.path()).await;assert_eq!(again,workspace);
    assert_eq!(call(&mut link,workspace,enqueue.clone()).await,receipt);
    let snapshot=get(&mut link,workspace).await;let item=&snapshot["state"]["items"]["process-item"];
    accepted(&call(&mut link,workspace,Action::Approve {mutation:mutation("a",snapshot["state"]["revision"].as_u64().unwrap(),2),
        item:"process-item".into(),run:item["run"].as_str().unwrap().into(),pin:item["pin"].as_str().unwrap().into(),candidate:None}).await);
    drop(link);
    // An entirely disconnected orchestrator sends no stage commands or ACKs.
    let store=farcooler_store::Store::open(dir.path().join("state/farcooler.db")).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30),async {
        loop {
            let state=store.workflow_state(&workspace.to_string()).unwrap().unwrap();
            if state.items["process-item"].phase==Phase::Done {assert_eq!(state.items["process-item"].iteration,1);break;}
            assert!(!matches!(state.items["process-item"].phase,Phase::Uncertain|Phase::Failed),"{state:?}");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    let mut link=Client::connect(dir.path().join("d.sock"),"reconnect","1").await.unwrap();
    assert_eq!(call(&mut link,workspace,enqueue).await,receipt);
    end_process(&mut child,&mut link).await;
}

// The caller-seat guard correctly sees Cargo's Codex ancestor as Agent(None).
// Run the real socket clients in a real, owned, isolated terminal, whose tmux
// server is detached from that ancestor. No authority override or stub switch.
async fn person_terminal(name: &str) -> bool {
    if std::env::var_os("FARCOOLER_WORKFLOW_PERSON_CHILD").is_some() { return false; }
    let dir=tempfile::Builder::new().prefix("w931-person").tempdir_in("/tmp").unwrap();
    let socket=dir.path().join("t.sock");
    struct Reap(std::path::PathBuf);
    impl Drop for Reap { fn drop(&mut self) { farcooler_tmux::reap_server(&self.0.to_string_lossy()); } }
    let _reap=Reap(socket.clone());
    let log=dir.path().join("output");let status=dir.path().join("status");
    let quote=|s:&str| format!("'{}'",s.replace('\'',"'\\''"));
    let command=format!("FARCOOLER_WORKFLOW_PERSON_CHILD=1 {} --exact {} --nocapture > {} 2>&1; code=$?; printf '%s' \"$code\" > {}",
        quote(&std::env::current_exe().unwrap().to_string_lossy()),quote(name),quote(&log.to_string_lossy()),quote(&status.to_string_lossy()));
    let output=tokio::process::Command::new("tmux").args(["-S",&socket.to_string_lossy(),"-f","/dev/null","new-session","-d","-s","person","/bin/sh","-c",&command]).output().await.unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    tokio::time::timeout(std::time::Duration::from_secs(90),async {
        while !status.exists() {tokio::time::sleep(std::time::Duration::from_millis(20)).await;}
    }).await.unwrap();
    let output=std::fs::read_to_string(log).unwrap();println!("{output}");
    assert_eq!(std::fs::read_to_string(status).unwrap(),"0","{output}");
    true
}

#[tokio::test]
async fn workflow_service_fast_ship_without_checks_or_review() {
    if person_terminal("workflow_service_fast_ship_without_checks_or_review").await { return; }
    let (dir,mut config)=configured(false,"pass").await;config.checks.clear();
    let host=Host::open(dir.path()).await;let mut link=host.connect().await;
    let definition=config.definition().unwrap();let adapter=Some(AdapterConfiguration::Coding(config));
    let pin=farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
    accepted(&call(&mut link,host.workspace,Action::Publish { notifications:None,mutation:mutation("p",0,1),expected_pin:None,definition,adapter:adapter.map(Box::new)}).await);
    accepted(&call(&mut link,host.workspace,Action::Enqueue {mutation:mutation("e",0,1),item:"fast".into(),pin}).await);
    for _ in 0..200 {
        let snapshot=get(&mut link,host.workspace).await;
        if snapshot["state"]["items"]["fast"]["phase"]=="Done" { break; }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(get(&mut link,host.workspace).await["state"]["items"]["fast"]["phase"],"Done");
    drop(link);host.close().await;
}

#[tokio::test]
async fn workflow_service_admitted_ci_recovers_original_bundle_after_reconfiguration() {
    if person_terminal("workflow_service_admitted_ci_recovers_original_bundle_after_reconfiguration").await { return; }
    let (dir,config)=configured(false,"ci-pending").await;
    let host=Host::open(dir.path()).await;let workspace=host.workspace;let mut link=host.connect().await;
    let definition=config.definition().unwrap();let adapter=Some(AdapterConfiguration::Coding(config.clone()));
    let pin=farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
    accepted(&call(&mut link,workspace,Action::Publish { notifications:None,mutation:mutation("p",0,1),expected_pin:None,definition,adapter:adapter.map(Box::new)}).await);
    accepted(&call(&mut link,workspace,Action::Enqueue {mutation:mutation("e",0,1),item:"recover".into(),pin:pin.clone()}).await);
    let snapshot=tokio::time::timeout(std::time::Duration::from_secs(20),async {
        loop {
            let snapshot=get(&mut link,workspace).await;
            if snapshot["state"]["items"]["recover"]["phase"]=="Uncertain" {break snapshot;}
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    let mut other=config;other.base=snapshot["state"]["items"]["recover"]["candidate"].as_str().unwrap().into();
    other.ci.workflow="different-ci-policy".into();
    accepted(&call(&mut link,workspace,Action::Publish { notifications:None,mutation:mutation("b",snapshot["state"]["revision"].as_u64().unwrap(),2),
        expected_pin:Some(pin.clone()),definition:other.definition().unwrap(),adapter:Some(Box::new(AdapterConfiguration::Coding(other)))}).await);
    let effects=host.service.store.workflow_outbox(&workspace.to_string()).unwrap();
    let ci=effects.iter().find(|e| matches!(e.effect.kind,farcooler_workflow::EffectKind::Execute(farcooler_workflow::StageKind::Ci))).unwrap();
    assert_eq!(ci.dispatch_state,"uncertain");
    let stem=farcooler_workflow::digest(&ci.effect);
    let receipts=dir.path().join("state/workflow/receipts");
    let fact=receipts.join(format!("{stem}.process.json"));
    // Process facts use the original invocation identity throughout recovery.
    let provider=receipts.join(format!("{stem}.provider.json"));
    let before=std::fs::read(&fact).unwrap();
    drop(link);host.close().await;
    let host=Host::open(dir.path()).await;let mut link=host.connect().await;
    let mut observed:serde_json::Value=serde_json::from_slice(&std::fs::read(&provider).unwrap()).unwrap();
    observed["run"]["status"]="completed".into();
    std::fs::write(provider,serde_json::to_vec(&observed).unwrap()).unwrap();
    // A durable trusted receipt update is reconciled; no process relaunch.
    for _ in 0..100 {
        if get(&mut link,workspace).await["state"]["items"]["recover"]["phase"]=="Done" {break;}
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let snapshot=get(&mut link,workspace).await;
    assert_eq!(snapshot["state"]["items"]["recover"]["phase"],"Done");
    assert_eq!(snapshot["state"]["items"]["recover"]["policy_pin"],pin);
    assert_eq!(std::fs::read(fact).unwrap(),before);
    assert_eq!(host.service.store.workflow_outbox(&workspace.to_string()).unwrap().iter()
        .filter(|e| matches!(e.effect.kind,farcooler_workflow::EffectKind::Execute(farcooler_workflow::StageKind::Ci))).count(),1);
    drop(link);host.close().await;
}

/// `call`, keeping a refusal the runner answers with an error instead of a receipt.
async fn try_call(link:&mut Link, workspace:Uuid, action:Action) -> Result<serde_json::Value,String> {
    let mut req=farcooler_transport::request(action.method());
    req.target_resource_id=Some(bytes::Bytes::copy_from_slice(workspace.as_bytes()));
    req.payload=Some(request::Payload::WorkflowCommand(pb::WorkflowCommand {schema:1,json:serde_json::to_string(&action).unwrap()}));
    match link.call(req).await { Ok(reply)=>match reply.value { Some(result::Value::WorkflowReply(reply))=>Ok(serde_json::from_str(&reply.json).unwrap()), other=>Err(format!("{other:?}")) },
        Err(error)=>Err(error.to_string()) }
}

/// ov-967: a ticket started with a worktree and branch the orchestrator names runs in a worktree the app lists,
/// claimed for the workspace, on that branch, from main as it is at Start (not as it was at publish). A branch
/// that already exists is refused, and nothing is made.
#[tokio::test]
async fn workflow_service_a_started_ticket_runs_in_a_named_worktree_from_main_at_start() {
    if person_terminal("workflow_service_a_started_ticket_runs_in_a_named_worktree_from_main_at_start").await { return; }
    use farcooler_protocol::workflow_commands::NamedTree;
    let (dir,config)=configured(false,"pass").await;
    let host=Host::open(dir.path()).await; let workspace=host.workspace; let mut link=host.connect().await;
    let definition=config.definition().unwrap();
    let adapter=Some(AdapterConfiguration::Coding(config.clone()));
    let pin=farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
    accepted(&call(&mut link,workspace,Action::Publish { notifications:None,mutation:mutation("p",0,1),expected_pin:None,definition,adapter:adapter.map(Box::new)}).await);
    let ticket=host.service.store.create_task(workspace,"named",farcooler_store::models::Actor::User).unwrap();
    host.service.track_tickets(Some(ticket.id)).await;
    // Main moves on after publishing.
    git(&config.repository,&["-c","user.name=Test","-c","user.email=test@localhost","-c","commit.gpgsign=false","commit","--allow-empty","-m","later"]).await.unwrap();
    let main=git(&config.repository,&["rev-parse","HEAD"]).await.unwrap();
    git(&config.repository,&["update-ref",&config.landing_ref,&main]).await.unwrap();
    assert_ne!(main,config.base);
    let existing=git(&config.repository,&["branch","--show-current"]).await.unwrap();
    let start=|id:&str,snapshot:&serde_json::Value,branch:&str| { let item=&snapshot["state"]["items"][&ticket.key];
        Action::Start { mutation:mutation(id,snapshot["state"]["revision"].as_u64().unwrap(),2),item:ticket.key.clone(),
            run:item["run"].as_str().unwrap().into(),pin:item["pin"].as_str().unwrap().into(),start_pin:pin.clone(),models:Default::default(),
            worktree:Some(NamedTree { name:"phones-plan".into(),branch:branch.into() }) } };
    let snapshot=get(&mut link,workspace).await;
    let refused=try_call(&mut link,workspace,start("taken",&snapshot,&existing)).await;
    assert!(refused.as_ref().is_err_and(|e| e.contains("already exists")),"{refused:?}");
    let listed=|| host.service.store.list_worktrees_for_repository(host.service.store.get_workspace(workspace).unwrap().repository_id).unwrap();
    assert!(!listed().iter().any(|w| w.worktree_path.ends_with("phones-plan")),"a refused Start makes nothing");
    let snapshot=get(&mut link,workspace).await;
    accepted(&call(&mut link,workspace,start("start",&snapshot,"e-liang/phones-plan")).await);
    tokio::time::timeout(std::time::Duration::from_secs(30),async { loop {
        let state=host.service.store.workflow_state(&workspace.to_string()).unwrap().unwrap();
        if state.items.get(&ticket.key).is_some_and(|i| i.phase==Phase::Done) { break; }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }}).await.unwrap();
    let state=host.service.store.workflow_state(&workspace.to_string()).unwrap().unwrap();
    let tree=state.items[&ticket.key].tree.clone().expect("the run records its tree");
    assert_eq!((tree.name.as_str(),tree.branch.as_str(),tree.base.as_str()),("phones-plan","e-liang/phones-plan",main.as_str()),"main at Start");
    let row=listed().into_iter().find(|w| w.worktree_path==tree.path).expect("the app lists it");
    assert_eq!((row.branch.as_str(),row.workspace_id),("e-liang/phones-plan",Some(workspace)),"on the named branch, claimed for the workspace");
    assert!(tree.path.contains("/state/worktrees/"),"where the app keeps worktrees, not the runtime's private root: {}",tree.path);
    assert_eq!(git(Path::new(&tree.path),&["merge-base","HEAD",&main]).await.unwrap(),main,"built on main as it was at Start");
    drop(link);host.close().await;
}

/// ov-967 (review): a Start that names a bad branch, one that clashes with a branch by prefix, or a tree an
/// earlier Start already holds is refused at Start, so no ticket sticks at Uncertain on a name git would refuse.
#[tokio::test]
async fn workflow_service_refuses_a_bad_or_taken_named_tree_at_start() {
    if person_terminal("workflow_service_refuses_a_bad_or_taken_named_tree_at_start").await { return; }
    use farcooler_protocol::workflow_commands::NamedTree;
    let (dir,config)=configured(false,"pass").await;
    let host=Host::open(dir.path()).await; let workspace=host.workspace; let mut link=host.connect().await;
    let definition=config.definition().unwrap();
    let adapter=Some(AdapterConfiguration::Coding(config.clone()));
    let pin=farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
    accepted(&call(&mut link,workspace,Action::Publish { notifications:None,mutation:mutation("p",0,1),expected_pin:None,definition,adapter:adapter.map(Box::new)}).await);
    let first=host.service.store.create_task(workspace,"first",farcooler_store::models::Actor::User).unwrap();
    let second=host.service.store.create_task(workspace,"second",farcooler_store::models::Actor::User).unwrap();
    host.service.track_tickets(Some(first.id)).await;
    host.service.track_tickets(Some(second.id)).await;
    git(&config.repository,&["branch","clash"]).await.unwrap();
    let start=|id:&str,key:&str,snapshot:&serde_json::Value,name:&str,branch:&str| { let item=&snapshot["state"]["items"][key];
        Action::Start { mutation:mutation(id,snapshot["state"]["revision"].as_u64().unwrap(),2),item:key.into(),
            run:item["run"].as_str().unwrap().into(),pin:item["pin"].as_str().unwrap().into(),start_pin:pin.clone(),models:Default::default(),
            worktree:Some(NamedTree { name:name.into(),branch:branch.into() }) } };
    // Names git refuses and the cheap check misses, and a branch that is the prefix of another.
    for (id,branch) in [("dot","e-liang/.hidden"),("lock","e-liang/x.lock/y"),("child","clash/child")] {
        let snapshot=get(&mut link,workspace).await;
        let refused=try_call(&mut link,workspace,start(id,&first.key,&snapshot,"fresh",branch)).await;
        assert!(refused.is_err(),"{branch} must be refused at Start: {refused:?}");
    }
    let snapshot=get(&mut link,workspace).await;
    assert!(snapshot["state"]["items"][&first.key]["tree"].is_null(),"a refused Start records nothing");
    // Hold the repository lock so the first run's tree is recorded but not yet made.
    let repository=host.service.store.get_workspace(workspace).unwrap().repository_id;
    let lock=host.service.repo_lock(repository);
    let held=lock.lock().await;
    accepted(&call(&mut link,workspace,start("one",&first.key,&snapshot,"shared","e-liang/one")).await);
    for (id,name,branch) in [("same-tree","shared","e-liang/two"),("same-branch","other","e-liang/one")] {
        let snapshot=get(&mut link,workspace).await;
        let refused=try_call(&mut link,workspace,start(id,&second.key,&snapshot,name,branch)).await;
        assert!(refused.as_ref().is_err_and(|e| e.contains("already exists")),"{id}: {refused:?}");
    }
    drop(held);
    drop(link);host.close().await;
}

/// ov-967 (review): when the run's tree is made but the app's row for it can't be written, the run takes the
/// tree and branch it just made back out, so a retry can name them again.
#[tokio::test]
async fn workflow_service_unmakes_a_named_tree_it_could_not_record() {
    if person_terminal("workflow_service_unmakes_a_named_tree_it_could_not_record").await { return; }
    use farcooler_protocol::workflow_commands::NamedTree;
    let (dir,config)=configured(false,"pass").await;
    let host=Host::open(dir.path()).await; let workspace=host.workspace; let mut link=host.connect().await;
    let definition=config.definition().unwrap();
    let adapter=Some(AdapterConfiguration::Coding(config.clone()));
    let pin=farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
    accepted(&call(&mut link,workspace,Action::Publish { notifications:None,mutation:mutation("p",0,1),expected_pin:None,definition,adapter:adapter.map(Box::new)}).await);
    let ticket=host.service.store.create_task(workspace,"unrecorded",farcooler_store::models::Actor::User).unwrap();
    host.service.track_tickets(Some(ticket.id)).await;
    let repository=host.service.store.get_workspace(workspace).unwrap().repository_id;
    let lock=host.service.repo_lock(repository);
    let held=lock.lock().await;
    let snapshot=get(&mut link,workspace).await; let item=&snapshot["state"]["items"][&ticket.key];
    accepted(&call(&mut link,workspace,Action::Start { mutation:mutation("start",snapshot["state"]["revision"].as_u64().unwrap(),2),item:ticket.key.clone(),
        run:item["run"].as_str().unwrap().into(),pin:item["pin"].as_str().unwrap().into(),start_pin:pin.clone(),models:Default::default(),
        worktree:Some(NamedTree { name:"unrecorded".into(),branch:"e-liang/unrecorded".into() }) }).await);
    let tree=host.service.store.workflow_state(&workspace.to_string()).unwrap().unwrap().items[&ticket.key].tree.clone().unwrap();
    // A row already holds that path, so the run's own row can't be written.
    host.service.store.create_worktree(repository,"blocker",&tree.path,false).unwrap();
    drop(held);
    tokio::time::timeout(std::time::Duration::from_secs(30),async { loop {
        let state=host.service.store.workflow_state(&workspace.to_string()).unwrap().unwrap();
        if state.items.get(&ticket.key).is_some_and(|i| matches!(i.phase,Phase::Uncertain|Phase::Failed)) { break; }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }}).await.unwrap();
    assert!(!Path::new(&tree.path).exists(),"the tree it made is taken out");
    let branches=git(&config.repository,&["branch","--list","e-liang/unrecorded"]).await.unwrap();
    assert!(branches.is_empty(),"and its branch: {branches}");
    drop(link);host.close().await;
}

#[path = "workflow_service/safety.rs"]
mod safety;
