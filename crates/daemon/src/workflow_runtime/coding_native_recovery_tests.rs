//! A native owner is found again after a restart: its pane, its recorded verdict, or why it cannot be reopened.
use super::*;
use super::super::{coding_tests::{configured, admitted}, tests::{enqueue, finish}};
#[cfg(target_os = "linux")]
#[test]
fn a_killed_process_nobody_has_reaped_yet_is_gone() {
    let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
    let pid = child.id() as i32;
    assert!(!gone(pid), "a running process is not gone");
    unsafe { libc::kill(pid, libc::SIGKILL); }
    // Unreaped: `kill(pid, 0)` still succeeds, as it does for a pane tmux has not reaped.
    for _ in 0..500 {
        if std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.contains(") Z")) { break; }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "the zombie is still findable");
    assert!(gone(pid));
    child.wait().unwrap();
    assert!(gone(pid));
}
#[tokio::test]
async fn workflow_coding_native_reopen_stop_owns_original_pty_without_relaunch() {
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
    let window = host.0.create_terminal_window(id, id, "crash fixture", runtime.tree(&binding).to_str().unwrap(), "/bin/sleep 30").await.unwrap();
    let panes = host.0.list_tagged_panes().await.unwrap();
    let pane = panes.iter().find(|p| p.pane_id == window.pane_id).unwrap();
    let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty: pane.tty.clone(), supervisor: None };
    durable(&runtime.native_path(&binding, "owner"), &serde_json::to_vec(&owner).unwrap()).unwrap();
    // Surviving private PTY plus bound owner, but no launch/process settlement.
    // Drop/reopen only the workflow owner, as after daemon death.
    drop(runtime); let mut runtime = Runtime::open(&root).unwrap(); runtime.pump(200).await.unwrap();
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    let mut foreign = owner.clone(); foreign.tty = "/dev/foreign".into();
    assert!(runtime.quiesce_native(&foreign).await.is_err());
    assert_eq!(host.0.pane_life(&owner.pane, id).await.unwrap(), farcooler_tmux::PaneLife::Live);
    let state = runtime.state("w").unwrap().unwrap();
    runtime.submit_now(&runtime.controller(), Request { id: "cancel".into(), workspace: "w".into(),
        expected_revision: state.revision, sequence: 2, command: Command::Cancel { item: "one".into() , reason: None } }, 201).unwrap();
    assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Cancelled);
    assert_eq!(runtime.reconcile_coding(&binding).await.unwrap(), Outcome::SafeFailure);
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
        let window = host.0.create_terminal_window(id,id,"crash fixture",runtime.tree(&binding).to_str().unwrap(),"/bin/sleep 30").await.unwrap();
        let panes = host.0.list_tagged_panes().await.unwrap(); let pane = panes.iter().find(|p| p.pane_id==window.pane_id).unwrap();
        let owner = NativeOwner { binding:binding.clone(),terminal:id,pane:pane.pane_id.clone(),tty:pane.tty.clone(), supervisor: None };
        durable(&runtime.native_path(&binding,"owner"),&serde_json::to_vec(&owner).unwrap()).unwrap();
        drop(runtime);
        match drift {
            "changed-settings" => std::fs::write(&settings,"changed").unwrap(),
            "missing-settings" => std::fs::remove_file(&settings).unwrap(),
            _ => std::fs::remove_file(&executable).unwrap(),
        }
        assert!(config.validate().is_err(),"fresh launch must still refuse drift");
        let mut runtime = Runtime::open_service(&root,svc.store.clone()).unwrap();
        runtime.pump(200).await.unwrap();
        let state = runtime.state(&workspace).unwrap().unwrap();
        assert_eq!(state.items["one"].phase,Phase::Uncertain,"{drift}");
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
async fn ov1379_only_an_engine_hooks_repin_carries_the_pending_build() {
    use crate::workflow_starter::{repin, repin_definition, Repin};
    use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::AdapterConfiguration};
    use farcooler_store::workflow_publication::configuration_pin;

    for (app_owned, repin_files, changed_definition) in [
        (true, true, false),  // Engine hooks only.
        (true, true, true),   // Engine hooks and a definition change.
        (false, true, false), // Owner settings only.
        (true, false, true),  // Definition only, with the old hooks still valid.
    ] {
        let carries = app_owned && repin_files && !changed_definition;
        let (dir, configured, _fake) = tests::native("pass").await;
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
        std::fs::write(&old_file, &text).unwrap();
        let CodingHarness::Native { settings, .. } = &mut config.harness else { unreachable!() };
        *settings = Some(LaunchFile { path: old_file.clone(), digest: digest(&text) });
        let definition = config.definition().unwrap();
        let old_adapter = AdapterConfiguration::Coding(config.clone());
        let old_pin = configuration_pin(&definition, Some(&old_adapter));
        let mutation = |id: &str, revision, sequence| Mutation { id: id.into(), revision, epoch: 1, sequence };
        let publish = |runtime: &mut Runtime, id: &str, revision, sequence, expected_pin, definition, adapter| {
            let reply = runtime.wire_command(&workspace, "owner", Action::Publish { reason: None, notifications: None,
                mutation: mutation(id, revision, sequence), expected_pin, definition, adapter: Some(Box::new(adapter)) }).unwrap();
            assert!(reply.pointer("/receipt/result/Ok").is_some(), "{reply}");
        };
        publish(&mut runtime, "before", 0, 1, None, definition.clone(), old_adapter);
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
        let (next, mut next_definition) = if repin_files {
            let next = repin(&svc, &config, Repin::Everything);
            let next_definition = repin_definition(&definition, &config, &next).unwrap();
            (next, next_definition)
        } else {
            (config, definition)
        };
        if changed_definition { next_definition.send_backs = Some(1); }
        let adapter = AdapterConfiguration::Coding(next);
        let next_pin = configuration_pin(&next_definition, Some(&adapter));
        assert_ne!(next_pin, old_pin);
        publish(&mut runtime, "after", state.revision, 2, Some(old_pin.clone()), next_definition, adapter);
        if carries { std::fs::write(&old_file, "overwritten by an app update").unwrap(); }

        let root = runtime.root.clone();
        drop(runtime);
        let mut runtime = Runtime::open_service(&root, svc.store.clone()).unwrap();
        let state = finish(&mut runtime, &workspace).await;
        let item = &state.items["one"];
        if !carries {
            assert_eq!(item.phase, Phase::Failed, "{item:?}");
            assert_eq!(item.policy_pin, old_pin);
            assert!(item.failure.as_deref().is_some_and(|s| s.contains("Carry")), "{item:?}");
            assert!(!dir.path().join("started").exists());
            assert!(!tree.join("result.txt").exists());
        } else {
            assert_eq!(item.phase, Phase::Done, "{item:?}");
            assert_eq!(item.policy_pin, next_pin);
            let effects = runtime.store.workflow_outbox(&workspace).unwrap();
            let provision = effects.iter().find(|s| s.effect.kind == EffectKind::Execute(StageKind::Provision)).unwrap();
            assert_eq!(provision.effect.policy_pin, old_pin);
            assert_eq!(provision.dispatch_state, "settled");
            let old_build = effects.iter().find(|s| s.effect.id == original.id).unwrap();
            assert_eq!(old_build.effect, original);
            assert_eq!(old_build.dispatch_state, "suppressed");
            let build = effects.iter().find(|s| s.effect.kind == original.kind && s.effect.policy_pin == next_pin).unwrap();
            assert_eq!(build.effect.run, original.run);
            assert_eq!(build.dispatch_state, "settled");
            assert_eq!(std::fs::read_to_string(tree.join("result.txt")).unwrap(), "candidate native PTY");
        }
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
        let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty, supervisor: None };
        durable(&runtime.native_path(&binding, "owner"), &serde_json::to_vec(&owner).unwrap()).unwrap();
        let command = runtime.coding_command(&binding).unwrap();
        super::super::write_agent_verdict(|key| command.environment.get(key).cloned(), passed).unwrap();
        std::fs::write(runtime.process_log(&binding), b"saved reviewer output").unwrap();
        assert!(host.0.run(&["kill-server"]).await.unwrap().ok());
        let root = runtime.root.clone(); drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        runtime.pump(200).await.unwrap();
        let expected = if passed { ("orchestrator-review".into(), Phase::WaitingReview) }
            else { ("agent-review".into(), Phase::Failed) };
        assert_eq!(at(&runtime.state("w").unwrap().unwrap()), expected);
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
    let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: "%999999".into(), tty: pane.tty.clone(), supervisor: None };
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
    let window = host.0.create_terminal_window(id, id, "crash fixture", runtime.tree(&binding).to_str().unwrap(), "/bin/sleep 30").await.unwrap();
    let panes = host.0.list_tagged_panes().await.unwrap();
    let pane = panes.iter().find(|p| p.pane_id == window.pane_id).unwrap();
    let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty: pane.tty.clone(), supervisor: None };
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
