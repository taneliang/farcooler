//! Actual isolated socket -> Session -> shared guard -> durable daemon receipt trace.
use super::*;
use std::sync::{Arc, atomic::{AtomicI32, Ordering}};
use farcooler_daemon::{rpc::RpcFactory, service::Service, watch::Watcher,
    workflow_runtime::{Runtime, coding_definition}};
use farcooler_protocol::v1::Scope;
use farcooler_transport::{Peer, HandshakeConfig, UnixListenerServer};
struct Harness {
    _dir: tempfile::TempDir, socket: std::path::PathBuf, workspace: Uuid, service: Arc<Service>,
    scope: Arc<AtomicI32>, server: tokio::task::JoinHandle<()>, worker: tokio::task::JoinHandle<()>,
    stop: tokio::sync::watch::Sender<bool>,
}
impl Drop for Harness { fn drop(&mut self) { let _=self.stop.send(true);self.server.abort();self.worker.abort(); } }
async fn start() -> Harness {
    let dir=tempfile::Builder::new().prefix("wf927").tempdir_in("/tmp").unwrap();
    let mut runtime=Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let handle=runtime.command_handle(&runtime.controller()).unwrap();
    let (stop,rx)=tokio::sync::watch::channel(false);
    let worker=tokio::spawn(async move {runtime.run(rx).await.unwrap()});
    let service=Arc::new(Service::open_in(dir.path().join("home")).await.unwrap());
    let root=service.store.create_repository_root(service.host_id,"/fixture",0).unwrap();
    let repo=service.store.create_repository(service.host_id,root.id,"fixture","/fixture/.git","").unwrap();
    let workspace=service.store.ensure_main_workspace(repo.id).unwrap().id;
    service.store.bind_owner_device("phone",None).unwrap();
    let socket=dir.path().join("d.sock"); let listener=UnixListenerServer::bind(&socket).unwrap();
    let svc=service.clone(); let scope=Arc::new(AtomicI32::new(Scope::Control as i32)); let scope2=scope.clone();
    let server=tokio::spawn(async move {listener.serve(move |_| {
        Some((HandshakeConfig {daemon_version:"fixture".into()},RpcFactory::new(svc.clone(),Watcher::new(svc.clone()),
            Arc::new(tokio::sync::Notify::new()),Peer {client_id:Some("phone".into()),scope:Scope::try_from(scope2.load(Ordering::SeqCst)).unwrap()})
            .with_workflow_fixture(handle.clone())))
    }).await.unwrap()});
    Harness {_dir:dir,socket,workspace,service,scope,server,worker,stop}
}
async fn seed(session: &Session, h: &Harness, item: &str, revision: u64, sequence: u64) {
    if sequence == 1 {
        let mut definition=coding_definition(false);
        definition.stages[0].progression=Progression::AwaitApproval {principal:"owner".into()};
        let a=wire::Action::Publish {mutation:Mutation {id:"publish".into(),sequence:1,revision:0,epoch:1},expected_pin:None,definition};
        let receipt=session.workflow_wire(h.workspace,serde_json::to_string(&a).unwrap()).await.unwrap();
    assert!(receipt.pointer("/receipt/result/Ok").is_some(),"seed {item} revision={revision} sequence={sequence}: {receipt}");
    }
    let raw=session.workflow_snapshot(h.workspace).await.unwrap();
    let pin=raw.configuration["pin"].as_str().unwrap().to_string();
    let a=wire::Action::Enqueue {mutation:Mutation {id:format!("enqueue-{item}"),sequence,revision,epoch:1},item:item.into(),pin};
    let receipt=session.workflow_wire(h.workspace,serde_json::to_string(&a).unwrap()).await.unwrap();
    assert!(receipt.pointer("/receipt/result/Ok").is_some(),"seed {item} revision={revision} sequence={sequence}: {receipt}");
}
async fn call(session: &Session, method: &str, args: serde_json::Value) -> serde_json::Value {
    // The same dispatch arm C and JNI use; WASM delegates to workflow_call.
    crate::ffi::workflow_test_dispatch(session,method,&args).await.unwrap()
}
#[tokio::test]
async fn workflow_session_socket_lost_reply_reconnect_stale_and_auth_change() {
    let h=start().await; let session=Session::connect_local(&h.socket).await.unwrap();
    seed(&session,&h,"a",0,1).await;
    let raw=call(&session,"workflow.snapshot",json!({"workspace":h.workspace})).await;
    let snapshot:WorkflowSnapshot=serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(snapshot.presentation.schema,2);
    assert_eq!(snapshot.context.capabilities.decision_principal.as_deref(),Some("owner"));
    let action=snapshot.presentation.runs[0].actions.iter().find(|a|a.kind==cards::ActionKind::Cancel).unwrap();
    let args=json!({"workspace":h.workspace,"item":"a","action":action,"identity":{
        "id":"cancel-a","sequence":2,"revision":snapshot.state.revision,"epoch":snapshot.state.epoch}});
    let result=call(&session,"workflow.prepare_action",args.clone()).await;
    let prepared:PreparedWorkflowAction=serde_json::from_value(result["prepared"].clone()).unwrap();
    let bytes=prepared.json.clone();
    let parsed=wire::parse(&bytes).unwrap();
    let pending=session.start(parsed.method(),Some(h.workspace),Some(request::Payload::WorkflowCommand(
        WorkflowCommand {schema:1,json:bytes.clone()})),vec![capability::WORKFLOW_COMMANDS.into()]).unwrap();
    let observer=Session::connect_local(&h.socket).await.unwrap();
    let accepted=tokio::time::timeout(std::time::Duration::from_secs(10),async {
        loop { let s=observer.workflow_snapshot(h.workspace).await.unwrap();
            if s.state.items["a"].phase==farcooler_workflow::Phase::Cancelled {break s;}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    drop(pending);drop(session);
    let restored:PreparedWorkflowAction=serde_json::from_str(&serde_json::to_string(&prepared).unwrap()).unwrap();
    assert_eq!(restored.json,bytes);
    let replacement=Session::connect_local(&h.socket).await.unwrap();
    let receipt=call(&replacement,"workflow.submit_action",json!({"prepared":restored})).await;
    assert!(receipt.pointer("/receipt/result/Ok").is_some());
    assert_eq!(call(&replacement,"workflow.submit_action",json!({"prepared":prepared})).await,receipt);
    assert_eq!(replacement.workflow_snapshot(h.workspace).await.unwrap().state,accepted.state);
    assert!(replacement.workflow_call("workflow.prepare_action",args).await.is_err());
    let mut conflict=prepared.clone();
    let mut tampered:serde_json::Value=serde_json::from_str(&conflict.json).unwrap();
    tampered["mutation"]["revision"]=json!(accepted.state.revision);
    conflict.json=tampered.to_string();
    assert!(replacement.workflow_submit_action(&conflict).await.is_err(),"identity reuse with changed bytes must conflict");
    // A new observation with identical state but a different authenticated scope
    // must not validate a descriptor from the previous caller context.
    seed(&replacement,&h,"b",accepted.state.revision,3).await;
    let old=replacement.workflow_snapshot(h.workspace).await.unwrap();
    h.scope.store(Scope::HostAdmin as i32,Ordering::SeqCst);
    let changed=Session::connect_local(&h.socket).await.unwrap();
    let new=changed.workflow_snapshot(h.workspace).await.unwrap();
    assert_eq!(old.state,new.state);assert_ne!(old.context.binding,new.context.binding);
    let a=old.presentation.runs.iter().find(|r|r.item=="b").unwrap().actions.iter().find(|a|a.kind==cards::ActionKind::Approve).unwrap();
    assert!(changed.workflow_prepare_action(h.workspace,"b",a.clone(),WorkflowCommandIdentity {
        id:"approve-b".into(),sequence:4,revision:old.state.revision,epoch:old.state.epoch }).await.is_err());
    let fresh=new.presentation.runs.iter().find(|r|r.item=="b").unwrap().actions.iter().find(|a|a.kind==cards::ActionKind::Cancel).unwrap();
    let PreparedWorkflowResult::Mutation {prepared:stale_pending}=changed.workflow_prepare_action(h.workspace,"b",fresh.clone(),
        WorkflowCommandIdentity {id:"cancel-b".into(),sequence:4,revision:new.state.revision,epoch:new.state.epoch}).await.unwrap() else {panic!()};
    let PreparedWorkflowResult::Mutation {prepared:concurrent}=changed.workflow_prepare_action(h.workspace,"b",fresh.clone(),
        WorkflowCommandIdentity {id:"concurrent-cancel-b".into(),sequence:4,revision:new.state.revision,epoch:new.state.epoch}).await.unwrap() else {panic!()};
    assert!(changed.workflow_submit_action(&concurrent).await.unwrap().pointer("/receipt/result/Ok").is_some());
    let refused=changed.workflow_submit_action(&stale_pending).await.unwrap();
    assert!(refused.pointer("/receipt/result/Err/Stale").is_some() || refused["receipt"]["result"]["Err"]=="Stale", "{refused}");
    assert_eq!(changed.workflow_submit_action(&stale_pending).await.unwrap(),refused);
    // A revoke/readmit cycle changes authenticated context even after replacement.
    let before=changed.workflow_snapshot(h.workspace).await.unwrap();
    h.service.sessions().mark_revoked("phone");
    h.service.sessions().readmit("phone").unwrap();
    let readmitted=Session::connect_local(&h.socket).await.unwrap();
    let after=readmitted.workflow_snapshot(h.workspace).await.unwrap();
    assert_eq!(before.state,after.state);assert_ne!(before.context.binding,after.context.binding);
    // Authority withdrawal also refuses already prepared receipt retries.
    h.service.store.set_device_cap("phone",Some(farcooler_store::access::Cap::Viewer)).unwrap();
    assert!(readmitted.workflow_submit_action(&prepared).await.is_err());
    for extra in ["capabilities","principal","grant","snapshot"] {
        let mut args=json!({"workspace":h.workspace});args[extra]=json!("forged");
        assert!(replacement.workflow_call("workflow.snapshot",args).await.is_err());
    }
    eprintln!("trace=Session/C-JNI-dispatch -> authenticated getter -> schema2 -> opaque Cancel -> prepared bytes -> dropped answer -> reconnect durable receipt; stale/auth-context/withdrawal refused; prepared={bytes}; receipt={receipt}");
}

#[tokio::test]
async fn workflow_session_socket_effectful_approval_retry_does_not_reexecute() {
    let h=start().await;let session=Session::connect_local(&h.socket).await.unwrap();
    seed(&session,&h,"a",0,1).await;
    let snapshot=session.workflow_snapshot(h.workspace).await.unwrap();
    let action=snapshot.presentation.runs[0].actions.iter().find(|a|a.kind==cards::ActionKind::Approve).unwrap();
    let PreparedWorkflowResult::Mutation {prepared}=session.workflow_prepare_action(h.workspace,"a",action.clone(),
        WorkflowCommandIdentity {id:"approve-effectful".into(),sequence:2,revision:snapshot.state.revision,epoch:snapshot.state.epoch}).await.unwrap() else {panic!()};
    let bytes=prepared.json.clone();let wire=wire::parse(&bytes).unwrap();
    let lost=session.start(wire.method(),Some(h.workspace),Some(request::Payload::WorkflowCommand(
        WorkflowCommand {schema:1,json:bytes.clone()})),vec![capability::WORKFLOW_COMMANDS.into()]).unwrap();
    let observer=Session::connect_local(&h.socket).await.unwrap();
    let done=tokio::time::timeout(std::time::Duration::from_secs(10),async {
        loop {let s=observer.workflow_snapshot(h.workspace).await.unwrap();
            if s.state.items["a"].phase==farcooler_workflow::Phase::Done {break s;}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    drop(lost);drop(session);
    let replacement=Session::connect_local(&h.socket).await.unwrap();
    let restored:PreparedWorkflowAction=serde_json::from_str(&serde_json::to_string(&prepared).unwrap()).unwrap();
    assert_eq!(restored.json,bytes);
    let receipt=replacement.workflow_submit_action(&restored).await.unwrap();
    assert!(receipt.pointer("/receipt/result/Ok").is_some());
    assert_eq!(replacement.workflow_submit_action(&restored).await.unwrap(),receipt);
    assert_eq!(replacement.workflow_snapshot(h.workspace).await.unwrap().state,done.state);
    assert!(done.state.revision>receipt["receipt"]["result"]["Ok"].as_u64().unwrap());
    eprintln!("effectful trace=opaque Approve -> pending exact bytes -> automatic fixture stages Done -> discard reply -> reconnect same receipt twice; unchanged final state revision={}; request={bytes}; receipt={receipt}",done.state.revision);
}

#[tokio::test]
async fn workflow_session_selection_route_stale_auth_and_retry() {
    let h=start().await; let session=Session::connect_local(&h.socket).await.unwrap();
    seed(&session,&h,"FeatureA",0,1).await;
    let snapshot=session.workflow_snapshot(h.workspace).await.unwrap();
    assert_eq!(snapshot.host,h.service.host_id.to_string());
    let action=snapshot.presentation.runs[0].actions.iter().find(|a|a.kind==cards::ActionKind::Approve).unwrap();
    let args=json!({"workspace":h.workspace,"item":"FeatureA","action":action,
        "revision":snapshot.presentation.revision.to_string(),"epoch":snapshot.presentation.epoch.to_string()});
    let result=call(&session,"workflow.prepare_selection",args.clone()).await;
    let prepared:PreparedWorkflowAction=serde_json::from_value(result["prepared"].clone()).unwrap();
    let before=prepared.json.clone();
    let wire: serde_json::Value=serde_json::from_str(&before).unwrap();
    let sequence=wire["mutation"]["sequence"].as_u64().unwrap();
    assert_eq!(sequence,snapshot.state.source_sequences.get("wire:owner").copied().unwrap_or(0)+1);
    let accepted=call(&session,"workflow.submit_action",json!({"prepared":prepared})).await;
    assert!(accepted.pointer("/receipt/result/Ok").is_some());
    assert_eq!(session.workflow_snapshot(h.workspace).await.unwrap().state.source_sequences.get("wire:owner"),Some(&sequence));
    drop(session);
    let replacement=Session::connect_local(&h.socket).await.unwrap();
    assert_eq!(call(&replacement,"workflow.submit_action",json!({"prepared":prepared})).await,accepted);
    assert_eq!(before,prepared.json);
    assert!(replacement.workflow_call("workflow.prepare_selection",args.clone()).await.is_err());
    let mut forged=args.clone();forged["capabilities"]=json!({"controller":true});
    assert!(replacement.workflow_call("workflow.prepare_selection",forged).await.is_err());
    let mut rounded=args.clone();rounded["revision"]=json!(snapshot.presentation.revision);
    assert!(replacement.workflow_call("workflow.prepare_selection",rounded).await.is_err());
    h.service.sessions().mark_revoked("phone");
    assert!(replacement.workflow_call("workflow.submit_action",json!({"prepared":prepared})).await.is_err());
    eprintln!("FeatureA actual socket/shared selection -> persisted envelope -> submit -> replacement Session same receipt -> stale selection and revoked retry refused; bytes={before}; receipt={accepted}");
}

/// Private browser evidence bridge. Explicit trusted fixture admission, never
/// a production listener or a claim about an enrolled owner's credentials.
#[tokio::test]
#[ignore = "owned browser harness only"]
async fn workflow_host_browser_bridge() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let output=std::path::PathBuf::from(std::env::var("OV930_BROWSER_BRIDGE").expect("private output directory"));
    let h=start().await;
    let session=Session::connect_local(&h.socket).await.unwrap();
    seed(&session,&h,"FeatureA",0,1).await; drop(session);
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    std::fs::write(output.join("ready.json"),json!({"port":listener.local_addr().unwrap().port(),"workspace":h.workspace}).to_string()).unwrap();
    let mut submitted=Vec::new(); let mut receipts=Vec::new(); let mut preparations=0; let mut lossless=Vec::new();
    loop {
        let (mut stream,_)=tokio::time::timeout(std::time::Duration::from_secs(120),listener.accept()).await.unwrap().unwrap();
        let mut bytes=Vec::new(); let (header_end,length)=loop {
            let mut buf=[0u8;4096];let n=stream.read(&mut buf).await.unwrap();assert!(n>0);bytes.extend_from_slice(&buf[..n]);
            if let Some(end)=bytes.windows(4).position(|v|v==b"\r\n\r\n") {
                let headers=String::from_utf8_lossy(&bytes[..end]);
                let length=headers.lines().find_map(|s|s.to_ascii_lowercase().strip_prefix("content-length:")
                    .map(|n|n.trim().parse::<usize>().unwrap())).unwrap_or(0);
                break (end+4,length)
            }
            assert!(bytes.len()<16*1024);
        };
        assert!(length<1024*1024);
        while bytes.len()<header_end+length {let mut buf=[0u8;4096];let n=stream.read(&mut buf).await.unwrap();assert!(n>0);bytes.extend_from_slice(&buf[..n]);}
        let body:serde_json::Value=serde_json::from_slice(&bytes[header_end..header_end+length]).unwrap();
        let method=body["method"].as_str().unwrap();
        let result=if method=="stop" {json!({"stopped":true})} else {
            // Fresh actual Session for each call, equivalent fixture admission.
            // The browser adapter still controls prepare, persistence and retry.
            let current=Session::connect_local(&h.socket).await.unwrap();
            let maximum=method=="workflow.prepare_selection" && body["args"]["revision"]=="18446744073709551614";
            if method=="workflow.prepare_selection" && !maximum {preparations+=1;}
            if method=="workflow.submit_action" {submitted.push(body["args"]["prepared"].clone());}
            let result=if maximum {
                assert_eq!(body["args"]["epoch"],"18446744073709551615");
                let error=current.workflow_call(method,body["args"].clone()).await.unwrap_err();
                assert!(matches!(error,SessionError::Refused { ref what,.. } if what=="workflow_stale"));
                lossless.push(body["args"].clone());
                json!({"error":"workflow_stale"})
            } else {call(&current,method,body["args"].clone()).await};
            if method=="workflow.submit_action" {receipts.push(result.clone());}
            result
        };
        let raw=result.to_string();
        let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",raw.len(),raw);
        stream.write_all(response.as_bytes()).await.unwrap();stream.shutdown().await.unwrap();
        if method=="stop" {break;}
    }
    assert_eq!(preparations,1);assert_eq!(submitted.len(),2);assert_eq!(submitted[0],submitted[1]);assert_eq!(receipts[0],receipts[1]);
    let current=Session::connect_local(&h.socket).await.unwrap();let final_state=current.workflow_snapshot(h.workspace).await.unwrap();
    assert_eq!(final_state.state.items["FeatureA"].phase,farcooler_workflow::Phase::Done);
    std::fs::write(output.join("result.json"),json!({"preparations":preparations,"submitted":submitted,"receipts":receipts,
        "lossless_stale_observations":lossless,"final_revision":final_state.state.revision,"phase":final_state.state.items["FeatureA"].phase}).to_string()).unwrap();
    eprintln!("actual browser -> host adapter -> shared Session/C-JNI dispatch -> Unix socket -> Runtime FeatureA Done; same prepared bytes/receipt after discarded answer, preparations={preparations}");
}
