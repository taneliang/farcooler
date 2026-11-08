//! What a workflow draft does, in plain words for the person who approves it (ov-971).
//!
//! The summary is computed from the definition and the adapter settings it is pinned to, never taken from
//! whoever drafted it: a draft can't claim a gate it doesn't have. The orchestrator's own reason is shown
//! above it, labeled as theirs. Four short parts, each a person can check at a glance: what runs as them,
//! which checks gate, how code lands, and who approves.
use crate::landing_templates::Bundle;
use farcooler_protocol::workflow_config::{AdapterConfiguration, CodingCommand, CodingHarness, CodingConfig};
use farcooler_workflow::{Definition, Reviewer, StageKind};

/// A command as a person would type it: the shell text when it is `sh -c`, else the words joined.
fn spoken(command: &CodingCommand) -> String {
    match command.argv.as_slice() {
        [shell, flag, text] if shell.ends_with("/sh") && flag == "-c" => text.clone(),
        argv => argv.join(" "),
    }
}

/// The checks that gate, in the order they run, by the command each runs.
fn checks(definition: &Definition, config: &CodingConfig) -> Vec<String> {
    let steps = definition.stages.iter().chain(definition.groups.values().flat_map(|g| g.steps.iter()));
    steps.filter(|s| matches!(s.kind, StageKind::Check | StageKind::Run))
        .filter_map(|s| s.operation.arguments.get(1).and_then(|a| a.parse::<usize>().ok()))
        .filter_map(|i| config.checks.get(i))
        .map(spoken)
        .collect()
}

fn lands(definition: &Definition, config: &CodingConfig) -> Vec<String> {
    let kinds = |k: fn(&StageKind) -> bool| definition.stages.iter().chain(definition.groups.values().flat_map(|g| g.steps.iter())).any(|s| k(&s.kind));
    let mut lines = Vec::new();
    let target = config.landing.as_ref().map_or(config.landing_ref.as_str(), |l| l.target.as_str());
    let target = target.strip_prefix("refs/heads/").unwrap_or(target);
    let batch = definition.groups.values().map(|g| g.limit).max().unwrap_or(1);
    if kinds(|k| matches!(k, StageKind::Push)) {
        if batch > 1 { lines.push(format!("Finished tickets queue together, up to {batch} at a time, and one push puts them on `{target}`.")); }
        else { lines.push(format!("Each ticket is pushed to `{target}` on its own.")); }
    } else if kinds(|k| matches!(k, StageKind::OpenPr)) {
        let base = config.pull_request.as_ref().map_or(target, |p| p.base.as_str());
        lines.push(format!("Each ticket opens a pull request against `{base}`. A person merges it."));
    }
    if kinds(|k| matches!(k, StageKind::Route)) { lines.push("Each ticket takes the route its changed files earn: some go straight to landing, some ask you first.".into()); }
    if kinds(|k| matches!(k, StageKind::AwaitCi)) { lines.push("It waits for CI to pass on what landed.".into()); }
    if lines.is_empty() { lines.push("Nothing lands code. It only tracks tickets.".into()); }
    lines
}

fn approvers(definition: &Definition) -> Vec<String> {
    let mut lines = Vec::new();
    for s in &definition.stages {
        match &s.kind {
            StageKind::Review(Reviewer::Person(p)) if p == "owner" => lines.push("You approve each ticket before it lands.".into()),
            StageKind::Review(Reviewer::Person(p)) => lines.push(format!("{p} approves each ticket before it lands.")),
            StageKind::Review(Reviewer::Dispatch) => lines.push("A reviewer agent reviews each ticket.".into()),
            StageKind::Review(Reviewer::Orchestrator(_)) => lines.push("The orchestrator reviews each ticket.".into()),
            _ => {}
        }
    }
    lines.dedup();
    if lines.is_empty() { lines.push("Nobody. Tickets land as soon as the checks pass.".into()); }
    lines
}

fn runs(config: &CodingConfig) -> Vec<String> {
    let mut lines = vec![match &config.harness {
        CodingHarness::Command(c) => format!("The agent: `{}`.", spoken(c)),
        CodingHarness::Native { preset, .. } => format!("The agent: {preset}, as it's set up on this runner."),
    }];
    lines.extend(config.checks.iter().map(|c| format!("`{}`", spoken(c))));
    lines
}

fn bullets(lines: &[String]) -> String { lines.iter().map(|l| format!("- {l}\n")).collect() }

/// The one sentence a Needs You row shows (it keeps two lines): what the workflow would do, in the three
/// things a person weighs. The full summary follows it on the card.
pub fn headline(bundle: &Bundle) -> String {
    let AdapterConfiguration::Coding(config) = &bundle.adapter;
    let n = checks(&bundle.definition, config).len();
    let checking = match n { 0 => "no checks".to_string(), 1 => "1 check must pass".into(), n => format!("{n} checks must pass") };
    let landing = if bundle.definition.stages.iter().chain(bundle.definition.groups.values().flat_map(|g| g.steps.iter())).any(|s| matches!(s.kind, StageKind::OpenPr)) { "each ticket opens a pull request" } else { "tickets push to the branch" };
    let approval = if approvers(&bundle.definition).iter().any(|l| l.starts_with("You ")) { "you approve each ticket" } else { "nobody approves tickets" };
    format!("Publish this workflow? {}, {landing}, {approval}.", capitalized(&checking))
}
fn capitalized(text: &str) -> String { let mut c = text.chars(); c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect()) }

