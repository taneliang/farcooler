//! Explicit workflow commands over the ordinary authenticated daemon link.
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
    /// Retransmit only to the same selected authenticated runner/principal, keeping reason,
    /// evidence contents, run/pin and all identity arguments unchanged. No state is reloaded.
    /// On an uncertain answer, inspect the original runner; do not allocate a fresh identity.
    /// A changed destination is a new invocation, not recovery of the original recipient.
    Rework { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,
        #[arg(long)] evidence:std::path::PathBuf,#[arg(long)] reason:String,#[command(flatten)] identity:Identity },
    /// Request cancellation of the inspected run.
    Cancel { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[command(flatten)] identity:Identity },
    /// Run a failed item's step again. At most three retries per item.
    Retry { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[command(flatten)] identity:Identity },
    /// Queue a ticket waiting at its join step into the open group for that step.
    Queue { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[command(flatten)] identity:Identity },
    /// Take a queued ticket out of a group that has not left yet.
    Leave { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[command(flatten)] identity:Identity },
    /// Settle a push whose outcome was lost, after looking at the target yourself. The runner reads the
    /// target first and refuses a word it contradicts. Pass `--landed true` if the commit is on the target.
    Settle { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[arg(long, action = clap::ArgAction::Set, required = true)] landed:bool,#[command(flatten)] identity:Identity },
    /// Bring a ticket under the engine with no run: it sits in Backlog until the orchestrator moves it.
    Track { workspace:Uuid,item:String,#[arg(long)] column:Option<String>,#[command(flatten)] identity:Identity },
    /// Move a tracked ticket to a column: backlog, todo, doing, review or decision; done or cancelled close it.
    Move { workspace:Uuid,item:String,#[arg(long)] to:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[command(flatten)] identity:Identity },
    /// Start a run on a tracked ticket under the published definition `--start-pin`.
    Start { workspace:Uuid,item:String,#[arg(long)] start_pin:String,#[arg(long)] run:String,#[arg(long)] pin:String,
        /// A build step's model, as `<step>=<model>` (`build=codex:gpt-5.5`), from the ones the workflow lists. Once per
        /// step; a step left out runs with its default.
        #[arg(long="model",value_name="STEP=MODEL")] models:Vec<String>,
        /// The worktree the run makes and works in, which the app lists, from main as it is at Start. With --branch.
        #[arg(long,requires="branch")] worktree:Option<String>,
        /// The new branch for --worktree. One that exists, here or on a remote, is refused.
        #[arg(long,requires="worktree")] branch:Option<String>,
        #[command(flatten)] identity:Identity },
    /// Stream a running engine agent's terminal to stdout, read-only: its picture, then live bytes, until
    /// the agent's turn ends. Nothing is ever sent to the agent. Owner only.
    Watch { workspace:Uuid,item:String },
}
impl WorkflowCmd {
    pub fn action(self) -> Result<(Uuid, Action), Box<dyn std::error::Error>> {
        Ok(match self {
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
            Self::Queue { workspace,item,run,pin,identity } => (workspace,Action::Queue { mutation:identity.mutation(),item,run,pin }),
            Self::Leave { workspace,item,run,pin,identity } => (workspace,Action::Leave { mutation:identity.mutation(),item,run,pin }),
            Self::Settle { workspace,item,run,pin,landed,identity } => (workspace,Action::Settle { mutation:identity.mutation(),item,run,pin,landed }),
            Self::Track { workspace,item,column,identity } => (workspace,Action::Track { mutation:identity.mutation(),item,column }),
            Self::Move { workspace,item,to,run,pin,identity } => (workspace,Action::Move { mutation:identity.mutation(),item,run,pin,to }),
            Self::Start { workspace,item,start_pin,run,pin,models,worktree,branch,identity } => (workspace,Action::Start { mutation:identity.mutation(),item,run,pin,start_pin,models:picks(&models)?,
                worktree:worktree.zip(branch).map(|(name,branch)| farcooler_protocol::workflow_commands::NamedTree { name,branch }) }),
            Self::Retry { workspace,item,run,pin,identity } => (workspace,Action::Retry { mutation:identity.mutation(),item,run,pin }),
            Self::Watch { .. } => return Err("watch is a stream, not a command".into()),
        })
    }
}
/// `<step>=<model>` pairs, each step once.
fn picks(given:&[String]) -> Result<std::collections::BTreeMap<String,String>,Box<dyn std::error::Error>> {
    let mut picks=std::collections::BTreeMap::new();
    for pair in given {
        let Some((step,model))=pair.split_once('=').filter(|(s,m)| !s.is_empty() && !m.is_empty()) else {
            return Err(format!("--model takes <step>=<model>, such as build=codex:gpt-5.5, not {pair}.").into());
        };
        if picks.insert(step.to_string(),model.to_string()).is_some() { return Err(format!("--model names the {step} step twice. Pick one model for it.").into()); }
    }
    Ok(picks)
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
    farcooler_protocol::workflow_commands::parse(&json)?;
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
/// Any workflow command's reply, unprinted.
pub async fn ask(link:&mut crate::Link, workspace:Uuid, action:&Action) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if !link.daemon_capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) { return Err("runner lacks workflow commands".into()); }
    let reply = link.call(request(workspace,action)?).await?;
    let result::Value::WorkflowReply(reply) = expect_value(reply.value)? else { return Err("unexpected workflow reply".into()) };
    Ok(serde_json::from_str(&reply.json)?)
}
/// The runner's `workflow get` reply, unprinted, for the commands that draft from it.
pub async fn get(link:&mut crate::Link, workspace:Uuid) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if !link.daemon_capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) { return Err("runner lacks workflow commands".into()); }
    let reply = link.call(request(workspace,&Action::Get)?).await?;
    let result::Value::WorkflowReply(reply) = expect_value(reply.value)? else { return Err("unexpected workflow reply".into()) };
    Ok(serde_json::from_str(&reply.json)?)
}
/// The `workflow.watch` request: the item, and nothing a viewer could type or size.
pub fn watch_request(workspace:Uuid, item:&str) -> pb::Request {
    let json = serde_json::json!({ "item": item }).to_string();
    let mut r = with(req_for("workflow.watch",workspace),request::Payload::WorkflowCommand(pb::WorkflowCommand { schema:1,json }));
    r.required_capabilities.push(capability::WORKFLOW_COMMANDS.into());
    r
}
/// Print the agent's terminal bytes as they arrive. Writes only to stdout; the link is never written to
/// again after the one request.
async fn watch(runner:Option<&str>, workspace:Uuid, item:String) -> Fallible {
    use std::io::Write;
    let mut link = crate::connect_to(runner).await?;
    if !link.daemon_capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) { return Err("runner lacks workflow commands".into()); }
    let reply = link.call(watch_request(workspace,&item)).await?;
    let result::Value::TerminalAttach(_) = expect_value(reply.value)? else { return Err("unexpected watch reply".into()) };
    let mut out = std::io::stdout().lock();
    loop {
        let event = link.next_event().await?;
        let Some(pb::event::Payload::TerminalFrame(frame)) = event.payload else { continue };
        let Some(pb::terminal_frame::Kind::Output(output)) = frame.kind else { continue };
        if out.write_all(&output.payload).is_err() || out.flush().is_err() { return Ok(()); }
    }
}
pub async fn workflow(runner:Option<&str>,cmd:WorkflowCmd,json:bool) -> Fallible {
    if let WorkflowCmd::Watch { workspace,item } = cmd { return watch(runner,workspace,item).await; }
    let (workspace,action) = cmd.action()?;
    let mut link = crate::connect_to(runner).await?;
    send(&mut link,workspace,action,json).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn workflow_removed_artifact_syntax_fails_at_parser() {
        assert!(crate::Cli::try_parse_from(["farcooler","workflow","retry","--prepared-file","absent"]).is_err());
        let args=["farcooler","workflow","rework","00000000-0000-0000-0000-000000000000","a",
            "--run","run","--pin","pin","--evidence","absent","--reason","fix",
            "--command-id","same","--revision","4","--epoch","1","--sequence","2"];
        assert!(crate::Cli::try_parse_from(args).is_ok());
        assert!(crate::Cli::try_parse_from(args.into_iter().chain(["--prepared-file","absent"])).is_err());
        let help=crate::Cli::try_parse_from(["farcooler","workflow","rework","--help"]).err().unwrap().to_string();
        assert!(help.contains("same selected authenticated runner/principal"),"{help}");
        assert!(help.contains("evidence contents"),"{help}");
    }
    #[test]
    fn workflow_reasoned_cli_keeps_reason_and_identity() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("evidence.json");
        std::fs::write(&path,r#"{"candidate":"c","manifest":"m","receipt":"r","policy_pin":"p"}"#).unwrap();
        let workspace=Uuid::nil().to_string();let evidence=path.to_str().unwrap();
        for reason in ["  修复\n  "," ","bad\0"] {
            let args=["farcooler","workflow","rework",&workspace,"a","--run","run","--pin","pin","--evidence",evidence,"--reason",reason,
                "--command-id","stable","--revision","18446744073709551614","--epoch","18446744073709551615","--sequence","18446744073709551615"];
            let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(args).unwrap().command else {panic!()};
            let result=cmd.action();
            if farcooler_protocol::workflow_commands::valid_rework_reason(reason) {
                let (workspace,action)=result.unwrap();let raw=serde_json::to_string(&action).unwrap();
                assert_eq!(super::request(workspace,&action).unwrap().payload,super::request(workspace,&action).unwrap().payload);
                assert_eq!(farcooler_protocol::workflow_commands::parse(&raw).unwrap(),action);
                assert!(matches!(action,Action::Review{reason:Some(r),passed:false,mutation,..} if r==reason && mutation.revision==u64::MAX-1 && mutation.epoch==u64::MAX && mutation.sequence==u64::MAX));
            } else {assert!(result.is_err());}
        }
    }
    /// ov-967: `--worktree` and `--branch` name the worktree the run makes, together or not at all.
    #[test]
    fn workflow_start_names_its_worktree_and_branch_together() {
        let args=["farcooler","workflow","start","00000000-0000-0000-0000-000000000000","ov-7","--start-pin","p","--run","run","--pin","pin",
            "--command-id","same","--revision","5","--epoch","1","--sequence","2"];
        let parsed=|extra:&[&str]| crate::Cli::try_parse_from(args.iter().chain(extra));
        let crate::Command::Workflow(cmd)=parsed(&["--worktree","phones","--branch","e-liang/phones"]).unwrap().command else {panic!()};
        let Action::Start{worktree,..}=cmd.action().unwrap().1 else {panic!()};
        assert_eq!(worktree,Some(farcooler_protocol::workflow_commands::NamedTree{name:"phones".into(),branch:"e-liang/phones".into()}));
        assert!(parsed(&["--worktree","phones"]).is_err() && parsed(&["--branch","b"]).is_err());
    }
    /// ov-966: `--model <step>=<model>` picks a step's model at Start, once per step.
    #[test]
    fn workflow_start_takes_a_model_per_step() {
        let args=["farcooler","workflow","start","00000000-0000-0000-0000-000000000000","ov-7","--start-pin","p","--run","run","--pin","pin",
            "--command-id","same","--revision","5","--epoch","1","--sequence","2"];
        let action=|extra:&[&str]| { let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(args.iter().chain(extra)).unwrap().command else {panic!()}; cmd.action() };
        let Action::Start{models,..}=action(&["--model","build=codex:gpt-5.5"]).unwrap().1 else {panic!()};
        assert_eq!(models,[("build".to_string(),"codex:gpt-5.5".to_string())].into());
        let Action::Start{models,..}=action(&[]).unwrap().1 else {panic!()};
        assert!(models.is_empty());
        for bad in [&["--model","codex"][..],&["--model","build=a","--model","build=b"],&["--model","=codex"],&["--model","build="]] {
            assert!(action(bad).is_err(),"{bad:?}");
        }
    }
    #[test]
    fn workflow_settle_says_which_way_and_travels_as_its_own_method() {
        let args=["farcooler","workflow","settle","00000000-0000-0000-0000-000000000000","group-1","--run","run","--pin","pin",
            "--command-id","same","--revision","5","--epoch","1","--sequence","2"];
        assert!(crate::Cli::try_parse_from(args).is_err(),"landed or not must be said");
        let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(args.into_iter().chain(["--landed","true"])).unwrap().command else {panic!()};
        let (workspace,action)=cmd.action().unwrap();
        assert!(matches!(action,Action::Settle{landed:true,..}));
        let request=super::request(workspace,&action).unwrap();
        assert_eq!(request.method,"workflow.settle");
        assert_eq!(request.required_capabilities,[capability::WORKFLOW_COMMANDS]);
    }
    #[test]
    fn workflow_legacy_review_bytes_stay_unchanged() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("evidence.json");
        std::fs::write(&path,r#"{"candidate":"c","manifest":"m","receipt":"r","policy_pin":"p"}"#).unwrap();
        let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(["farcooler","workflow","review",
            "00000000-0000-0000-0000-000000000000","a","--run","run","--pin","pin",
            "--evidence",path.to_str().unwrap(),"--command-id","same","--revision","5","--epoch","1","--sequence","2"]).unwrap().command else {panic!()};
        let (workspace,action)=cmd.action().unwrap();
        let Some(request::Payload::WorkflowCommand(body))=super::request(workspace,&action).unwrap().payload else {panic!()};
        assert_eq!(body.json,r#"{"action":"Review","mutation":{"id":"same","revision":5,"epoch":1,"sequence":2},"item":"a","run":"run","pin":"pin","evidence":{"candidate":"c","manifest":"m","receipt":"r","policy_pin":"p"},"passed":false}"#);
    }
    #[test]
    #[ignore = "requires private ov-941 seeded receipt fixture"]
    fn workflow_cli_emit_receipt_fixture() {
        let root=std::path::PathBuf::from(std::env::var_os("FARCOOLER_OV941_FIXTURE").expect("private fixture root"));
        let observed:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("observed.json")).unwrap()).unwrap();
        let workspace=observed["workspace"].as_str().unwrap();
        let revision=observed["revision"].to_string();
        let epoch=observed["epoch"].to_string();
        let evidence=root.join("evidence.json");
        let reason="  请修复 <script>\n $(never execute)  ";
        let args=["farcooler","--runner","fixture-selected","workflow","rework",workspace,"a",
            "--run",observed["run"].as_str().unwrap(),"--pin",observed["pin"].as_str().unwrap(),
            "--evidence",evidence.to_str().unwrap(),"--reason",reason,"--command-id","cli-rework",
            "--revision",&revision,"--epoch",&epoch,"--sequence","2"];
        let mut bodies=Vec::new();
        for _ in 0..2 {
            let cli=crate::Cli::try_parse_from(args).unwrap();
            assert_eq!(cli.runner.as_deref(),Some("fixture-selected"));
            let crate::Command::Workflow(cmd)=cli.command else {panic!("workflow command")};
            let (workspace,action)=cmd.action().unwrap();
            let request=super::request(workspace,&action).unwrap();
            assert_eq!(request.method,"workflow.review");
            assert_eq!(request.target_resource_id.as_deref(),Some(workspace.as_bytes().as_slice()));
            assert_eq!(request.required_capabilities,[capability::WORKFLOW_COMMANDS]);
            let Some(request::Payload::WorkflowCommand(body))=request.payload else {panic!("workflow payload")};
            assert_eq!(body.schema,1);
            assert!(matches!(farcooler_protocol::workflow_commands::parse(&body.json).unwrap(),
                Action::Review{reason:Some(r),passed:false,..} if r==reason));
            bodies.push(body.json);
        }
        assert_eq!(bodies[0].as_bytes(),bodies[1].as_bytes());
        std::fs::write(root.join("cli-payload.json"),&bodies[0]).unwrap();
        std::fs::write(root.join("cli-resubmitted.json"),&bodies[1]).unwrap();
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
