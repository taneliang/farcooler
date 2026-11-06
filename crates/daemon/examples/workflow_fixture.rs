//! Explicit isolated smoke only. Root must not exist; no owner DB or provider path.
use std::collections::BTreeSet;
use std::path::PathBuf;
use farcooler_daemon::workflow_runtime::{Runtime, Request, coding_definition};
use farcooler_workflow::{Command, Phase};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("supply a new fixture root")?);
    let mut runtime = Runtime::create(&root).await?;
    let controller = runtime.controller();
    let receipt = runtime.submit(&controller, Request { id: "enqueue-fixture".into(), workspace: "fixture".into(),
        expected_revision: 0, sequence: 1, command: Command::Enqueue { item: "coding".into(),
            definition: coding_definition(true), policy_pin: "policy/v1".into(), resource: runtime.resource().into(),
            dependencies: BTreeSet::new(), initial_candidate: None } }, 1)?;
    if receipt.result.is_err() { return Err("fixture enqueue refused".into()); }
    // No controller commands or notice reads/ACKs occur during execution.
    for time in 2..32 {
        runtime.pump(time).await?;
        if runtime.state("fixture")?.ok_or("missing state")?.items["coding"].phase == Phase::Done { break; }
    }
    let state = runtime.state("fixture")?.ok_or("missing state")?;
    if state.items["coding"].phase != Phase::Done { return Err("fixture did not finish".into()); }
    let trace = serde_json::json!({ "state": state, "events": runtime.events("fixture")?,
        "notices": runtime.notices("fixture", 0)? });
    let raw = serde_json::to_vec_pretty(&trace)?;
    std::fs::write(root.join("trace.json"), &raw)?;
    println!("{}", String::from_utf8(raw)?);
    Ok(())
}
