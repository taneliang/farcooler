//! `workspace landing-templates`, `landing-template` and `import-landing` (ov-959): draft a landing
//! definition for a person to publish. Drafting changes nothing on the runner; publishing is
//! `workspace set-workflow`, which only a person may do.
use clap::Args;
use farcooler_daemon::landing_templates::{self as lt, Bundle, Extra};
use farcooler_protocol::workflow_config::AdapterConfiguration;
use lt::Template;
use uuid::Uuid;

#[derive(Args, Clone, Default)]
pub struct Gates {
    /// Add a gate an agent decides. The engine starts the agent, and it must give a verdict; on a runner that
    /// can't start agents, its configured reviewer runs instead.
    #[arg(long)]
    pub agent_gate: bool,
    /// Add a gate the orchestrator decides: the ticket waits for its word, however it chooses to review.
    #[arg(long)]
    pub orchestrator_gate: bool,
    /// Add a gate a person decides: the ticket waits for their word. Only `owner` can decide. Repeat for several.
    #[arg(long, value_name = "PERSON")]
    pub person_gate: Vec<String>,
    /// A setting every command gate runs with, as NAME=value. Repeat for several. A gate gets PATH and HOME
    /// unless you set them here; nothing else from the runner reaches it.
    #[arg(long, value_name = "NAME=VALUE")]
    pub gate_env: Vec<String>,
}
impl Gates {
    fn extras(&self) -> Vec<Extra> {
        self.agent_gate.then_some(Extra::Agent).into_iter().chain(self.orchestrator_gate.then_some(Extra::Orchestrator))
            .chain(self.person_gate.iter().map(|p| Extra::Person(p.clone()))).collect()
    }
}

#[derive(Args, Clone, Default)]
pub struct Routes {
    /// Route tickets by the paths they change: PATTERN=push or PATTERN=ask, first match wins per file, and the most
    /// careful file decides. `ask` sends a ticket through the owner first. Repeat for several.
    #[arg(long, value_name = "PATTERN=ROUTE")]
    pub route: Vec<String>,
    /// The route a ticket takes when no `--route` pattern matches it: push (the default) or ask.
    #[arg(long, value_name = "ROUTE")]
    pub route_default: Option<String>,
}

#[derive(Args, Clone)]
pub struct Output {
    /// Write the draft here instead of printing it. Publish it with `workspace set-workflow --file`.
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
}

/// What a runner already holds, from `workflow get`: the adapter to pin to and the definition published.
pub struct Held { pub pin: Option<String>, pub adapter: Option<farcooler_protocol::workflow_config::CodingConfig>, pub source: Option<String> }

pub fn held(get: &serde_json::Value) -> Result<Held, Box<dyn std::error::Error>> {
    let Some(configuration) = get.get("configuration").filter(|c| !c.is_null()) else { return Ok(Held { pin: None, adapter: None, source: None }) };
    let adapter = match configuration.get("adapter").filter(|a| !a.is_null()) {
        Some(a) => { let AdapterConfiguration::Coding(config) = serde_json::from_value(a.clone())?; Some(config) }
        None => None,
    };
    Ok(Held { pin: configuration.get("pin").and_then(|p| p.as_str()).map(String::from),
        source: configuration.pointer("/definition/source").and_then(|s| s.as_str()).map(String::from), adapter })
}

pub fn list() -> String {
    Template::ALL.iter().map(|t| format!("{:<14} {}\n", t.name(), t.about())).collect()
}

pub fn import(charter: &str, held: &Held, gates: &Gates) -> Result<(Bundle, Vec<String>), lt::Error> {
    let config = held.adapter.as_ref().ok_or(lt::Error::NoAdapter)?;
    lt::import_charter(charter, config, held.source.as_deref(), &gates.extras(), &gates.gate_env)
}

pub fn template(name: &str, held: &Held, commands: &[String], gates: &Gates, routes: &Routes, batch: Option<u8>) -> Result<Bundle, lt::Error> {
    let config = held.adapter.as_ref().ok_or(lt::Error::NoAdapter)?;
    let options = lt::Options { batch, environment: gates.gate_env.clone(), route_default: routes.route_default.clone(), routes: routes.route.clone() };
    lt::template(name, config, commands, &gates.extras(), &options)
}

