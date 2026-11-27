//! A native owner is found again after a restart: its pane, its recorded verdict, or why it cannot be reopened.
use super::*;
use super::super::{coding_tests::{configured, admitted}, tests::{enqueue, finish}};
#[tokio::test]
async fn workflow_coding_native_reopen_ends_original_pty_before_resuming() {
    assert_native_reopen(None).await;
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn workflow_coding_native_reopen_retains_a_quarantine_refusal() {
    assert_native_reopen(Some("/fixture/quarantined-agent")).await;
}

async fn assert_native_reopen(quarantined: Option<&str>) {
    let (dir, mut runtime) = configured(false, "pass").await;
    let settings = dir.path().join("settings.json");
    std::fs::write(&settings, serde_json::to_vec(&crate::hook_install::claude_settings(&runtime.root.join("h.sock"))).unwrap()).unwrap();
    let CodingHarness::Command(mut process) = runtime.coding.as_ref().unwrap().harness.clone() else { unreachable!() };
    process.argv.clear();
    runtime.coding.as_mut().unwrap().harness = CodingHarness::Native { preset: "claude".into(),
        settings: Some(LaunchFile { path: settings.canonicalize().unwrap(), digest: digest(&std::fs::read(settings).unwrap()) }),
        project_hooks: None, codex_hooks: None, plugin_dir: None, prompt: "unused crash fixture".into(), process };
    let root = runtime.root.clone();
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&runtime.coding.as_ref().unwrap()).unwrap()).unwrap();
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap(); let binding = admitted(&runtime).await;
    let host = Host(server(&root, &binding)); let id = terminal(&binding);
    let held = runtime.held(&binding, "/bin/sleep 30").unwrap();
    let window = host.0.create_terminal_window(id, id, "crash fixture", runtime.tree(&binding).to_str().unwrap(), &held).await.unwrap();
    assert!(runtime.hold_ready(&binding).await);
    let panes = host.0.list_tagged_panes().await.unwrap();
    let pane = panes.iter().find(|p| p.pane_id == window.pane_id).unwrap();
    let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty: pane.tty.clone(), hold: true, supervisor: None };
    durable(&runtime.native_path(&binding, "owner"), &serde_json::to_vec(&owner).unwrap()).unwrap();
    // A surviving private PTY with a bound owner is ended during recovery before the turn resumes.
    let mut foreign = owner.clone(); foreign.tty = "/dev/foreign".into();
    assert!(runtime.quiesce_native(&foreign).await.is_err());
    assert_eq!(host.0.pane_life(&owner.pane, id).await.unwrap(), farcooler_tmux::PaneLife::Live);
    if let Some(executable) = quarantined {
        // The launch check left its refusal on disk just before the runner went away.
        durable(&runtime.quarantine_receipt(&binding), executable.as_bytes()).unwrap();
    }
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(200).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    let item = &state.items["one"];
    assert_eq!(item.attempt, 1);
    assert_ne!(host.0.pane_life(&owner.pane, id).await.unwrap(), farcooler_tmux::PaneLife::Live);
    let outcome = runtime.reconcile_coding(&binding).await.unwrap();
    if let Some(executable) = quarantined {
        assert_eq!((&item.phase, item.attempt), (&Phase::Failed, 1));
        let reason = item.failure.as_deref().unwrap();
        assert!(reason.contains("com.apple.quarantine") && reason.contains(executable), "{reason}");
        assert!(reason.contains("/usr/bin/xattr -d com.apple.quarantine"), "{reason}");
        assert_eq!(outcome, Outcome::SafeFailureBecause(reason.into()));
        let mut needs_you = Vec::new();
        crate::engine_stuck::include_refusals(&runtime.store, &mut needs_you);
        assert!(needs_you.iter().any(|entry| entry.item == "one" && entry.question == reason));
    } else {
        assert_eq!(item.phase, Phase::Running);
        assert!(item.reason.as_deref().is_some_and(|s| s.contains("interrupted") && s.contains("resumed")));
        assert_eq!(outcome, Outcome::SafeFailure);
    }
}
#[tokio::test]
async fn workflow_service_native_stop_ignores_launch_drift_with_original_owner() {
    use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::AdapterConfiguration};
    for (agent, drift) in ["claude", "codex"].into_iter().flat_map(|agent|
        ["changed-settings", "missing-settings", "missing-executable"].map(|drift| (agent, drift))) {
        let (dir, mut runtime) = configured(false, "pass").await;
        let svc = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
        let repo_root = svc.store.create_repository_root(svc.host_id,dir.path().to_str().unwrap(),0).unwrap();
        let repo = svc.store.create_repository(svc.host_id,repo_root.id,"private",
            runtime.repository().join(".git").to_str().unwrap(),"").unwrap();
        let workspace = svc.store.ensure_main_workspace(repo.id).unwrap().id.to_string();
        runtime.store = svc.store.clone();
        let settings = dir.path().join("settings.json");
        let socket = runtime.root.join("h.sock");
        let hooks = if agent == "claude" { crate::hook_install::claude_settings(&socket) } else { crate::hook_install::native_codex_hooks(&socket) };
        std::fs::write(&settings,serde_json::to_vec(&hooks).unwrap()).unwrap();
        let executable = dir.path().join("executable");
        std::fs::copy("/bin/sh",&executable).unwrap();
        std::fs::set_permissions(&executable,std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = runtime.coding.as_mut().unwrap();
        config.checks[0].argv[0] = executable.to_string_lossy().into_owned();
        let CodingHarness::Command(mut process) = config.harness.clone() else { unreachable!() };
        process.argv.clear();
        let hooks = LaunchFile { path:settings.canonicalize().unwrap(),digest:digest(&std::fs::read(&settings).unwrap()) };
        config.harness = CodingHarness::Native { preset:agent.into(),
            settings:(agent == "claude").then(|| hooks.clone()),
            project_hooks:None, codex_hooks:(agent == "codex").then_some(hooks),plugin_dir:None,prompt:"unused".into(),process };
        let config = config.clone(); let definition = config.definition().unwrap();
        let adapter = Some(AdapterConfiguration::Coding(config.clone()));
        let pin = farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
        let m = |id: &str, revision, sequence| Mutation { id:id.into(),revision,epoch:1,sequence };
        runtime.wire_command(&workspace,"owner",Action::Publish { reason:None, notifications:None, mutation:m("p",0,1),expected_pin:None,
            definition,adapter:adapter.map(Box::new) }).unwrap();
        runtime.wire_command(&workspace,"owner",Action::Enqueue { mutation:m("e",0,1),item:"one".into(),pin:pin.clone() }).unwrap();
        runtime.pump(2).await.unwrap();
        let state = runtime.state(&workspace).unwrap().unwrap();
        let stored = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap();
        let claim = runtime.store.workflow_claim_effect(&workspace,&stored.effect.id,"crash-test",state.revision,state.epoch,10,100).unwrap().unwrap();
        assert!(runtime.store.workflow_admit_effect(&claim,10).unwrap());
        let binding = Binding::new(&workspace,&stored.effect,claim.resource_generation,0,&runtime.item_base(&workspace,&stored.effect).unwrap());
        let root = runtime.root.clone();
        let host = Host(server(&root,&binding)); let id = terminal(&binding);
        let held = runtime.held(&binding,"/bin/sleep 30").unwrap();
        let window = host.0.create_terminal_window(id,id,"crash fixture",runtime.tree(&binding).to_str().unwrap(),&held).await.unwrap();
        assert!(runtime.hold_ready(&binding).await);
        let panes = host.0.list_tagged_panes().await.unwrap(); let pane = panes.iter().find(|p| p.pane_id==window.pane_id).unwrap();
        let owner = NativeOwner { binding:binding.clone(),terminal:id,pane:pane.pane_id.clone(),tty:pane.tty.clone(),hold:true, supervisor: None };
        durable(&runtime.native_path(&binding,"owner"),&serde_json::to_vec(&owner).unwrap()).unwrap();
        drop(runtime);
        match drift {
            "changed-settings" => std::fs::write(&settings,"changed").unwrap(),
            "missing-settings" => std::fs::remove_file(&settings).unwrap(),
            _ => std::fs::remove_file(&executable).unwrap(),
        }
        assert!(config.validate().is_err(),"fresh launch must still refuse drift");
        let mut runtime = Runtime::open_service(&root,svc.store.clone()).unwrap();
        // Cancel before the recovery sweep, so Stop still owns the surviving process.
        let state = runtime.state(&workspace).unwrap().unwrap();
        assert_eq!(host.0.pane_life(&owner.pane,id).await.unwrap(),farcooler_tmux::PaneLife::Live);
        runtime.submit_now(&runtime.controller(),Request { id:"cancel".into(),workspace:workspace.clone(),
            expected_revision:state.revision,sequence:2,command:Command::Cancel { item:"one".into() , reason: None } },201).unwrap();
        assert_eq!(finish(&mut runtime,&workspace).await.items["one"].phase,Phase::Cancelled,"{drift}");
        assert_eq!(runtime.reconcile_coding(&binding).await.unwrap(),Outcome::SafeFailure);
        assert_eq!(runtime.state(&workspace).unwrap().unwrap().items["one"].policy_pin,pin);
        assert!(!dir.path().join("started").exists(),"recovery launched an adapter");
    }
}
/// ov-1259: a pinned file that changed after publishing refuses the launch before anything runs, and the item says
/// which file, on the run that `workflow get` and the board read, instead of ending Failed with no reason.
#[tokio::test]
async fn workflow_service_changed_pinned_file_names_it_on_the_item() {
    use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::AdapterConfiguration};
    let (dir, mut runtime) = configured(false, "pass").await;
    let svc = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
    let repo_root = svc.store.create_repository_root(svc.host_id,dir.path().to_str().unwrap(),0).unwrap();
    let repo = svc.store.create_repository(svc.host_id,repo_root.id,"private",
        runtime.repository().join(".git").to_str().unwrap(),"").unwrap();
    let workspace = svc.store.ensure_main_workspace(repo.id).unwrap().id.to_string();
    runtime.store = svc.store.clone();
    let settings = dir.path().join("settings.json");
    std::fs::write(&settings,serde_json::to_vec(&crate::hook_install::claude_settings(&runtime.root.join("h.sock"))).unwrap()).unwrap();
    let config = runtime.coding.as_mut().unwrap();
    let CodingHarness::Command(mut process) = config.harness.clone() else { unreachable!() };
    process.argv.clear();
    let pinned = LaunchFile { path:settings.canonicalize().unwrap(),digest:digest(&std::fs::read(&settings).unwrap()) };
    config.harness = CodingHarness::Native { preset:"claude".into(), settings:Some(pinned.clone()),
        project_hooks:None, codex_hooks:None, plugin_dir:None, prompt:"unused".into(), process };
    let definition = config.definition().unwrap();
    let adapter = Some(AdapterConfiguration::Coding(config.clone()));
    let pin = farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
    let m = |id: &str, revision, sequence| Mutation { id:id.into(),revision,epoch:1,sequence };
    runtime.wire_command(&workspace,"owner",Action::Publish { reason:None, notifications:None, mutation:m("p",0,1),expected_pin:None,
        definition,adapter:adapter.map(Box::new) }).unwrap();
    runtime.wire_command(&workspace,"owner",Action::Enqueue { mutation:m("e",0,1),item:"one".into(),pin }).unwrap();
    let root = runtime.root.clone(); drop(runtime);
    std::fs::write(&settings,"changed by an update").unwrap();
    let mut runtime = Runtime::open_service(&root,svc.store.clone()).unwrap();
    let state = finish(&mut runtime,&workspace).await;
    let failure = state.items["one"].failure.as_deref().unwrap_or_default();
    assert!(failure.contains(&pinned.path.display().to_string()) && failure.contains("re-pin"), "{failure:?}");
    assert!(!dir.path().join("started").exists(),"nothing ran on a changed pin");
}
/// Re-pin after Provision, with the old Build effect already in the outbox.
#[tokio::test]
async fn ov1425_publications_carry_compatible_runs_and_restart_other_runs() {
    use crate::workflow_starter::{repin, repin_definition, Repin};
    use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::{AdapterConfiguration, NoticePoint, NotificationPolicy}};
    use farcooler_store::workflow_publication::notification_configuration_pin;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Change { None, StartPolicy, Limits, ModelDefaults, Command, Approval, Route, Notification }

    for (app_owned, repin_files, change) in [
        (true, true, Change::None),
        (false, true, Change::None),
        (true, false, Change::StartPolicy),
        (true, false, Change::Limits),
        (true, false, Change::ModelDefaults),
        (true, false, Change::Command),
        (true, true, Change::Command),
        (true, false, Change::Approval),
        (true, true, Change::Approval),
        (true, false, Change::Route),
        (true, true, Change::Route),
        (true, false, Change::Notification),
        (true, true, Change::Notification),
    ] {
        let carries = app_owned && repin_files && change == Change::None
            || matches!(change, Change::StartPolicy | Change::Limits | Change::ModelDefaults);
        let (dir, configured, _fake) = if repin_files {
            let (dir, runtime, fake) = tests::native("pass").await;
            (dir, runtime, Some(fake))
        } else {
            let (dir, runtime) = configured(false, "pass").await;
            (dir, runtime, None)
        };
        let svc = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
        let repo_root = svc.store.create_repository_root(svc.host_id, dir.path().to_str().unwrap(), 0).unwrap();
        let mut config = configured.coding.clone().unwrap();
        let repo = svc.store.create_repository(svc.host_id, repo_root.id, "private",
            config.repository.join(".git").to_str().unwrap(), "").unwrap();
        let workspace = svc.store.ensure_main_workspace(repo.id).unwrap().id.to_string();
        drop(configured);
        let mut runtime = Runtime::open_service(&svc.root_dir().join("workflow"), svc.store.clone()).unwrap();
        let old_file = svc.root_dir().canonicalize().unwrap().join(
            if app_owned { "workflow-claude-hooks.json" } else { "owner-settings.json" });
        let text = serde_json::to_vec(&crate::hook_install::claude_settings(&runtime.root.join("h.sock"))).unwrap();
        if repin_files {
            std::fs::write(&old_file, &text).unwrap();
            let CodingHarness::Native { settings, .. } = &mut config.harness else { unreachable!() };
            *settings = Some(LaunchFile { path: old_file.clone(), digest: digest(&text) });
        }
        let mut definition = config.definition().unwrap();
        definition.schema = 2;
        if change == Change::ModelDefaults {
            definition.models.insert("build".into(), vec!["claude:opus".into(), "claude:sonnet".into()]);
        }
        let notices = NotificationPolicy { points: [NoticePoint::StageChanged].into() };
        let old_adapter = AdapterConfiguration::Coding(config.clone());
        let old_pin = notification_configuration_pin(&definition, Some(&old_adapter), Some(&notices));
        let mutation = |id: &str, revision, sequence| Mutation { id: id.into(), revision, epoch: 1, sequence };
        let publish = |runtime: &mut Runtime, id: &str, revision, sequence, expected_pin, definition, adapter, notifications| {
            let reply = runtime.wire_command(&workspace, "owner", Action::Publish { reason: None, notifications,
                mutation: mutation(id, revision, sequence), expected_pin, definition, adapter: Some(Box::new(adapter)) }).unwrap();
            assert!(reply.pointer("/receipt/result/Ok").is_some(), "{change:?}, file re-pin {repin_files}: {reply}");
        };
        publish(&mut runtime, "before", 0, 1, None, definition.clone(), old_adapter, Some(notices.clone()));
        runtime.wire_command(&workspace, "owner", Action::Enqueue { mutation: mutation("enqueue", 0, 1),
            item: "one".into(), pin: old_pin.clone() }).unwrap();
        runtime.pump(2).await.unwrap();
        let state = runtime.state(&workspace).unwrap().unwrap();
        let original = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap().effect;
        assert_eq!(original.kind, EffectKind::Execute(StageKind::Build));
        assert_eq!(original.policy_pin, old_pin);
        let tree = runtime.item_tree(&workspace, "one");
        assert!(tree.exists());

        if !app_owned {
            let mut edited = text;
            edited.push(b'\n');
            std::fs::write(&old_file, edited).unwrap();
        }
        let mut next = if repin_files { repin(&svc, &config, Repin::Everything) } else { config.clone() };
        if change == Change::Command { next.checks[0].argv.push("different argument".into()); }
        let mut next_definition = repin_definition(&definition, &config, &next).unwrap();
        let mut next_notices = notices;
        match change {
            Change::StartPolicy => next_definition.start_policy = Some(serde_json::from_value(serde_json::json!({
                "columns": ["todo"], "budget": "Unlimited",
            })).unwrap()),
            Change::Limits => {
                let mut limits = next_definition.execution_limits();
                limits.send_backs += 1;
                next_definition.send_backs = Some(limits.send_backs);
                next_definition.limits = Some(limits);
            }
            Change::ModelDefaults => next_definition.models.get_mut("build").unwrap().reverse(),
            Change::Approval => next_definition.stages[1].progression = Progression::AwaitApproval { principal: "owner".into() },
            Change::Route => { next_definition.stages[1].routes.insert("rejected".into(), Target::Done); }
            Change::Notification => { next_notices.points.insert(NoticePoint::Done); }
            Change::None | Change::Command => {}
        }
        let adapter = AdapterConfiguration::Coding(next);
        let next_pin = notification_configuration_pin(&next_definition, Some(&adapter), Some(&next_notices));
        assert_ne!(next_pin, old_pin);
        let action = Action::Publish { reason: None, notifications: Some(next_notices), mutation: mutation("after", state.revision, 2),
            expected_pin: Some(old_pin.clone()), definition: next_definition, adapter: Some(Box::new(adapter)) };
        assert_eq!(runtime.publication_preview(&workspace, "owner", &action).unwrap(), usize::from(!carries));
        assert_eq!(runtime.store.workflow_configuration(&workspace).unwrap().unwrap().pin, old_pin);
        let reply = runtime.wire_command(&workspace, "owner", action).unwrap();
        assert!(reply.pointer("/receipt/result/Ok").is_some(), "{reply}");
        if carries && repin_files { std::fs::write(&old_file, "overwritten by an app update").unwrap(); }

        let root = runtime.root.clone();
        drop(runtime);
        let mut runtime = Runtime::open_service(&root, svc.store.clone()).unwrap();
        assert!(runtime.adopt_publication(&workspace, 3).unwrap());
        let state = runtime.state(&workspace).unwrap().unwrap();
        let item = &state.items["one"];
        let effects = runtime.store.workflow_outbox(&workspace).unwrap();
        let old_build = effects.iter().find(|s| s.effect.id == original.id).unwrap();
        assert_eq!(old_build.effect, original);
        assert_eq!(old_build.dispatch_state, "suppressed");
        assert_eq!(item.policy_pin, next_pin);
        assert_eq!(item.phase, Phase::Running);
        let next = effects.iter().find(|s| s.active).unwrap();
        let notices: Vec<_> = runtime.store.workflow_notice_deliveries(&workspace).unwrap().into_iter()
            .filter(|d| d.notice.point == NoticePoint::PublicationRestarted).collect();
        if carries {
            assert_eq!(next.effect.kind, original.kind);
            assert_eq!(next.effect.run, original.run);
            assert!(item.history.is_empty());
            assert!(notices.is_empty());
            let state = finish(&mut runtime, &workspace).await;
            let item = &state.items["one"];
            assert_eq!(item.phase, Phase::Done, "{change:?}: {:?}", item.failed_reason());
        } else {
            assert_eq!(next.effect.kind, EffectKind::Execute(StageKind::Provision));
            assert_ne!(next.effect.run, original.run);
            assert_eq!(item.history.len(), 1);
            assert_eq!(notices.len(), 1);
            assert_eq!(notices[0].notice.tickets, ["one"]);
            runtime.pump(4).await.unwrap();
            let state = runtime.state(&workspace).unwrap().unwrap();
            assert!(matches!(state.items["one"].phase, Phase::Running | Phase::WaitingApproval));
            assert!(tree.exists());
            assert!(!runtime.adopt_publication(&workspace, 5).unwrap());
        }
    }
}

