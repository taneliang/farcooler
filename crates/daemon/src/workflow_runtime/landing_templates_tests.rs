//! The landing templates pinned to a configured runner, and run against a real git repository with a
//! local bare repository standing in for the remote. No network push is ever made (ov-959).
use super::*;
use super::coding_verbs_tests::{at_join, command, lander, outbox, phase_of, remote_head, until};
use crate::landing_templates::{self as lt, Extra, Host};
use farcooler_workflow::templates::{self, Gate, Plan, Template};

fn plan(t: Template, gates: Vec<Gate>) -> Plan {
    let mut p = Plan::new(t, gates);
    (p.green_base, p.every_ms, p.base_patience_ms, p.ci_patience_ms) = (false, 20, 5000, 5000);
    p
}
fn built(runtime: &Runtime, p: &Plan) -> Definition { templates::build(p, &Host(runtime.coding.as_ref().unwrap())).unwrap() }

#[tokio::test]
async fn a_batched_push_template_sends_tickets_that_queue_together_in_one_push() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = built(&runtime, &plan(Template::BatchedPush, vec![Gate::Command(1)]));
    runtime.coding.as_ref().unwrap().validate_definition(&definition).unwrap();
    let first = remote_head(&runtime).await;
    for item in ["x", "a", "b"] { at_join(&mut runtime, item, &definition).await; }
    // `x` takes the target; `a` and `b` queue behind it and travel together.
    for item in ["x", "a", "b"] { assert!(command(&runtime, Command::Queue { item: item.into() }).result.is_ok()); }
    for item in ["x", "a", "b"] { until(&mut runtime, |s| phase_of(s, item) == Some(Phase::Done)).await; }
    let landing = runtime.coding.as_ref().unwrap().landing.clone().unwrap();
    assert_eq!(adapters::git(&landing.repository, &["rev-list", "--count", &format!("{first}..{}", landing.target)]).await.unwrap(), "3", "each ticket's commit landed");
    assert_eq!(outbox(&runtime, StageKind::Push), 2, "three tickets, two pushes: the pair went together");
    assert_eq!(outbox(&runtime, StageKind::Run), 2, "and one gate judged the pair");
}

#[tokio::test]
async fn a_per_ticket_push_template_lands_each_ticket_on_its_own() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = built(&runtime, &plan(Template::TicketPush, vec![Gate::Command(1)]));
    runtime.coding.as_ref().unwrap().validate_definition(&definition).unwrap();
    for item in ["x", "a", "b"] { at_join(&mut runtime, item, &definition).await; }
    for item in ["x", "a", "b"] { assert!(command(&runtime, Command::Queue { item: item.into() }).result.is_ok()); }
    for item in ["x", "a", "b"] { until(&mut runtime, |s| phase_of(s, item) == Some(Phase::Done)).await; }
    assert_eq!(outbox(&runtime, StageKind::Push), 3, "a push for each ticket");
    assert_eq!(outbox(&runtime, StageKind::Run), 3, "and a gate for each");
}

#[tokio::test]
async fn a_failing_command_gate_keeps_the_batch_from_ever_pushing() {
    let (_dir, mut runtime) = lander("gate-fail").await;
    let definition = built(&runtime, &plan(Template::BatchedPush, vec![Gate::Command(1)]));
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &definition).await;
    assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
    // The group fails at the gate; its ticket goes back to be reworked and waits to be queued again.
    until(&mut runtime, |s| s.items.iter().any(|(_, i)| i.group_of.is_some() && i.phase == Phase::Failed)).await;
    assert_eq!(remote_head(&runtime).await, first);
    assert_eq!(outbox(&runtime, StageKind::Push), 0);
}

#[tokio::test]
async fn a_person_gate_needs_no_configured_reviewer_and_an_agent_gate_does() {
    let (_dir, runtime) = lander("pass").await;
    assert!(runtime.coding.as_ref().unwrap().reviewer.is_none());
    let person = built(&runtime, &plan(Template::BatchedPush, vec![Gate::Command(1), Gate::Person { principal: "owner".into() }]));
    runtime.coding.as_ref().unwrap().validate_definition(&person).expect("a person decides; nothing runs");
    let agent = built(&runtime, &plan(Template::BatchedPush, vec![Gate::Command(1), Gate::Agent { argument: "reviewer".into() }]));
    assert!(runtime.coding.as_ref().unwrap().validate_definition(&agent).is_err(), "no reviewer is configured to run");
}

