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
    /// Add a gate a person decides: the ticket waits for their word. Name `owner`, a member as `member:<handle>`, or
    /// `role:maintainer` for any Maintainer. Repeat for several.
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

/// What a runner already holds, from `workflow get`: the adapter to pin to and the definition published. A workspace
/// that has never published has neither, and the runner suggests settings for its first draft instead (ov-1010).
pub struct Held { pub pin: Option<String>, pub adapter: Option<farcooler_protocol::workflow_config::CodingConfig>, pub source: Option<String>,
    pub suggested: Result<Option<farcooler_protocol::workflow_config::CodingConfig>, String> }
impl Held {
    /// The settings a draft pins to: the published ones, else the runner's suggestion.
    fn config(&self) -> Result<&farcooler_protocol::workflow_config::CodingConfig, lt::Error> {
        if let Some(config) = &self.adapter { return Ok(config); }
        match &self.suggested { Ok(Some(config)) => Ok(config), Ok(None) => Err(lt::Error::NoAdapter), Err(why) => Err(lt::Error::NoSuggestion(why.clone())) }
    }
    /// Whether a draft would pin to the runner's suggestion rather than published settings.
    pub fn suggesting(&self) -> bool { self.adapter.is_none() && matches!(self.suggested, Ok(Some(_))) }
}

pub fn held(get: &serde_json::Value) -> Result<Held, Box<dyn std::error::Error>> {
    let Some(configuration) = get.get("configuration").filter(|c| !c.is_null()) else {
        // A runner from before ov-1010 sends neither, and the draft is refused as it was.
        let suggested = match (get.get("suggested_adapter").filter(|a| !a.is_null()), get.get("suggested_adapter_refused").and_then(|w| w.as_str())) {
            (Some(a), _) => { let AdapterConfiguration::Coding(config) = serde_json::from_value(a.clone())?; Ok(Some(config)) }
            (None, Some(why)) => Err(why.to_string()),
            (None, None) => Ok(None),
        };
        return Ok(Held { pin: None, adapter: None, source: None, suggested });
    };
    // The runner's re-pin of the published adapter, when a pinned file changed since (ov-1259): a draft pins to the
    // files as they are now, never to a digest copied from the publication that stopped being true.
    let adapter = match get.get("repinned_adapter").filter(|a| !a.is_null()).or_else(|| configuration.get("adapter").filter(|a| !a.is_null())) {
        Some(a) => { let AdapterConfiguration::Coding(config) = serde_json::from_value(a.clone())?; Some(config) }
        None => None,
    };
    Ok(Held { pin: configuration.get("pin").and_then(|p| p.as_str()).map(String::from),
        source: configuration.pointer("/definition/source").and_then(|s| s.as_str()).map(String::from), adapter, suggested: Ok(None) })
}

/// `workspace re-pin` (ov-1259): the published definition with its adapter pinned to the files as they are now, both of
/// which the runner computes (`repinned_definition`, `repinned_adapter`). `None` when every pin still holds, so there is nothing to publish.
pub fn repinned(get: &serde_json::Value) -> Result<Option<Bundle>, Box<dyn std::error::Error>> {
    get.get("configuration").filter(|c| !c.is_null()).ok_or("This workspace has no published workflow to re-pin.")?;
    let Some(adapter) = get.get("repinned_adapter").filter(|a| !a.is_null()) else {
        if get.get("engine_codex_hooks").is_none() { return Err("This runner is too old to re-pin a workflow. Update Far Cooler on it first.".into()); }
        return Ok(None);
    };
    // The definition names the adapter's pin in every step, so the runner re-pins it too.
    let definition = get.get("repinned_definition").ok_or("the runner sent no re-pinned definition")?;
    Ok(Some(Bundle { definition: serde_json::from_value(definition.clone())?, adapter: serde_json::from_value(adapter.clone())? }))
}

/// Why a re-pin draft is filed, when the person gives no reason of their own.
pub const REPIN_REASON: &str = "Re-pin the published workflow to its files as they are now. Only the pinned file digests change.";

pub fn list() -> String {
    Template::ALL.iter().map(|t| format!("{:<14} {}\n", t.name(), t.about())).collect()
}

pub fn import(charter: &str, held: &Held, gates: &Gates) -> Result<(Bundle, Vec<String>), lt::Error> {
    let config = held.config()?;
    lt::import_charter(charter, config, held.source.as_deref(), &gates.extras(), &gates.gate_env)
}

