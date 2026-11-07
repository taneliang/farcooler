//! The landing templates pinned to a configured runner, and run against a real git repository with a
//! local bare repository standing in for the remote. No network push is ever made (ov-959).
use super::*;
use super::concurrency_tests::enqueue_item;
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
    let (bundle, notes) = lt::import_charter(&charter("- Route: push\n- Gate: `scripts/a.sh`\n- Gate: `scripts/b.sh --fast`\n- CI: none"), &config, None, &[], &[]).unwrap();
    assert_eq!(notes.len(), usize::from(!config.checks.is_empty()), "only a note that the runner's own checks were set aside: {notes:?}");
    let before = 0;
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(adapted) = &bundle.adapter;
    assert_eq!(adapted.checks.len(), before + 2);
    for (i, text) in [(before, "scripts/a.sh"), (before + 1, "scripts/b.sh --fast")] {
        assert_eq!(adapted.checks[i].argv, ["/bin/sh", "-c", text]);
        let names: Vec<_> = adapted.checks[i].environment.keys().map(String::as_str).collect();
        assert!(names == ["HOME", "PATH"] || names == ["PATH"], "a gate gets a pinned environment, not the runner's: {names:?}");
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
    let (bundle, notes) = lt::import_charter(&charter("- Gate: none"), &config, None, &[], &[]).unwrap();
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(adapted) = &bundle.adapter;
    assert_eq!(adapted.checks.last().unwrap().argv, [lt::NO_GATE]);
    assert!(notes.iter().any(|n| n.contains("always passes")), "{notes:?}");
    adapted.validate_definition(&bundle.definition).unwrap();
}

#[tokio::test]
async fn path_rules_become_a_route_step_and_one_route_for_every_path_needs_none() {
    let (_dir, config) = config_for_import().await;
    let (bundle, notes) = lt::import_charter(&charter("- Route: push\n- Route `services/site/**`: ask\n- Gate: `scripts/g.sh`"), &config, None, &[], &[]).unwrap();
    assert!(notes.iter().any(|n| n.contains("services/site/**") && n.contains("ask")), "{notes:?}");
    let ids: Vec<_> = bundle.definition.stages.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["provision", "build", "route", "approve", "join"]);
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(adapted) = &bundle.adapter;
    adapted.validate_definition(&bundle.definition).expect("a route step pins");
    // Every path asking is one more gate, not a step; every path pushing is neither.
    let (all_ask, _) = lt::import_charter(&charter("- Route: ask\n- Gate: `scripts/g.sh`"), &config, None, &[], &[]).unwrap();
    assert!(all_ask.definition.stages.iter().all(|s| s.kind != StageKind::Route));
    assert_eq!(all_ask.definition.stages.iter().filter(|s| matches!(&s.kind, StageKind::Review(Reviewer::Person(p)) if p == "owner")).count(), 1);
    let (quiet, _) = lt::import_charter(&charter("- Route: push\n- Route `docs/**`: push\n- Gate: `scripts/g.sh`"), &config, None, &[], &[]).unwrap();
    assert!(quiet.definition.stages.iter().all(|s| s.kind != StageKind::Route && !matches!(s.kind, StageKind::Review(_))));
}

#[tokio::test]
async fn a_charter_that_mixes_pull_requests_with_other_routes_is_refused_not_folded() {
    let (_dir, config) = config_for_import().await;
    for section in ["- Route: pull request\n- Route `services/site/**`: ask\n- Gate: `g`", "- Route: push\n- Route `docs/**`: pull request\n- Gate: `g`"] {
        assert_eq!(lt::import_charter(&charter(section), &config, None, &[], &[]).unwrap_err(), lt::Error::MixedRoutes, "{section}");
    }
    let (bundle, notes) = lt::import_charter(&charter("- Route: pull request\n- Route `docs/**`: pull request"), &config, None, &[], &[]).unwrap();
    assert_eq!(bundle.definition.source, "landing/ticket-pr");
    assert!(notes.iter().any(|n| n.contains("can't be published yet")), "{notes:?}");
}

