//! What a workflow draft does, in plain words for the person who approves it (ov-971).
//!
//! The summary is computed from the definition and the adapter settings it is pinned to, never taken from
//! whoever drafted it: a draft can't claim a gate it doesn't have. The orchestrator's own reason is shown
//! above it, labeled as theirs. Four short parts, each a person can check at a glance: what runs as them,
//! which checks gate, how code lands, and who approves.
use crate::landing_templates::Bundle;
use farcooler_protocol::workflow_config::{AdapterConfiguration, CodingCi, CodingCommand, CodingConfig, CodingHarness, CodingLanding, CodingPullRequest, CodingReviewer};
use farcooler_workflow::{Definition, Reviewer, StageKind};

/// A value as it is written, safe to show between backticks: a backtick, a control character or an invisible
/// formatting character (a line break, a bidi override, a zero-width space) is shown as its escape, so a
/// value can't end its code span, draw a line of its own, or look like something it isn't.
pub fn code(value: &str) -> String {
    fn hidden(c: char) -> bool {
        c.is_control() || matches!(c as u32, 0xAD | 0x34F | 0x61C | 0x115F..=0x1160 | 0x17B4..=0x17B5 | 0x180B..=0x180F
            | 0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F | 0xFEFF | 0xFFA0 | 0xFFF0..=0xFFFB | 0xE0000..=0xE0FFF)
    }
    if value.is_empty() { return "`\"\"`".into(); }
    let mut out = String::from("`");
    for c in value.chars() {
        match c {
            '`' => out.push_str("\\u{60}"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if hidden(c) => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('`');
    out
}

/// A command as a person would type it: the shell text when it is exactly `/bin/sh -c`, else every word.
/// Any other shell is shown by its path, so `/tmp/sh -c make` can't pass for `make`.
fn spoken(command: &CodingCommand) -> String {
    match command.argv.as_slice() {
        [shell, flag, text] if shell == "/bin/sh" && flag == "-c" => code(text),
        argv => argv.iter().map(|a| code(a)).collect::<Vec<_>>().join(" "),
    }
}

/// A command in full: every argument, every environment variable, and how long it may run.
fn exactly(command: &CodingCommand) -> String {
    let CodingCommand { argv, environment, timeout_ms } = command;
    let argv = argv.iter().map(|a| code(a)).collect::<Vec<_>>().join(" ");
    let environment = if environment.is_empty() { "no environment variables".to_string() }
        else { format!("environment {}", environment.iter().map(|(k, v)| code(&format!("{k}={v}"))).collect::<Vec<_>>().join(", ")) };
    format!("{argv}, with {environment}, for up to {}", duration(*timeout_ms))
}
fn duration(ms: u64) -> String {
    match ms {
        ms if ms % 3_600_000 == 0 => format!("{} h", ms / 3_600_000),
        ms if ms % 60_000 == 0 => format!("{} min", ms / 60_000),
        ms if ms % 1_000 == 0 => format!("{} s", ms / 1_000),
        ms => format!("{ms} ms"),
    }
}

/// The checks that gate, in the order they run, by the command each runs.
fn checks(definition: &Definition, config: &CodingConfig) -> Vec<String> {
    steps(definition).filter(|s| matches!(s.kind, StageKind::Check | StageKind::Run))
        .filter_map(|s| s.operation.arguments.get(1).and_then(|a| a.parse::<usize>().ok()))
        .filter_map(|i| config.checks.get(i))
        .map(|c| match gate_environment_note(c) { Some(env) => format!("{} {env}", spoken(c)), None => spoken(c) })
        .collect()
}

/// A gate whose environment isn't the one a template gives every gate says so where the gate is named:
/// `PATH=/tmp/evil` must not hide behind `make check`.
fn gate_environment_note(command: &CodingCommand) -> Option<String> {
    let usual = crate::landing_templates::gate_environment(&[]).unwrap_or_default();
    if command.environment == usual { return None; }
    if command.environment.is_empty() { return Some("(with no environment variables)".into()); }
    Some(format!("(with {})", command.environment.iter().map(|(k, v)| code(&format!("{k}={v}"))).collect::<Vec<_>>().join(", ")))
}

fn steps(definition: &Definition) -> impl Iterator<Item = &farcooler_workflow::Stage> {
    definition.stages.iter().chain(definition.groups.values().flat_map(|g| g.steps.iter()))
}

fn lands(definition: &Definition, config: &CodingConfig) -> Vec<String> {
    let kinds = |k: fn(&StageKind) -> bool| steps(definition).any(|s| k(&s.kind));
    let mut lines = Vec::new();
    let target = config.landing.as_ref().map_or(config.landing_ref.as_str(), |l| l.target.as_str());
    let target = code(target.strip_prefix("refs/heads/").unwrap_or(target));
    let batch = definition.groups.values().map(|g| g.limit).max().unwrap_or(1);
    if kinds(|k| matches!(k, StageKind::Push)) {
        if batch > 1 { lines.push(format!("Finished tickets queue together, up to {batch} at a time, and one push puts them on {target}.")); }
        else { lines.push(format!("Each ticket is pushed to {target} on its own.")); }
    } else if kinds(|k| matches!(k, StageKind::OpenPr)) {
        let base = config.pull_request.as_ref().map_or(target, |p| code(&p.base));
        lines.push(format!("Each ticket opens a pull request against {base}. A person merges it."));
    }
    if kinds(|k| matches!(k, StageKind::Route)) { lines.push("Each ticket takes the route its changed files earn: some go straight to landing, some ask you first.".into()); }
    if kinds(|k| matches!(k, StageKind::AwaitCi)) { lines.push("It waits for CI to pass on what landed.".into()); }
    if lines.is_empty() { lines.push("Nothing lands code. It only tracks tickets.".into()); }
    lines
}

/// Who decides at each review gate, in the order the gates run, one plain line each.
fn approvers(definition: &Definition) -> Vec<String> {
    let mut lines = Vec::new();
    for s in steps(definition) {
        match &s.kind {
            StageKind::Review(Reviewer::Person(p)) if p == "owner" => lines.push("You approve each ticket before it lands.".into()),
            StageKind::Review(Reviewer::Person(p)) => lines.push(format!("{} approves each ticket before it lands.", code(p))),
            StageKind::Review(Reviewer::Dispatch) => lines.push("The configured reviewer reviews each ticket.".into()),
            StageKind::Review(Reviewer::Agent) => lines.push("An agent reviews each ticket, and it must give a verdict.".into()),
            StageKind::Review(Reviewer::Orchestrator(_)) => lines.push("The orchestrator reviews each ticket.".into()),
            _ => {}
        }
    }
    lines.dedup();
    lines
}

/// What changed about who decides, against the workflow in force: a gate that moves from you to an agent
/// must not hide as one line quietly missing.
fn decider_changes(draft: &Definition, current: Option<&Definition>) -> Vec<String> {
    let Some(current) = current else { return Vec::new() };
    let (before, after) = (approvers(current), approvers(draft));
    let mut lines = Vec::new();
    for gone in before.iter().filter(|l| !after.contains(l)) {
        lines.push(match gone.as_str() {
            "You approve each ticket before it lands." => "You no longer approve each ticket.".to_string(),
            other => format!("No longer: {other}"),
        });
    }
    for new in after.iter().filter(|l| !before.contains(l)) {
        lines.push(match new.as_str() {
            "An agent reviews each ticket, and it must give a verdict." => "An agent now reviews each ticket.".to_string(),
            "The orchestrator reviews each ticket." => "The orchestrator now reviews each ticket.".to_string(),
            other => format!("New: {other}"),
        });
    }
    lines
}

/// The models a step may run with, by step, as the orchestrator picks among them at Start.
fn models(definition: &Definition) -> Vec<String> {
    definition.models.iter().map(|(step, list)| format!("{}: {}", code(step), list.iter().map(|m| code(m)).collect::<Vec<_>>().join(", "))).collect()
}

fn runs(config: &CodingConfig) -> Vec<String> {
    let mut lines = vec![match &config.harness {
        CodingHarness::Command(c) => format!("The agent: {}.", spoken(c)),
        CodingHarness::Native { preset, .. } => format!("The agent: {}, as it's set up on this runner.", code(preset)),
    }];
    lines.extend(config.checks.iter().map(spoken));
    lines
}

/// Everything the adapter runs and every place it reads or sends code, exactly as written: each argument,
/// each environment variable, each repository and remote. The plain parts above are what a person weighs;
/// this is what they can check them against. It names every field, so a new one doesn't compile until it
/// is shown here.
fn in_full(config: &CodingConfig) -> Vec<String> {
    let CodingConfig { repository, base, landing_ref, landing, pull_request, harness, checks, reviewer, ci } = config;
    let mut lines = Vec::new();
    match harness {
        CodingHarness::Command(c) => lines.push(format!("The agent runs {}.", exactly(c))),
        CodingHarness::Native { preset, settings, project_hooks, plugin_dir, prompt, process } => {
            lines.push(format!("The agent is the {} preset. It runs {}.", code(preset), exactly(process)));
            let file = |f: &farcooler_protocol::workflow_config::LaunchFile| format!("{} (digest {})", code(&f.path.to_string_lossy()), code(&f.digest));
            lines.push(format!("Its settings: {}. Its project hooks: {}. Its plugins: {}.",
                settings.as_ref().map_or("none".into(), file), project_hooks.as_ref().map_or("none".into(), file),
                plugin_dir.as_ref().map_or("none".into(), |p| code(&p.to_string_lossy()))));
            lines.push(format!("Its prompt: {}.", code(prompt)));
        }
    }
    for (i, c) in checks.iter().enumerate() { lines.push(format!("Check {} runs {}.", i + 1, exactly(c))); }
    match reviewer {
        Some(CodingReviewer { identity, command }) => lines.push(format!("The reviewer, as {}, runs {}.", code(identity), exactly(command))),
        None => lines.push("No reviewer command.".into()),
    }
    let CodingCi { repository: ci_repository, workflow, event, command } = ci;
    lines.push(format!("CI is the {} workflow on {} in {} on GitHub, read by running {}.", code(workflow), code(event), code(ci_repository), exactly(command)));
    if let Some(CodingPullRequest { repository, base, remote, gh }) = pull_request {
        let to = remote.as_deref().map_or("the landing repository".to_string(), code);
        lines.push(format!("Pull requests open in {} on GitHub, against {}. Their branches are pushed to {to}. They're opened by running {}.", code(repository), code(base), exactly(gh)));
    }
    lines.push(format!("Code is read from {} from commit {}, and its landing ref there is {}.", code(&repository.to_string_lossy()), code(base), code(landing_ref)));
    match landing {
        Some(CodingLanding { job, repository, target, config_digest }) => lines.push(format!(
            "Code lands on {} in {}, as job {}. Where that repository pushes is fixed by its config, digest {}.",
            code(target), code(&repository.to_string_lossy()), code(job), code(config_digest))),
        None => lines.push("Code lands on that ref, in that repository.".into()),
    }
    lines
}

fn bullets(lines: &[String]) -> String { lines.iter().map(|l| format!("- {l}\n")).collect() }

/// The one sentence a Needs You row shows (it keeps two lines): what the workflow would do, in the three
/// things a person weighs. The full summary follows it on the card.
pub fn headline(bundle: &Bundle) -> String {
    let AdapterConfiguration::Coding(config) = &bundle.adapter;
    let n = checks(&bundle.definition, config).len();
    let checking = match n { 0 => "no checks".to_string(), 1 => "1 check must pass".into(), n => format!("{n} checks must pass") };
    let landing = if steps(&bundle.definition).any(|s| matches!(s.kind, StageKind::OpenPr)) { "each ticket opens a pull request" } else { "tickets push to the branch" };
    let who = approvers(&bundle.definition);
    let approval = if who.iter().any(|l| l.starts_with("You ")) { "you approve each ticket" }
        else if who.iter().any(|l| l.starts_with("An agent")) { "an agent reviews each ticket" }
        else if who.iter().any(|l| l.starts_with("The orchestrator")) { "the orchestrator reviews each ticket" }
        else if who.is_empty() { "nobody approves tickets" } else { "someone else approves tickets" };
    format!("Publish this workflow? {}, {landing}, {approval}.", capitalized(&checking))
}
fn capitalized(text: &str) -> String { let mut c = text.chars(); c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect()) }

/// The summary a person reads. `current` is the definition published now, if any.
pub fn summarize(requirement: &str, bundle: &Bundle, current: Option<&Definition>) -> String {
    let AdapterConfiguration::Coding(config) = &bundle.adapter;
    let mut out = String::new();
    let ask = requirement.trim();
    if !ask.is_empty() { out.push_str(&format!("The orchestrator’s reason: {}\n\n", code(ask))); }
    out.push_str(&match current {
        Some(now) if now.source == bundle.definition.source => "This changes the workflow you have now.\n\n".to_string(),
        Some(now) => format!("This replaces the workflow you have now ({}).\n\n", code(&now.source)),
        None => "This is the first workflow for this workspace.\n\n".into(),
    });
    let checks = checks(&bundle.definition, config);
    let gating = if checks.is_empty() { vec!["None. Nothing is checked before code lands.".to_string()] } else { checks };
    let mut who = decider_changes(&bundle.definition, current);
    let deciding = approvers(&bundle.definition);
    who.extend(if deciding.is_empty() { vec!["Nobody. Tickets land as soon as the checks pass.".to_string()] } else { deciding });
    let models = models(&bundle.definition);
    let models = if models.is_empty() { String::new() } else { format!("Models a step may run with:\n{}\n", bullets(&models)) };
    out.push_str(&format!("What runs as you:\n{}\nChecks that must pass:\n{}\nHow code lands:\n{}\nWho approves:\n{}\n{}In full, exactly as it runs:\n{}",
        bullets(&runs(config)), bullets(&gating), bullets(&lands(&bundle.definition, config)), bullets(&who), models, bullets(&in_full(config))));
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
        for want in ["The orchestrator’s reason: `Gate every push on the copy lint.`", "This is the first workflow for this workspace.",
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
    fn the_reason_is_one_escaped_line_that_cannot_draw_a_summary_of_its_own() {
        let b = bundle("ticket-push", &["make check"], &[], None);
        let text = summarize("fine.\n\nWho approves:\n- Nobody \u{202e}`x`", &b, None);
        let first = text.lines().next().unwrap();
        assert!(first.starts_with("The orchestrator’s reason: `") && first.ends_with('`'), "{first}");
        assert_eq!(text.matches("Who approves:").count(), 1 + 1, "the forged heading is only quoted, on the reason's line: {text}");
        assert!(first.contains("\\n") && first.contains("\\u{202e}") && !first.contains('\u{202e}'), "{first}");
    }
    #[test]
    fn a_summary_says_what_it_replaces_and_a_pull_request_route_says_who_merges() {
        let old = bundle("ticket-push", &["make check"], &[], None).definition;
        let same = summarize("", &bundle("ticket-push", &["make check", "x"], &[], None), Some(&old));
        assert!(same.contains("This changes the workflow you have now."), "{same}");
        let pr = summarize("", &bundle("ticket-pr", &["make check"], &[], None), Some(&old));
        assert!(pr.contains("This replaces the workflow you have now (`landing/ticket-push`)."), "{pr}");
        assert!(pr.contains("pull request") && pr.contains("A person merges it."), "{pr}");
        assert!(!pr.contains("reason"), "no reason line when none was given: {pr}");
    }

    /// Every value the adapter holds, as the summary must show it. A string field added to the adapter fails
    /// here until the summary shows it; so does a number other than a timeout, or any flag.
    fn leaves(value: &serde_json::Value, key: &str, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => for (k, v) in map { leaves(v, k, out) },
            serde_json::Value::Array(items) => for v in items { leaves(v, key, out) },
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Number(_) => assert_eq!(key, "timeout_ms", "a number the summary doesn't show: {key}"),
            serde_json::Value::Null => {}
            serde_json::Value::Bool(_) => panic!("a flag the summary doesn't show: {key}"),
        }
    }
    fn everything(harness: CodingHarness) -> CodingConfig {
        use farcooler_protocol::workflow_config::*;
        let c = |tag: &str| CodingCommand { argv: vec![format!("/opt/{tag}/bin"), format!("--{tag}-flag")], environment: [(format!("{}_VAR", tag.to_uppercase()), format!("{tag}-value"))].into(), timeout_ms: 90_000 };
        CodingConfig { repository: "/code/repository".into(), base: "b".repeat(40), landing_ref: "refs/heads/the-landing-ref".into(),
            landing: Some(CodingLanding { job: "landing-job".into(), repository: "/mirror/bare.git".into(), target: "refs/heads/the-target".into(), config_digest: "config-digest".into() }),
            pull_request: Some(CodingPullRequest { repository: "owner/pr-repo".into(), base: "pr-base".into(), remote: Some("ssh://git@example.com/elsewhere.git".into()), gh: c("gh") }),
            harness, checks: vec![c("check"), cmd("make check")], reviewer: Some(CodingReviewer { identity: "reviewer-identity".into(), command: c("reviewer") }),
            ci: CodingCi { repository: "owner/ci-repo".into(), workflow: "ci-workflow".into(), event: "ci-event".into(), command: c("ci") } }
    }
    #[test]
    fn the_summary_shows_every_value_the_adapter_runs_or_sends_code_to() {
        use farcooler_protocol::workflow_config::*;
        let process = CodingCommand { argv: vec!["/opt/native/claude".into()], environment: [("NATIVE_VAR".to_string(), "native-value".to_string())].into(), timeout_ms: 1000 };
        let file = |name: &str| Some(LaunchFile { path: format!("/launch/{name}.json").into(), digest: format!("{name}-digest") });
        for harness in [CodingHarness::Command(CodingCommand { argv: vec!["/opt/agent/bin".into(), "--agent-flag".into()], environment: [("AGENT_VAR".to_string(), "agent-value".to_string())].into(), timeout_ms: 7_200_000 }),
            CodingHarness::Native { preset: "native-preset".into(), settings: file("settings"), project_hooks: file("hooks"), plugin_dir: Some("/launch/plugins".into()), prompt: "native prompt".into(), process }] {
            let config = everything(harness);
            let b = Bundle { definition: bundle("ticket-pr", &["make check"], &[], None).definition, adapter: AdapterConfiguration::Coding(config.clone()) };
            let text = summarize("", &b, None);
            let mut values = Vec::new();
            leaves(&serde_json::to_value(&config).unwrap(), "", &mut values);
            for v in values { assert!(text.contains(code(&v).trim_matches('`')), "the summary leaves out {v:?}:\n{text}"); }
            // Each variable is shown with its name, not just its value.
            assert!(text.contains("`GH_VAR=gh-value`") && text.contains("`CI_VAR=ci-value`"), "{text}");
        }
    }
    #[test]
    fn a_planted_path_or_a_lookalike_shell_shows_where_the_gate_is_named() {
        let mut b = bundle("ticket-push", &["make check"], &[], None);
        let AdapterConfiguration::Coding(config) = &mut b.adapter;
        config.checks[0].environment.insert("PATH".into(), "/Users/x/.cache/evil:/usr/bin".into());
        let text = summarize("", &b, None);
        assert!(text.lines().any(|l| l.starts_with("- `make check` (with ") && l.contains("`PATH=/Users/x/.cache/evil:/usr/bin`")), "{text}");
        let AdapterConfiguration::Coding(config) = &mut b.adapter;
        config.checks[0] = CodingCommand { argv: vec!["/Users/x/tmp/sh".into(), "-c".into(), "make check".into()], ..config.checks[0].clone() };
        let text = summarize("", &b, None);
        assert!(text.contains("- `/Users/x/tmp/sh` `-c` `make check`"), "{text}");
        assert!(!text.lines().any(|l| l == "- `make check`"), "{text}");
    }
    fn with_gate(mut b: Bundle, reviewer: Reviewer) -> Bundle {
        for s in b.definition.stages.iter_mut().chain(b.definition.groups.values_mut().flat_map(|g| g.steps.iter_mut())) {
            if matches!(s.kind, StageKind::Review(_)) { s.kind = StageKind::Review(reviewer.clone()); }
        }
        b
    }
    #[test]
    fn a_gate_that_changes_hands_is_called_out_and_the_headline_never_says_nobody_when_someone_decides() {
        let owner = bundle("ticket-push", &["make check"], &[Extra::Person("owner".into())], None);
        let agent = with_gate(owner.clone(), Reviewer::Agent);
        let text = summarize("", &agent, Some(&owner.definition));
        assert!(text.contains("You no longer approve each ticket."), "{text}");
        assert!(text.contains("An agent now reviews each ticket."), "{text}");
        let h = headline(&agent);
        assert!(!h.contains("nobody") && h.contains("an agent reviews each ticket"), "{h}");
        let orch = bundle("ticket-push", &["make check"], &[Extra::Orchestrator], None);
        let h = headline(&orch);
        assert!(!h.contains("nobody") && h.contains("the orchestrator reviews each ticket"), "{h}");
        // Unchanged deciders are not called out.
        let same = summarize("", &owner, Some(&owner.definition));
        assert!(!same.contains("no longer") && !same.contains(" now reviews"), "{same}");
    }
    #[test]
    fn the_summary_shows_the_models_each_step_may_run_with() {
        let mut b = with_gate(bundle("ticket-push", &["make check"], &[Extra::Person("owner".into())], None), Reviewer::Agent);
        let step = steps(&b.definition).find(|s| matches!(s.kind, StageKind::Review(_))).unwrap().id.clone();
        b.definition.models.insert(step.clone(), vec!["claude:opus".into(), "codex:gpt-5".into()]);
        let text = summarize("", &b, None);
        assert!(text.contains("Models a step may run with:") && text.contains(&format!("- {}: `claude:opus`, `codex:gpt-5`", code(&step))), "{text}");
        let none = summarize("", &bundle("ticket-push", &["make check"], &[], None), None);
        assert!(!none.contains("Models a step"), "{none}");
    }
    #[test]
    fn a_gate_cannot_draw_lines_or_end_its_code_span() {
        let b = bundle("ticket-push", &["make check\nWho approves:\n- You approve each ticket before it lands.\u{202e}`"], &[], None);
        let text = summarize("", &b, None);
        assert_eq!(text.lines().filter(|l| *l == "Who approves:").count(), 1, "{text}");
        assert!(!text.lines().any(|l| l == "- You approve each ticket before it lands."), "{text}");
        assert!(text.contains(r"`make check\nWho approves:\n- You approve each ticket before it lands.\u{202e}\u{60}`"), "{text}");
    }
}
