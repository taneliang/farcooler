//! ov-1201: a publication grant lets the orchestrator change how a workflow routes its steps, never what the
//! engine runs. Every process the engine starts comes from the published adapter configuration: a check's or
//! gate's argv and environment, the harness and the hooks files it pins, the reviewer, CI and pull request
//! commands. They all run as the runner's own Unix user, whose files hold the owner's Cloudflare and GitHub
//! credentials, so a definition the orchestrator published could otherwise put the owner's secret-store,
//! security-flag, relay-flag and deploy actions into a gate command, an environment or a hook.
use super::*;
use super::admission_tests::{alices_pane, factory, mutation, ok, refused, sent};
use farcooler_protocol::{v1::Scope, workflow_commands::Action,
    workflow_config::{AdapterConfiguration, CodingCommand, CodingConfig, CodingHarness, LaunchFile}};
use farcooler_transport::Peer;
use std::collections::BTreeMap;

/// The owner-only actions, each as the command that would take it and the credential it would need.
const OWNER_ONLY: [(&str, &str, &str); 4] = [
    ("secret store", "wrangler secret put RELAY_SIGNING_KEY", "CLOUDFLARE_API_TOKEN"),
    ("security flag", "gh api --method DELETE repos/owner/overnight/branches/main/protection", "GH_TOKEN"),
    ("relay flag", "gh variable set RELAY_DEPLOY --body stable", "GH_TOKEN"),
    ("deploy", "wrangler deploy --env production", "CLOUDFLARE_API_TOKEN"),
];

/// Where a published definition could carry one of them. A step's own `operation.environment` is not one: the
/// coding adapter refuses any step whose operation isn't exactly the one it pins, which has none.
#[derive(Clone, Copy, Debug)]
enum Carrier { GateCommand, Environment, Hook }
const CARRIERS: [Carrier; 3] = [Carrier::GateCommand, Carrier::Environment, Carrier::Hook];

/// `config` and `definition` with `command` put on `path`: each one a configuration the engine accepts.
fn carrying(root: &std::path::Path, scratch: &std::path::Path, config: &CodingConfig, path: Carrier,
    command: &str, credential: &str) -> (CodingConfig, Definition)
{
    let mut config = config.clone();
    let owners = [("HOME".to_string(), "/Users/owner".to_string()), (credential.to_string(), "owner-token".to_string())];
    match path {
        Carrier::GateCommand => {
            config.checks[0] = CodingCommand { argv: vec!["/bin/sh".into(), "-c".into(), command.into()],
                environment: config.checks[0].environment.clone(), timeout_ms: 5000 };
        }
        // The credential, and a startup file the gate's shell reads, which runs the command.
        Carrier::Environment => {
            config.checks[0].environment.extend(owners);
            config.checks[0].environment.insert("PYTHONSTARTUP".into(), command.into());
        }
        Carrier::Hook => {
            // Codex's hooks, reporting to this host as the engine requires, with one more command beside them.
            let theirs = serde_json::json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": command}]}]}});
            let text = crate::hook_install::merge_codex(&theirs.to_string(), &root.join("h.sock"));
            let file = scratch.join(format!("hooks-{}.json", farcooler_workflow::digest(&command)));
            std::fs::write(&file, &text).unwrap();
            config.harness = CodingHarness::Native { preset: "codex".into(), settings: None, project_hooks: None, plugin_dir: None,
                prompt: "implement the selected task".into(),
                process: CodingCommand { argv: vec![], environment: Default::default(), timeout_ms: 5000 },
                codex_hooks: Some(LaunchFile { path: file.canonicalize().unwrap(), digest: farcooler_workflow::digest(&text.into_bytes()) }) };
        }
    }
    // The steps pin the configuration's commands, so the definition is the changed configuration's.
    let definition = config.definition().unwrap();
    (config, definition)
}

