//! Actual private native producer bytes; no owner/provider terminal or database.
use super::super::*;
use crate::workflow_notices::{self as notices, TEST_PROGRAM, TEST_LOST_REPLY};
use farcooler_workflow::{Definition, Input, Command, Grant};
const THREAD:&str="01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d";
fn enqueue(b:&Board,n:u64) { enqueue_definition(b,n,Definition::tracker("owner")); }
fn enqueue_definition(b:&Board,n:u64,definition:Definition) {
    let input=Input {schema:1,id:format!("notice-event-{n}"),workspace:b.task.workspace_id.to_string(),expected_revision:n-1,
        epoch:1,source:"controller".into(),source_sequence:n,now_ms:0,grant:Grant::Controller,busy_resources:Default::default(),
        command:Command::Enqueue {item:format!("notice-item-{n}"),definition,policy_pin:"policy".into(),resource:format!("notice-resource-{n}"),dependencies:Default::default(),initial_candidate:Some("candidate".into())}};
    assert!(b.svc.store.workflow_apply(&input).unwrap().result.is_ok());
}
async fn target(b:&Board)->Terminal {
    // Launch a private shell, replace it with the existing local process stand-in,
    // then record exactly the native role/session facts. No real harness launch.
    let term=b.svc.create_terminal(b.lane.id,"Native workflow target","shell").await.unwrap();
    let (rollout,_)=super::super::codex_turn_tests::running_rollout(b);
    b.stand_in_with(&term,"codex","codex",&format!("STAND_IN_HOLD='{}'",rollout.display())).await;
    let current=b.svc.store.get_terminal(term.id).unwrap();
    b.svc.store.update_terminal(current.id,current.resource_version,farcooler_store::models::TerminalUpdate {
        title:current.title,command_preset:"codex".into(),intent:current.intent,runtime_confirmed:current.runtime_confirmed,
        exit_code:current.exit_code,exit_signal:current.exit_signal,lease_generation:current.lease_generation,epoch:current.epoch,
        columns:current.columns,rows:current.rows }).unwrap();
    let current=b.svc.store.get_terminal(term.id).unwrap();
    b.svc.store.set_pane_mode(current.id,current.resource_version,current.pane_mode,Some(THREAD.into()),false).unwrap();
    b.svc.store.set_terminal_role(term.id,TerminalRole::Orchestrator).unwrap()
}
fn queue(b:&Board,status:i32)->(PathBuf,PathBuf) {
    let program=b.dir.path().join(format!("native-queue-{status}"));let log=b.dir.path().join("native-queue.jsonl");
    let source=format!("#!/usr/bin/python3\nimport os,sys,json\nwith open({:?},'a') as f: f.write(json.dumps([os.environ.get('CODEX_HOME'),sys.argv[1:]])+'\\n')\nsys.exit({status})\n",log.to_string_lossy());
    std::fs::write(&program,source).unwrap();
    std::fs::set_permissions(&program,std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    (program,log)
}
async fn endpoint(b:&Board,notice:&farcooler_store::workflow_notices::Notice)->notices::Endpoint {
    let owner=Arc::new(codex_turn::DiscoveryOwner::default());
    let endpoint=owner.clone().scope(notices::discover(&b.svc.store,&b.svc.inventory,notice)).await;
    assert!(owner.stop_and_reap().await);
    endpoint.expect("current private Codex target proven by actual process/rollout")
}
#[tokio::test]
async fn workflow_notice_native_backlog_equal_events_stale_epoch_and_restored_delivery() {
    let b=board().await;
    let mut definition=Definition::tracker("owner");let mut second=definition.stages[0].clone();second.id="second".into();definition.stages.push(second);
    enqueue_definition(&b,1,definition);
    let state=b.svc.store.workflow_state(&b.task.workspace_id.to_string()).unwrap().unwrap();let item=&state.items["notice-item-1"];
    let mut input=Input {schema:1,id:"approval".into(),workspace:state.workspace.clone(),expected_revision:state.revision,epoch:1,source:"decision".into(),source_sequence:1,now_ms:0,
        grant:Grant::Decision {principal:"owner".into()},busy_resources:Default::default(),command:Command::Approve {item:"notice-item-1".into(),run:item.run.clone(),pin:item.pin.clone(),candidate:item.candidate.clone()}};
    assert!(b.svc.store.workflow_apply(&input).unwrap().result.is_ok());
    let state=b.svc.store.workflow_state(&state.workspace).unwrap().unwrap();let item=&state.items["notice-item-1"];let effect=item.pending.as_ref().unwrap();
    let claim=b.svc.store.workflow_claim_effect(&state.workspace,effect,"fixture",state.revision,1,0,1000).unwrap().unwrap();assert!(b.svc.store.workflow_admit_effect(&claim,1).unwrap());
    input.id="second-phase-entry".into();input.source="adapter".into();input.expected_revision=state.revision;
    input.grant=Grant::Operation {effect:effect.clone(),run:item.run.clone(),fence:item.fence};
    input.command=Command::Observe {item:"notice-item-1".into(),effect:effect.clone(),run:item.run.clone(),fence:item.fence,outcome:farcooler_workflow::Outcome::Passed(farcooler_workflow::Evidence {candidate:item.candidate.clone().unwrap(),manifest:"manifest".into(),receipt:"receipt".into(),policy_pin:item.policy_pin.clone()})};
    assert!(b.svc.store.workflow_apply(&input).unwrap().result.is_ok());
    let (program,log)=queue(&b,0);
    TEST_PROGRAM.scope(program,async {
        let (_send,stop)=tokio::sync::watch::channel(false);let mut stop=stop;
        let pending=b.svc.store.workflow_pending_notices(0).unwrap();assert_eq!(pending.len(),2);
        let owner=Arc::new(codex_turn::DiscoveryOwner::default());
        assert!(owner.clone().scope(notices::discover(&b.svc.store,&b.svc.inventory,&pending[0].1.notice)).await.is_none());
        assert!(owner.stop_and_reap().await);assert!(!log.exists(),"offline backlog has no bytes");
        let term=target(&b).await;
        let stale=endpoint(&b,&pending[0].1.notice).await;
        let row=b.svc.store.get_terminal(term.id).unwrap();
        b.svc.store.set_pane_mode(row.id,row.resource_version,row.pane_mode,None,true).unwrap();
        notices::invoke(&b.svc.store,&b.svc.inventory,&pending[0].1.notice,stale,&mut stop).await.unwrap();
        assert!(!log.exists(),"stale epoch must receive zero bytes");
        let refused=b.svc.store.workflow_notice_delivery(&pending[0].1.notice.id).unwrap().unwrap();
        assert_eq!(refused.state,"pending");
        let binding:serde_json::Value=serde_json::from_str(refused.target.as_deref().unwrap()).unwrap();
        assert_eq!(binding["authority"]["terminal"],term.id.to_string());
        assert_eq!(binding["thread"],THREAD);
        let facts:serde_json::Value=serde_json::from_str(binding["authority"]["row_facts"].as_str().unwrap()).unwrap();
        assert_eq!(facts["epoch"],term.epoch,"attempt retains exact old epoch for reconciliation");
        // Lead ABA must invalidate the retained proof even when the terminal
        // row/session/epoch itself remains identical after restoration.
        let aba=endpoint(&b,&pending[0].1.notice).await;
        b.svc.store.set_member("notice-lead","Notice Lead",&[]).unwrap();
        b.svc.store.set_workspace_lead(b.task.workspace_id,Some("notice-lead")).unwrap();
        b.svc.store.set_workspace_lead(b.task.workspace_id,None).unwrap();
        notices::invoke(&b.svc.store,&b.svc.inventory,&pending[0].1.notice,aba,&mut stop).await.unwrap();
        assert!(!log.exists(),"lead ABA must receive zero bytes");
        for (_,d) in &pending {
            let fresh=endpoint(&b,&d.notice).await;
            notices::invoke(&b.svc.store,&b.svc.inventory,&d.notice,fresh,&mut stop).await.unwrap();
            assert_eq!(b.svc.store.workflow_notice_delivery(&d.notice.id).unwrap().unwrap().state,"handed_off");
        }
        let rows=std::fs::read_to_string(&log).unwrap();println!("actual private native argv bytes: {rows}");let rows:Vec<serde_json::Value>=rows.lines().map(|s|serde_json::from_str(s).unwrap()).collect();assert_eq!(rows.len(),2);
        let messages:Vec<serde_json::Value>=rows.iter().map(|r|{
            assert_eq!(r[1][0],"queue");assert_eq!(r[1][1],"--thread");assert_eq!(r[1][2],THREAD);
            serde_json::from_str(r[1][3].as_str().unwrap().strip_prefix("--message=").unwrap()).unwrap()
        }).collect();
        assert_ne!(messages[0]["notice"]["id"],messages[1]["notice"]["id"]);
        assert_eq!(messages[0]["notice"]["point"],messages[1]["notice"]["point"]);
        assert_eq!(messages[0]["notice"]["item"],messages[1]["notice"]["item"],"equal rendered item:phase from distinct committed events must survive");
        // Repeated identity is lookup-only even when handed a freshly proven endpoint.
        let duplicate=endpoint(&b,&pending[0].1.notice).await;
        notices::invoke(&b.svc.store,&b.svc.inventory,&pending[0].1.notice,duplicate,&mut stop).await.unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(),2);
    }).await;
}
#[tokio::test]
async fn workflow_notice_native_reply_loss_and_nonzero_are_unknown_without_resend() {
    for lost in [false,true] {
        let b=board().await;enqueue(&b,1);target(&b).await;
        let (program,log)=queue(&b,if lost{0}else{1});
        TEST_PROGRAM.scope(program,TEST_LOST_REPLY.scope(lost,async {
            let (_send,mut stop)=tokio::sync::watch::channel(false);
            let notice=b.svc.store.workflow_pending_notices(0).unwrap()[0].1.notice.clone();
            let proof=endpoint(&b,&notice).await;
            notices::invoke(&b.svc.store,&b.svc.inventory,&notice,proof,&mut stop).await.unwrap();
            println!("actual private native possibly-accepted bytes (lost_reply={lost}): {}",std::fs::read_to_string(&log).unwrap());
            assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(),1);
            b.svc.store.workflow_recover_notice_attempts().unwrap();
            assert_eq!(b.svc.store.workflow_notice_delivery(&notice.id).unwrap().unwrap().state,"unknown");
            assert!(b.svc.store.workflow_pending_notices(0).unwrap().is_empty());
            let proof=endpoint(&b,&notice).await;
            notices::invoke(&b.svc.store,&b.svc.inventory,&notice,proof,&mut stop).await.unwrap();
            assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(),1,"no blind retry after possible native acceptance");
        })).await;
    }
}