#[tokio::test]
async fn ov1425_restarts_reviewed_tickets_in_place_with_one_notice() {
    use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::{AdapterConfiguration, NoticePoint}};
    use farcooler_store::workflow_publication::configuration_pin;

    let (dir, fixture) = configured(false, "pass").await;
    let script = dir.path().join("adapter.py");
    let build = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, build.replace("if stage == 'build':",
        "if stage == 'build' and pathlib.Path('result.txt').exists():\n    sys.exit(0)\nif stage == 'build':")).unwrap();
    let service = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
    let repository_root = service.store.create_repository_root(service.host_id, dir.path().to_str().unwrap(), 0).unwrap();
    let config = fixture.coding.clone().unwrap();
    let repository = service.store.create_repository(service.host_id, repository_root.id, "private",
        config.repository.join(".git").to_str().unwrap(), "").unwrap();
    let workspace = service.store.ensure_main_workspace(repository.id).unwrap().id.to_string();
    drop(fixture);
    let mut runtime = Runtime::open_service(&service.root_dir().join("workflow"), service.store.clone()).unwrap();

    let mut definition = config.definition().unwrap();
    definition.stages.insert(4, config.stage("review", StageKind::Review(Reviewer::Person("owner".into())),
        "coding-review/v1", "person:owner"));
    definition.stages[5].progression = Progression::AwaitApproval { principal: "owner".into() };
    let adapter = AdapterConfiguration::Coding(config);
    let pin = configuration_pin(&definition, Some(&adapter));
    let mutation = |id: &str, revision, sequence| Mutation { id: id.into(), revision, epoch: 1, sequence };
    let publish = |id, revision, expected_pin, definition| Action::Publish { reason: None, notifications: None,
        mutation: mutation(id, revision, 1), expected_pin, definition, adapter: Some(Box::new(adapter.clone())) };
    let send = |runtime: &Runtime, action| {
        let reply = runtime.wire_command(&workspace, "owner", action).unwrap();
        assert!(reply.pointer("/receipt/result/Ok").is_some(), "{reply}");
    };
    send(&runtime, publish("original", 0, None, definition.clone()));
    let keys = ["ov-41", "ov-42"];
    for (revision, key) in keys.iter().enumerate() {
        send(&runtime, Action::Enqueue { mutation: mutation(key, revision as u64, revision as u64 + 1),
            item: (*key).into(), pin: pin.clone() });
    }
    for clock in 2..6 { runtime.pump(clock).await.unwrap(); }
    let state = runtime.state(&workspace).unwrap().unwrap();
    assert!(keys.iter().all(|key| state.items[*key].phase == Phase::WaitingReview));

    // A compatible publication preserves the original review evidence and allows it to be decided.
    definition.send_backs = Some(2);
    let compatible_pin = configuration_pin(&definition, Some(&adapter));
    send(&runtime, publish("limits", state.revision, Some(pin), definition.clone()));
    runtime.pump(6).await.unwrap();
    let mut reviewed = std::collections::BTreeMap::new();
    for (index, key) in keys.iter().enumerate() {
        let state = runtime.state(&workspace).unwrap().unwrap();
        let item = &state.items[*key];
        assert_eq!(item.phase, Phase::WaitingReview);
        assert_eq!(item.policy_pin, compatible_pin);
        send(&runtime, Action::Review { mutation: mutation(&format!("review-{key}"), state.revision, index as u64 + 3),
            item: (*key).into(), run: item.run.clone(), pin: item.pin.clone(),
            evidence: item.last_evidence.clone().unwrap(), passed: true, reason: None });
        let tree = runtime.item_tree(&workspace, key);
        let branch = adapters::git(&tree, &["symbolic-ref", "HEAD"]).await.unwrap();
        let candidate = adapters::git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
        reviewed.insert(*key, (tree, branch, candidate));
    }

    let state = runtime.state(&workspace).unwrap().unwrap();
    assert!(keys.iter().all(|key| state.items[*key].phase == Phase::WaitingApproval && state.items[*key].stage == 5));
    definition.stages[1].progression = Progression::AwaitApproval { principal: "owner".into() };
    let new_pin = configuration_pin(&definition, Some(&adapter));
    let publication = publish("approval", state.revision, Some(compatible_pin), definition);
    assert_eq!(runtime.publication_preview(&workspace, "owner", &publication).unwrap(), 2);
    send(&runtime, publication.clone());
    runtime.pump(7).await.unwrap();
    let state = runtime.state(&workspace).unwrap().unwrap();
    for key in keys {
        let item = &state.items[key];
        assert_eq!(item.phase, Phase::WaitingApproval);
        assert_eq!(item.stage, 1, "the newly required build approval has to be made");
        assert_eq!(item.policy_pin, new_pin);
        assert_eq!(item.history.len(), 1);
        let (tree, branch, candidate) = &reviewed[key];
        assert_eq!(&runtime.item_tree_for(&workspace, key, item.restart.as_deref()), tree);
        assert_eq!(&adapters::git(tree, &["symbolic-ref", "HEAD"]).await.unwrap(), branch);
        assert_eq!(&adapters::git(tree, &["rev-parse", "HEAD"]).await.unwrap(), candidate);
    }

    send(&runtime, publication);
    runtime.pump(8).await.unwrap();
    let notices: Vec<_> = runtime.store.workflow_notice_deliveries(&workspace).unwrap().into_iter()
        .filter(|d| d.notice.point == NoticePoint::PublicationRestarted).collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].notice.tickets, keys);

    for (index, key) in keys.iter().enumerate() {
        let state = runtime.state(&workspace).unwrap().unwrap();
        let item = &state.items[*key];
        send(&runtime, Action::Approve { mutation: mutation(&format!("approve-{key}"), state.revision, index as u64 + 5),
            item: (*key).into(), run: item.run.clone(), pin: item.pin.clone(), candidate: item.candidate.clone() });
    }
    for clock in 9..13 { runtime.pump(clock).await.unwrap(); }
    let state = runtime.state(&workspace).unwrap().unwrap();
    for key in keys {
        let item = &state.items[key];
        assert_eq!(item.phase, Phase::WaitingReview, "{key}: {:?}", item.failed_reason());
        assert_eq!(item.candidate.as_ref(), Some(&reviewed[key].2));
    }
}