pub fn template(name: &str, held: &Held, commands: &[String], gates: &Gates, routes: &Routes, batch: Option<u32>) -> Result<Bundle, lt::Error> {
    let config = held.config()?;
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
        CodingConfig { security_reviews: vec![], repository: "/r".into(), base: "a".repeat(40), landing_ref: "refs/heads/main".into(), landing: None, pull_request: None,
            harness: CodingHarness::Command(cmd()), checks: vec![], reviewer: None,
            ci: CodingCi { repository: "o/r".into(), workflow: "ci".into(), event: "push".into(), command: cmd(), unrun: None } }
    }
    fn get(source: Option<&str>) -> serde_json::Value {
        let adapter = serde_json::to_value(AdapterConfiguration::Coding(config())).unwrap();
        match source {
            Some(s) => serde_json::json!({"schema": 1, "configuration": {"pin": "wf1:abc", "definition": {"source": s}, "adapter": adapter}}),
            None => serde_json::json!({"schema": 1, "configuration": {"pin": "wf1:abc", "definition": {"source": "coding/v1"}, "adapter": adapter}}),
        }
    }

    /// ov-1259: a draft pins to the runner's re-pin of the published adapter, never to a digest copied from it, and
    /// `workspace re-pin` is the published definition with that adapter, or nothing when every pin holds.
    #[test]
    fn drafts_pin_to_the_files_as_they_are_now() {
        let mut fresh = config();
        fresh.ci.workflow = "repinned".into();
        let mut reply = get(None);
        reply["engine_codex_hooks"] = serde_json::json!({"path": "/h.json", "digest": "wf1:h"});
        assert!(repinned(&reply).unwrap().is_none(), "every pin holds");
        let mut old = reply.clone();
        old.as_object_mut().unwrap().remove("engine_codex_hooks");
        assert!(repinned(&old).is_err(), "a runner too old to say is refused, not read as current");
        let definition = serde_json::to_value(lt::template("ticket-push", &fresh, &["make check".into()], &[], &Default::default()).unwrap().definition).unwrap();
        reply["repinned_definition"] = definition.clone();
        reply["repinned_adapter"] = serde_json::to_value(AdapterConfiguration::Coding(fresh.clone())).unwrap();
        let held = held(&reply).unwrap();
        assert_eq!(held.adapter.as_ref(), Some(&fresh));
        assert_eq!(held.pin.as_deref(), Some("wf1:abc"), "still drafted against the published pin");
        let bundle = repinned(&reply).unwrap().unwrap();
        assert_eq!(bundle.adapter, AdapterConfiguration::Coding(fresh));
        assert_eq!(serde_json::to_value(&bundle.definition).unwrap(), definition);
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
    fn a_workspace_with_no_workflow_drafts_on_the_runners_suggestion_or_says_why_it_cant() {
        let suggested = serde_json::to_value(AdapterConfiguration::Coding(config())).unwrap();
        let held = held(&serde_json::json!({"schema": 1, "configuration": null, "suggested_adapter": suggested})).unwrap();
        assert!(held.suggesting() && held.pin.is_none());
        let bundle = template("batched-push", &held, &["make check".into()], &Gates::default(), &Routes::default(), Some(4)).unwrap();
        let AdapterConfiguration::Coding(pinned) = &bundle.adapter;
        assert_eq!((&pinned.repository, &pinned.harness, &pinned.ci), (&config().repository, &config().harness, &config().ci), "pinned to the suggestion");
        let held = super::held(&serde_json::json!({"schema": 1, "configuration": null, "suggested_adapter_refused": "Its origin isn’t on GitHub."})).unwrap();
        let err = template("batched-push", &held, &["make check".into()], &Gates::default(), &Routes::default(), None).unwrap_err();
        assert_eq!(err, lt::Error::NoSuggestion("Its origin isn’t on GitHub.".into()));
        assert!(err.to_string().ends_with("Its origin isn’t on GitHub."), "{err}");
        // Published settings win over a suggestion, which a runner never sends beside them anyway.
        assert!(!super::held(&get(None)).unwrap().suggesting());
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
        assert_eq!(bundle.definition.groups["landing"].limit, Some(4));
        let reviews: Vec<_> = bundle.definition.stages.iter().filter_map(|s| match &s.kind { lt::StageKind::Review(r) => Some(r.clone()), _ => None }).collect();
        assert_eq!(reviews, [lt::Reviewer::Orchestrator("orchestrator".into()), lt::Reviewer::Person("owner".into())]);
        // An agent gate needs an agent to start or a reviewer to run, and this runner has neither.
        let agent = Gates { agent_gate: true, orchestrator_gate: false, person_gate: vec![], gate_env: vec![] };
        assert_eq!(template("batched-push", &held, &["make check".into()], &agent, &Routes::default(), None).unwrap_err(), lt::Error::NoReviewer);
    }
}
