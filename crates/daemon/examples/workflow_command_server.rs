//! Private fixture embedding for CLI transport evidence. No installed daemon activation.
use std::{path::PathBuf, sync::Arc};
use farcooler_daemon::{rpc::RpcFactory, service::Service, watch::Watcher, workflow_runtime::{Runtime,coding_definition}};
use farcooler_protocol::v1::Scope;
use farcooler_transport::{UnixListenerServer,HandshakeConfig,Peer};
use farcooler_workflow::Progression;

#[tokio::main]
async fn main() -> Result<(),Box<dyn std::error::Error>> {
    let root=PathBuf::from(std::env::args_os().nth(1).ok_or("supply a fresh private fixture path")?);
    std::fs::create_dir(&root)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&root,std::fs::Permissions::from_mode(0o700))?;
    let mut runtime=Runtime::create(&root.join("runtime")).await?;
    let controller=runtime.controller();let handle=runtime.command_handle(&controller)?;
    let (_shutdown,rx)=tokio::sync::watch::channel(false);
    let worker=tokio::spawn(async move {runtime.run(rx).await});
    let home=root.join("home");std::fs::create_dir(&home)?;
    farcooler_daemon::service::stub_agents_in_this_process();
    let service=Arc::new(Service::open_in(home.clone()).await?);
    let store=&service.store;
    let repository_root=store.create_repository_root(service.host_id,&root.to_string_lossy(),0)?;
    let repo=store.create_repository(service.host_id,repository_root.id,"workflow-fixture",&root.join("runtime/repo/.git").to_string_lossy(),"")?;
    let workspace=store.ensure_main_workspace(repo.id)?;
    let watcher=Watcher::new(service.clone());
    let socket=home.join("farcoolerd.sock");
    let server=UnixListenerServer::bind(&socket)?;
    let mut definition=coding_definition(false);
    definition.stages[0].progression=Progression::AwaitApproval { principal:"owner".into() };
    std::fs::write(root.join("definition.json"),serde_json::to_vec_pretty(&definition)?)?;
    std::fs::write(root.join("ready.json"),serde_json::to_vec(&serde_json::json!({"workspace":workspace.id,"home":home,"pin":definition.pin()}))?)?;
    // The development harness supplies an authenticated owner peer, just as existing
    // socket tests do. Production PID/SSH admission is unchanged and not activated.
    let serving=server.serve(move |_| Some((HandshakeConfig {daemon_version:"workflow-fixture".into()},
        RpcFactory::new(service.clone(),watcher.clone(),Arc::new(tokio::sync::Notify::new()),Peer {client_id:None,scope:Scope::Control}).with_workflow_fixture(handle.clone()))));
    tokio::select! { result=serving => result?, _=tokio::signal::ctrl_c()=>{} }
    worker.abort();let _=worker.await;
    Ok(())
}