#[tokio::test]
async fn a_template_draft_takes_path_routes_from_flags() {
    let (_dir, config) = config_for_import().await;
    let options = lt::Options { routes: vec!["site/**=ask".into()], ..Default::default() };
    let bundle = lt::template("ticket-push", &config, &["make check".into()], &[], &options).unwrap();
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(adapted) = &bundle.adapter;
    adapted.validate_definition(&bundle.definition).unwrap();
    assert!(bundle.definition.stages.iter().any(|s| s.kind == StageKind::Route));
    for bad in ["site/**", "=ask", "site/**=maybe", "site/**=pull_request"] {
        let options = lt::Options { routes: vec![bad.into()], ..Default::default() };
        assert!(matches!(lt::template("ticket-push", &config, &["x".into()], &[], &options).unwrap_err(), lt::Error::BadRoute(_)), "{bad}");
    }
}

#[tokio::test]
async fn a_route_step_with_rules_nobody_can_read_does_not_pin() {
    let (_dir, runtime) = lander("pass").await;
    let config = runtime.coding.clone().unwrap();
    let mut plan = plan(Template::TicketPush, vec![Gate::Command(1)]);
    plan.paths = Some(templates::PathRoutes { argument: "not rules".into(), approver: None });
    let definition = templates::build(&plan, &Host(&config)).unwrap();
    assert!(config.validate_definition(&definition).is_err());
}

fn routed_definition(runtime: &Runtime, rules: &[(&str, &str)], default: &str, approver: bool) -> Definition {
    let rules = crate::path_routes::PathRules { default: default.into(), rules: rules.iter().map(|(g, r)| (g.to_string(), r.to_string())).collect() };
    let mut p = plan(Template::TicketPush, vec![Gate::Command(1)]);
    p.paths = Some(templates::PathRoutes { argument: rules.argument(), approver: approver.then(|| "owner".into()) });
    let definition = built(runtime, &p);
    runtime.coding.as_ref().unwrap().validate_definition(&definition).unwrap();
    definition
}

#[tokio::test]
async fn a_ticket_whose_paths_earn_push_lands_without_a_person() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = routed_definition(&runtime, &[("site/**", "ask")], "push", true);
    let first = remote_head(&runtime).await;
    // The build writes result-*.txt at the top, which no rule asks about.
    at_join(&mut runtime, "quiet", &definition).await;
    assert!(command(&runtime, Command::Queue { item: "quiet".into() }).result.is_ok());
    until(&mut runtime, |s| phase_of(s, "quiet") == Some(Phase::Done)).await;
    assert_ne!(remote_head(&runtime).await, first);
    assert_eq!(outbox(&runtime, StageKind::Route), 1);
}

#[tokio::test]
async fn a_ticket_whose_paths_earn_ask_waits_for_the_owner_and_pushes_nothing_until_then() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = routed_definition(&runtime, &[("result-*.txt", "ask")], "push", true);
    let first = remote_head(&runtime).await;
    enqueue_item(&mut runtime, "w", "loud", definition.clone());
    until(&mut runtime, |s| phase_of(s, "loud") == Some(Phase::WaitingReview)).await;
    assert_eq!(remote_head(&runtime).await, first, "nothing is pushed while the owner hasn't said");
    assert_eq!(outbox(&runtime, StageKind::Push), 0);
}

#[tokio::test]
async fn a_route_the_definition_cannot_take_ends_the_run_and_pushes_nothing() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = routed_definition(&runtime, &[("result-*.txt", "pull_request")], "push", false);
    let first = remote_head(&runtime).await;
    enqueue_item(&mut runtime, "w", "odd", definition);
    until(&mut runtime, |s| phase_of(s, "odd") == Some(Phase::Failed)).await;
    assert_eq!(remote_head(&runtime).await, first);
    assert_eq!(outbox(&runtime, StageKind::Push), 0);
}

#[tokio::test]
async fn the_import_is_once_only_and_refuses_what_it_cannot_pin() {
    let (_dir, config) = config_for_import().await;
    let ok = charter("- Gate: `scripts/g.sh`");
    assert!(lt::import_charter(&ok, &config, Some("coding/v1"), &[], &[]).is_ok(), "a different, non-landing definition doesn't count");
    assert_eq!(lt::import_charter(&ok, &config, Some("landing/batched-push"), &[], &[]).unwrap_err(), lt::Error::AlreadyImported("landing/batched-push".into()));
    assert_eq!(lt::import_charter("# Charter\n", &config, None, &[], &[]).unwrap_err(), lt::Error::NoSection);
    assert!(matches!(lt::import_charter(&charter("- Gate: `x`\n- Base: release"), &config, None, &[], &[]).unwrap_err(), lt::Error::BaseDiffers { .. }));
    assert!(matches!(lt::import_charter(&charter("- Gate: `x`\n- Colour: red"), &config, None, &[], &[]).unwrap_err(), lt::Error::Charter(_)));
    let nine: String = (0..9).map(|i| format!("- Gate: `g{i}`\n")).collect();
    assert_eq!(lt::import_charter(&charter(&nine), &config, None, &[], &[]).unwrap_err(), lt::Error::TooManyGates);
}