/// ov-1130: an install removed the server after the verdict and output were saved, before settlement.
#[tokio::test]
async fn workflow_native_review_vanished_server_keeps_output_and_recorded_verdict() {
    use super::super::gate_tests::{three_gates, held_review, at};
    for passed in [true, false] {
        let (dir, mut runtime, _fake) = three_gates("never").await;
        let binding = held_review(&mut runtime).await;
        let host = Host(server(&runtime.root, &binding)); let id = terminal(&binding);
        let window = host.0.create_terminal_window(id, id, "lost reviewer", runtime.tree(&binding).to_str().unwrap(), "/bin/sleep 30").await.unwrap();
        let panes = host.0.list_tagged_panes().await.unwrap();
        let pane = panes.iter().find(|p| p.pane_id == window.pane_id).unwrap();
        // Model the vanished pane's now-empty tty with a reserved PTY. Holding its master is the
        // synchronization point: another concurrent test cannot reuse this tty before recovery reads it.
        use std::os::fd::FromRawFd;
        let (mut master, mut slave) = (-1, -1);
        let mut name = [0 as libc::c_char; 128];
        assert_eq!(unsafe { libc::openpty(&mut master, &mut slave, name.as_mut_ptr(), std::ptr::null_mut(), std::ptr::null_mut()) }, 0);
        let (_master, _slave) = unsafe { (std::fs::File::from_raw_fd(master), std::fs::File::from_raw_fd(slave)) };
        let tty = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }.to_str().unwrap().to_owned();
        let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty, hold: false, supervisor: None };
        durable(&runtime.native_path(&binding, "owner"), &serde_json::to_vec(&owner).unwrap()).unwrap();
        let command = runtime.coding_command(&binding).unwrap();
        let finding = "The candidate still drops the caller's deadline.";
        super::super::write_agent_verdict_with_summary(|key| command.environment.get(key).cloned(),
            passed, (!passed).then(|| finding.into())).unwrap();
        std::fs::write(runtime.process_log(&binding), b"saved reviewer output").unwrap();
        assert!(host.0.run(&["kill-server"]).await.unwrap().ok());
        let root = runtime.root.clone(); drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        runtime.pump(200).await.unwrap();
        let expected = if passed { ("orchestrator-review".into(), Phase::WaitingReview) }
            else { ("agent-review".into(), Phase::Failed) };
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(at(&state), expected);
        if !passed {
            assert!(state.items["one"].failed_reason().unwrap().contains(finding));
        }
        assert_eq!(std::fs::read(runtime.process_log(&binding)).unwrap(), b"saved reviewer output");
        assert!(runtime.root.join("receipts").join(format!("{}.process.json", digest(&binding.effect))).exists());
        assert!(runtime.native_path(&binding, "settled").exists());
        assert!(!dir.path().join("reviews").exists(), "recovery launched another reviewer");
    }
}
/// A review refused before opening its PTY has no owner file. Its admitted Stop must still finish.
#[tokio::test]
async fn workflow_native_review_never_launched_stop_ends_cancelled() {
    use super::super::gate_tests::{three_gates, held_review};
    let (dir, mut runtime, _fake) = three_gates("never").await;
    let binding = held_review(&mut runtime).await;
    let state = runtime.state("w").unwrap().unwrap();
    assert!(runtime.submit_now(&runtime.controller(), Request { id: "cancel".into(), workspace: "w".into(),
        expected_revision: state.revision, sequence: 2,
        command: Command::Cancel { item: "one".into(), reason: None } }, 200).unwrap().result.is_ok());
    runtime.pump(201).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Cancelled);
    let fact_path = runtime.root.join("receipts").join(format!("{}.process.json", digest(&binding.effect)));
    let fact: serde_json::Value = serde_json::from_slice(&runtime.bytes(&fact_path).unwrap()).unwrap();
    assert!(fact["exit"].is_null());
    assert!(!dir.path().join("reviews").exists());
}
/// Without a Stop, that same unstarted effect is safe to retry, including after it was marked Uncertain.
#[tokio::test]
async fn workflow_native_review_never_launched_runs_again() {
    use super::super::gate_tests::{three_gates, held_review, at};
    let (dir, mut runtime, _fake) = three_gates("subdir").await;
    let binding = held_review(&mut runtime).await;
    runtime.observe("w", &binding.effect, Outcome::Uncertain, false, 200).unwrap();
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(201).await.unwrap();
    assert_eq!(runtime.reconcile_coding(&binding).await.unwrap(), Outcome::SafeFailure);
    for time in 202..240 {
        if at(&runtime.state("w").unwrap().unwrap()).0 == "orchestrator-review" { break; }
        runtime.pump(time).await.unwrap();
    }
    assert_eq!(at(&runtime.state("w").unwrap().unwrap()), ("orchestrator-review".into(), Phase::WaitingReview));
    assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap(), "claude:sonnet\n");
}
/// Missing ownership is not proof that nothing launched while the effect's server still has a pane.
#[tokio::test]
async fn workflow_native_review_without_owner_refuses_an_unfinished_open() {
    use super::super::gate_tests::{three_gates, held_review};
    let (_dir, mut runtime, _fake) = three_gates("never").await;
    let binding = held_review(&mut runtime).await;
    let host = Host(server(&runtime.root, &binding));
    // Deliberately untagged: a tagged-only inventory must not mistake it for an empty server.
    assert!(host.0.run(&["new-session", "-d", "/bin/sleep 30"]).await.unwrap().ok());
    assert!(runtime.cancel_interrupted_native(&binding, None).await.is_err());
    assert!(!runtime.native_path(&binding, "settled").exists());
    assert!(!runtime.process_log(&binding).exists());
}
/// A tty that still has processes (including one reused since the crash) is never killed or called stopped.
#[tokio::test]
async fn workflow_native_review_vanished_pane_refuses_a_busy_tty() {
    use super::super::gate_tests::{three_gates, held_review};
    let (_dir, mut runtime, _fake) = three_gates("never").await;
    let binding = held_review(&mut runtime).await;
    let host = Host(server(&runtime.root, &binding)); let id = terminal(&binding);
    let window = host.0.create_terminal_window(id, id, "busy tty", runtime.tree(&binding).to_str().unwrap(), "/bin/sleep 30").await.unwrap();
    let panes = host.0.list_tagged_panes().await.unwrap();
    let pane = panes.iter().find(|p| p.pane_id == window.pane_id).unwrap();
    let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: "%999999".into(), tty: pane.tty.clone(), hold: false, supervisor: None };
    durable(&runtime.native_path(&binding, "owner"), &serde_json::to_vec(&owner).unwrap()).unwrap();
    std::fs::write(runtime.process_log(&binding), b"saved output").unwrap();
    assert!(runtime.cancel_interrupted_native(&binding, Some(true)).await.is_err());
    assert!(!runtime.native_path(&binding, "settled").exists());
    assert_eq!(std::fs::read(runtime.process_log(&binding)).unwrap(), b"saved output");
    assert_eq!(host.0.pane_life(&window.pane_id, id).await.unwrap(), farcooler_tmux::PaneLife::Live);
}
/// ov-1082: the daemon died while a reviewer that had recorded its verdict was still in its pane. The restart
/// settles the review from that verdict and ends the reviewer, instead of leaving the run uncertain.
#[tokio::test]
async fn workflow_native_review_recovers_from_its_recorded_verdict_after_a_crash() {
    use super::super::gate_tests::{three_gates, definition, at};
    let (dir, mut runtime, _fake) = three_gates("never").await;
    let definition = definition(&runtime); enqueue(&mut runtime, "w", "one", definition);
    for time in 2..20 {
        if at(&runtime.state("w").unwrap().unwrap()).0 == "agent-review" { break; }
        runtime.pump(time).await.unwrap();
    }
    let binding = admitted(&runtime).await;
    let root = runtime.root.clone();
    let host = Host(server(&root, &binding)); let id = terminal(&binding);
    let held = runtime.held(&binding, "/bin/sleep 30").unwrap();
    let window = host.0.create_terminal_window(id, id, "crash fixture", runtime.tree(&binding).to_str().unwrap(), &held).await.unwrap();
    assert!(runtime.hold_ready(&binding).await);
    let panes = host.0.list_tagged_panes().await.unwrap();
    let pane = panes.iter().find(|p| p.pane_id == window.pane_id).unwrap();
    let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty: pane.tty.clone(), hold: true, supervisor: None };
    durable(&runtime.native_path(&binding, "owner"), &serde_json::to_vec(&owner).unwrap()).unwrap();
    let command = runtime.coding_command(&binding).unwrap();
    super::super::write_agent_verdict(|key| command.environment.get(key).cloned(), true).unwrap();
    let log = runtime.process_log(&binding);
    durable(&log, b"evidence saved before the crash\n").unwrap();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(200).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(at(&state), ("orchestrator-review".into(), Phase::WaitingReview));
    assert!(std::fs::read_to_string(log).unwrap().contains("evidence saved before the crash"));
    assert!(!dir.path().join("reviews").exists(), "recovery launched a reviewer");
    assert_ne!(host.0.pane_life(&owner.pane, id).await.ok(), Some(farcooler_tmux::PaneLife::Live), "the reviewer was left running");
}