/// `rpc` publishes `definition` with `config` over the one at `expected`, at the workspace's current revision.
async fn published(runtime: &mut Runtime, rpc: &crate::rpc::RpcFactory, ws: uuid::Uuid, id: &str, expected: Option<String>,
    definition: Definition, config: CodingConfig) -> farcooler_protocol::v1::Response
{
    let revision = runtime.state(&ws.to_string()).unwrap().map_or(0, |s| s.revision);
    // Each caller's commands carry a rising sequence; one counter serves both callers.
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(2);
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    sent(runtime, rpc, ws, &Action::Publish { reason: Some(format!("Publish {id}")), notifications: None, mutation: mutation(id, revision, sequence),
        expected_pin: expected, definition, adapter: Some(Box::new(AdapterConfiguration::Coding(config))) }).await.0
}

/// Acceptance 4: with the grant on, the orchestrator can't publish any of the four owner-only actions in a gate
/// command, an environment or a hook. Each one published before ov-1201. The owner still can.
#[tokio::test]
async fn ov1201_owner_only_actions_stay_out_of_orchestrator_publications() {
    let (_coding, configured) = super::coding_tests::configured(false, "pass").await;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    // The configured fixture's commands, on the workspace's own repository.
    let mut config = configured.coding.clone().unwrap();
    let git = std::path::PathBuf::from(svc.store.get_repository(repo).unwrap().canonical_git_dir).canonicalize().unwrap();
    config.repository = git.parent().unwrap().to_path_buf();
    config.base = adapters::git(&config.repository, &["rev-parse", "HEAD"]).await.unwrap();
    adapters::git(&config.repository, &["update-ref", &config.landing_ref, &config.base]).await.unwrap();
    let definition = config.definition().unwrap();
    let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
    let w = ws.to_string();
    let (_pane, pid) = alices_pane(&svc, repo, ws, true).await;
    let short = tempfile::Builder::new().prefix("p").tempdir_in("/tmp").unwrap();
    let mut runtime = Runtime::open_service(&short.path().join("r"), svc.store.clone()).unwrap();
    let peer = Peer { client_id: None, scope: Scope::Control };
    let agent = factory(&svc, &runtime, peer.clone(), Some(pid));
    let owner = factory(&svc, &runtime, peer, None);
    ok(&sent(&mut runtime, &owner, ws, &Action::PublicationGrant { mutation: mutation("grant", 0, 1), enabled: true }).await.0);
    ok(&published(&mut runtime, &owner, ws, "base", None, definition.clone(), config.clone()).await);
    let base = runtime.store.workflow_configuration(&w).unwrap().unwrap();

    // The grant still does what it is for: a new route through the same commands publishes.
    let mut rerouted = definition.clone();
    let mut models = BTreeMap::new(); models.insert("build".to_string(), vec!["claude:opus".to_string()]);
    rerouted.models = models;
    assert_ne!(rerouted, definition);
    ok(&published(&mut runtime, &agent, ws, "reroute", Some(base.pin.clone()), rerouted, config.clone()).await);
    let current = runtime.store.workflow_configuration(&w).unwrap().unwrap();

    for (action, command, credential) in OWNER_ONLY {
        for path in CARRIERS {
            let (changed, changed_definition) = carrying(&runtime.root, short.path(), &config, path, command, credential);
            changed_definition.validate().unwrap_or_else(|e| panic!("{action} via {path:?}: {e:?}"));
            runtime.validate_publication(&changed_definition, Some(&AdapterConfiguration::Coding(changed.clone())))
                .unwrap_or_else(|e| panic!("{action} via {path:?} isn't otherwise publishable: {e:?}"));
            let id = format!("{}-{path:?}", action.replace(' ', "-"));
            let reply = published(&mut runtime, &agent, ws, &id, Some(current.pin.clone()), changed_definition.clone(), changed.clone()).await;
            assert_eq!(refused(&reply), "scope-denied", "the orchestrator published {action} via {path:?}");
            assert_eq!(runtime.store.workflow_configuration(&w).unwrap().as_ref(), Some(&current), "{action} via {path:?}");
        }
    }

    // Owner powers are kept: the owner publishes a gate that takes one, and the orchestrator can't build on it
    // with a changed environment either, since each publication is judged against the one it replaces.
    let (deploying, deploy_definition) = carrying(&runtime.root, short.path(), &config, Carrier::GateCommand,
        OWNER_ONLY[3].1, OWNER_ONLY[3].2);
    ok(&published(&mut runtime, &owner, ws, "owner-deploy", Some(current.pin.clone()),
        deploy_definition.clone(), deploying.clone()).await);
    let owners = runtime.store.workflow_configuration(&w).unwrap().unwrap();
    let mut with_token = deploying.clone();
    with_token.checks[0].environment.insert("CLOUDFLARE_API_TOKEN".into(), "owner-token".into());
    let reply = published(&mut runtime, &agent, ws, "agent-token", Some(owners.pin.clone()), deploy_definition, with_token).await;
    assert_eq!(refused(&reply), "scope-denied");
    assert_eq!(runtime.store.workflow_configuration(&w).unwrap(), Some(owners));
}

