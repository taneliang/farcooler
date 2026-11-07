use super::*;
use farcooler_protocol::workflow_config::LaunchFile;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

fn native_config(config: &mut CodingConfig, settings: &Path, socket: &Path) {
    std::fs::write(settings, serde_json::to_vec(&farcooler_daemon::hook_install::claude_settings(socket)).unwrap()).unwrap();
    let CodingHarness::Command(mut process) = config.harness.clone() else { unreachable!() };
    process.argv.clear();
    config.harness = CodingHarness::Native { preset:"claude".into(),
        settings:Some(LaunchFile { path:settings.canonicalize().unwrap(), digest:farcooler_workflow::digest(&std::fs::read(settings).unwrap()) }),
        project_hooks:None, plugin_dir:None, prompt:"private fixture".into(), process };
}
async fn publish_waiting(link: &mut Link, workspace: Uuid, config: &CodingConfig) -> String {
    let mut definition = config.definition().unwrap();
    definition.stages[0].progression = Progression::AwaitApproval { principal:"owner".into() };
    let adapter = Some(AdapterConfiguration::Coding(config.clone()));
    let pin = farcooler_store::workflow_publication::configuration_pin(&definition, adapter.as_ref());
    accepted(&call(link,workspace,Action::Publish { mutation:mutation("p",0,1), expected_pin:None,
        definition, adapter:adapter.map(Box::new) }).await);
    accepted(&call(link,workspace,Action::Enqueue { mutation:mutation("e",0,1),item:"drift".into(),pin:pin.clone() }).await);
    pin
}
async fn unaffected(host: &Host, link: &mut Link) {
    let repo = host.service.store.get_workspace(host.workspace).unwrap().repository_id;
    let workspace = host.service.store.create_workspace(repo,"unaffected","other").unwrap().id;
    let mut definition = Definition::tracker("other");
    definition.stages[0].progression = Progression::AwaitApproval { principal:"owner".into() };
    let pin = definition.pin();
    accepted(&call(link,workspace,Action::Publish { mutation:mutation("p",0,1),expected_pin:None,definition,adapter:None }).await);
    accepted(&call(link,workspace,Action::Enqueue { mutation:mutation("e",0,1),item:"cancel".into(),pin:pin.clone() }).await);
    let state = get(link,workspace).await;
    let item = &state["state"]["items"]["cancel"];
    accepted(&call(link,workspace,Action::Cancel { mutation:mutation("c",1,2),item:"cancel".into(),
        run:item["run"].as_str().unwrap().into(),pin:item["pin"].as_str().unwrap().into() }).await);
    assert_eq!(get(link,workspace).await["state"]["items"]["cancel"]["phase"],"Cancelled");
    // Fresh automatic tracker work must also progress on the same owner.
    let mut definition = Definition::tracker("automatic");
    definition.stages[0].progression = Progression::Automatic; let pin = definition.pin();
    let revision = get(link,workspace).await["state"]["revision"].as_u64().unwrap();
    accepted(&call(link,workspace,Action::Publish { mutation:mutation("p2",revision,2),expected_pin:Some(state["configuration"]["pin"].as_str().unwrap().into()),definition,adapter:None }).await);
    accepted(&call(link,workspace,Action::Enqueue { mutation:mutation("e2",revision,3),item:"done".into(),pin }).await);
    for _ in 0..100 {
        if get(link,workspace).await["state"]["items"]["done"]["phase"] == "Done" { return; }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("unaffected workspace stopped progressing");
}
async fn launch_drift(mode: &str) {
        let (dir,mut config) = configured(false,"pass").await;
        let host = Host::open(dir.path()).await; let mut link = host.connect().await;
        let executable = dir.path().join("executable");
        let settings = dir.path().join("settings.json");
        if mode == "executable" {
            std::fs::copy("/bin/sh",&executable).unwrap();
            std::fs::set_permissions(&executable,std::fs::Permissions::from_mode(0o700)).unwrap();
            config.checks[0].argv[0] = executable.to_string_lossy().into_owned();
        } else if mode == "settings" {
            native_config(&mut config,&settings,&dir.path().canonicalize().unwrap().join("state/workflow/h.sock"));
        }
        let pin = publish_waiting(&mut link,host.workspace,&config).await;
        let lock = if mode == "resource" {
            let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
                .open(config.repository.join(".git/farcooler-workflow.lock")).unwrap();
            rustix::fs::flock(&file,rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
            Some(file)
        } else { None };
        if mode == "executable" { std::fs::remove_file(&executable).unwrap(); }
        if mode == "settings" { std::fs::write(&settings,"changed").unwrap(); }
        let state = get(&mut link,host.workspace).await;
        let item = &state["state"]["items"]["drift"];
        accepted(&call(&mut link,host.workspace,Action::Approve { mutation:mutation("approve",1,2),item:"drift".into(),
            run:item["run"].as_str().unwrap().into(),pin:item["pin"].as_str().unwrap().into(),candidate:None }).await);
        for _ in 0..100 {
            let state = get(&mut link,host.workspace).await;
            if state["state"]["items"]["drift"]["phase"] == "Failed" { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let state = get(&mut link,host.workspace).await;
        assert_eq!(state["state"]["items"]["drift"]["phase"],"Failed","{mode}: {state}");
        assert_eq!(state["state"]["items"]["drift"]["policy_pin"],pin);
        assert!(!dir.path().join("started").exists());
        unaffected(&host,&mut link).await;
        drop(lock); drop(link); host.close().await;
}
#[tokio::test]
async fn workflow_service_executable_drift_is_local() {
    if person_terminal("safety::workflow_service_executable_drift_is_local").await { return; }
    launch_drift("executable").await;
}
#[tokio::test]
async fn workflow_service_settings_drift_is_local() {
    if person_terminal("safety::workflow_service_settings_drift_is_local").await { return; }
    launch_drift("settings").await;
}
#[tokio::test]
async fn workflow_service_resource_refusal_is_local() {
    if person_terminal("safety::workflow_service_resource_refusal_is_local").await { return; }
    launch_drift("resource").await;
}
#[tokio::test]
async fn workflow_service_wrong_native_target_refused_before_publication() {
    if person_terminal("safety::workflow_service_wrong_native_target_refused_before_publication").await { return; }
    let (dir,mut config) = configured(false,"pass").await;
    let host = Host::open(dir.path()).await; let mut link = host.connect().await;
    native_config(&mut config,&dir.path().join("settings.json"),&dir.path().join("wrong.sock"));
    let action = Action::Publish { mutation:mutation("wrong",0,1),expected_pin:None,
        definition:config.definition().unwrap(),adapter:Some(Box::new(AdapterConfiguration::Coding(config))) };
    let mut req = farcooler_transport::request(action.method());
    req.target_resource_id = Some(bytes::Bytes::copy_from_slice(host.workspace.as_bytes()));
    req.payload = Some(request::Payload::WorkflowCommand(pb::WorkflowCommand { schema:1,json:serde_json::to_string(&action).unwrap() }));
    assert!(link.call(req).await.is_err(),"invalid endpoint received publication acknowledgment");
    assert!(host.service.store.workflow_configuration(&host.workspace.to_string()).unwrap().is_none());
    assert!(host.service.store.workflow_state(&host.workspace.to_string()).unwrap().is_none());
    unaffected(&host,&mut link).await;
    drop(link); host.close().await;
}
