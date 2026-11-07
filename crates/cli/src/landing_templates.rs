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
    /// Add a gate an agent decides, using the runner's configured reviewer.
    #[arg(long)]
    pub agent_gate: bool,
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
        self.agent_gate.then_some(Extra::Agent).into_iter().chain(self.person_gate.iter().map(|p| Extra::Person(p.clone()))).collect()
    }
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

pub fn template(name: &str, held: &Held, commands: &[String], gates: &Gates, batch: Option<u8>) -> Result<Bundle, lt::Error> {
    let config = held.adapter.as_ref().ok_or(lt::Error::NoAdapter)?;
    lt::template(name, config, commands, &gates.extras(), batch, &gates.gate_env)
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

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::workflow_config::*;

    fn cmd() -> CodingCommand { CodingCommand { argv: vec!["/usr/bin/true".into()], environment: Default::default(), timeout_ms: 1000 } }
    fn config() -> CodingConfig {
        CodingConfig { repository: "/r".into(), base: "a".repeat(40), landing_ref: "refs/heads/main".into(), landing: None,
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
    fn a_template_takes_gates_of_all_three_kinds_from_flags() {
        let held = held(&get(None)).unwrap();
        let gates = Gates { agent_gate: false, person_gate: vec!["owner".into()], gate_env: vec![] };
        let bundle = template("batched-push", &held, &["make check".into()], &gates, Some(4)).unwrap();
        assert_eq!(bundle.definition.groups["landing"].limit, 4);
        assert!(bundle.definition.stages.iter().any(|s| matches!(&s.kind, lt::StageKind::Review(lt::Reviewer::Person(p)) if p == "owner")));
        // An agent gate needs the reviewer this runner doesn't have.
        let agent = Gates { agent_gate: true, person_gate: vec![], gate_env: vec![] };
        assert_eq!(template("batched-push", &held, &["make check".into()], &agent, None).unwrap_err(), lt::Error::NoReviewer);
    }
}