struct ReleaseLeftover(PathBuf);
impl Drop for ReleaseLeftover {
    fn drop(&mut self) { let _ = std::fs::write(&self.0, []); }
}

#[tokio::test]
async fn ov_1475_resolve_settles_an_uncertain_build_while_a_leftover_is_alive() {
    let (dir, mut runtime, _fake) = tests::native("leftover-resolve").await;
    let release = ReleaseLeftover(dir.path().join("release-leftover"));
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let mut build = Box::pin(runtime.execute_coding("w", &binding, cancelled));
    let waiting = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let reply = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
            if let Some(reason) = reply["state"]["items"]["one"]["reason"].as_str()
                && reason.contains("Waiting for processes left by the agent") {
                break reason.to_owned();
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    });
    let reason = tokio::select! {
        outcome = &mut build => panic!("the build settled while its detached process was still running: {outcome:?}"),
        reason = waiting => reason.expect("workflow get names the leftover while it is running"),
    };
    let pid = std::fs::read_to_string(dir.path().join("leftover")).unwrap();
    assert!(reason.contains(&format!("sh (PID {pid})")), "{reason}");
    assert!(crate::engine_stuck::list().iter().any(|stuck| stuck.item == "one" && stuck.question == reason));
    let turn = runtime.saved_native_turn(&binding).unwrap().expect("the turn is durable while the hold is still waiting");
    assert_eq!(turn.completion(&binding).unwrap(), Some(true));
    let NativeSettlement::Launched { hooks, .. } = turn else { panic!("the agent ran") };
    assert!(hooks.iter().any(|hook| hook.event == "Stop"));
    assert!(!runtime.process_fact_recorded(&binding).unwrap());

    // Lose the owned future while the hold still only has leftovers.
    drop(build);
    runtime.observe("w", &binding.effect, Outcome::Uncertain, false, 3).unwrap();
    let socket = runtime.native_hold_socket(&binding);
    let state = runtime.state("w").unwrap().unwrap();
    let item = &state.items["one"];
    let reply = runtime.resolve_wire("w", "one", &item.run, &item.pin, state.epoch).await.unwrap();
    assert_eq!(reply["receipt"]["result"], serde_json::json!({"Ok": null}));
    assert!(runtime.process_fact_recorded(&binding).unwrap(), "Resolve publishes the saved result immediately");
    let settled: NativeSettlement = serde_json::from_slice(&std::fs::read(runtime.native_path(&binding, "settled")).unwrap()).unwrap();
    let NativeSettlement::Launched { hooks, .. } = settled else { panic!("the saved turn came from a launched agent") };
    assert!(hooks.iter().any(|hook| hook.payload["last_assistant_message"] == "Saved before quiescence."));
    let state = super::super::tests::finish(&mut runtime, "w").await;
    assert_eq!(state.items["one"].phase, Phase::Done, "{state:?}");
    assert!(socket.exists(), "the hold still accounts for the detached process after settlement");
    drop(release);
    tokio::time::timeout(Duration::from_secs(5), async {
        while socket.exists() { tokio::time::sleep(Duration::from_millis(25)).await; }
    }).await.expect("the hold exits by itself after the detached process exits");
}

