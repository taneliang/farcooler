//! File publication and explicit identity retries over the ordinary daemon link.
use std::io::Read;
use clap::{Args, Subcommand};
use farcooler_protocol::{capability, workflow_commands::{Action, Mutation, MAX_BYTES}, v1::{self as pb, request, result}};
use uuid::Uuid;
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
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW).open(&self.file)?;
        if !file.metadata()?.is_file() { return Err("workflow definition must be a regular file".into()); }
        let mut bytes = Vec::new();
        file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES { return Err("workflow definition exceeds 60 KiB".into()); }
        let value:serde_json::Value = serde_json::from_slice(&bytes)?;
        let definition = serde_json::from_value(value.clone())?;
        if serde_json::to_value(&definition)? != value { return Err("unknown definition fields".into()); }
        Ok(Action::Publish { mutation:self.identity.mutation(),expected_pin:self.expected_pin.clone(),definition })
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
    /// Request cancellation of the inspected run.
    Cancel { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[command(flatten)] identity:Identity },
}
impl WorkflowCmd {
    pub fn action(self) -> Result<(Uuid, Action), Box<dyn std::error::Error>> {
        Ok(match self {
            Self::Get { workspace } => (workspace,Action::Get),
            Self::Enqueue { workspace,item,pin,identity } => (workspace,Action::Enqueue { mutation:identity.mutation(),item,pin }),
            Self::Approve { workspace,item,run,pin,candidate,identity } => (workspace,Action::Approve { mutation:identity.mutation(),item,run,pin,candidate }),
            Self::Review { workspace,item,run,pin,evidence,passed,identity } => {
                let file=std::fs::File::open(evidence)?; let mut bytes=Vec::new();
                file.take((MAX_BYTES+1) as u64).read_to_end(&mut bytes)?;
                if bytes.len()>MAX_BYTES { return Err("evidence exceeds bound".into()); }
                (workspace,Action::Review { mutation:identity.mutation(),item,run,pin,evidence:serde_json::from_slice(&bytes)?,passed })
            }
            Self::Cancel { workspace,item,run,pin,identity } => (workspace,Action::Cancel { mutation:identity.mutation(),item,run,pin }),
        })
    }
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
pub async fn workflow(runner:Option<&str>,cmd:WorkflowCmd,json:bool) -> Fallible {
    let (workspace,action) = cmd.action()?;
    let mut link = crate::connect_to(runner).await?;
    send(&mut link,workspace,action,json).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
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