#[tokio::test]
async fn an_imported_gate_of_each_kind_can_be_added_by_hand() {
    let (_dir, config) = config_for_import().await;
    let (bundle, _) = lt::import_charter(&charter("- Gate: `scripts/g.sh`"), &config, None, &[Extra::Person("owner".into())], &[]).unwrap();
    assert!(bundle.definition.stages.iter().any(|s| matches!(&s.kind, StageKind::Review(Reviewer::Person(_)))));
    let bundle = lt::template("ticket-push", &config, &["scripts/g.sh".into()], &[Extra::Person("owner".into())], &lt::Options::default()).unwrap();
    assert_eq!(bundle.definition.groups["landing"].limit, 1);
    assert_eq!(lt::template("shipping", &config, &[], &[], &lt::Options::default()).unwrap_err(), lt::Error::NoSuchTemplate("shipping".into()));
    assert_eq!(lt::template("batched-push", &config, &[], &[], &lt::Options::default()).unwrap_err(), lt::Error::Unsound, "a push needs a command gate");
}

/// Import a charter against the live runner's own settings, swap the imported settings in, and reopen.
async fn imported(mode: &str, gate: &str) -> (tempfile::TempDir, Runtime, lt::Bundle) {
    let (dir, runtime) = lander(mode).await;
    let config = runtime.coding.clone().unwrap();
    let section = format!("- Route: push\n- Gate: `{gate}`\n- Base: main\n- CI: none");
    let (bundle, notes) = lt::import_charter(&charter(&section), &config, None, &[], &[]).unwrap();
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

#[tokio::test]
async fn a_bundle_drafted_from_the_runners_adapter_still_publishes_after_a_ticket_has_landed() {
    let (_dir, mut runtime) = lander("pass").await;
    let definition = built(&runtime, &plan(Template::TicketPush, vec![Gate::Command(1)]));
    at_join(&mut runtime, "one", &definition).await;
    assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    // The target has moved on from the base the adapter was set up at.
    let config = runtime.coding.clone().unwrap();
    assert_ne!(remote_head(&runtime).await, config.base);
    let draft = lt::template("batched-push", &config, &["make check".into()], &[], &lt::Options::default()).unwrap();
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(drafted) = &draft.adapter;
    drafted.validate_definition(&draft.definition).unwrap();
    super::service_owner::publication_base_current(drafted).await.expect("a moved target does not refuse a draft");
}

#[tokio::test]
async fn a_base_the_target_does_not_contain_still_refuses_a_publication() {
    let (_dir, runtime) = lander("pass").await;
    let mut config = runtime.coding.clone().unwrap();
    let tree = format!("{}^{{tree}}", config.base);
    let unrelated = adapters::git(&runtime.repository(), &["commit-tree", "-m", "unrelated", &tree]).await.unwrap();
    config.base = unrelated;
    assert!(super::service_owner::publication_base_current(&config).await.is_err());
}

#[tokio::test]
async fn redrafting_lists_every_gate_so_none_is_lost_and_the_cap_is_never_reached_by_repeating() {
    let (_dir, runtime) = lander("pass").await;
    let mut config = runtime.coding.clone().unwrap();
    for round in 0..4 {
        let gates = ["a", "b", "c"].map(String::from);
        let bundle = lt::template("batched-push", &config, &gates, &[], &lt::Options::default()).unwrap();
        let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(drafted) = bundle.adapter;
        assert_eq!(drafted.checks.len(), 3, "round {round}");
        assert_eq!(bundle.definition.groups["landing"].steps.iter().filter(|s| s.kind == StageKind::Run).count(), 3, "round {round}");
        config = drafted;
    }
    // Dropping a gate is saying the shorter list.
    let bundle = lt::template("batched-push", &config, &["a".into()], &[], &lt::Options::default()).unwrap();
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(drafted) = bundle.adapter;
    assert_eq!(drafted.checks.len(), 1);
}

#[tokio::test]
async fn a_gate_gets_home_and_whatever_environment_the_draft_names() {
    let (_dir, runtime) = lander("pass").await;
    let config = runtime.coding.clone().unwrap();
    let env = ["CARGO_HOME=/opt/cargo".to_string(), "PATH=/opt/bin:/usr/bin".to_string()];
    let bundle = lt::template("ticket-push", &config, &["cargo test".into()], &[], &lt::Options { environment: env.to_vec(), ..Default::default() }).unwrap();
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(drafted) = &bundle.adapter;
    let got = &drafted.checks[0].environment;
    assert_eq!(got.get("CARGO_HOME").map(String::as_str), Some("/opt/cargo"));
    assert_eq!(got.get("PATH").map(String::as_str), Some("/opt/bin:/usr/bin"), "a named PATH wins over the default");
    assert!(got.contains_key("HOME"), "HOME is there by default");
    assert!(matches!(lt::template("ticket-push", &config, &["x".into()], &[], &lt::Options { environment: vec!["nonsense".into()], ..Default::default() }).unwrap_err(), lt::Error::BadEnvironment(_)));
    // The import takes the same setting.
    let (imported, _) = lt::import_charter(&charter("- Gate: `cargo test`"), &config, None, &[], &["CARGO_HOME=/opt/cargo".into()]).unwrap();
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(c) = &imported.adapter;
    assert!(c.checks[0].environment.contains_key("HOME") && c.checks[0].environment.contains_key("CARGO_HOME"));
}

#[tokio::test]
async fn a_draft_refuses_what_it_cannot_publish_or_what_nobody_can_decide() {
    let (_dir, runtime) = lander("pass").await;
    let config = runtime.coding.clone().unwrap();
    let gates = ["make check".to_string()];
    for who in ["alice", "orchestrator"] {
        assert_eq!(lt::template("ticket-push", &config, &gates, &[Extra::Person(who.into())], &lt::Options::default()).unwrap_err(), lt::Error::NotADecider(who.into()));
    }
    assert_eq!(lt::template("batched-push", &config, &gates, &[], &lt::Options { batch: Some(16), ..Default::default() }).unwrap_err(), lt::Error::BatchTooLarge);
    assert!(lt::template("batched-push", &config, &gates, &[], &lt::Options { batch: Some(8), ..Default::default() }).is_ok());
    assert_eq!(lt::import_charter(&charter("- Gate: `x`"), &config, None, &[Extra::Agent], &[]).unwrap_err(), lt::Error::NoReviewer);
    // The branch a push lands on is the target, whatever the local ref is called.
    let on_target = format!("- Gate: `x`\n- Base: {}", config.landing.as_ref().unwrap().target.trim_start_matches("refs/heads/"));
    assert!(lt::import_charter(&charter(&on_target), &config, None, &[], &[]).is_ok());
}

/// Pump a routed ticket until its route step is the active effect, then admit it: the state a daemon dies in.
async fn at_admitted_route(runtime: &mut Runtime, item: &str) -> Binding {
    let definition = routed_definition(runtime, &[("site/**", "ask")], "push", true);
    enqueue_item(runtime, "w", item, definition);
    for time in 2..200 {
        runtime.pump(time).await.unwrap();
        let active = runtime.store.workflow_outbox("w").unwrap().into_iter().any(|s| s.active && s.effect.kind == EffectKind::Execute(StageKind::Route));
        if active { return super::coding_tests::admitted(runtime).await; }
    }
    panic!("the route step never became active");
}

#[tokio::test]
async fn a_restart_with_the_route_admitted_reads_the_paths_again_instead_of_sticking() {
    let (_dir, mut runtime) = lander("pass").await;
    at_admitted_route(&mut runtime, "r").await;
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    let state = until(&mut runtime, |s| matches!(phase_of(s, "r"), Some(Phase::WaitingJoin | Phase::Failed | Phase::Uncertain))).await;
    assert_eq!(phase_of(&state, "r"), Some(Phase::WaitingJoin), "the route read again and the ticket moved on");
}

#[tokio::test]
async fn a_route_that_cannot_read_its_diff_fails_the_ticket_where_a_person_can_see_it() {
    let (_dir, mut runtime) = lander("pass").await;
    let binding = at_admitted_route(&mut runtime, "r").await;
    std::fs::write(runtime.tree(&binding).join("stray.txt"), "dirty").unwrap();
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    let state = until(&mut runtime, |s| matches!(phase_of(s, "r"), Some(Phase::WaitingJoin | Phase::Failed | Phase::Uncertain))).await;
    assert_eq!(phase_of(&state, "r"), Some(Phase::Failed), "failed with attempts spent, not stuck uncertain");
}