#[tokio::test]
async fn a_gate_may_run_for_three_hours() {
    let (_dir, runtime) = lander("pass").await;
    let mut config = runtime.coding.clone().unwrap();
    config.checks[1].timeout_ms = 3 * 3_600_000;
    config.validate().expect("the landing job allowed three hours");
    config.checks[1].timeout_ms = 3 * 3_600_000 + 1;
    assert!(config.validate().is_err());
}

fn charter(section: &str) -> String { format!("# Charter\n\n## Landing\n\n{section}\n\n## Other\n\nx\n") }
async fn config_for_import() -> (tempfile::TempDir, CodingConfig) {
    let (dir, runtime) = lander("pass").await;
    let mut config = runtime.coding.clone().unwrap();
    config.landing_ref = "refs/heads/main".into();
    (dir, config)
}

#[tokio::test]
async fn a_charter_imports_to_a_batched_push_with_each_gate_a_pinned_command() {
    let (_dir, config) = config_for_import().await;
    let before = config.checks.len();
    let (bundle, notes) = lt::import_charter(&charter("- Route: push\n- Gate: `scripts/a.sh`\n- Gate: `scripts/b.sh --fast`\n- CI: none"), &config, None, &[]).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(adapted) = &bundle.adapter;
    assert_eq!(adapted.checks.len(), before + 2);
    for (i, text) in [(before, "scripts/a.sh"), (before + 1, "scripts/b.sh --fast")] {
        assert_eq!(adapted.checks[i].argv, ["/bin/sh", "-c", text]);
        assert_eq!(adapted.checks[i].environment.keys().collect::<Vec<_>>(), ["PATH"], "a gate gets a pinned environment, not the runner's");
    }
    adapted.validate().unwrap();
    adapted.validate_definition(&bundle.definition).unwrap();
    let group = &bundle.definition.groups["landing"];
    assert_eq!(group.steps.iter().filter(|s| s.kind == StageKind::Run).count(), 2);
    assert!(group.steps.iter().all(|s| s.kind != StageKind::AwaitCi), "CI: none drops the wait");
    assert_eq!(bundle.definition.source, "landing/batched-push");
}

#[tokio::test]
async fn gate_none_is_written_out_as_a_gate_that_passes() {
    let (_dir, config) = config_for_import().await;
    let (bundle, notes) = lt::import_charter(&charter("- Gate: none"), &config, None, &[]).unwrap();
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(adapted) = &bundle.adapter;
    assert_eq!(adapted.checks.last().unwrap().argv, [lt::NO_GATE]);
    assert!(notes.iter().any(|n| n.contains("always passes")), "{notes:?}");
    adapted.validate_definition(&bundle.definition).unwrap();
}

#[tokio::test]
async fn the_most_careful_route_wins_and_ask_adds_a_gate_only_a_person_decides() {
    let (_dir, config) = config_for_import().await;
    let (bundle, notes) = lt::import_charter(&charter("- Route: push\n- Route `services/site/**`: ask\n- Gate: `scripts/g.sh`"), &config, None, &[]).unwrap();
    assert!(notes.iter().any(|n| n.contains("ask") && n.contains("services/site/**")), "{notes:?}");
    let person: Vec<_> = bundle.definition.stages.iter().filter(|s| matches!(&s.kind, StageKind::Review(Reviewer::Person(p)) if p == "owner")).collect();
    assert_eq!(person.len(), 1);
    let (bundle, _) = lt::import_charter(&charter("- Route: pull request"), &config, None, &[]).unwrap();
    assert_eq!(bundle.definition.source, "landing/ticket-pr");
    assert!(bundle.definition.groups.is_empty());
}