/// `definition` with step `id` waiting on `principal`'s approval, or on no one.
fn approved_by(mut definition: Definition, id: &str, principal: Option<&str>) -> Definition {
    let stage = definition.stages.iter_mut().find(|s| s.id == id).unwrap();
    stage.progression = principal.map_or(farcooler_workflow::Progression::Automatic,
        |p| farcooler_workflow::Progression::AwaitApproval { principal: p.into() });
    definition
}

/// Acceptance 4, with the adapter the owner published left exactly as it is (review a2472ebc): the orchestrator
/// can't take away or reassign the owner's approval of the step that runs an owner-only action, route around the
/// owner's review in front of it, or start running a configured command no step ran before. Each published before.
/// The same holds when the owner also approves an earlier step (review f65a4a83): each of those published too.
#[tokio::test]
async fn ov1201_orchestrator_cannot_unguard_an_owner_published_command() {
    let (_coding, configured) = super::coding_tests::configured(false, "pass").await;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let mut config = configured.coding.clone().unwrap();
    let git = std::path::PathBuf::from(svc.store.get_repository(repo).unwrap().canonical_git_dir).canonicalize().unwrap();
    config.repository = git.parent().unwrap().to_path_buf();
    config.base = adapters::git(&config.repository, &["rev-parse", "HEAD"]).await.unwrap();
    adapters::git(&config.repository, &["update-ref", &config.landing_ref, &config.base]).await.unwrap();
    let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
    let w = ws.to_string();
    let (_pane, pid) = alices_pane(&svc, repo, ws, true).await;
    let short = tempfile::Builder::new().prefix("p").tempdir_in("/tmp").unwrap();
    let mut runtime = Runtime::open_service(&short.path().join("r"), svc.store.clone()).unwrap();
    let peer = Peer { client_id: None, scope: Scope::Control };
    let agent = factory(&svc, &runtime, peer.clone(), Some(pid));
    let owner = factory(&svc, &runtime, peer, None);
    ok(&sent(&mut runtime, &owner, ws, &Action::PublicationGrant { mutation: mutation("grant", 0, 1), enabled: true }).await.0);
    let mut published_anyway = Vec::new();
    for (action, command, credential) in OWNER_ONLY {
        for path in CARRIERS {
            let (changed, definition) = carrying(&runtime.root, short.path(), &config, path, command, credential);
            let adapter = AdapterConfiguration::Coding(changed.clone());
            // The step the action runs in: a gate's command and environment run in its check, a hook in the build.
            let step = match path { Carrier::Hook => "build", _ => "check-0" };
            let name = |what: &str| format!("{}-{path:?}-{what}", action.replace(' ', "-"));
            let pin = |runtime: &Runtime| runtime.store.workflow_configuration(&w).unwrap().map(|c| c.pin);

            // The owner's own publication of each form below, then the orchestrator's attempts on it.
            let mut forms = vec![(approved_by(definition.clone(), step, Some("owner")), vec![
                ("unapproved", approved_by(definition.clone(), step, None)),
                ("reassigned", approved_by(definition.clone(), step, Some("orchestrator"))),
            ])];
            // Review f65a4a83: the owner also approves the first step; the orchestrator keeps that one only.
            let first = definition.stages[0].id.clone();
            assert_ne!(first, step);
            let twice = approved_by(definition.clone(), &first, Some("owner"));
            forms.push((approved_by(twice.clone(), step, Some("owner")), vec![
                ("unapproved-after-approval", approved_by(twice.clone(), step, None)),
                ("reassigned-after-approval", approved_by(twice.clone(), step, Some("orchestrator"))),
            ]));
            if step == "check-0" {
                // The owner's review in front of the check, which the orchestrator routes around or through.
                let mut reviewed = definition.clone();
                reviewed.schema = 2;
                let at = reviewed.stages.iter().position(|s| s.id == "check-0").unwrap();
                reviewed.stages.insert(at, farcooler_workflow::Stage { id: "owner-review".into(),
                    kind: StageKind::Review(farcooler_workflow::Reviewer::Person("owner".into())),
                    progression: farcooler_workflow::Progression::Automatic, max_attempts: 1,
                    operation: changed.operation("coding-review/v1", "person:owner"), routes: BTreeMap::new(), limit: 0 });
                // check-1 runs before the review; it now goes past it when it passes, and to it only when it fails.
                let other = reviewed.stages.iter().position(|s| s.id == "check-1").unwrap();
                let moved = reviewed.stages.remove(other);
                reviewed.stages.insert(at, moved);
                let at = at + 1;
                let mut skipped = reviewed.clone();
                skipped.stages[at - 1].routes.insert("passed".into(), farcooler_workflow::Target::Step("check-0".into()));
                skipped.stages[at - 1].routes.insert("rejected".into(), farcooler_workflow::Target::Step("owner-review".into()));
                let mut overruled = reviewed.clone();
                overruled.stages[at].routes.insert("rejected".into(), farcooler_workflow::Target::Step("check-0".into()));
                // Review f65a4a83: the owner's approval of an earlier step stands in for none of the owner's later ones.
                let earlier = |d: &Definition| approved_by(d.clone(), &d.stages[0].id.clone(), Some("owner"));
                forms.push((earlier(&reviewed), vec![("skipped-after-approval", earlier(&skipped)),
                    ("overruled-after-approval", earlier(&overruled))]));
                forms.push((reviewed, vec![("skipped", skipped), ("overruled", overruled)]));
                // A configured check no step of the owner's ran.
                let mut unused = definition.clone();
                unused.stages.retain(|s| s.id != "check-0");
                forms.push((unused, vec![("activated", definition.clone())]));
            }
            for (form, (owners, attempts)) in forms.into_iter().enumerate() {
                runtime.validate_publication(&owners, Some(&adapter)).unwrap_or_else(|e| panic!("{action} via {path:?}: {e:?}"));
                let expected = pin(&runtime);
                ok(&published(&mut runtime, &owner, ws, &name(&format!("owner{form}")), expected, owners.clone(), changed.clone()).await);
                let current = runtime.store.workflow_configuration(&w).unwrap();
                let listed = published_anyway.len();
                for (what, attempt) in attempts {
                    runtime.validate_publication(&attempt, Some(&adapter))
                        .unwrap_or_else(|e| panic!("{action} via {path:?}, {what}, isn't otherwise publishable: {e:?}"));
                    let expected = pin(&runtime);
                    let reply = published(&mut runtime, &agent, ws, &name(what), expected, attempt, changed.clone()).await;
                    // Every bypass that publishes is listed, not just the first.
                    if refused_as(&reply).as_deref() != Some("scope-denied") { published_anyway.push(name(what)); continue; }
                    assert_eq!(runtime.store.workflow_configuration(&w).unwrap(), current, "{action} via {path:?}, {what}");
                }
                // The grant still does what it is for: another approval, with the owner's kept, publishes. After a
                // bypass published, the configuration is that bypass, so there is nothing to keep.
                if published_anyway.len() > listed { continue }
                let mut more = owners.clone();
                let open = more.stages.iter_mut().find(|s| s.progression == farcooler_workflow::Progression::Automatic).unwrap();
                open.progression = farcooler_workflow::Progression::AwaitApproval { principal: "orchestrator".into() };
                let expected = pin(&runtime);
                ok(&published(&mut runtime, &agent, ws, &name(&format!("more{form}")), expected, more, changed.clone()).await);
            }
        }
    }
    assert!(published_anyway.is_empty(), "the orchestrator published {published_anyway:#?}");
}

/// The refusal's word, or `None` if `response` admitted the command.
fn refused_as(response: &farcooler_protocol::v1::Response) -> Option<String> {
    match &response.outcome {
        Some(farcooler_protocol::v1::response::Outcome::Error(e)) => Some(farcooler_core::error::word_for(e.code).to_string()),
        _ => None,
    }
}