#[tokio::test]
async fn workflow_notice_native_owned_reconnect_wake_and_shutdown() {
    let b=board().await;enqueue(&b,1);enqueue(&b,2);
    let (program,log)=queue(&b,0);
    let wake=b.svc.workflow_notice_wake.clone();
    let committed_wake=wake.clone();
    b.svc.store.set_workflow_notice_waker(Some(Arc::new(move||committed_wake.notify_one())));
    let (stop,signal)=tokio::sync::watch::channel(false);
    let store=b.svc.store.clone();let inventory=b.svc.inventory.clone();
    let owner=tokio::spawn(TEST_PROGRAM.scope(program,notices::run(store,inventory,wake,signal)));
    // No target exists at initial recovery. Availability's actual Store role/
    // session write must wake the sleeping owner without a delivery timer.
    tokio::task::yield_now().await;
    target(&b).await;
    tokio::time::timeout(Duration::from_secs(20),async {
        loop {
            if b.svc.store.workflow_notice_deliveries(&b.task.workspace_id.to_string()).unwrap().iter().all(|d|d.state=="handed_off") {break}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("native lifecycle wake drained offline backlog");
    stop.send_replace(true);
    tokio::time::timeout(Duration::from_secs(20),owner).await.unwrap().unwrap().unwrap();
    assert_eq!(std::fs::read_to_string(log).unwrap().lines().count(),2);
}

#[tokio::test]
async fn workflow_notice_native_cancel_and_timeout_reap_owned_child_and_keep_unknown() {
    for cancel in [false,true] {
        let b=board().await;enqueue(&b,1);target(&b).await;
        let (program,log)=queue(&b,0);let pidfile=b.dir.path().join("queue.pid");
        let source=std::fs::read_to_string(&program).unwrap().replace("sys.exit(0)",&format!("import time\nwith open({:?},'w') as f: f.write(str(os.getpid()))\ntime.sleep(30)",pidfile.to_string_lossy()));
        std::fs::write(&program,source).unwrap();
        let (stop,mut signal)=tokio::sync::watch::channel(false);
        let notice=b.svc.store.workflow_pending_notices(0).unwrap()[0].1.notice.clone();
        let store=b.svc.store.clone();let inventory=b.svc.inventory.clone();
        let proof=TEST_PROGRAM.scope(program,endpoint(&b,&notice)).await;
        let id=notice.id.clone();
        let owner=tokio::spawn(async move{notices::invoke(&store,&inventory,&notice,proof,&mut signal).await});
        tokio::time::timeout(Duration::from_secs(5),async{while !pidfile.exists(){tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
        let pid:i32=std::fs::read_to_string(&pidfile).unwrap().parse().unwrap();
        if cancel{stop.send_replace(true);}
        tokio::time::timeout(Duration::from_secs(8),owner).await.unwrap().unwrap().unwrap();
        assert_eq!(unsafe{libc::kill(pid,0)},-1,"native child was killed AND reaped before settlement");
        assert_eq!(b.svc.store.workflow_notice_delivery(&id).unwrap().unwrap().state,"unknown");
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(),1);
        println!("owned native child pid={pid} cancel={cancel}: reaped, exact delivery Unknown");
    }
}

#[tokio::test]
async fn workflow_notice_native_inventory_recovery_without_store_write_and_unchanged_refresh_silence() {
    let b=board().await;enqueue(&b,1);let terminal=target(&b).await;
    let row=b.svc.store.get_terminal(terminal.id).unwrap();
    let notice=b.svc.store.workflow_pending_notices(0).unwrap()[0].1.notice.clone();
    // A fresh view of the SAME PRIVATE native server starts unavailable. No
    // Store writer or synthetic target grant is involved in its later recovery.
    let inventory=farcooler_tmux::LiveInventory::new(b.svc.tmux.clone());
    let (program,log)=queue(&b,0);let idle=Arc::new(tokio::sync::Notify::new());
    let (stop,signal)=tokio::sync::watch::channel(false);let wake=Arc::new(tokio::sync::Notify::new());
    let store=b.svc.store.clone();let live=inventory.clone();
    let owner=tokio::spawn(TEST_PROGRAM.scope(program,notices::TEST_IDLE.scope(idle.clone(),notices::run(store,live,wake,signal))));
    tokio::time::timeout(Duration::from_secs(5),idle.notified()).await.unwrap();
    assert!(!log.exists());assert_eq!(b.svc.store.workflow_notice_delivery(&notice.id).unwrap().unwrap().state,"pending");
    assert!(inventory.refresh().await.inventory_healthy);
    tokio::time::timeout(Duration::from_secs(8),async {
        loop {if b.svc.store.workflow_notice_delivery(&notice.id).unwrap().unwrap().state=="handed_off" {break}
            tokio::time::sleep(Duration::from_millis(20)).await;}
    }).await.expect("inventory recovery alone must wake and drain the original pending identity");
    tokio::time::timeout(Duration::from_secs(5),idle.notified()).await.unwrap();
    assert_eq!(b.svc.store.get_terminal(terminal.id).unwrap(),row,"no Store mutation manufactured a reconnect wake");
    let changes=inventory.subscribe_targets();let generation=*changes.borrow();
    for _ in 0..3 {inventory.refresh().await;}
    assert!(!changes.has_changed().unwrap(),"unchanged native refresh must not wake the sender");
    assert_eq!(*changes.borrow(),generation);
    assert!(tokio::time::timeout(Duration::from_millis(100),idle.notified()).await.is_err(),"no refresh/self-wake spin");
    stop.send_replace(true);tokio::time::timeout(Duration::from_secs(5),owner).await.unwrap().unwrap().unwrap();
    let bytes=std::fs::read_to_string(log).unwrap();assert_eq!(bytes.lines().count(),1);
    assert!(bytes.contains(&notice.id));println!("actual recovery-only native bytes, unchanged Store row: {bytes}");
}

#[tokio::test]
async fn workflow_notice_native_closed_owner_cannot_prepare_or_spawn() {
    let b=board().await;enqueue(&b,1);target(&b).await;
    let (program,log)=queue(&b,0);
    TEST_PROGRAM.scope(program,async {
        let notice=b.svc.store.workflow_pending_notices(0).unwrap()[0].1.notice.clone();
        let proof=endpoint(&b,&notice).await;
        let (owner,mut signal)=tokio::sync::watch::channel(false);
        drop(owner); // A closed channel still retains its last false bool.
        notices::invoke(&b.svc.store,&b.svc.inventory,&notice,proof,&mut signal).await.unwrap();
        let delivery=b.svc.store.workflow_notice_delivery(&notice.id).unwrap().unwrap();
        assert_eq!(delivery.state,"pending","closed owner cannot admit another native attempt");
        assert!(delivery.attempt_id.is_none(),"no durable preparation without a live owner");
        assert!(!log.exists(),"closed owner receives zero native bytes");
    }).await;
}