#[tokio::test]
async fn the_import_is_once_only_and_refuses_what_it_cannot_pin() {
    let (_dir, config) = config_for_import().await;
    let ok = charter("- Gate: `scripts/g.sh`");
    assert!(lt::import_charter(&ok, &config, Some("coding/v1"), &[]).is_ok(), "a different, non-landing definition doesn't count");
    assert_eq!(lt::import_charter(&ok, &config, Some("landing/batched-push"), &[]).unwrap_err(), lt::Error::AlreadyImported("landing/batched-push".into()));
    assert_eq!(lt::import_charter("# Charter\n", &config, None, &[]).unwrap_err(), lt::Error::NoSection);
    assert!(matches!(lt::import_charter(&charter("- Gate: `x`\n- Base: release"), &config, None, &[]).unwrap_err(), lt::Error::BaseDiffers { .. }));
    assert!(matches!(lt::import_charter(&charter("- Gate: `x`\n- Colour: red"), &config, None, &[]).unwrap_err(), lt::Error::Charter(_)));
    let nine: String = (0..9).map(|i| format!("- Gate: `g{i}`\n")).collect();
    assert_eq!(lt::import_charter(&charter(&nine), &config, None, &[]).unwrap_err(), lt::Error::TooManyGates);
}

#[tokio::test]
async fn an_imported_gate_of_each_kind_can_be_added_by_hand() {
    let (_dir, config) = config_for_import().await;
    let (bundle, _) = lt::import_charter(&charter("- Gate: `scripts/g.sh`"), &config, None, &[Extra::Person("owner".into())]).unwrap();
    assert!(bundle.definition.stages.iter().any(|s| matches!(&s.kind, StageKind::Review(Reviewer::Person(_)))));
    let bundle = lt::template("ticket-push", &config, &["scripts/g.sh".into()], &[Extra::Person("owner".into())], None).unwrap();
    assert_eq!(bundle.definition.groups["landing"].limit, 1);
    assert_eq!(lt::template("shipping", &config, &[], &[], None).unwrap_err(), lt::Error::NoSuchTemplate("shipping".into()));
    assert_eq!(lt::template("batched-push", &config, &[], &[], None).unwrap_err(), lt::Error::Unsound, "a push needs a command gate");
}

/// Import a charter against the live runner's own settings, swap the imported settings in, and reopen.
async fn imported(mode: &str, gate: &str) -> (tempfile::TempDir, Runtime, lt::Bundle) {
    let (dir, runtime) = lander(mode).await;
    let config = runtime.coding.clone().unwrap();
    let section = format!("- Route: push\n- Gate: `{gate}`\n- Base: workflow-target\n- CI: none");
    let (bundle, notes) = lt::import_charter(&charter(&section), &config, None, &[]).unwrap();
    assert!(notes.iter().all(|n| !n.contains("most careful")), "{notes:?}");
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(adapted) = &bundle.adapter;
    let root = runtime.root.clone();
    std::fs::write(root.join("coding.json"), serde_json::to_vec(adapted).unwrap()).unwrap();
    drop(runtime);
    (dir, Runtime::open(&root).unwrap(), bundle)
}

#[tokio::test]
async fn an_imported_charter_gate_runs_on_the_folded_result_and_a_passing_one_lands() {
    let (_dir, mut runtime, bundle) = imported("pass", "ls result-*.txt > /dev/null").await;
    runtime.coding.as_ref().unwrap().validate_definition(&bundle.definition).unwrap();
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &bundle.definition).await;
    assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_ne!(remote_head(&runtime).await, first, "the imported gate passed, so the commit was pushed");
    assert_eq!(outbox(&runtime, StageKind::Run), 1);
    assert_eq!(outbox(&runtime, StageKind::AwaitCi), 0, "CI: none");
}

#[tokio::test]
async fn an_imported_charter_gate_that_fails_stops_the_push() {
    let (_dir, mut runtime, bundle) = imported("pass", "echo no >&2; exit 3").await;
    let first = remote_head(&runtime).await;
    at_join(&mut runtime, "one", &bundle.definition).await;
    assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
    until(&mut runtime, |s| s.items.iter().any(|(_, i)| i.group_of.is_some() && i.phase == Phase::Failed)).await;
    assert_eq!(remote_head(&runtime).await, first);
    assert_eq!(outbox(&runtime, StageKind::Push), 0);
}