/// Print or write a draft, and say how to publish it. The words go to standard error so `--json`
/// output and a redirected draft stay clean.
pub fn deliver(bundle: &Bundle, notes: &[String], held: &Held, workspace: Uuid, out: &Output) -> Result<(), Box<dyn std::error::Error>> {
    let text = serde_json::to_string_pretty(bundle)?;
    match &out.out {
        Some(path) => {
            std::fs::write(path, format!("{text}\n")).map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
            eprintln!("Draft written to {}.", path.display());
        }
        None => println!("{text}"),
    }
    for note in notes { eprintln!("Note: {note}"); }
    let expected = held.pin.as_deref().map(|p| format!(" --expected-pin {p}")).unwrap_or_default();
    eprintln!("Read it, then publish it yourself: farcooler workspace set-workflow {workspace} --file <draft>{expected} --command-id <id> --revision <n> --epoch <n> --sequence <n>");
    Ok(())
}

/// The command that proposes `bundle` to a person, in the orchestrator's `reason`.
pub fn propose_action(reason: &str, bundle: Bundle) -> farcooler_protocol::workflow_commands::Action {
    farcooler_protocol::workflow_commands::Action::Propose { reason: reason.into(), definition: bundle.definition, adapter: Box::new(bundle.adapter) }
}

fn state_words(state: &str) -> &'static str {
    match state {
        "pending" => "Waiting for a person to approve it.",
        "approved" => "Approved and published.",
        "declined" => "Declined. The workflow is unchanged.",
        "out_of_date" => "Not published: the workflow changed after this was drafted. Draft it again.",
        "replaced" => "Replaced by a newer draft.",
        "failed" => "Approved, but publishing was refused. Draft it again.",
        _ => "In an unknown state.",
    }
}

/// One draft: its id and where it stands, then what it does.
pub fn print_draft(reply: &serde_json::Value, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    if json { println!("{reply}"); return Ok(()); }
    let d = reply.get("draft").ok_or("the runner sent no draft")?;
    let text = |k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or("");
    println!("Draft {}: {}", text("id"), state_words(text("state")));
    if text("state") == "pending" { println!("A card in Needs Decision asks for approval. Nothing is published until a person answers it."); }
    println!("\n{}", text("summary"));
    Ok(())
}

