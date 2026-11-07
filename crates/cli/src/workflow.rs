//! File publication and refusal of unbound retained retries over the ordinary daemon link.
use std::io::Read;
use clap::{Args, Subcommand};
use farcooler_protocol::{capability, workflow_commands::{Action, Mutation, MAX_BYTES}, v1::{self as pb, request, result}};
use uuid::Uuid;
use farcooler_client::session::workflow::PreparedWorkflowAction;
use crate::{Fallible, req_for, with, expect_value};

#[derive(Args, Clone)]
pub struct Identity {
    /// Keep this identity and every argument unchanged when retrying a lost reply.
    #[arg(long)] pub command_id: String,
    #[arg(long)] pub revision: u64,
    #[arg(long)] pub epoch: u64,
    #[arg(long)] pub sequence: u64,
}
impl Identity {
    fn mutation(&self) -> Mutation { Mutation { id:self.command_id.clone(),revision:self.revision,epoch:self.epoch,sequence:self.sequence } }
}
#[derive(Args)]
pub struct Publication {
    #[arg(long)] pub file: std::path::PathBuf,
    #[arg(long)] pub expected_pin: Option<String>,
    #[command(flatten)] pub identity: Identity,
}
impl Publication {
    pub fn action(&self) -> Result<Action, Box<dyn std::error::Error>> {
        let bytes = read_file(&self.file)?;
        let value:serde_json::Value = serde_json::from_slice(&bytes)?;
        let (definition, adapter, notifications) = if value.get("definition").is_some() {
            #[derive(serde::Deserialize, serde::Serialize)]
            #[serde(deny_unknown_fields)]
            struct Bundle { definition: farcooler_protocol::workflow_commands::Definition, #[serde(default, skip_serializing_if = "Option::is_none")] adapter: Option<farcooler_protocol::workflow_config::AdapterConfiguration>, #[serde(default, skip_serializing_if = "Option::is_none")] notifications: Option<farcooler_protocol::workflow_config::NotificationPolicy> }
            let bundle: Bundle = serde_json::from_value(value.clone())?;
            if serde_json::to_value(&bundle)? != value { return Err("unknown bundle fields".into()); }
            (bundle.definition, bundle.adapter.map(Box::new), bundle.notifications)
        } else {
            let definition = serde_json::from_value(value.clone())?;
            if serde_json::to_value(&definition)? != value { return Err("unknown definition fields".into()); }
            (definition, None, None)
        };
        Ok(Action::Publish { notifications, adapter, mutation:self.identity.mutation(),expected_pin:self.expected_pin.clone(),definition })
    }
}
#[derive(Subcommand)]
pub enum WorkflowCmd {
    /// Read raw shared state, configuration and notification history.
    Get { workspace: Uuid },
    /// Admit against the exact configuration pin selected from get.
    Enqueue { workspace: Uuid, item: String, #[arg(long)] pin:String, #[command(flatten)] identity:Identity },
    /// Approve only the run, candidate and definition that were inspected.
    Approve { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[arg(long)] candidate:Option<String>,#[command(flatten)] identity:Identity },
    /// Accept a semantic review or request rework using the inspected evidence JSON file.
    Review { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[arg(long)] evidence:std::path::PathBuf,#[arg(long)] passed:bool,#[command(flatten)] identity:Identity },
    /// Request bounded rework with a reason tied to the exact inspected evidence.
    ///
    /// Durable CLI retry is unsupported: the saved file cannot verify its original recipient.
    /// If the outcome is uncertain, inspect the original runner state; do not issue a fresh command.
    Rework { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,
        #[arg(long)] evidence:std::path::PathBuf,#[arg(long)] reason:String,
        /// Save immutable command bytes before the first attempt; must be a new file. Durable CLI retry is unsupported.
        #[arg(long)] prepared_file:std::path::PathBuf,#[command(flatten)] identity:Identity },
    /// Refuse an unbound retained command without connecting or sending.
    ///
    /// Durable CLI retry is unsupported: the saved file cannot verify its original recipient.
    /// If the outcome is uncertain, inspect the original runner state; do not issue a fresh command.
    Retry { #[arg(long)] prepared_file:std::path::PathBuf },
    /// Request cancellation of the inspected run.
    Cancel { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[command(flatten)] identity:Identity },
}
impl WorkflowCmd {
    pub fn action(self) -> Result<(Uuid, Action), Box<dyn std::error::Error>> {
        Ok(match self {
            Self::Retry { .. } => return Err("retry uses retained prepared bytes".into()),
            Self::Get { workspace } => (workspace,Action::Get),
            Self::Enqueue { workspace,item,pin,identity } => (workspace,Action::Enqueue { mutation:identity.mutation(),item,pin }),
            Self::Approve { workspace,item,run,pin,candidate,identity } => (workspace,Action::Approve { mutation:identity.mutation(),item,run,pin,candidate }),
            Self::Review { workspace,item,run,pin,evidence,passed,identity } => {
                let bytes=read_file(&evidence)?;
                (workspace,Action::Review { mutation:identity.mutation(),item,run,pin,evidence:serde_json::from_slice(&bytes)?,passed,reason:None })
            }
            Self::Rework { workspace,item,run,pin,evidence,reason,identity,.. } => {
                if !farcooler_protocol::workflow_commands::valid_rework_reason(&reason) {
                    return Err("invalid rework reason: nonblank, at most 4096 UTF-8 bytes, no control characters except tabs/line breaks".into());
                }
                let bytes=read_file(&evidence)?;
                (workspace,Action::Review { mutation:identity.mutation(),item,run,pin,evidence:serde_json::from_slice(&bytes)?,passed:false,reason:Some(reason) })
            }
            Self::Cancel { workspace,item,run,pin,identity } => (workspace,Action::Cancel { mutation:identity.mutation(),item,run,pin }),
        })
    }
}
fn read_file(path:&std::path::Path) -> Result<Vec<u8>,Box<dyn std::error::Error>> {
    use std::os::unix::fs::OpenOptionsExt;
    let file=std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW).open(path)?;
    if !file.metadata()?.is_file() { return Err("workflow input must be a regular file".into()); }
    let mut bytes=Vec::new();file.take((MAX_BYTES+1) as u64).read_to_end(&mut bytes)?;
    if bytes.len()>MAX_BYTES { return Err("workflow input exceeds 60 KiB".into()); }
    Ok(bytes)
}
pub fn request(workspace:Uuid, action:&Action) -> Result<pb::Request, Box<dyn std::error::Error>> {
    let json = serde_json::to_string(action)?;
    if json.len() > MAX_BYTES { return Err("workflow command exceeds 60 KiB".into()); }
    let mut r = with(req_for(action.method(),workspace),request::Payload::WorkflowCommand(pb::WorkflowCommand { schema:1,json }));
    r.required_capabilities.push(capability::WORKFLOW_COMMANDS.into());
    Ok(r)
}
pub async fn send(link:&mut crate::Link, workspace:Uuid, action:Action, json:bool) -> Fallible {
    if !link.daemon_capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) { return Err("runner lacks workflow commands".into()); }
    let reply = link.call(request(workspace,&action)?).await?;
    let result::Value::WorkflowReply(reply) = expect_value(reply.value)? else { return Err("unexpected workflow reply".into()) };
    if reply.schema != 1 { return Err("unsupported workflow reply schema".into()); }
    let value:serde_json::Value = serde_json::from_str(&reply.json)?;
    if json { println!("{}",value); } else { println!("{}",serde_json::to_string_pretty(&value)?); }
    if value.pointer("/receipt/result/Err").is_some() { return Err("workflow command refused; receipt shown above".into()); }
    Ok(())
}
fn save_prepared(path:&std::path::Path, prepared:&PreparedWorkflowAction) -> Result<(),Box<dyn std::error::Error>> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    prepared.validate()?;
    let bytes=serde_json::to_vec(prepared)?;
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW).open(path)?;
    file.write_all(&bytes)?;file.sync_all()?;
    let parent=path.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
#[cfg(test)]
fn load_prepared(path:&std::path::Path) -> Result<PreparedWorkflowAction,Box<dyn std::error::Error>> {
    let prepared:PreparedWorkflowAction=serde_json::from_slice(&read_file(path)?)?;
    prepared.validate()?;Ok(prepared)
}
fn prepared_request(prepared:&PreparedWorkflowAction) -> Result<pb::Request,Box<dyn std::error::Error>> {
    let action=prepared.validate()?;
    let mut r=with(req_for(action.method(),prepared.workspace),request::Payload::WorkflowCommand(pb::WorkflowCommand{schema:1,json:prepared.json.clone()}));
    r.required_capabilities.push(capability::WORKFLOW_COMMANDS.into());Ok(r)
}
async fn send_prepared(link:&mut crate::Link,prepared:&PreparedWorkflowAction,json:bool) -> Fallible {
    if !link.daemon_capabilities().iter().any(|c|c==capability::WORKFLOW_COMMANDS){return Err("runner lacks workflow commands".into());}
    let reply=link.call(prepared_request(prepared)?).await?;
    let result::Value::WorkflowReply(reply)=expect_value(reply.value)? else {return Err("unexpected workflow reply".into())};
    if reply.schema!=1{return Err("unsupported workflow reply schema".into());}
    let value:serde_json::Value=serde_json::from_str(&reply.json)?;
    if json{println!("{}",reply.json);}else{println!("{}",serde_json::to_string_pretty(&value)?);}
    if value.pointer("/receipt/result/Err").is_some(){return Err("workflow command refused; receipt shown above".into());}Ok(())
}
pub async fn workflow(runner:Option<&str>,cmd:WorkflowCmd,json:bool) -> Fallible {
    workflow_with_connect(cmd,json,||crate::connect_to(runner)).await
}
async fn workflow_with_connect<F, C>(cmd:WorkflowCmd,json:bool,connect:C) -> Fallible
where F:std::future::Future<Output=Result<crate::Link,Box<dyn std::error::Error>>>, C:FnOnce()->F {

    if matches!(cmd,WorkflowCmd::Retry{..}) {
        return Err("This CLI connection cannot verify the original recipient of this retained command; retry was not sent. Durable CLI retry is unsupported (ov-941). If the outcome is uncertain, inspect the original runner state; do not issue a fresh command.".into());
    }
    let saved=match &cmd{WorkflowCmd::Rework{prepared_file,..}=>Some(prepared_file.clone()),_=>None};
    let (workspace,action) = cmd.action()?;
    if let Some(path)=saved {
        let prepared=PreparedWorkflowAction{schema:1,workspace,json:serde_json::to_string(&action)?};
        save_prepared(&path,&prepared)?;
        eprintln!("Saved immutable command bytes. Durable CLI retry is unsupported (ov-941): this file cannot verify its original recipient. If the outcome is uncertain, inspect the original runner state; do not issue a fresh command.");
        let mut link=connect().await?;
        return send_prepared(&mut link,&prepared,json).await;
    }
    let mut link = connect().await?;
    send(&mut link,workspace,action,json).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    async fn assert_retry_does_not_connect(runner:Option<&str>) {
        use std::cell::Cell;
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("retained.json");
        // Same workspace/action can exist at another runtime; freshness is not destination proof.
        let action=Action::Review{mutation:Mutation{id:"retained-id".into(),revision:4,epoch:1,sequence:2},
            item:"a".into(),run:"same-cloned-run".into(),pin:"same-pin".into(),passed:false,
            evidence:serde_json::from_str(r#"{"candidate":"c","manifest":"m","receipt":"r","policy_pin":"p"}"#).unwrap(),
            reason:Some("  修复\n ".into())};
        let prepared=PreparedWorkflowAction{schema:1,workspace:Uuid::nil(),json:format!("  {} ",serde_json::to_string(&action).unwrap())};
        for bytes in [serde_json::to_vec(&prepared).unwrap(),b"malformed".to_vec(),
            br#"{"schema":99,"runner":"alias","host":"install-id"}"#.to_vec()] {
            std::fs::write(&path,&bytes).unwrap();
            let connections=Cell::new(0);
            let error=workflow_with_connect(WorkflowCmd::Retry{prepared_file:path.clone()},false,||async {
                connections.set(connections.get()+1);
                // Never call a real connector or construct a Link: no sender can be reached.
                Err(format!("private connector reached for {runner:?}; no mutation sent").into())
            }).await.unwrap_err();
            assert_eq!(std::fs::read(&path).unwrap(),bytes,"retained bytes changed");
            assert_eq!(connections.get(),0,"retry reached connector for {runner:?}");
            assert!(error.to_string().contains("retry was not sent"));
        }
        let absent=dir.path().join("absent.json");
        let error=workflow_with_connect(WorkflowCmd::Retry{prepared_file:absent.clone()},false,||async {
            panic!("missing-file retry reached connector")
        }).await.unwrap_err();
        assert!(!absent.exists());assert!(error.to_string().contains("retry was not sent"));
    }
    #[tokio::test]
    async fn workflow_retry_local_refuses_before_connector() { assert_retry_does_not_connect(None).await; }
    #[tokio::test]
    async fn workflow_retry_remote_refuses_before_connector() { assert_retry_does_not_connect(Some("other-cloned-runner")).await; }
    #[test]
    fn workflow_retry_help_states_unsupported_recovery() {
        for command in ["rework","retry"] {
            let help=crate::Cli::try_parse_from(["farcooler","workflow",command,"--help"]).err().unwrap().to_string();
            assert!(help.contains("Durable CLI retry is unsupported"),"{help}");
            assert!(help.contains("If the outcome is uncertain"),"{help}");
            assert!(help.contains("do not issue a fresh command"),"{help}");
        }
    }
    #[test]
    fn workflow_reasoned_cli_keeps_reason_and_identity() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("evidence.json");
        std::fs::write(&path,r#"{"candidate":"c","manifest":"m","receipt":"r","policy_pin":"p"}"#).unwrap();
        let workspace=Uuid::nil().to_string();let evidence=path.to_str().unwrap();
        for reason in ["  修复\n  "," ","bad\0"] {
            let args=["farcooler","workflow","rework",&workspace,"a","--run","run","--pin","pin","--evidence",evidence,"--reason",reason,"--prepared-file",evidence,
                "--command-id","stable","--revision","18446744073709551614","--epoch","1","--sequence","2"];
            let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(args).unwrap().command else {panic!()};
            let result=cmd.action();
            if farcooler_protocol::workflow_commands::valid_rework_reason(reason) {
                let (_,action)=result.unwrap();let raw=serde_json::to_string(&action).unwrap();
                assert_eq!(farcooler_protocol::workflow_commands::parse(&raw).unwrap(),action);
                assert!(matches!(action,Action::Review{reason:Some(r),passed:false,mutation,..} if r==reason && mutation.revision==u64::MAX-1));
            } else {assert!(result.is_err());}
        }
    }
    #[test]
    fn workflow_prepared_retry_uses_exact_retained_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("prepared.json");
        let action=Action::Review{mutation:Mutation{id:"same".into(),revision:4,epoch:1,sequence:2},item:"a".into(),run:"run".into(),pin:"pin".into(),
            evidence:serde_json::from_str(r#"{"candidate":"c","manifest":"m","receipt":"r","policy_pin":"p"}"#).unwrap(),passed:false,reason:Some("  修复\n ".into())};
        let prepared=PreparedWorkflowAction{schema:1,workspace:Uuid::nil(),json:format!("  {} ",serde_json::to_string(&action).unwrap())};
        save_prepared(&path,&prepared).unwrap();assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode()&0o777,0o600);
        let original=std::fs::read(&path).unwrap();assert!(save_prepared(&path,&prepared).is_err());
        let restored=load_prepared(&path).unwrap();assert_eq!(restored,prepared);
        let r=prepared_request(&restored).unwrap();let Some(request::Payload::WorkflowCommand(body))=r.payload else {panic!()};
        assert_eq!(body.json,prepared.json);assert_eq!(std::fs::read(&path).unwrap(),original);
        let args=["farcooler","workflow","retry","--prepared-file",path.to_str().unwrap()];
        assert!(matches!(crate::Cli::try_parse_from(args).unwrap().command,crate::Command::Workflow(WorkflowCmd::Retry{..})));
        let mut invalid=prepared;invalid.schema=99;std::fs::write(&path,serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(load_prepared(&path).is_err());
        let symlink=dir.path().join("link");std::os::unix::fs::symlink(&path,&symlink).unwrap();assert!(load_prepared(&symlink).is_err());
    }
    #[test]
    fn workflow_real_producer_parser_fixture_roundtrip_and_bounds() {
        let workspace=Uuid::now_v7();
        let args=["farcooler","workflow","enqueue",&workspace.to_string(),"item","--pin","wf1:abc","--command-id","stable","--revision","4","--epoch","1","--sequence","2"];
        let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(args).unwrap().command else {panic!("wrong command")};
        let (workspace,action)=cmd.action().unwrap();
        let request=super::request(workspace,&action).unwrap();
        assert_eq!(request.method,"workflow.enqueue");
        assert_eq!(request.required_capabilities,[capability::WORKFLOW_COMMANDS]);
        let Some(request::Payload::WorkflowCommand(body))=request.payload else {panic!("wrong payload")};
        assert_eq!(body.schema,1);
        assert_eq!(farcooler_protocol::workflow_commands::parse(&body.json).unwrap(),action);
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("workflow.json");
        let definition=farcooler_daemon::workflow_runtime::coding_definition(false);
        std::fs::write(&path,serde_json::to_vec(&definition).unwrap()).unwrap();
        let publication=Publication { file:path.clone(),expected_pin:None,identity:Identity {command_id:"publish".into(),revision:0,epoch:1,sequence:1} };
        let action=publication.action().unwrap();let request=super::request(workspace,&action).unwrap();
        let Some(request::Payload::WorkflowCommand(body))=request.payload else {panic!("wrong payload")};
        assert_eq!(farcooler_protocol::workflow_commands::parse(&body.json).unwrap(),action);
        std::fs::write(&path,vec![b' ';MAX_BYTES+1]).unwrap();
        assert!(publication.action().is_err());
        assert!(crate::Cli::try_parse_from(["farcooler","workflow","enqueue",&workspace.to_string(),"item","--pin","x"]).is_err());
    }
}

#[cfg(test)]
mod bundle_tests {
    use super::*;
    #[test]
    fn workflow_bundle_file_closed_schema_and_bounded_inputs() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("workflow.json");
        let definition=farcooler_protocol::workflow_commands::Definition::tracker("owner");
        let publication=Publication { file:path.clone(),expected_pin:None,identity:Identity {command_id:"publish".into(),revision:0,epoch:1,sequence:1} };
        // Bad adapter tags and extra envelope fields must not be silently lost.
        for value in [serde_json::json!({"definition":definition,"adapter":{"version":"untrusted/v9","config":{} }}),
            serde_json::json!({"definition":definition,"adapter":null,"principal":"owner"})] {
            std::fs::write(&path,serde_json::to_vec(&value).unwrap()).unwrap();assert!(publication.action().is_err());
        }
        let command=serde_json::json!({"argv":["/usr/bin/true"],"environment":{},"timeout_ms":1000});
        let config=serde_json::json!({"repository":"/isolated","base":"0000000000000000000000000000000000000000","landing_ref":"refs/heads/main","landing":null,
            "harness":{"Command":command},"checks":[command],"reviewer":null,"ci":{"repository":"offline/repo","workflow":"check","event":"push","command":command}});
        let value=serde_json::json!({"definition":definition,"adapter":{"version":"coding/v1","config":config}});
        std::fs::write(&path,serde_json::to_vec(&value).unwrap()).unwrap();
        let action=publication.action().unwrap();
        assert!(matches!(&action,Action::Publish {adapter:Some(_),..}));
        assert_eq!(farcooler_protocol::workflow_commands::parse(&serde_json::to_string(&action).unwrap()).unwrap(),action);
        // Real file producer supports a tracker policy without a coding adapter.
        let policy=serde_json::json!({"definition":definition,"notifications":{"points":["waiting_approval","stage_changed"]}});
        std::fs::write(&path,serde_json::to_vec(&policy).unwrap()).unwrap();
        let policy_action=publication.action().unwrap();
        assert!(matches!(&policy_action,Action::Publish {adapter:None,notifications:Some(_),..}));
        assert_eq!(farcooler_protocol::workflow_commands::parse(&serde_json::to_string(&policy_action).unwrap()).unwrap(),policy_action);
        for notifications in [serde_json::json!({"points":["untrusted-prose-filter"]}),
            serde_json::json!({"points":[],"grant":"Controller"})] {
            std::fs::write(&path,serde_json::to_vec(&serde_json::json!({"definition":definition,"notifications":notifications})).unwrap()).unwrap();
            assert!(publication.action().is_err(),"unknown policy points or caller authority must not survive the producer");
        }
        let fifo=dir.path().join("fifo");let name=std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe {libc::mkfifo(name.as_ptr(),0o600)},0);
        assert!(read_file(&fifo).is_err());
        std::fs::write(&path,vec![b' ';MAX_BYTES+1]).unwrap();assert!(publication.action().is_err());
    }
}
