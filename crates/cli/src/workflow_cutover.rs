//! `farcooler workflow-cutover`: the owner's explicit engine authority cutover
//! steps over the ordinary authenticated link. Nothing here asserts quiescence.
use clap::Subcommand;
use farcooler_protocol::{capability, workflow_cutover::{Cutover, METHOD}, v1::{self as pb, request, result}};

type Fallible = Result<(), Box<dyn std::error::Error>>;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum CutoverCmd {
    /// Read-only: phase, incarnation, schema and whether artifacts exist.
    Inspect,
    /// Join the daemon's writers, capture the baseline and upgrade-and-freeze. Restart the daemon after.
    Prepare { #[arg(long)] attempt: String },
    /// In the recovery daemon: re-check a committed transfer against its original baseline.
    Verify,
    /// In the recovery daemon: release a verified transfer.
    Release,
    /// In the recovery daemon: record the terminal disposition of a released attempt.
    Finish,
}
impl CutoverCmd {
    pub fn op(self) -> Cutover {
        match self {
            Self::Inspect => Cutover::Inspect,
            Self::Prepare { attempt } => Cutover::Prepare { attempt },
            Self::Verify => Cutover::Verify,
            Self::Release => Cutover::Release,
            Self::Finish => Cutover::Finish,
        }
    }
}
pub fn request(op: &Cutover) -> Result<pb::Request, Box<dyn std::error::Error>> {
    let json = serde_json::to_string(op)?;
    farcooler_protocol::workflow_cutover::parse(&json)?;
    let mut r = crate::daemon_link::with(crate::daemon_link::req(METHOD), request::Payload::WorkflowCommand(pb::WorkflowCommand { schema: 1, json }));
    r.required_capabilities.push(capability::WORKFLOW_COMMANDS.into());
    Ok(r)
}
pub async fn run(runner: Option<&str>, cmd: CutoverCmd, json: bool) -> Fallible {
    let mut link = crate::daemon_link::connect_to(runner).await?;
    let reply = link.call(request(&cmd.op())?).await?;
    let result::Value::WorkflowReply(reply) = crate::daemon_link::expect_value(reply.value)? else { return Err("unexpected cutover reply".into()) };
    let value: serde_json::Value = serde_json::from_str(&reply.json)?;
    if json { println!("{value}"); } else { println!("{}", serde_json::to_string_pretty(&value)?); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn cutover_cli_produces_only_the_closed_vocabulary() {
        let cli = crate::Cli::try_parse_from(["farcooler", "workflow-cutover", "prepare", "--attempt", "a-1"]).unwrap();
        let crate::Command::WorkflowCutover(cmd) = cli.command else { panic!("cutover command") };
        let r = request(&cmd.op()).unwrap();
        assert_eq!(r.method, "workflow.cutover");
        let Some(request::Payload::WorkflowCommand(body)) = r.payload else { panic!("payload") };
        assert_eq!(farcooler_protocol::workflow_cutover::parse(&body.json).unwrap(), Cutover::Prepare { attempt: "a-1".into() });
        assert!(crate::Cli::try_parse_from(["farcooler", "workflow-cutover", "commit"]).is_err(), "commit is not offered");
        assert!(crate::Cli::try_parse_from(["farcooler", "workflow-cutover", "prepare"]).is_err(), "attempt is required");
    }
}