/// Every draft, one line each.
pub fn print_drafts(reply: &serde_json::Value, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    if json { println!("{reply}"); return Ok(()); }
    let drafts = reply.get("drafts").and_then(|d| d.as_array()).ok_or("the runner sent no drafts")?;
    if drafts.is_empty() { println!("No workflow drafts yet."); }
    for d in drafts {
        let text = |k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or("");
        println!("{}  {}  {}", text("id"), text("source"), state_words(text("state")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::workflow_config::*;

    fn cmd() -> CodingCommand { CodingCommand { argv: vec!["/usr/bin/true".into()], environment: Default::default(), timeout_ms: 1000 } }
    fn config() -> CodingConfig {
        CodingConfig { repository: "/r".into(), base: "a".repeat(40), landing_ref: "refs/heads/main".into(), landing: None, pull_request: None,
            harness: CodingHarness::Command(cmd()), checks: vec![], reviewer: None,
            ci: CodingCi { repository: "o/r".into(), workflow: "ci".into(), event: "push".into(), command: cmd() } }
    }
    fn get(source: Option<&str>) -> serde_json::Value {
        let adapter = serde_json::to_value(AdapterConfiguration::Coding(config())).unwrap();
        match source {
            Some(s) => serde_json::json!({"schema": 1, "configuration": {"pin": "wf1:abc", "definition": {"source": s}, "adapter": adapter}}),
            None => serde_json::json!({"schema": 1, "configuration": {"pin": "wf1:abc", "definition": {"source": "coding/v1"}, "adapter": adapter}}),
        }
    }

    #[test]
    fn the_list_names_all_four_templates() {
        let text = list();
        for name in ["batched-push", "ticket-push", "ticket-pr", "batched-pr"] { assert!(text.contains(name), "{text}"); }
    }
    #[test]
    fn a_draft_pins_to_the_runners_adapter_and_names_the_pin_it_was_read_at() {
        let held = held(&get(None)).unwrap();
        assert_eq!(held.pin.as_deref(), Some("wf1:abc"));
        let (bundle, _) = import("## Landing\n\n- Gate: `make check`\n", &held, &Gates::default()).unwrap();
        assert_eq!(bundle.definition.source, "landing/batched-push");
    }
    #[test]
    fn the_import_is_once_only_whatever_way_the_landing_definition_got_there() {
        for template in Template::ALL {
            let held = held(&get(Some(&format!("landing/{}", template.name())))).unwrap();
            let err = import("## Landing\n\n- Gate: `make check`\n", &held, &Gates::default()).unwrap_err();
            assert!(matches!(err, lt::Error::AlreadyImported(_)), "{err}");
        }
    }
    #[test]
    fn a_runner_with_no_adapter_gets_a_plain_refusal_not_a_guess() {
        let held = held(&serde_json::json!({"schema": 1, "configuration": null})).unwrap();
        assert_eq!(import("## Landing\n\n- Gate: none\n", &held, &Gates::default()).unwrap_err(), lt::Error::NoAdapter);
        assert!(lt::Error::NoAdapter.to_string().ends_with('.'));
    }
    #[test]
    fn a_template_routes_tickets_by_path_from_flags() {
        let held = held(&get(None)).unwrap();
        let routes = Routes { route: vec!["site/**=ask".into()], route_default: None };
        let bundle = template("ticket-push", &held, &["make check".into()], &Gates::default(), &routes, None).unwrap();
        assert!(bundle.definition.stages.iter().any(|s| s.kind == lt::StageKind::Route));
        let bad = Routes { route: vec!["site/**".into()], route_default: None };
        assert!(matches!(template("ticket-push", &held, &["make check".into()], &Gates::default(), &bad, None).unwrap_err(), lt::Error::BadRoute(_)));
    }
    #[test]
    fn a_proposal_travels_as_a_proposal_and_every_draft_state_has_a_sentence() {
        let held = held(&get(None)).unwrap();
        let bundle = template("ticket-push", &held, &["make check".into()], &Gates::default(), &Routes::default(), None).unwrap();
        let action = propose_action("Gate on make check.", bundle);
        assert_eq!(action.method(), "workflow.propose");
        let raw = serde_json::to_string(&action).unwrap();
        assert_eq!(farcooler_protocol::workflow_commands::parse(&raw).unwrap(), action, "the daemon reads exactly what the CLI sends");
        for state in ["pending", "approved", "declined", "out_of_date", "replaced", "failed"] {
            assert!(state_words(state).ends_with('.') && !state_words(state).contains("unknown"), "{state}");
        }
    }
    #[test]
    fn a_template_takes_gates_of_all_three_kinds_from_flags() {
        let held = held(&get(None)).unwrap();
        let gates = Gates { agent_gate: false, orchestrator_gate: true, person_gate: vec!["owner".into()], gate_env: vec![] };
        let bundle = template("batched-push", &held, &["make check".into()], &gates, &Routes::default(), Some(4)).unwrap();
        assert_eq!(bundle.definition.groups["landing"].limit, 4);
        let reviews: Vec<_> = bundle.definition.stages.iter().filter_map(|s| match &s.kind { lt::StageKind::Review(r) => Some(r.clone()), _ => None }).collect();
        assert_eq!(reviews, [lt::Reviewer::Orchestrator("orchestrator".into()), lt::Reviewer::Person("owner".into())]);
        // An agent gate needs an agent to start or a reviewer to run, and this runner has neither.
        let agent = Gates { agent_gate: true, orchestrator_gate: false, person_gate: vec![], gate_env: vec![] };
        assert_eq!(template("batched-push", &held, &["make check".into()], &agent, &Routes::default(), None).unwrap_err(), lt::Error::NoReviewer);
    }
}