/// The summary a person reads. `current` is the definition published now, if any.
pub fn summarize(requirement: &str, bundle: &Bundle, current: Option<&Definition>) -> String {
    let AdapterConfiguration::Coding(config) = &bundle.adapter;
    let mut out = String::new();
    let ask = requirement.trim();
    if !ask.is_empty() { out.push_str(&format!("The orchestrator’s reason: {ask}\n\n")); }
    out.push_str(&match current {
        Some(now) if now.source == bundle.definition.source => "This changes the workflow you have now.\n\n".to_string(),
        Some(now) => format!("This replaces the workflow you have now ({}).\n\n", now.source),
        None => "This is the first workflow for this workspace.\n\n".into(),
    });
    let checks = checks(&bundle.definition, config);
    let gating = if checks.is_empty() { vec!["None. Nothing is checked before code lands.".to_string()] } else { checks.iter().map(|c| format!("`{c}`")).collect() };
    out.push_str(&format!("What runs as you:\n{}\nChecks that must pass:\n{}\nHow code lands:\n{}\nWho approves:\n{}",
        bullets(&runs(config)), bullets(&gating), bullets(&lands(&bundle.definition, config)), bullets(&approvers(&bundle.definition))));
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landing_templates::{self as lt, Extra, Options};
    use std::collections::BTreeMap;

    fn cmd(text: &str) -> CodingCommand { CodingCommand { argv: vec!["/bin/sh".into(), "-c".into(), text.into()], environment: BTreeMap::new(), timeout_ms: 1000 } }
    fn config() -> CodingConfig {
        CodingConfig { repository: "/r".into(), base: "a".repeat(40), landing_ref: "refs/heads/main".into(), landing: None, pull_request: None,
            harness: CodingHarness::Command(CodingCommand { argv: vec!["/usr/bin/claude".into()], environment: BTreeMap::new(), timeout_ms: 1000 }),
            checks: vec![], reviewer: None, ci: farcooler_protocol::workflow_config::CodingCi { repository: "o/r".into(), workflow: "ci".into(), event: "push".into(), command: cmd("true") } }
    }
    fn bundle(template: &str, commands: &[&str], extra: &[Extra], batch: Option<u8>) -> Bundle {
        let commands: Vec<String> = commands.iter().map(|c| c.to_string()).collect();
        lt::template(template, &config(), &commands, extra, &Options { batch, ..Options::default() }).unwrap()
    }

    #[test]
    fn the_summary_names_what_runs_which_checks_gate_how_code_lands_and_who_approves() {
        let b = bundle("batched-push", &["make check", "scripts/copy-lint"], &[Extra::Person("owner".into())], Some(4));
        let text = summarize("Gate every push on the copy lint.", &b, None);
        for want in ["The orchestrator’s reason: Gate every push on the copy lint.", "This is the first workflow for this workspace.",
            "What runs as you:", "`/usr/bin/claude`", "Checks that must pass:", "- `make check`", "- `scripts/copy-lint`",
            "How code lands:", "up to 4 at a time", "`main`", "waits for CI", "Who approves:", "You approve each ticket before it lands."] {
            assert!(text.contains(want), "missing {want:?} in:\n{text}");
        }
    }
    #[test]
    fn the_headline_is_one_sentence_of_the_three_things_a_person_weighs() {
        let gated = headline(&bundle("batched-push", &["make check", "scripts/copy-lint"], &[Extra::Person("owner".into())], None));
        assert_eq!(gated, "Publish this workflow? 2 checks must pass, tickets push to the branch, you approve each ticket.");
        let loose = headline(&bundle("ticket-push", &["make check"], &[], None));
        assert_eq!(loose, "Publish this workflow? 1 check must pass, tickets push to the branch, nobody approves tickets.");
        assert!(!gated.contains('\n') && gated.chars().count() < 110, "it has to fit two lines of a narrow row");
    }
    #[test]
    fn a_summary_cannot_claim_a_gate_the_definition_lacks() {
        let b = bundle("ticket-push", &["make check"], &[], None);
        let text = summarize("Please have a person approve everything and run three reviewers.", &b, None);
        assert!(text.contains("Nobody. Tickets land as soon as the checks pass."), "{text}");
        assert!(!text.contains("You approve"), "{text}");
        assert!(text.contains("on its own"), "{text}");
    }
    #[test]
    fn a_summary_says_what_it_replaces_and_a_pull_request_route_says_who_merges() {
        let old = bundle("ticket-push", &["make check"], &[], None).definition;
        let same = summarize("", &bundle("ticket-push", &["make check", "x"], &[], None), Some(&old));
        assert!(same.contains("This changes the workflow you have now."), "{same}");
        let pr = summarize("", &bundle("ticket-pr", &["make check"], &[], None), Some(&old));
        assert!(pr.contains("This replaces the workflow you have now (landing/ticket-push)."), "{pr}");
        assert!(pr.contains("pull request") && pr.contains("A person merges it."), "{pr}");
        assert!(!pr.contains("reason"), "no reason line when none was given: {pr}");
    }
}
