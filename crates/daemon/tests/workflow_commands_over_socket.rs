//! Existing transport/RPC admission and isolated runtime, including reconnect receipts.
use std::sync::Arc;
use farcooler_daemon::{rpc::RpcFactory,service::Service,watch::Watcher,workflow_runtime::{Runtime,coding_definition}};
use farcooler_protocol::{capability,workflow_commands::{Action,Mutation},v1::{self as pb,request,result,Scope}};
use farcooler_transport::{Client,HandshakeConfig,Peer,UnixListenerServer};
use farcooler_workflow::Progression;
use uuid::Uuid;

type Link=Client<tokio::net::unix::OwnedReadHalf,tokio::net::unix::OwnedWriteHalf>;
struct Harness { _dir:tempfile::TempDir,socket:std::path::PathBuf,workspace:Uuid,server:tokio::task::JoinHandle<()>,worker:tokio::task::JoinHandle<()>,stop:tokio::sync::watch::Sender<bool> }
impl Drop for Harness { fn drop(&mut self) {let _=self.stop.send(true);self.server.abort();self.worker.abort();} }
async fn start(scope:Scope,pid:Option<i32>,enabled:bool)->Harness {
    let dir=tempfile::Builder::new().prefix("wf925").tempdir_in("/tmp").unwrap();
    let mut runtime=Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let handle=runtime.command_handle(&runtime.controller()).unwrap();
    let (stop,rx)=tokio::sync::watch::channel(false);
    let worker=tokio::spawn(async move {runtime.run(rx).await.unwrap()});
    let service=Arc::new(Service::open_in(dir.path().join("home")).await.unwrap());
    let root=service.store.create_repository_root(service.host_id,"/fixture",0).unwrap();
    let repo=service.store.create_repository(service.host_id,root.id,"fixture","/fixture/.git","").unwrap();
    let workspace=service.store.ensure_main_workspace(repo.id).unwrap().id;
    let watcher=Watcher::new(service.clone());let socket=dir.path().join("d.sock");
    let listener=UnixListenerServer::bind(&socket).unwrap();
    let server=tokio::spawn(async move {listener.serve(move |_| {
        let factory=RpcFactory::new(service.clone(),watcher.clone(),Arc::new(tokio::sync::Notify::new()),Peer {client_id:None,scope}).with_caller_pid(pid);
        Some((HandshakeConfig {daemon_version:"fixture".into()},if enabled {factory.with_workflow_fixture(handle.clone())} else {factory}))
    }).await.unwrap()});
    Harness {_dir:dir,socket,workspace,server,worker,stop}
}
async fn connect(h:&Harness)->Link {Client::connect(&h.socket,"workflow-producer","1").await.unwrap()}
fn req(workspace:Uuid,action:&Action)->pb::Request {
    let mut req=farcooler_transport::request(action.method());req.target_resource_id=Some(bytes::Bytes::copy_from_slice(workspace.as_bytes()));
    req.required_capabilities=vec![capability::WORKFLOW_COMMANDS.into()];
    req.payload=Some(request::Payload::WorkflowCommand(pb::WorkflowCommand {schema:1,json:serde_json::to_string(action).unwrap()}));req
}
async fn call(link:&mut Link,workspace:Uuid,action:&Action)->serde_json::Value {
    let result::Value::WorkflowReply(reply)=link.call(req(workspace,action)).await.unwrap().value.unwrap() else {panic!("wrong result")};
    assert_eq!(reply.schema,1);serde_json::from_str(&reply.json).unwrap()
}
fn m(id:&str,revision:u64,sequence:u64)->Mutation {Mutation {id:id.into(),revision,epoch:1,sequence}}
#[tokio::test]
async fn workflow_socket_scope_unknown_caller_disabled_runtime_and_forged_authority_refuse() {
    for (scope,pid,enabled) in [(Scope::Read,None,true),(Scope::Control,Some(-1),true),(Scope::Control,None,false)] {
        let h=start(scope,pid,enabled).await;let mut link=connect(&h).await;
        assert!(link.call(req(h.workspace,&Action::Get)).await.is_err());
    }
    let h=start(Scope::Control,None,true).await;let mut link=connect(&h).await;
    let mut request=req(h.workspace,&Action::Get);
    let Some(request::Payload::WorkflowCommand(body))=request.payload.as_mut() else {unreachable!()};
    body.json=r#"{"action":"Get","grant":"Controller","principal":"owner"}"#.into();
    assert!(link.call(request).await.is_err());
    let mut request=req(h.workspace,&Action::Get);request.method="workflow.approve".into();
    assert!(link.call(request).await.is_err());
}
#[tokio::test]
async fn workflow_socket_publication_approve_cancel_and_reconnect_receipts() {
    let h=start(Scope::Control,None,true).await;let mut link=connect(&h).await;
    let mut definition=coding_definition(false);definition.stages[0].progression=Progression::AwaitApproval {principal:"owner".into()};let pin=definition.pin();
    let publish=Action::Publish {mutation:m("publish",0,1),expected_pin:None,definition};
    let receipt=call(&mut link,h.workspace,&publish).await;
    assert!(receipt.pointer("/receipt/result/Ok").is_some());
    let enqueue=Action::Enqueue {mutation:m("enqueue",0,1),item:"a".into(),pin:pin.clone()};
    let accepted=call(&mut link,h.workspace,&enqueue).await;
    drop(link);let mut link=connect(&h).await;
    assert_eq!(call(&mut link,h.workspace,&enqueue).await,accepted);
    let snapshot=call(&mut link,h.workspace,&Action::Get).await;
    assert_eq!(snapshot["state"]["items"]["a"]["phase"],"WaitingApproval");
    let approve=Action::Approve {mutation:m("approve",snapshot["state"]["revision"].as_u64().unwrap(),2),item:"a".into(),run:snapshot["state"]["items"]["a"]["run"].as_str().unwrap().into(),pin:pin.clone(),candidate:None};
    let approval=call(&mut link,h.workspace,&approve).await;assert!(approval.pointer("/receipt/result/Ok").is_some());
    // No informational notice ACK or stage commands; runtime owns all automatic edges.
    let mut done=serde_json::Value::Null;
    for _ in 0..100 {
        done=call(&mut link,h.workspace,&Action::Get).await;
        if done["state"]["items"]["a"]["phase"]=="Done" {break;}
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(done["state"]["items"]["a"]["phase"],"Done");
    assert_eq!(call(&mut link,h.workspace,&approve).await,approval);
    let revision=done["state"]["revision"].as_u64().unwrap();
    let second=Action::Enqueue {mutation:m("second",revision,3),item:"b".into(),pin:pin.clone()};
    assert!(call(&mut link,h.workspace,&second).await.pointer("/receipt/result/Ok").is_some());
    let snapshot=call(&mut link,h.workspace,&Action::Get).await;
    let cancel=Action::Cancel {mutation:m("cancel",snapshot["state"]["revision"].as_u64().unwrap(),4),item:"b".into(),run:snapshot["state"]["items"]["b"]["run"].as_str().unwrap().into(),pin};
    let receipt=call(&mut link,h.workspace,&cancel).await;assert!(receipt.pointer("/receipt/result/Ok").is_some());
    assert_eq!(call(&mut link,h.workspace,&Action::Get).await["state"]["items"]["b"]["phase"],"Cancelled");
    assert_eq!(call(&mut link,h.workspace,&cancel).await,receipt);
}