#[tokio::test]
async fn ov_1475_resolve_can_release_a_build_waiting_for_leftovers() {
    let (dir, mut runtime, _fake) = tests::native("leftover-resolve").await;
    let release = ReleaseLeftover(dir.path().join("release-leftover"));
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let root = runtime.root.clone();
    let handle = runtime.command_handle(&runtime.controller()).unwrap();
    let send = runtime.send.clone();
    let command = async {
        loop {
            let state = handle.state("w").await.unwrap().unwrap();
            if state.items["one"].reason.as_deref().is_some_and(|reason| reason.contains("Waiting for processes left by the agent")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let (reply, result) = tokio::sync::oneshot::channel();
        send.send(Inbox::Resolve("w".into(), "one".into(), reply)).await.ok().unwrap();
        result.await.unwrap().unwrap();
        std::fs::read_dir(root).unwrap().map(|entry| entry.unwrap().path()).find(|path| {
            path.file_name().unwrap().to_string_lossy().starts_with("h-") && path.extension().is_some_and(|ext| ext == "sock")
        }).expect("the build hold is still listening")
    };
    let (sweep, socket) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(runtime.pump(3), command)
    }).await.expect("Resolve settles while the detached process is still running");
    sweep.unwrap();
    let state = super::super::tests::finish(&mut runtime, "w").await;
    assert_eq!(state.items["one"].phase, Phase::Done, "{state:?}");
    let status = native_hold::status(&socket).await;
    assert!(matches!(status, Some(native_hold::HoldStatus::Exited { group_empty: true, leftovers, .. })
        if native_hold::leftover_reason(&leftovers).is_some()), "the hold still sees the detached process");
    drop(release);
    tokio::time::timeout(Duration::from_secs(5), async {
        while socket.exists() { tokio::time::sleep(Duration::from_millis(25)).await; }
    }).await.expect("the hold exits by itself after the detached process exits");
}

#[tokio::test]
async fn ov_1475_native_build_passes_after_an_exited_agent_leaves_a_detached_sleep() {
    let (dir, mut runtime, _fake) = tests::native("leftover-exit").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let started = tokio::time::Instant::now();
    for time in 3..40 {
        runtime.pump(time).await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        let item = &state.items["one"];
        assert_ne!(item.phase, Phase::Uncertain, "{item:?}");
        if matches!(item.phase, Phase::Done | Phase::Failed) {
            assert!(dir.path().join("leftover").exists(), "the agent launched its detached sleep");
            assert_eq!(item.phase, Phase::Done, "{item:?}");
            assert!(started.elapsed() >= Duration::from_secs(3), "the run waited for the detached sleep");
            return;
        }
    }
    panic!("the build never settled");
}
