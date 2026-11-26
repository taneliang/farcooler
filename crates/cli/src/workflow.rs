//! Explicit workflow commands over the ordinary authenticated daemon link.
use std::io::Read;
use clap::{Args, Subcommand};
use farcooler_protocol::{capability, workflow_commands::{Action, HistorySection, Mutation, MAX_BYTES}, v1::{self as pb, request, result}};
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
/// Optional bindings for routine verbs. Supply all of them to retain an inspected command.
#[derive(Args, Clone)]
pub struct RoutineIdentity {
    #[arg(long)] pub command_id: Option<String>,
    #[arg(long)] pub revision: Option<u64>,
    #[arg(long)] pub epoch: Option<u64>,
    #[arg(long)] pub sequence: Option<u64>,
}
impl RoutineIdentity {
    fn mutation(&self) -> Result<Mutation, Box<dyn std::error::Error>> {
        Ok(Mutation { id:self.command_id.clone().ok_or("missing --command-id")?, revision:self.revision.ok_or("missing --revision")?,
            epoch:self.epoch.ok_or("missing --epoch")?, sequence:self.sequence.ok_or("missing --sequence")? })
    }
    fn explicit(&self) -> bool { self.command_id.is_some() || self.revision.is_some() || self.epoch.is_some() || self.sequence.is_some() }
}
#[derive(Args)]
pub struct Publication {
    /// Why this workflow changes. Required for an orchestrator publication.
    #[arg(long)] pub reason: Option<String>,
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
        Ok(Action::Publish { reason:self.reason.clone(), notifications, adapter, mutation:self.identity.mutation(),expected_pin:self.expected_pin.clone(),definition })
    }
}
#[derive(Subcommand, Clone)]
pub enum WorkflowCmd {
    /// Read current state and history previews. --full includes definitions and recent notices.
    ///
    /// Pipe --json stdout directly to a parser. If you store it in a shell variable,
    /// use printf '%s\n' "$value" to pass it on. Shell echo can interpret JSON escapes.
    Get { workspace: Uuid, #[arg(long)] full: bool },
    /// Export the published definition and adapter as JSON for workspace workflow-draft --file.
    Bundle { workspace: Uuid },
    /// Read one ticket's recent process logs across runs. Use `history logs --item` for all retained entries.
    Logs { workspace: Uuid, item: String },
    /// Read a page of retained history. Concatenate chunk strings using next_offset, then parse the JSON.
    History {
        workspace: Uuid,
        #[arg(value_parser = ["items", "groups", "archive", "cleanup", "publications", "notices", "logs", "runs", "source_sequences"])]
        section: String,
        #[arg(long, default_value_t = 0)] offset: u64,
        /// Filter items, groups, archive, logs or runs to this item.
        #[arg(long)] item: Option<String>,
    },
    /// Restore the previous workflow bundle, retaining a new audit entry. Owner in person only.
    Rollback { workspace: Uuid, #[arg(long, default_value="Restore previous workflow")] reason:String },
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
    Cancel { workspace:Uuid,item:String,#[arg(long)] run:Option<String>,#[arg(long)] pin:Option<String>,#[command(flatten)] identity:RoutineIdentity },
    /// Close an Uncertain cancellation after you have checked that its processes have ended and its external
    /// effects are accounted for. Owner in person only. --reason records what was lost and what you checked.
    /// Then reopen the ticket to run it again. A push must be settled with `settle` instead.
    ResolveCancellation { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,
        #[arg(long)] reason:String,#[command(flatten)] identity:Identity },
    /// Close work completed outside this run. --sha must be a full commit on the landing target;
    /// --continues-under names another open ticket in this workspace. Any active operation is stopped before the card closes.
    Close { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,
        #[arg(long,required_unless_present="continues_under",conflicts_with="continues_under")] sha:Option<String>,
        #[arg(long,required_unless_present="sha")] continues_under:Option<String>,#[command(flatten)] identity:Identity },
    /// Retry a failed step within its published retry allowance. After the review send-back limit,
    /// rebuild in the existing checkout with the last finding and a fresh allowance at that gate.
    Retry { workspace:Uuid,item:String,#[arg(long)] run:Option<String>,#[arg(long)] pin:Option<String>,#[command(flatten)] identity:RoutineIdentity },
    /// Settle an uncertain build or worktree step, or a build waiting for detached processes. For a native build,
    /// a saved result can settle the step once its hold confirms the agent exited and its process group is empty.
    /// For a worktree step, the runner checks for a directory, registered worktree or branch before allowing a retry.
    Resolve { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[command(flatten)] identity:Identity },
    /// Queue a ticket waiting at its join step into the open group for that step.
    Queue { workspace:Uuid,item:String,#[arg(long)] run:Option<String>,#[arg(long)] pin:Option<String>,#[command(flatten)] identity:RoutineIdentity },
    /// Take a queued ticket out of a group that has not left yet.
    Leave { workspace:Uuid,item:String,#[arg(long)] run:Option<String>,#[arg(long)] pin:Option<String>,#[command(flatten)] identity:RoutineIdentity },
    /// Settle a push whose outcome was lost, after looking at the target yourself. The runner reads the
    /// target first and refuses a word it contradicts. Pass `--landed true` if the commit is on the target.
    Settle { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[arg(long, action = clap::ArgAction::Set, required = true)] landed:bool,#[command(flatten)] identity:Identity },
    /// Bring a ticket under the engine with no run: it sits in Backlog until the orchestrator moves it.
    Track { workspace:Uuid,item:String,#[arg(long)] column:Option<String>,#[command(flatten)] identity:RoutineIdentity },
    /// Move a tracked ticket to a column: backlog, todo, doing, review or decision; done or cancelled close it.
    Move { workspace:Uuid,item:String,#[arg(long)] to:String,#[arg(long)] run:String,#[arg(long)] pin:String,#[command(flatten)] identity:Identity },
    /// Start a run on a tracked ticket, or replace a cancelled or failed run under its key.
    Start { workspace:Uuid,item:String,#[arg(long)] start_pin:Option<String>,#[arg(long)] run:Option<String>,#[arg(long)] pin:Option<String>,
        /// A build step's model, as `<step>=<model>` (`build=codex:gpt-5.5`), from the ones the workflow lists. Once per
        /// step; a step left out runs with its default.
        #[arg(long="model",value_name="STEP=MODEL")] models:Vec<String>,
        /// The worktree the run makes and works in, which the app lists, from main as it is at Start. With --branch.
        #[arg(long,requires="branch")] worktree:Option<String>,
        /// The new branch for --worktree. One that exists, here or on a remote, is refused, unless --adopt.
        #[arg(long,requires="worktree")] branch:Option<String>,
        /// Carry on from --branch, which exists with work on it: in the worktree that already has it checked out,
        /// or in a new one named --worktree. The agent is told the work is there.
        #[arg(long,requires="branch")] adopt:bool,
        /// Another tracked ticket at rest that this run takes on as well: one agent, one branch and one pull request
        /// for all of them, and its card goes where the run goes. Repeat for more.
        #[arg(long="with",value_name="KEY")] with:Vec<String>,
        #[command(flatten)] identity:RoutineIdentity },
    /// Send a run back to its build step, which starts again in the same worktree. The agent is told --reason.
    /// Taken while the run waits for a review, an approval or its landing group, or has failed, or while its build is
    /// Uncertain once that build's agent has ended.
    SendBack { workspace:Uuid,item:String,#[arg(long)] run:Option<String>,#[arg(long)] pin:Option<String>,
        /// Why, for the agent: what to change. At most 4096 bytes; the first 1500 are in its brief.
        #[arg(long)] reason:String,#[command(flatten)] identity:RoutineIdentity },
    /// Close a failed run as landed, when its commit is already on the target branch. The runner reads the remote
    /// first and refuses a commit that is not on the target there. Taken only while the run is Failed.
    Landed { workspace:Uuid,item:String,#[arg(long)] run:String,#[arg(long)] pin:String,
        /// The commit, in full, that the run's work landed as.
        #[arg(long)] sha:String,#[command(flatten)] identity:Identity },
    /// Reopen a closed ticket: it comes back as a ticket at rest, in --column (todo when left out), under its own name.
    Reopen { workspace:Uuid,item:String,#[arg(long)] column:Option<String>,#[command(flatten)] identity:Identity },
    /// Stream a running engine agent's terminal to stdout, read-only: its picture, then live bytes, until
    /// the agent's turn ends. Nothing is ever sent to the agent. Owner only.
    Watch { workspace:Uuid,item:String },
    /// Suspend the current native agent step and its timeout, keeping the same run and session.
    Pause { workspace:Uuid,item:String },
    /// Continue a paused native agent step in the same session.
    Resume { workspace:Uuid,item:String },
    /// For an agent the engine started to review a ticket: record your verdict. Only this counts; what you
    /// say in your turn doesn't. Run it before you end your turn, or the review runs again. Writes the step's
    /// receipt locally and talks to no runner.
    Verdict {
        #[arg(long, action = clap::ArgAction::Set, required = true)]
        passed: bool,
        /// A brief finding to keep with the verdict. Required when --passed is false.
        #[arg(long)]
        summary: Option<String>,
    },
}
impl WorkflowCmd {
    pub fn action(self) -> Result<(Uuid, Action), Box<dyn std::error::Error>> {
        Ok(match self {
            Self::Rollback { .. } => return Err("rollback is prepared from current state".into()),
            Self::Get { workspace, .. } | Self::Bundle { workspace } => (workspace, Action::Get),
            Self::Logs { workspace, item } => (workspace,Action::Logs { item }),
            Self::History { workspace, section, offset, item } =>
                (workspace, Action::History { section: serde_json::from_value(section.into())?, offset, item }),
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
            Self::ResolveCancellation { workspace,item,run,pin,reason,identity } => {
                if !farcooler_protocol::workflow_commands::valid_rework_reason(&reason) {
                    return Err("The reason must record what was lost and what you checked, in at most 4096 bytes.".into());
                }
                (workspace,Action::ResolveCancellation { mutation:identity.mutation(),item,run,pin,reason })
            }
            Self::Close { workspace,item,run,pin,sha,continues_under,identity } => {
                let resolution = match (sha,continues_under) {
                    (Some(sha),None) => farcooler_protocol::workflow_commands::CloseResolution::Landed { sha },
                    (None,Some(item)) => farcooler_protocol::workflow_commands::CloseResolution::Continued { item },
                    _ => return Err("Say either --sha or --continues-under.".into()),
                };
                (workspace,Action::Close { mutation:identity.mutation(),item,run,pin,resolution })
            }
            Self::Cancel { workspace,item,run,pin,identity } => (workspace,Action::Cancel { mutation:identity.mutation()?,item,run:run.ok_or("missing --run")?,pin:pin.ok_or("missing --pin")? }),
            Self::Queue { workspace,item,run,pin,identity } => (workspace,Action::Queue { mutation:identity.mutation()?,item,run:run.ok_or("missing --run")?,pin:pin.ok_or("missing --pin")? }),
            Self::Leave { workspace,item,run,pin,identity } => (workspace,Action::Leave { mutation:identity.mutation()?,item,run:run.ok_or("missing --run")?,pin:pin.ok_or("missing --pin")? }),
            Self::Settle { workspace,item,run,pin,landed,identity } => (workspace,Action::Settle { mutation:identity.mutation(),item,run,pin,landed }),
            Self::Track { workspace,item,column,identity } => (workspace,Action::Track { mutation:identity.mutation()?,item,column }),
            Self::Move { workspace,item,to,run,pin,identity } => (workspace,Action::Move { mutation:identity.mutation(),item,run,pin,to }),
            Self::Start { workspace,item,start_pin,run,pin,models,worktree,branch,adopt,with,identity } => (workspace,Action::Start { mutation:identity.mutation()?,item,run:run.ok_or("missing --run")?,pin:pin.ok_or("missing --pin")?,start_pin:start_pin.ok_or("missing --start-pin")?,models:picks(&models)?,
                worktree:worktree.zip(branch).map(|(name,branch)| farcooler_protocol::workflow_commands::NamedTree { name,branch,adopt }),with }),
            Self::SendBack { workspace,item,run,pin,reason,identity } => {
                if !farcooler_protocol::workflow_commands::valid_rework_reason(&reason) {
                    return Err("The reason must say something, in at most 4096 bytes, with no control characters but tabs and line breaks.".into());
                }
                (workspace,Action::SendBack { mutation:identity.mutation()?,item,run:run.ok_or("missing --run")?,pin:pin.ok_or("missing --pin")?,reason })
            }
            Self::Landed { workspace,item,run,pin,sha,identity } => {
                if sha.len()!=40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("--sha must be the full 40-character commit hash.".into()); }
                (workspace,Action::Landed { mutation:identity.mutation(),item,run,pin,sha })
            }
            Self::Reopen { workspace,item,column,identity } => (workspace,Action::Reopen { mutation:identity.mutation(),item,column }),
            Self::Retry { workspace,item,run,pin,identity } => (workspace,Action::Retry { mutation:identity.mutation()?,item,run:run.ok_or("missing --run")?,pin:pin.ok_or("missing --pin")? }),
            Self::Pause { .. } | Self::Resume { .. } => return Err("steering is a live step command".into()),
            Self::Resolve { workspace,item,run,pin,identity } => (workspace,Action::Resolve { mutation:identity.mutation(),item,run,pin }),
            Self::Watch { .. } => return Err("watch is a stream, not a command".into()),
            Self::Verdict { .. } => return Err("a verdict is written where the review step reads it, not sent".into()),
        })
    }
}
type RoutineBindings<'a> = (Uuid, &'a str, Option<(&'a mut Option<String>, &'a mut Option<String>)>, Option<&'a mut Option<String>>, &'a mut RoutineIdentity);
impl WorkflowCmd {
    // None means this is a command whose bindings must remain explicitly inspected.
    fn routine(&mut self) -> Option<RoutineBindings<'_>> {
        match self {
            Self::Queue {workspace,item,run,pin,identity} | Self::Leave {workspace,item,run,pin,identity}
            | Self::Retry {workspace,item,run,pin,identity} | Self::Cancel {workspace,item,run,pin,identity}
            | Self::SendBack {workspace,item,run,pin,identity,..} => Some((*workspace,item,Some((run,pin)),None,identity)),
            Self::Start {workspace,item,run,pin,start_pin,identity,..} => Some((*workspace,item,Some((run,pin)),Some(start_pin),identity)),
            Self::Track {workspace,item,identity,..} => Some((*workspace,item,None,None,identity)),
            _ => None,
        }
    }
    fn prepare_routine(&mut self, snapshot:&serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
        let (_, item, bindings, start_pin, identity) = self.routine().ok_or("not a routine verb")?;
        let state = &snapshot["state"];
        let revision = if state.is_null() { 0 } else { state["revision"].as_u64().ok_or("missing workflow revision")? };
        let epoch = if state.is_null() { 1 } else { state["epoch"].as_u64().ok_or("missing workflow epoch")? };
        let sequences = &state["source_sequences"];
        let last_sequence = if let Some(principal) = snapshot.pointer("/context/capabilities/decision_principal").and_then(serde_json::Value::as_str) {
            sequences[format!("wire:{principal}")].as_u64()
        } else {
            sequences.as_object().and_then(|map| map.values().filter_map(serde_json::Value::as_u64).max())
        };
        let sequence = last_sequence.unwrap_or_default().checked_add(1).ok_or("workflow sequence exhausted")?;
        if let Some((run,pin)) = bindings {
            let current = state["items"].get(item)
                .or_else(|| start_pin.as_ref().and_then(|_| state["archive"].get(item))
                    .filter(|archived| archived["phase"] == "Cancelled" && archived["run"].is_string()))
                .ok_or("ticket is not tracked")?;
            *run = Some(current["run"].as_str().ok_or("missing ticket run")?.into());
            *pin = Some(current["pin"].as_str().ok_or("missing ticket pin")?.into());
        }
        if let Some(start_pin) = start_pin { *start_pin = Some(snapshot["configuration"]["pin"].as_str().ok_or("no workflow has been published")?.into()); }
        *identity = RoutineIdentity { command_id:Some(Uuid::now_v7().to_string()),revision:Some(revision),epoch:Some(epoch),sequence:Some(sequence) };
        Ok(())
    }
}
#[derive(Clone, Copy)]
enum RoutineTarget<'a> {
    Archive(&'a str),
    Group(&'a str),
}

trait RoutineTransport {
    async fn read(&mut self, workspace:Uuid, target:Option<RoutineTarget<'_>>) -> Result<serde_json::Value, Box<dyn std::error::Error>>;
    async fn write(&mut self, workspace:Uuid, action:&Action) -> Result<serde_json::Value, Box<dyn std::error::Error>>;
}
impl RoutineTransport for crate::Link {
    async fn read(&mut self, workspace:Uuid, target:Option<RoutineTarget<'_>>) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let mut snapshot = read_snapshot(self, workspace, false).await?;
        if let Some(RoutineTarget::Archive(key)) = target
            && snapshot["archive_omitted"].as_u64().is_some_and(|count| count > 0)
            && snapshot["state"]["items"].get(key).is_none()
            && snapshot["state"]["archive"].get(key).is_none()
        {
            let archive = read_deadline(ask(self, workspace, &Action::ArchiveItem { item: key.into() })).await??;
            if archive["revision"] != snapshot["state"]["revision"] || archive["epoch"] != snapshot["state"]["epoch"] {
                return Err("The workflow changed while reading the ticket. Try again.".into());
            }
            if archive["item"]["key"] == key { snapshot["state"]["archive"][key] = archive["item"].clone(); }
        }
        if let Some(RoutineTarget::Group(key)) = target
            && snapshot["landing_groups_omitted"].as_u64().is_some_and(|count| count > 0)
            && snapshot["state"]["items"].get(key).is_none()
        {
            // The mutation still checks the snapshot revision and the looked-up run and pin.
            let groups = read_deadline(group_history(self, workspace, key)).await??;
            if let Some(group) = groups.get(key) { snapshot["state"]["items"][key] = group.clone(); }
        }
        Ok(snapshot)
    }
    async fn write(&mut self, workspace:Uuid, action:&Action) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        ask(self, workspace, action).await
    }
}
async fn group_history(link:&mut crate::Link, workspace:Uuid, key:&str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut offset = 0;
    let mut digest = None;
    let mut encoded = String::new();
    loop {
        let page = ask(link, workspace, &Action::History {
            section: HistorySection::Groups, offset, item: Some(key.into()),
        }).await?;
        let page_digest = page["digest"].as_str().ok_or("missing history digest")?;
        if page["offset"].as_u64() != Some(offset) || digest.get_or_insert_with(|| page_digest.to_owned()) != page_digest {
            return Err("The workflow changed while reading the landing group. Try again.".into());
        }
        encoded.push_str(page["chunk"].as_str().ok_or("missing history chunk")?);
        if page["next_offset"].is_null() { return Ok(serde_json::from_str(&encoded)?); }
        offset = page["next_offset"].as_u64().filter(|&next| next > offset).ok_or("invalid history offset")?;
    }
}

async fn run_routine(transport:&mut impl RoutineTransport, workspace:Uuid, cmd:&WorkflowCmd) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let target = match cmd {
        WorkflowCmd::Start { item, .. } => Some(RoutineTarget::Archive(item)),
        WorkflowCmd::Cancel { item, .. } => Some(RoutineTarget::Group(item)),
        _ => None,
    };
    for attempt in 0..=3 {
        let snapshot = transport.read(workspace, target).await?;
        let mut prepared = cmd.clone();
        prepared.prepare_routine(&snapshot)?;
        let (_, action) = prepared.action()?;
        let value = transport.write(workspace, &action).await?;
        if value.pointer("/receipt/result/Err").and_then(|v|v.as_str()) != Some("Stale") || attempt == 3 { return Ok(value); }
    }
    unreachable!()
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
    if matches!(action, Action::Get) { r.accepts.push("workflow_summary_v1".into()); }
    Ok(r)
}
/// Read definitions for internal decisions and explicit `--full` inspection.
pub(crate) fn full_request(workspace:Uuid) -> Result<pb::Request, Box<dyn std::error::Error>> {
    let mut req = request(workspace,&Action::Get)?;
    req.accepts = vec!["workflow_full_v1".into()];
    Ok(req)
}
async fn read_deadline<T>(future:impl std::future::Future<Output=T>) -> Result<T, Box<dyn std::error::Error>> {
    tokio::time::timeout(std::time::Duration::from_secs(10), future).await
        .map_err(|_| "Workflow read timed out after 10 seconds. The runner may be busy; try workflow get again.".into())
}
pub async fn send(link:&mut crate::Link, workspace:Uuid, action:Action, json:bool) -> Fallible {
    if !link.daemon_capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) { return Err("runner lacks workflow commands".into()); }
    let value = if matches!(action, Action::Get) {
        read_snapshot(link, workspace, false).await?
    } else {
        let call = link.call(request(workspace,&action)?);
        let reply = if matches!(action, Action::Logs { .. } | Action::History { .. } | Action::ArchiveItem { .. }) { read_deadline(call).await?? } else { call.await? };
        let result::Value::WorkflowReply(reply) = expect_value(reply.value)? else { return Err("unexpected workflow reply".into()) };
        if reply.schema != 1 { return Err("unsupported workflow reply schema".into()); }
        serde_json::from_str(&reply.json)?
    };
    if json { println!("{}",value); } else { println!("{}",serde_json::to_string_pretty(&value)?); }
    if value.pointer("/receipt/result/Err").is_some() { return Err("workflow command refused; receipt shown above".into()); }
    Ok(())
}
/// Preview the exact publication before the CLI sends its write.
pub(crate) async fn publication_restart_count(link: &mut crate::Link, workspace: Uuid, action: &Action)
    -> Result<u64, Box<dyn std::error::Error>>
{
    let req = request(workspace, &Action::PreviewPublication { publication: Box::new(action.clone()) })?;
    let reply = read_deadline(link.call(req)).await??;
    let result::Value::WorkflowReply(reply) = expect_value(reply.value)? else { return Err("unexpected workflow preview".into()) };
    let value: serde_json::Value = serde_json::from_str(&reply.json)?;
    value["restarts"].as_u64().ok_or_else(|| "runner returned no publication restart count".into())
}
/// Any workflow command's reply, unprinted.
pub async fn ask(link:&mut crate::Link, workspace:Uuid, action:&Action) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if !link.daemon_capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) { return Err("runner lacks workflow commands".into()); }
    if matches!(action, Action::Get) { return read_snapshot(link, workspace, false).await; }
    let reply = link.call(request(workspace,action)?).await?;
    let result::Value::WorkflowReply(reply) = expect_value(reply.value)? else { return Err("unexpected workflow reply".into()) };
    Ok(serde_json::from_str(&reply.json)?)
}
/// Read the snapshot for either CLI projection, with one deadline across all pages.
pub(crate) async fn read_snapshot(link: &mut impl crate::tasks::DispatchLink, workspace: Uuid, full: bool)
    -> Result<serde_json::Value, Box<dyn std::error::Error>>
{
    use farcooler_client::session::SessionError;
    let view = if full { "workflow_full_v1" } else { "workflow_summary_v1" };
    let snapshot = read_deadline(farcooler_client::plan_json::read_workflow_view(workspace, Some(view), async |request| {
        let answer = link.call(request).await?;
        answer.value.ok_or_else(|| SessionError::Protocol(crate::daemon_link::UNREADABLE.into()))
    })).await??;
    if snapshot["source_sequences_omitted"].as_u64().unwrap_or_default() > 0 {
        return Err("The runner returned an incomplete workflow sequence map.".into());
    }
    Ok(snapshot)
}

/// The full `workflow get` reply, unprinted, for the commands that draft from it.
pub async fn get(link:&mut crate::Link, workspace:Uuid) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if !link.daemon_capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) { return Err("runner lacks workflow commands".into()); }
    read_snapshot(link, workspace, true).await
}

/// Draft commands can wait for the runner's background starter settings.
pub async fn get_for_draft(link:&mut crate::Link, workspace:Uuid) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let value = get(link, workspace).await?;
            if value["suggested_adapter_pending"] != true { return Ok(value); }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }).await.map_err(|_| "The runner is still reading local Git settings. Try the draft command again.")?
}
/// `get` for `workspace re-pin`: the published workflow only. A busy workspace's state and history can outgrow
/// the daemon's 1 MiB reply cap, and re-pinning reads none of it (ov-1373).
pub async fn get_published(link:&mut crate::Link, workspace:Uuid) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if !link.daemon_capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) { return Err("runner lacks workflow commands".into()); }
    let mut req = full_request(workspace)?;
    req.accepts.push("workflow_repin_v1".into());
    let reply = read_deadline(link.call(req)).await??;
    let result::Value::WorkflowReply(reply) = expect_value(reply.value)? else { return Err("unexpected workflow reply".into()) };
    Ok(serde_json::from_str(&reply.json)?)
}
/// Export the exact published bundle, including on runners that support the earlier re-pin projection.
async fn export_bundle(link: &mut crate::Link, workspace: Uuid, json: bool) -> Fallible {
    use farcooler_daemon_contracts::landing_templates::Bundle;
    if !link.daemon_capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) {
        return Err("runner lacks workflow commands".into());
    }
    let mut req = full_request(workspace)?;
    req.accepts.extend([farcooler_protocol::workflow_commands::BUNDLE_VIEW.into(), "workflow_repin_v1".into()]);
    let reply = read_deadline(link.call(req)).await??;
    let result::Value::WorkflowReply(reply) = expect_value(reply.value)? else { return Err("unexpected workflow reply".into()) };
    if reply.schema != 1 { return Err("unsupported workflow reply schema".into()); }
    let value: serde_json::Value = serde_json::from_str(&reply.json)?;
    let bundle: Bundle = if let Some(config) = value.get("configuration") {
        if config.is_null() { return Err("This workspace has no published workflow.".into()); }
        if config["adapter"].is_null() { return Err("This workspace's published workflow has no adapter to draft from.".into()); }
        serde_json::from_value(serde_json::json!({ "definition": config["definition"], "adapter": config["adapter"] }))?
    } else {
        serde_json::from_value(value)?
    };
    if json { println!("{}", serde_json::to_string(&bundle)?); }
    else { println!("{}", serde_json::to_string_pretty(&bundle)?); }
    Ok(())
}

/// The `workflow.watch` request: the item, and nothing a viewer could type or size.
pub fn watch_request(workspace:Uuid, item:&str) -> pb::Request {
    let json = serde_json::json!({ "item": item }).to_string();
    let mut r = with(req_for("workflow.watch",workspace),request::Payload::WorkflowCommand(pb::WorkflowCommand { schema:1,json }));
    r.required_capabilities.push(capability::WORKFLOW_COMMANDS.into());
    r
}
pub(crate) fn size_marker(columns:u32, rows:u32) -> Vec<u8> { format!("\x1bP>farcooler-size;{columns};{rows}\x1b\\").into_bytes() }
/// The runner's word that the watched pane was reaped (`GAP_REASON_RUNTIME_LOST`).
fn watch_ended(frame:&pb::TerminalFrame) -> bool {
    matches!(&frame.kind, Some(pb::terminal_frame::Kind::Gap(g)) if g.reason==pb::GapReason::RuntimeLost as i32)
}
/// Print the agent's terminal bytes as they arrive. Writes only to stdout; the link is never written to
/// again after the one request.
async fn watch(runner:Option<&str>, workspace:Uuid, item:String) -> Fallible {
    use std::io::Write;
    let mut link = crate::connect_to(runner).await?;
    if !link.daemon_capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) { return Err("runner lacks workflow commands".into()); }
    let reply = link.call(watch_request(workspace,&item)).await?;
    let result::Value::TerminalAttach(attached) = expect_value(reply.value)? else { return Err("unexpected watch reply".into()) };
    let mut out = std::io::stdout().lock();
    // The agent's pane size, first, as the runner's own streams announce it (`farcooler_vt::size_marker`),
    // so a viewer sizes its screen to the pane before the picture arrives and does not ask for a resize.
    if out.write_all(&size_marker(attached.columns, attached.rows)).is_err() || out.flush().is_err() { return Ok(()); }
    loop {
        let event = link.next_event().await?;
        let Some(pb::event::Payload::TerminalFrame(frame)) = event.payload else { continue };
        if watch_ended(&frame) { return Ok(()); }
        let Some(pb::terminal_frame::Kind::Output(output)) = frame.kind else { continue };
        if out.write_all(&output.payload).is_err() || out.flush().is_err() { return Ok(()); }
    }
}
pub async fn workflow(runner:Option<&str>,cmd:WorkflowCmd,json:bool) -> Fallible {
    if let WorkflowCmd::Pause { workspace,item } | WorkflowCmd::Resume { workspace,item } = &cmd {
        let method = if matches!(cmd, WorkflowCmd::Pause { .. }) { "workflow.pause" } else { "workflow.resume" };
        let mut link = crate::connect_to(runner).await?;
        let mut request = watch_request(*workspace, item);
        request.method = method.into();
        let reply = expect_value(link.call(request).await?.value)?;
        let result::Value::WorkflowReply(reply) = reply else { return Err("unexpected steering reply".into()) };
        println!("{}", reply.json);
        return Ok(());
    }
    if let WorkflowCmd::Watch { workspace,item } = cmd { return watch(runner,workspace,item).await; }
    if let WorkflowCmd::Verdict { passed, summary } = cmd {
        farcooler_daemon::workflow_runtime::write_agent_verdict_with_summary(|key| std::env::var(key).ok(), passed, summary)
            .map_err(|error| match error {
                farcooler_daemon::workflow_runtime::Error::Refused(_) => "This only works inside an agent the engine started to review a ticket.".to_string(),
                error => format!("Couldn't record the verdict: {error}"),
            })?;
        println!("{}", if passed { "Recorded: passed." } else { "Recorded: not passed." });
        return Ok(());
    }
    if let WorkflowCmd::Bundle { workspace } = cmd {
        let mut link = crate::connect_to(runner).await?;
        return export_bundle(&mut link, workspace, json).await;
    }
    let full = matches!(&cmd, WorkflowCmd::Get { full:true, .. });
    if let WorkflowCmd::Rollback { workspace, reason } = cmd {
        let mut link = crate::connect_to(runner).await?;
        return publication_change(&mut link,workspace,None,reason,json).await;
    }
    let mut cmd = cmd;
    if let Some((workspace, _, bindings, start_pin, identity)) = cmd.routine() {
        let explicit = identity.explicit() || bindings.as_ref().is_some_and(|(r,p)| r.is_some() || p.is_some()) || start_pin.as_ref().is_some_and(|p| p.is_some());
        if !explicit {
            let mut link = crate::connect_to(runner).await?;
            let value = run_routine(&mut link, workspace, &cmd).await?;
            if json { println!("{value}"); } else { println!("{}",serde_json::to_string_pretty(&value)?); }
            if value.pointer("/receipt/result/Err").is_some() { return Err("workflow command refused; receipt shown above".into()); }
            return Ok(());
        }
    }
    let (workspace,action) = cmd.action()?;
    let mut link = crate::connect_to(runner).await?;
    if full {
        let value = get(&mut link,workspace).await?;
        if json { println!("{value}"); } else { println!("{}",serde_json::to_string_pretty(&value)?); }
        return Ok(());
    }
    send(&mut link,workspace,action,json).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[tokio::test(start_paused = true)]
    async fn workflow_get_deadline_explains_the_timeout() {
        let result = tokio::time::timeout(std::time::Duration::from_secs(11),
            read_deadline(std::future::pending::<()>())).await;
        assert!(result.is_ok(), "a read must have its own deadline");
        assert!(result.unwrap().unwrap_err().to_string().contains("timed out after 10 seconds"));
    }
    #[test]
    fn workflow_history_selects_a_section_and_offset() {
        for (name, section, item) in [
            ("archive", HistorySection::Archive, Some("ov-9")),
            ("source_sequences", HistorySection::SourceSequences, None),
        ] {
            let mut args = vec!["farcooler", "workflow", "history",
                "00000000-0000-0000-0000-000000000000", name, "--offset", "12", "--json"];
            if let Some(item) = item { args.extend(["--item", item]); }
            let cli = crate::Cli::try_parse_from(args).unwrap();
            let crate::Command::Workflow(command) = cli.command else { panic!("expected workflow command") };
            let (workspace, action) = command.action().unwrap();
            assert_eq!(workspace, Uuid::nil());
            assert_eq!(action, Action::History { section, offset: 12, item: item.map(String::from) });
            assert_eq!(request(workspace, &action).unwrap().method, "workflow.get");
        }
    }

    #[test]
    fn workflow_get_accepts_full() {
        let args = ["farcooler", "workflow", "get", "00000000-0000-0000-0000-000000000000", "--full", "--json"];
        assert!(crate::Cli::try_parse_from(args).is_ok());
    }
    #[test]
    fn workflow_logs_reads_one_items_tails_under_workflow_get() {
        // ov-1298: `get` carries live tails only; an archived ticket's are read by name.
        let cli = crate::Cli::try_parse_from(["farcooler", "workflow", "logs", "00000000-0000-0000-0000-000000000000", "ov-9"]).unwrap();
        let crate::Command::Workflow(cmd) = cli.command else { panic!("not a workflow command") };
        let (_, action) = cmd.action().unwrap();
        assert_eq!(action, Action::Logs { item: "ov-9".into() });
        let r = request(Uuid::nil(), &action).unwrap();
        assert_eq!(r.method, "workflow.get");
        let Some(request::Payload::WorkflowCommand(p)) = r.payload else { panic!() };
        assert_eq!(p.json, r#"{"action":"Logs","item":"ov-9"}"#);
        assert_eq!(farcooler_protocol::workflow_commands::parse(&p.json).unwrap(), action);
    }
    #[test]
    fn watch_asks_for_the_item_only_and_announces_the_panes_size() {
        let r = watch_request(Uuid::nil(), "ov-9");
        assert_eq!(r.method, "workflow.watch");
        let Some(request::Payload::WorkflowCommand(p)) = r.payload else { panic!() };
        assert_eq!(p.json, r#"{"item":"ov-9"}"#);
        assert_eq!(size_marker(120, 40), b"\x1bP>farcooler-size;120;40\x1b\\".to_vec(), "the viewer reads this as the runner's own marker");
    }
    #[test]
    fn a_watch_ends_when_the_runner_says_the_pane_is_gone() {
        let gap = |reason:pb::GapReason| pb::TerminalFrame { kind: Some(pb::terminal_frame::Kind::Gap(pb::Gap { reason: reason as i32, ..Default::default() })), ..Default::default() };
        assert!(watch_ended(&gap(pb::GapReason::RuntimeLost)));
        assert!(!watch_ended(&gap(pb::GapReason::ClientTooSlow)), "a slow client is resynced, not finished");
        assert!(!watch_ended(&pb::TerminalFrame::default()));
    }
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
        assert_eq!(worktree,Some(farcooler_protocol::workflow_commands::NamedTree{name:"phones".into(),branch:"e-liang/phones".into(),adopt:false}));
        assert!(parsed(&["--worktree","phones"]).is_err() && parsed(&["--branch","b"]).is_err());
        // ov-968: --adopt carries on from the branch, which must be named.
        let crate::Command::Workflow(cmd)=parsed(&["--worktree","phones","--branch","e-liang/phones","--adopt"]).unwrap().command else {panic!()};
        let (workspace,action)=cmd.action().unwrap();
        let Some(request::Payload::WorkflowCommand(body))=super::request(workspace,&action).unwrap().payload else {panic!()};
        assert!(body.json.ends_with(r#""worktree":{"name":"phones","branch":"e-liang/phones","adopt":true}}"#),"{}",body.json);
        assert!(parsed(&["--adopt"]).is_err());
    }
    #[test]
    fn workflow_resolve_cancellation_requires_a_checked_reason() {
        let args=["farcooler","workflow","resolve-cancellation","00000000-0000-0000-0000-000000000000","ov-7",
            "--run","cancel:run","--pin","pin","--command-id","same","--revision","5","--epoch","1","--sequence","2"];
        let action=|reason:&str| {
            let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(args.iter().copied().chain(["--reason",reason])).unwrap().command else {panic!()};
            cmd.action()
        };
        let (workspace,sent)=action("The review receipt was lost. Its processes have ended.").unwrap();
        assert_eq!(super::request(workspace,&sent).unwrap().method,"workflow.resolve_cancellation");
        for bad in [" ","bell\u{7}"] { assert!(action(bad).is_err()); }
        assert!(crate::Cli::try_parse_from(args).is_err());
    }

    /// ov-968: send a run back with a reason the agent is told, and reopen a closed ticket, each its own method.
    #[test]
    fn workflow_send_back_and_reopen_travel_as_their_own_methods() {
        let ids=["--command-id","same","--revision","5","--epoch","1","--sequence","2"];
        let action=|args:&[&str]| { let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(["farcooler","workflow"].iter().chain(args).chain(&ids)).unwrap().command else {panic!()}; cmd.action() };
        let back=["send-back","00000000-0000-0000-0000-000000000000","ov-7","--run","run","--pin","pin","--reason"];
        let (workspace,sent)=action(&[&back[..],&["Bound the retry loop."]].concat()).unwrap();
        let request=super::request(workspace,&sent).unwrap();
        assert_eq!(request.method,"workflow.send_back");
        let Some(request::Payload::WorkflowCommand(body))=request.payload else {panic!()};
        assert_eq!(body.json,r#"{"action":"SendBack","mutation":{"id":"same","revision":5,"epoch":1,"sequence":2},"item":"ov-7","run":"run","pin":"pin","reason":"Bound the retry loop."}"#);
        for bad in [" ","bell\u{7}"] { assert!(action(&[&back[..],&[bad]].concat()).is_err(),"{bad:?}"); }
        let (workspace,reopen)=action(&["reopen","00000000-0000-0000-0000-000000000000","ov-7","--column","doing"]).unwrap();
        assert!(matches!(&reopen,Action::Reopen{item,column:Some(c),..} if item=="ov-7" && c=="doing"));
        assert_eq!(super::request(workspace,&reopen).unwrap().method,"workflow.reopen");
        let (_,reopen)=action(&["reopen","00000000-0000-0000-0000-000000000000","ov-7"]).unwrap();
        assert!(matches!(reopen,Action::Reopen{column:None,..}),"the runner picks the column");
    }
    /// ov-1231: `workflow landed` closes a failed run with a full commit hash, as its own method.
    #[test]
    fn workflow_landed_travels_as_its_own_method_and_wants_a_full_hash() {
        let ids=["--command-id","same","--revision","5","--epoch","1","--sequence","2"];
        let action=|args:&[&str]| { let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(["farcooler","workflow"].iter().chain(args).chain(&ids)).unwrap().command else {panic!()}; cmd.action() };
        let sha="73974f89898d716c36595eade2511ba5f8df7a8f";
        let landed=["landed","00000000-0000-0000-0000-000000000000","ov-7","--run","run","--pin","pin","--sha"];
        let (workspace,sent)=action(&[&landed[..],&[sha]].concat()).unwrap();
        let request=super::request(workspace,&sent).unwrap();
        assert_eq!(request.method,"workflow.landed");
        let Some(request::Payload::WorkflowCommand(body))=request.payload else {panic!()};
        assert_eq!(body.json,format!(r#"{{"action":"Landed","mutation":{{"id":"same","revision":5,"epoch":1,"sequence":2}},"item":"ov-7","run":"run","pin":"pin","sha":"{sha}"}}"#));
        for bad in ["73974f8","not-a-hash-not-a-hash-not-a-hash-not-a-hash"] { assert!(action(&[&landed[..],&[bad]].concat()).is_err(),"{bad:?}"); }
    }
    /// ov-966: `--model <step>=<model>` picks a step's model at Start, once per step.
    #[test]
    fn workflow_start_takes_a_model_per_step() {
        let args=["farcooler","workflow","start","00000000-0000-0000-0000-000000000000","ov-7","--start-pin","p","--run","run","--pin","pin",
            "--command-id","same","--revision","5","--epoch","1","--sequence","2"];
        let action=|extra:&[&str]| { let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(args.iter().chain(extra)).unwrap().command else {panic!()}; cmd.action() };
        for model in ["codex:gpt-5.5", "codex:gpt-6.1-sol", "codex:gpt-6.2-sol"] {
            let pick = format!("build={model}");
            let Action::Start{models,..}=action(&["--model",&pick]).unwrap().1 else {panic!()};
            assert_eq!(models,[("build".to_string(),model.to_string())].into());
        }
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
    fn workflow_resolve_travels_as_its_own_method() {
        let args=["farcooler","workflow","resolve","00000000-0000-0000-0000-000000000000","ov-1","--run","run","--pin","pin",
            "--command-id","same","--revision","5","--epoch","1","--sequence","2"];
        let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(args).unwrap().command else {panic!()};
        let (workspace,action)=cmd.action().unwrap();
        assert!(matches!(&action,Action::Resolve{item,..} if item=="ov-1"));
        let request=super::request(workspace,&action).unwrap();
        assert_eq!(request.method,"workflow.resolve");
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
        let publication=Publication { reason:None, file:path.clone(),expected_pin:None,identity:Identity {command_id:"publish".into(),revision:0,epoch:1,sequence:1} };
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
        let publication=Publication { reason:None, file:path.clone(),expected_pin:None,identity:Identity {command_id:"publish".into(),revision:0,epoch:1,sequence:1} };
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

#[cfg(test)]
mod close_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn workflow_close_requires_one_resolution_and_preserves_the_run_identity() {
        let args = ["farcooler", "workflow", "close", "00000000-0000-0000-0000-000000000000", "ov-1",
            "--run", "run", "--pin", "pin", "--command-id", "external-close", "--revision", "4", "--epoch", "1", "--sequence", "5"];
        assert!(crate::Cli::try_parse_from(args).is_err());
        assert!(crate::Cli::try_parse_from(args.into_iter().chain(["--sha", &"a".repeat(40), "--continues-under", "ov-2"])).is_err());
        for flags in [["--sha", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"], ["--continues-under", "ov-2"]] {
            let cli = crate::Cli::try_parse_from(args.into_iter().chain(flags)).unwrap();
            let crate::Command::Workflow(command) = cli.command else { panic!("workflow command") };
            let (workspace, action) = command.action().unwrap();
            assert!(matches!(&action, Action::Close { mutation, item, run, pin, .. }
                if mutation.id == "external-close" && mutation.revision == 4 && item == "ov-1" && run == "run" && pin == "pin"));
            let req = request(workspace, &action).unwrap();
            assert_eq!(req.method, "workflow.close");
        }
    }
}

/// Prepare a workspace-level change from an inspected state, with a unique retry identity.
pub(crate) async fn publication_change(link:&mut crate::Link, workspace:Uuid, enabled:Option<bool>, reason:String, json:bool) -> crate::Fallible {
    let current = ask(link, workspace, &Action::Get).await?;
    let mutation = Mutation { id:Uuid::now_v7().to_string(), revision:current.pointer("/state/revision").and_then(|v|v.as_u64()).unwrap_or(0),
        epoch:current.pointer("/state/epoch").and_then(|v|v.as_u64()).unwrap_or(1), sequence:1 };
    let action = match enabled {
        Some(enabled) => Action::PublicationGrant { mutation,enabled },
        None => Action::Rollback { mutation,reason, expected_pin:current.pointer("/configuration/pin").and_then(|v|v.as_str()).ok_or("no workflow has been published")?.into() },
    };
    send(link,workspace,action,json).await
}

#[cfg(test)]
mod ov1179_cli_tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn ov1179_cli_grant_and_single_command_rollback() {
        let parsed=crate::Cli::try_parse_from(["farcooler","workspace","set","Main","--orchestrator-publishes-workflow","on"]).unwrap();
        let crate::Command::Workspace(cmd)=parsed.command else {panic!()};
        cmd.check().unwrap();
        assert!(matches!(cmd,crate::workspaces::WorkspaceCmd::Set {orchestrator_publishes_workflow:Some(crate::workspaces::OnOff::On),..}));
        let parsed=crate::Cli::try_parse_from(["farcooler","workspace","set","Main","--orchestrator-publishes-workflow","off","--landing","direct"]).unwrap();
        let crate::Command::Workspace(cmd)=parsed.command else {panic!()};
        assert!(cmd.check().is_err(),"never silently drop a grant or combine partial settings writes");
        let parsed=crate::Cli::try_parse_from(["farcooler","workflow","rollback","00000000-0000-0000-0000-000000000001"]).unwrap();
        assert!(matches!(parsed.command,crate::Command::Workflow(WorkflowCmd::Rollback {..})));
    }
}

#[cfg(test)]
mod routine_tests {
    use super::*;
    use clap::Parser;
    #[derive(Default)]
    struct Racing { actions: Vec<Action>, stale: usize }
    impl RoutineTransport for Racing {
        async fn read(&mut self, _:Uuid, _:Option<RoutineTarget<'_>>) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
            Ok(serde_json::json!({
                "state":{"revision":self.actions.len(),"epoch":1,"source_sequences":{"wire:owner":4,"wire:member:alice":u64::MAX},
                    "items":{"ov-1":{"run":"r","pin":"p"}}},
                "context":{"capabilities":{"decision_principal":"owner"}},
                "configuration":{"pin":"current"}
            }))
        }
        async fn write(&mut self, _:Uuid, action:&Action) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
            self.actions.push(action.clone());
            Ok(if self.actions.len() <= self.stale { serde_json::json!({"receipt":{"result":{"Err":"Stale"}}}) } else { serde_json::json!({"receipt":{"result":{"Ok":null}}}) })
        }
    }
    #[tokio::test]
    async fn queue_resolves_and_retries_stale_at_most_three_times() {
        let ws="00000000-0000-0000-0000-000000000001";
        let crate::Command::Workflow(cmd)=crate::Cli::try_parse_from(["farcooler","workflow","queue",ws,"ov-1"]).unwrap().command else { panic!() };
        for stale in [2, 5] {
            let mut racing=Racing { stale, ..Default::default() };
            let reply=run_routine(&mut racing,Uuid::parse_str(ws).unwrap(),&cmd).await.unwrap();
            assert_eq!(racing.actions.len(), if stale==2 {3} else {4});
            assert_eq!(reply.pointer("/receipt/result/Err").is_some(),stale==5);
            let mut ids=std::collections::HashSet::new();
            for (n,action) in racing.actions.iter().enumerate() {
                let Action::Queue { mutation, item, run, pin }=action else { panic!() };
                assert_eq!((item.as_str(),run.as_str(),pin.as_str()),("ov-1","r","p"));
                assert_eq!((mutation.revision,mutation.epoch,mutation.sequence),(n as u64,1,5));
                assert!(ids.insert(&mutation.id));
            }
        }
        for verb in ["approve","review"] { assert!(crate::Cli::try_parse_from(["farcooler","workflow",verb,ws,"ov-1"]).is_err()); }
    }
}
