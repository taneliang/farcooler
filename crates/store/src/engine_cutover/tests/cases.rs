use super::*;
use farcooler_workflow::{Command, Definition, Grant, Input};
use rusqlite::params;

fn synthetic_uuid() -> uuid::Uuid { uuid::Uuid::from_bytes([0x11;16]) }
fn input(workspace:String,item:String,resource:&str) -> Input {
    Input {schema:1,id:"enqueue".into(),workspace,expected_revision:0,epoch:7,source:"controller".into(),source_sequence:1,now_ms:100,
        grant:Grant::Controller,busy_resources:BTreeSet::new(),command:Command::Enqueue {item,definition:Definition::coding(),policy_pin:"fixture-policy".into(),resource:resource.into(),dependencies:BTreeSet::new(),initial_candidate:None}}
}
#[test]
fn engine_cutover_f02_stack_same_branch_order_and_hold_are_exact() {
    let store=all_fixture();
    let ws=synthetic_uuid();
    let second=store.create_task(ws,"stacked",Actor::User).unwrap();
    store.conn().execute("INSERT INTO ticket_lines(task_id,stage,reason,branch,worktree_id,worktree_path,stack_parent,same_branch,pr_number,landed_sha,plan_rank,stage_since,held_off,created_at,resource_version) VALUES(?1,'waiting',' raw hold ','topic',NULL,'resources/private-pointer',?2,?2,NULL,NULL,42,3,1,4,9)",params![second.id.as_bytes().as_slice(),ws.as_bytes().as_slice()]).unwrap();
    let plan=conserved(&store,"f02");
    assert_eq!(plan.raw.tables["ticket_lines"].rows.len(),2);
    assert!(store.conn().execute("UPDATE ticket_lines SET plan_rank=0",[]).is_err());
    assert!(store.conn().execute("DELETE FROM ticket_lines",[]).is_err());
}
#[test]
fn engine_cutover_f03_agents_multitask_history_and_open_work_are_quarantined() {
    let store=all_fixture();
    let second=store.create_task(synthetic_uuid(),"second worker task",Actor::User).unwrap();
    let sub=uuid::Uuid::now_v7();
    store.conn().execute("INSERT INTO agents(id,workspace_id,role,kind,harness,subagent_id,parent_terminal_id,session_id,session_cwd,started_at) VALUES(?1,?2,'reviewer','subagent','fake','child','parent','session','resources/private-pointer',3)",params![sub.as_bytes().as_slice(),synthetic_uuid().as_bytes().as_slice()]).unwrap();
    store.conn().execute("INSERT INTO ticket_work(agent_id,task_id,role,started_at,ended_at,verdict,linked_by) VALUES(?1,?2,'review',5,NULL,NULL,'user')",params![sub.as_bytes().as_slice(),second.id.as_bytes().as_slice()]).unwrap();
    store.conn().execute("INSERT INTO ticket_work(agent_id,task_id,role,started_at,ended_at,verdict,linked_by) VALUES(?1,?2,'build',2,3,'finished','user')",params![synthetic_uuid().as_bytes().as_slice(),second.id.as_bytes().as_slice()]).unwrap();
    let plan=conserved(&store,"f03");
    assert!(plan.bindings.iter().any(|b|b.task==second.id.to_string() && b.disposition=="quarantined"));
    assert_eq!(plan.raw.tables["agents"].rows.len(),2);
    assert_eq!(store.conn().query_row("SELECT count(*) FROM terminals",[],|r|r.get::<_,usize>(0)).unwrap(),1,"pane history retained");
    assert!(store.conn().execute("UPDATE ticket_work SET verdict='passed'",[]).is_err());
}
#[test]
fn engine_cutover_f04_landing_routes_sha_events_and_ci_are_not_regenerated() {
    let store=all_fixture();
    for (n,route,state) in [(2,"pull_request","pushed"),(3,"ask","open"),(4,"push","green"),(5,"ask","red")] {
        let id=uuid::Uuid::now_v7();
        store.conn().execute("INSERT INTO landings(id,workspace_id,number,route,base,pushed_sha,state,state_since,title,collecting,actor,created_at,resource_version) VALUES(?1,?2,?3,?4,'exact-base','exact-sha',?5,2,'historical',0,'user',1,7)",params![id.as_bytes().as_slice(),synthetic_uuid().as_bytes().as_slice(),n,route,state]).unwrap();
    }
    let plan=conserved(&store,"f04");
    assert_eq!(plan.raw.tables["landings"].rows.len(),5);
    assert_eq!(plan.raw.tables["line_events"].rows.len(),1);
    assert!(store.conn().execute("UPDATE landings SET state='landed'",[]).is_err());
    assert!(store.conn().execute("UPDATE board_ci SET sha='new',status='green'",[]).is_err());
}
#[test]
fn engine_cutover_f05_budget_scopes_usage_null_cost_and_cache_split_survive() {
    let store=all_fixture();
    let id=uuid::Uuid::now_v7();
    store.conn().execute("INSERT INTO plan_budgets VALUES(?1,?2,?2,NULL,999,8)",params![id.as_bytes().as_slice(),synthetic_uuid().as_bytes().as_slice()]).unwrap();
    store.conn().execute("UPDATE agent_turn_models SET cost_micros=NULL,cache_write_1h_tokens=0,price_table=NULL",[]).unwrap();
    for (n,usage) in ["reported","partial","not_reported"].into_iter().enumerate() {
        let id=uuid::Uuid::now_v7();store.conn().execute("INSERT INTO agent_turns(id,turn_key,harness,surface,ended_at,usage) VALUES(?1,?2,'fake','pane',8,?3)",params![id.as_bytes().as_slice(),format!("usage-{n}"),usage]).unwrap();
    }
    let plan=conserved(&store,"f05");
    assert_eq!(plan.raw.tables["plan_budgets"].rows.len(),2);
    assert_eq!(plan.raw.tables["agent_turns"].rows.len(),4);
    assert!(plan.raw.tables["agent_turn_models"].rows[0].contains(&Cell::Null));
    assert!(store.conn().execute("INSERT INTO agent_turns SELECT * FROM agent_turns",[]).is_err(),"turn identity must deduplicate");
}
#[test]
fn engine_cutover_f06_all_wake_claim_states_and_compose_image_bytes_do_not_send() {
    let store=all_fixture();
    for table in ["answer_wakes","hold_wakes","message_wakes"] {
        for state in 0..4 {
            let note=uuid::Uuid::now_v7();
            store.conn().execute("INSERT INTO task_notes(id,task_id,kind,actor,at,body,extra) VALUES(?1,?2,'answer','user',8,'unicode 空\n exact ',' { } ')",params![note.as_bytes().as_slice(),synthetic_uuid().as_bytes().as_slice()]).unwrap();
            store.conn().execute(&format!("INSERT INTO {table}(note_id,task_id,enqueued_at,claimed_at,pasted_at,done_at) VALUES(?1,?2,1,?3,?4,?5)"),params![note.as_bytes().as_slice(),synthetic_uuid().as_bytes().as_slice(),(state>=1).then_some(2),(state>=2).then_some(3),(state>=3).then_some(4)]).unwrap();
        }
    }
    store.conn().execute("UPDATE compose_hold_images SET bytes=X'00ff102000',mime='image/synthetic'",[]).unwrap();
    let tmp=tempfile::tempdir().unwrap();let private=PrivateRehearsal::create(tmp.path(),"legacy-effects").unwrap();
    let plan=conserved(&store,"f06");
    assert_eq!(plan.raw.tables["answer_wakes"].rows.len(),5);
    assert!(plan.quarantines.iter().any(|q|q.reason.contains("answer_wakes")));
    assert!(store.conn().execute("UPDATE message_wakes SET claimed_at=100 WHERE done_at IS NULL",[]).is_err());
    for (kind,sql) in [("send","UPDATE message_wakes SET claimed_at=100 WHERE done_at IS NULL"),("push","UPDATE landings SET state='pushed'"),("enter","UPDATE check_in_occurrences SET state='enter_admitted'")] {
        if store.conn().execute(sql,[]).is_ok() {
            private.fake_apply(kind,&ExternalFact {operation:format!("stale-{kind}"),request_hash:"original-operation".into(),resource:format!("{kind}-resource"),incarnation:"retired-owner".into(),receipt:None}).unwrap();
        }
        assert_eq!(private.fake_counter(kind).unwrap(),0,"retired executor must refuse before fake {kind}");
    }
}
#[test]
fn engine_cutover_f07_all_finite_checkin_states_and_existing_fences_survive() {
    for schedule in ["enabled","disabled","suspended"] {
    for state in ["pending","claimed","enter_admitted","submitted","confirmed","cancelled","uncertain","resolved_uncertain"] {
        let store=all_fixture_schedule(schedule);
        store.conn().execute("UPDATE check_in_instance SET clean=0",[]).unwrap();
        // Existing target/Enter/cleanup and unclean-instance records are seeded.
        store.conn().execute("UPDATE check_in_occurrences SET state=?1,tty=NULL,process_incarnation=NULL",[state]).unwrap();
        let plan=conserved(&store,&format!("f07-{schedule}-{state}"));
        assert_eq!(plan.raw.tables["check_in_enter_fences"].rows.len(),1);
        assert!(store.conn().execute("UPDATE check_in_schedules SET state='enabled'",[]).is_err());
        assert!(store.conn().execute("DELETE FROM check_in_enter_fences",[]).is_err());
        assert!(store.conn().execute("UPDATE owner_devices SET state='check'",[]).is_err());
    }
    }
}
#[test]
fn engine_cutover_f09_existing_engine_claims_pins_and_duplicate_receipts_survive() {
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("engine.sqlite");
    let old=Store::open_fixture(&path).unwrap();
    let repo=old.register_repository_for_test("Engine");let ws=old.ensure_main_workspace(repo).unwrap();
    let definition=Definition::coding();
    old.workflow_wire_write(&ws.id.to_string(),"publish","authenticated-fixture-publish",crate::workflow_publication::Write::Publish {expected_pin:None,definition:&definition,adapter:None,notifications:None,revision:0}).unwrap();
    let mut command=input(ws.id.to_string(),"standalone".into(),"engine-resource");
    if let Command::Enqueue {policy_pin,..}=&mut command.command {*policy_pin=definition.pin();}
    let receipt=old.workflow_apply(&command).unwrap();assert_eq!(receipt.result,Ok(1));
    let effect=receipt.effects[0].clone();
    let claim=old.workflow_claim_effect(&command.workspace,&effect,"old-worker",1,7,100,200).unwrap().unwrap();
    old.conn().execute("UPDATE workflow_resource_generations SET generation=8",[]).unwrap();
    old.conn().execute("UPDATE workflow_resources SET generation=8",[]).unwrap();
    let plan=conserved(&old,"f09");
    old.engine_release(&plan,&joined(&old)).unwrap();
    assert!(old.workflow_apply(&command).is_err(),"old Store handle cannot write after transfer");
    assert!(!old.workflow_admit_effect(&claim,101).unwrap());
    let new=Store::open_fixture(&path).unwrap();
    assert_eq!(new.workflow_apply(&command).unwrap(),receipt,"duplicate gets original receipt");
    let newclaim=new.workflow_claim_effect(&command.workspace,&effect,"new-worker",1,7,300,400).unwrap().unwrap();
    assert_eq!(newclaim.resource_generation,9);
    assert!(!new.workflow_admit_effect(&claim,101).unwrap());
    let mut send=0;
    if new.workflow_admit_effect(&newclaim,301).unwrap() {send+=1;}
    if new.workflow_admit_effect(&newclaim,302).unwrap() {send+=1;}
    assert_eq!(send,1,"admit CAS is the fake adapter call boundary");
    assert_eq!(new.workflow_events(&command.workspace).unwrap().len(),1);
    assert_eq!(plan.raw.tables["workflow_receipts"].rows.len(),1);
    assert_eq!(plan.raw.tables["workflow_bundles"].rows.len(),1);
    let mut replay=farcooler_workflow::State::empty(command.workspace.clone(),7);
    for event in new.workflow_events(&command.workspace).unwrap() {replay=farcooler_workflow::step(&replay,&event).unwrap().state;}
    assert_eq!(Some(replay),new.workflow_state(&command.workspace).unwrap());
}
#[test]
fn engine_cutover_f11_restored_pending_cannot_replay_a_postbackup_push() {
    let tmp=tempfile::tempdir().unwrap();let private=PrivateRehearsal::create(tmp.path(),"f11").unwrap();
    let path=private.live().join("db.sqlite");
    let old=Store::open_fixture(&path).unwrap();let repo=old.register_repository_for_test("Push");let ws=old.ensure_main_workspace(repo).unwrap();
    let mut command=input(ws.id.to_string(),"push-item".into(),"branch-resource");
    if let Command::Enqueue {definition,policy_pin,..}=&mut command.command {
        definition.source="private-fake-push".into();definition.max_reworks=0;definition.rework_target=None;
        definition.stages=vec![farcooler_workflow::Stage {id:"fake-push".into(),kind:farcooler_workflow::StageKind::Action("fake-git-push".into()),progression:farcooler_workflow::Progression::Automatic,max_attempts:1,operation:farcooler_workflow::OperationSpec {adapter_version:"fake-git/v1".into(),arguments:vec!["push".into(),"exact-operation".into()],environment:BTreeMap::new()}}];
        *policy_pin=definition.pin();
    }
    let receipt=old.workflow_apply(&command).unwrap();drop(old);
    let backup=private.backup("before-push").unwrap();
    let store=Store::open_fixture(&path).unwrap();
    let claim=store.workflow_claim_effect(&command.workspace,&receipt.effects[0],"push-owner",1,7,100,200).unwrap().unwrap();
    assert!(store.workflow_admit_effect(&claim,101).unwrap());
    let fact=ExternalFact {operation:receipt.effects[0].clone(),request_hash:farcooler_workflow::digest(&store.workflow_outbox(&command.workspace).unwrap()[0].effect),resource:"branch-resource".into(),incarnation:"admitted-owner".into(),receipt:None};
    private.retain_effect(&fact).unwrap();
    let remote=private.fake_apply("push",&fact).unwrap();
    assert!(private.reconcile(&fact,Some(&remote)).unwrap());
    let write=ExternalFact {operation:"fake-mcp-write".into(),resource:"document-resource".into(),..fact.clone()};
    private.retain_effect(&write).unwrap();private.fake_apply("write",&write).unwrap();
    drop(store);
    let restored=private.restore("before-push","after-restore",&backup).unwrap();
    let store=Store::open_fixture(restored.join("db.sqlite")).unwrap();
    assert_eq!(store.engine_authority().unwrap().phase,"quiesced");
    assert!(store.workflow_claim_effect(&command.workspace,&receipt.effects[0],"restored-owner",1,7,100,200).unwrap().is_none());
    assert!(!private.reconcile(&fact,None).unwrap());
    assert_eq!(private.fake_counter("push").unwrap(),1,"uncertain remote push is never repeated");
    assert_eq!(private.fake_counter("write").unwrap(),1,"ambiguous fake write is never repeated");
    assert_eq!(private.fake_counter("send").unwrap(),0);
    assert_eq!(private.fake_counter("enter").unwrap(),0);
    assert_eq!(store.conn().query_row("SELECT count(*) FROM engine_cutover_external",[],|r|r.get::<_,usize>(0)).unwrap(),2);
    let plan=preflight(&store.conn()).unwrap();
    store.engine_commit_for_test(&plan,&proof(&store),"restored-forward").unwrap();
    store.engine_release(&plan,&joined(&store)).unwrap();drop(store);
    let store=Store::open_fixture(restored.join("db.sqlite")).unwrap();
    if let Some(retry)=store.workflow_claim_effect(&command.workspace,&receipt.effects[0],"restored-new-owner",1,7,300,400).unwrap() {
        if store.workflow_admit_effect(&retry,301).unwrap() {let _=private.fake_apply("push",&fact);}
    }
    assert_eq!(private.fake_counter("push").unwrap(),1,"post-backup quarantine must survive forward release");
    save_json("f11-external-reconciliation",&serde_json::json!({"admitted_fact":fact,"exact_fake_remote_receipt":remote,"ambiguous_write":write,"backup":backup,"push":private.fake_counter("push").unwrap(),"write":private.fake_counter("write").unwrap()}));
    drop(store);
    save_db("f11-before-push",&private.root().join("before-push/db.sqlite"));
    save_db("f11-restored-fenced",&restored.join("db.sqlite"));
}

#[test]
fn engine_cutover_precommit_transaction_failure_rolls_back_every_workspace_and_ledger() {
    let store=fixture();
    let repo=store.register_repository_for_test("Second repo");
    let ws=store.ensure_main_workspace(repo).unwrap();store.create_task(ws.id,"second",Actor::User).unwrap();
    let plan=preflight(&store.conn()).unwrap();store.engine_quiesce(&joined(&store)).unwrap();
    store.conn().execute_batch("CREATE TRIGGER injected BEFORE INSERT ON engine_cutover_attempts BEGIN SELECT RAISE(ABORT,'cutover mid-map interruption'); END;").unwrap();
    assert!(store.engine_commit_for_test(&plan,&proof(&store),"interrupted").is_err());
    assert_eq!(store.engine_authority().unwrap().phase,"quiesced");
    for table in ["engine_cutover_workspaces","engine_cutover_items","engine_cutover_ledger"] {
        assert_eq!(store.conn().query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get::<_,usize>(0)).unwrap(),0,"partial {table}");
    }
    store.conn().execute_batch("DROP TRIGGER injected;").unwrap();
    store.engine_commit_for_test(&plan,&proof(&store),"restored").unwrap();
    store.engine_verify(&plan).unwrap();
}
#[test]
fn engine_cutover_interrupt_reopen_normalization_and_forward_recovery() {
    let tmp=tempfile::tempdir().unwrap();let private=PrivateRehearsal::create(tmp.path(),"interrupt").unwrap();
    let path=private.live().join("db.sqlite");let store=Store::open_fixture(&path).unwrap();
    let repo=store.register_repository_for_test("Interrupted");let ws=store.ensure_main_workspace(repo).unwrap();
    let task=store.create_task(ws.id,"raw wait",Actor::User).unwrap();
    store.conn().execute("UPDATE tasks SET wait_kind='future-kind',wait_event=' raw invalid event ' WHERE id=?1",[task.id.as_bytes().as_slice()]).unwrap();
    let plan=preflight(&store.conn()).unwrap();
    private.record(0,&JournalEntry {attempt:"interrupt".into(),stage:"prepared".into(),evidence:plan.raw.hash()}).unwrap();
    store.engine_quiesce(&joined(&store)).unwrap();drop(store); // interruption before commit
    let store=Store::open_fixture(&path).unwrap();
    let conn=Connection::open(&path).unwrap();assert_eq!(private.recover(&conn,"interrupt",1).unwrap().phase,"quiesced");drop(conn);
    store.engine_commit_for_test(&plan,&proof(&store),"interrupt").unwrap();drop(store); // committed DB, stale journal
    let conn=Connection::open(&path).unwrap();assert_eq!(private.recover(&conn,"interrupt",2).unwrap().phase,"committed");drop(conn);
    let store=Store::open_fixture(&path).unwrap();store.engine_verify(&plan).unwrap();let first=Manifest::capture(&store.conn()).unwrap();drop(store);
    let store=Store::open_fixture(&path).unwrap();assert_eq!(first,Manifest::capture(&store.conn()).unwrap());
    assert!(store.get_task(task.id).is_ok(),"retained board reader remains usable");
    assert!(private.record(2,&JournalEntry {attempt:"reuse".into(),stage:"prepared".into(),evidence:"overwrite".into()}).is_err());
    if let Ok(output)=std::env::var("OV936_OLD_READER_FIXTURE") { if !output.is_empty() {
        drop(store);
        let output=std::path::Path::new(&output);
        assert!(output.starts_with("/private/tmp") && output.parent().unwrap().file_name().unwrap().to_string_lossy().starts_with("ov936-old-reader-"));
        std::fs::create_dir(output.parent().unwrap()).unwrap();
        std::fs::copy(&path,output).unwrap();
    }}
}
#[test]
fn engine_cutover_unknown_schema_field_and_standalone_are_retained_but_block_release() {
    let store=fixture();
    store.conn().execute_batch("ALTER TABLE tasks ADD COLUMN future_field BLOB; UPDATE tasks SET future_field=X'ff00'; CREATE TABLE future_table(key BLOB PRIMARY KEY,value TEXT); INSERT INTO future_table VALUES(X'01','unknown raw');").unwrap();
    let plan=conserved(&store,"unknown");
    assert!(plan.quarantines.iter().any(|q|q.resource=="schema:unknown"));
    assert!(store.engine_release(&plan,&joined(&store)).is_err());
    assert!(plan.raw.tables.contains_key("future_table"));
}
#[test]
fn engine_cutover_stale_source_and_per_field_corruption_refuse_release() {
    let store=fixture();let plan=preflight(&store.conn()).unwrap();
    store.engine_quiesce(&joined(&store)).unwrap();
    store.conn().execute("UPDATE tasks SET title='changed after preflight'",[]).unwrap();
    assert!(store.engine_commit_for_test(&plan,&proof(&store),"changed").is_err());
    let plan=preflight(&store.conn()).unwrap();
    store.engine_commit_for_test(&plan,&proof(&store),"fresh").unwrap();store.engine_verify(&plan).unwrap();
    store.conn().execute("UPDATE tasks SET title='same counts corrupted bytes'",[]).unwrap();
    assert!(store.engine_verify(&plan).is_err());assert!(store.engine_release(&plan,&joined(&store)).is_err());
}
#[test]
fn engine_cutover_f12_pre58_upgrade_is_a_separate_actual_registry_case() {
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("pre58.sqlite");
    let conn=Connection::open(&path).unwrap();conn.execute_batch(include_str!("../../../fixtures/before_the_line.sql")).unwrap();
    assert_eq!(crate::migrate::read_schema_version(&conn).unwrap(),57);drop(conn);
    let store=Store::open_fixture(&path).unwrap();assert_eq!(store.schema().unwrap().version,66);
    assert_eq!(store.engine_authority().unwrap().phase,"legacy");
    assert_eq!(store.conn().query_row("SELECT count(*) FROM tasks",[],|r|r.get::<_,usize>(0)).unwrap(),11);
    let plan=conserved(&store,"pre58-after-upgrade");assert_eq!(plan.bindings.len(),11);
}
#[test]
fn engine_cutover_closed_backup_controls_reject_wal_collision_and_resource_corruption() {
    let tmp=tempfile::tempdir().unwrap();let private=PrivateRehearsal::create(tmp.path(),"backup-controls").unwrap();
    let path=private.live().join("db.sqlite");let store=Store::open_fixture(&path).unwrap();drop(store);
    std::fs::write(private.live().join("db.sqlite-wal"),b"truncated synthetic WAL").unwrap();
    assert!(private.backup("wal-must-refuse").is_err());
    // Delete only this test's explicit synthetic sidecar; keep original failed
    // bytes in a separate evidence file rather than cleanup an unknown path.
    std::fs::rename(private.live().join("db.sqlite-wal"),private.live().join("truncated-wal-evidence")).unwrap();
    std::fs::write(private.live().join("resource"),b"original").unwrap();
    let backup=private.backup("closed").unwrap();
    std::fs::write(private.root().join("closed/resource"),b"corrupted same count").unwrap();
    assert!(private.restore("closed","corrupt-restore",&backup).is_err());
}

#[test]
#[ignore = "owned subprocess helper; invoked only by engine_cutover_process_kill_boundaries"]
fn engine_cutover_process_child() {
    let directory=std::env::var("OV936_CRASH_ROOT").unwrap();let directory=std::path::Path::new(&directory);
    assert!(directory.starts_with(std::env::temp_dir()));
    let store=Store::open_fixture(directory.join("db.sqlite")).unwrap();
    let repo=store.register_repository_for_test("Crash boundary");let ws=store.ensure_main_workspace(repo).unwrap();store.create_task(ws.id,"preserve on kill",Actor::User).unwrap();
    let plan=preflight(&store.conn()).unwrap();
    let mut file=std::fs::File::create(directory.join("plan.json")).unwrap();
    use std::io::Write;
    file.write_all(&serde_json::to_vec(&plan).unwrap()).unwrap();file.sync_all().unwrap();
    store.engine_quiesce(&joined(&store)).unwrap();
    if std::env::var("OV936_CRASH_BOUNDARY").unwrap()=="after" {store.engine_commit_for_test(&plan,&proof(&store),"killed-after-commit").unwrap();}
    let marker=std::fs::File::create(directory.join("kill-now")).unwrap();marker.sync_all().unwrap();
    loop {std::thread::park();}
}
#[test]
fn engine_cutover_process_kill_boundaries() {
    for boundary in ["before","after"] {
        let tmp=tempfile::tempdir().unwrap();
        let mut child=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored","--exact","engine_cutover::tests::cases::engine_cutover_process_child","--nocapture"])
            .env("OV936_CRASH_ROOT",tmp.path()).env("OV936_CRASH_BOUNDARY",boundary)
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        let start=std::time::Instant::now();
        while !tmp.path().join("kill-now").exists() && start.elapsed()<std::time::Duration::from_secs(10) {
            if child.try_wait().unwrap().is_some() {break;}
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let reached=tmp.path().join("kill-now").exists();
        child.kill().unwrap();let output=child.wait_with_output().unwrap();
        println!("actual child SIGKILL boundary={boundary} status={} stdout={}",output.status,String::from_utf8_lossy(&output.stdout));
        assert!(reached,"child failed to reach boundary: {}",String::from_utf8_lossy(&output.stderr));
        let plan:Preflight=serde_json::from_slice(&std::fs::read(tmp.path().join("plan.json")).unwrap()).unwrap();
        let store=Store::open_fixture(tmp.path().join("db.sqlite")).unwrap();
        if boundary=="before" { assert_eq!(store.engine_authority().unwrap().phase,"quiesced");store.engine_commit_for_test(&plan,&proof(&store),"recover-forward").unwrap(); }
        else { assert_eq!(store.engine_authority().unwrap().phase,"committed"); }
        store.engine_verify(&plan).unwrap();
    }
}

#[test]
fn engine_cutover_f09_all_dispatch_states_retain_exact_history_and_no_admitted_retry() {
    for dispatch in ["pending","claimed","admitted","uncertain","settled","suppressed"] {
        let store=fixture();let ws=store.list_workspaces(None).unwrap()[0].id;
        let command=input(ws.to_string(),"engine-standalone".into(),"finite-resource");
        let receipt=store.workflow_apply(&command).unwrap();
        store.conn().execute("UPDATE workflow_outbox SET dispatch_state=?1,lease_owner='old',lease_until=200,lease_generation=4,resource_generation=7",[dispatch]).unwrap();
        store.conn().execute("UPDATE workflow_resources SET generation=7",[]).unwrap();
        store.conn().execute("UPDATE workflow_resource_generations SET generation=7",[]).unwrap();
        let plan=conserved(&store,&format!("dispatch-{dispatch}"));
        assert_eq!(store.workflow_outbox(&command.workspace).unwrap()[0].dispatch_state,dispatch);
        assert_eq!(store.workflow_outbox(&command.workspace).unwrap()[0].effect.id,receipt.effects[0]);
        assert_eq!(plan.raw.tables["workflow_receipts"].rows.len(),1);
    }
}

#[test]
fn engine_cutover_all_field_closed_file_backup_public_permit_and_restore() {
    let tmp=tempfile::tempdir().unwrap();let private=PrivateRehearsal::create(tmp.path(),"all-field-file").unwrap();
    let path=private.live().join("db.sqlite");
    let mut conn=Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut conn,65);
    let tx=conn.transaction().unwrap();tx.execute_batch(include_str!("../../../fixtures/engine_cutover_all_columns.sql")).unwrap();tx.commit().unwrap();
    std::fs::create_dir(private.live().join("resources")).unwrap();
    let repository=private.live().join("resources/repo");std::fs::create_dir(&repository).unwrap();
    std::fs::create_dir(repository.join(".fixture-hooks")).unwrap();
    let template=private.live().join("resources/empty-git-template");std::fs::create_dir(&template).unwrap();
    fixture_git(&repository,&["init","-q","--initial-branch=synthetic",&format!("--template={}",template.display())]);
    std::fs::write(repository.join("tracked.txt"),b"committed baseline\n").unwrap();
    fixture_git(&repository,&["add","tracked.txt"]);fixture_git(&repository,&["commit","--no-gpg-sign","-q","-m","private fixture baseline"]);
    let head=fixture_git(&repository,&["rev-parse","HEAD"]).trim().to_string();
    fixture_git(&repository,&["branch","preserved-history","HEAD"]);
    std::fs::write(repository.join("tracked.txt"),b"dirty bytes after commit\n").unwrap();
    std::fs::write(repository.join("untracked.bin"),[0,255,32,10]).unwrap();
    let dirty=fixture_git(&repository,&["status","--porcelain=v1","--untracked-files=all"]);
    std::fs::create_dir(private.live().join("resources/config")).unwrap();
    std::fs::create_dir(private.live().join("resources/native")).unwrap();
    std::fs::write(private.live().join("resources/config/settings.json"),b"{\"synthetic\":true}\n").unwrap();
    std::fs::write(private.live().join("resources/native/rollout.jsonl"),b"{\"thread\":\"private-thread\"}\n").unwrap();
    conn.execute_batch("UPDATE repository_roots SET path='resources/repo'; UPDATE repositories SET canonical_git_dir='resources/repo/.git'; UPDATE worktrees SET worktree_path='resources/repo'; UPDATE agents SET session_cwd='resources/repo'; UPDATE terminal_logins SET config_dir='resources/config'; UPDATE check_in_occurrences SET rollout_path='resources/native/rollout.jsonl';").unwrap();
    conn.execute("UPDATE landings SET pushed_sha=?1",[&head]).unwrap();conn.execute("UPDATE board_ci SET sha=?1",[&head]).unwrap();
    let raw=preflight(&conn).unwrap();conn.close().unwrap();
    std::fs::create_dir(private.live().join("resources/empty-directory")).unwrap();
    std::fs::write(private.live().join("resources/private-pointer"),b"dirty worktree/ref/config/attachment fixture bytes\0\xff").unwrap();
    std::os::unix::fs::symlink("missing-target",private.live().join("resources/retained-link")).unwrap();
    let (store,plan)=transfer::begin(&path,&join_db(&path,"full-file")).unwrap();assert_eq!(plan.raw,raw.raw);
    let evidence=proof(&store);
    private.record(0,&JournalEntry {attempt:"full-file".into(),stage:"prepared".into(),evidence:plan.raw.hash()}).unwrap();
    store.engine_quiesce(&JoinedOwners::unit(evidence.clone(),"unit")).unwrap();drop(store);
    std::fs::set_permissions(private.live(),std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let backup=private.backup("baseline").unwrap();
    assert_eq!(backup.root_mode,0o700);
    let permit=private.commit_permit(&plan,&evidence,"full-file","baseline",&backup,1).unwrap();
    let store=Store::open_fixture(&path).unwrap();store.engine_commit(&plan,&JoinedOwners::unit(evidence.clone(),"full-file"),&permit).unwrap();store.engine_verify(&plan).unwrap();
    record_fixture("closed-all-fields",&plan,&store);
    let first=Manifest::capture(&store.conn()).unwrap();drop(store);
    let store=Store::open_fixture(&path).unwrap();assert_eq!(first,Manifest::capture(&store.conn()).unwrap());drop(store);
    let restored=private.restore("baseline","restored-all-fields",&backup).unwrap();
    let conn=Connection::open(restored.join("db.sqlite")).unwrap();
    assert!(raw.raw.conserved_by(&Manifest::capture(&conn).unwrap()));
    assert_eq!(authority(&conn).unwrap().phase,"quiesced");
    assert_eq!(std::fs::read_link(restored.join("resources/retained-link")).unwrap(),std::path::Path::new("missing-target"));
    assert!(restored.join("resources/empty-directory").is_dir());
    let restored_repo=restored.join("resources/repo");
    assert_eq!(fixture_git(&restored_repo,&["rev-parse","HEAD"]).trim(),head);
    assert_eq!(fixture_git(&restored_repo,&["show","HEAD:tracked.txt"]),"committed baseline\n");
    assert_eq!(fixture_git(&restored_repo,&["status","--porcelain=v1","--untracked-files=all"]),dirty);
    assert_eq!(std::fs::read(restored_repo.join("untracked.bin")).unwrap(),[0,255,32,10]);
    save_json("all-field-git-restore",&serde_json::json!({"head":head,"dirty_status":dirty,"private_git_only":true,"no_owner_configuration":true}));
    drop(conn);
    save_json("all-field-filesystem-manifest",&serde_json::to_value(&backup).unwrap());
    save_db("all-field-closed-baseline",&private.root().join("baseline/db.sqlite"));
    save_db("all-field-migrated",&path);
    save_db("all-field-restored",&restored.join("db.sqlite"));
}

#[test]
fn engine_cutover_unknown_engine_json_fields_cannot_be_erased_after_release() {
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("unknown-engine.sqlite");
    let store=Store::open_fixture(&path).unwrap();let repo=store.register_repository_for_test("Unknown engine");let ws=store.ensure_main_workspace(repo).unwrap();
    let mut command=input(ws.id.to_string(),"item".into(),"resource");store.workflow_apply(&command).unwrap();
    store.conn().execute("UPDATE workflow_states SET state=json_set(state,'$.future_authority','must-retain')",[]).unwrap();
    let plan=conserved(&store,"unknown-engine");store.engine_release(&plan,&joined(&store)).unwrap();drop(store);
    let store=Store::open_fixture(&path).unwrap();let raw=Manifest::capture(&store.conn()).unwrap().tables["workflow_states"].clone();
    command.id="advance".into();command.expected_revision=1;command.source_sequence=2;command.command=Command::Advance;
    assert!(store.workflow_apply(&command).is_err());
    assert_eq!(raw,Manifest::capture(&store.conn()).unwrap().tables["workflow_states"]);
}

#[test]
fn engine_cutover_task_move_rekey_and_terminal_hook_cannot_regain_authority() {
    let store=fixture();let ws=store.list_workspaces(None).unwrap()[0].id;
    let task=store.tasks_with_key(None,"con-1").unwrap_or_default().first().map(|t|t.id)
        .unwrap_or_else(|| {let bytes:Vec<u8>=store.conn().query_row("SELECT id FROM tasks LIMIT 1",[],|r|r.get(0)).unwrap();uuid::Uuid::from_slice(&bytes).unwrap()});
    let repo=store.get_workspace(ws).unwrap().repository_id;
    let tree=store.create_worktree(repo,"topic","resources/private-worktree",false).unwrap();
    let terminal=store.create_terminal_for_task(tree.id,"worker","fake",farcooler_protocol::v1::TerminalIntent::Running,80,24,Some(task)).unwrap();
    let plan=conserved(&store,"identity-fences");
    assert!(store.conn().execute("UPDATE tasks SET workspace_id=X'11111111111111111111111111111111'",[]).is_err());
    assert!(store.conn().execute("UPDATE tasks SET former_key='new-alias'",[]).is_err());
    assert!(store.conn().execute("UPDATE terminals SET epoch=epoch+1 WHERE id=?1",[terminal.id.as_bytes().as_slice()]).is_err());
    assert!(store.create_terminal_for_task(tree.id,"late-hook","fake",farcooler_protocol::v1::TerminalIntent::Running,80,24,Some(task)).is_err());
    assert!(store.get_task(task).is_ok());assert!(store.get_terminal(terminal.id).is_ok());
    store.engine_verify(&plan).unwrap();
}

#[test]
fn engine_cutover_old_sql_claims_and_engine_writes_are_frozen_before_commit() {
    let store=fixture();let ws=store.list_workspaces(None).unwrap()[0].id;
    let command=input(ws.to_string(),"standalone".into(),"before-commit-resource");store.workflow_apply(&command).unwrap();
    store.engine_quiesce(&joined(&store)).unwrap();
    assert!(store.conn().execute("UPDATE workflow_outbox SET dispatch_state='admitted'",[]).is_err());
    assert!(store.conn().execute("UPDATE workflow_states SET epoch=epoch+1",[]).is_err());
    assert!(store.workflow_claim_effect(&command.workspace,&store.workflow_outbox(&command.workspace).unwrap()[0].effect.id,"old",1,7,100,200).unwrap().is_none());
}

#[test]
fn engine_cutover_equal_time_rowid_order_is_conserved_not_just_field_counts() {
    let first=Connection::open_in_memory().unwrap();let second=Connection::open_in_memory().unwrap();
    for conn in [&first,&second] {conn.execute_batch("CREATE TABLE notes(id TEXT PRIMARY KEY,at INTEGER,body TEXT);").unwrap();}
    first.execute_batch("INSERT INTO notes VALUES('b',1,'equal time'); INSERT INTO notes VALUES('a',1,'equal time');").unwrap();
    second.execute_batch("INSERT INTO notes VALUES('a',1,'equal time'); INSERT INTO notes VALUES('b',1,'equal time');").unwrap();
    let original=Manifest::capture(&first).unwrap();let reordered=Manifest::capture(&second).unwrap();
    assert_eq!(original.tables["notes"].rows,reordered.tables["notes"].rows);
    assert!(!original.conserved_by(&reordered),"rowid tie-break order is a retained fact");
}
#[test]
fn engine_cutover_completed_legacy_line_landing_and_pane_allow_fresh_validated_item() {
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("inert.sqlite");let store=Store::open_fixture(&path).unwrap();
    let repo=store.register_repository_for_test("Inert");let ws=store.ensure_main_workspace(repo).unwrap();let task=store.create_task(ws.id,"historical done",Actor::User).unwrap();
    let tree=store.create_worktree(repo,"old","resources/old",false).unwrap();
    store.create_terminal_for_task(tree.id,"history","fake",farcooler_protocol::v1::TerminalIntent::Running,80,24,Some(task.id)).unwrap();
    store.conn().execute("UPDATE tasks SET status='done' WHERE id=?1",[task.id.as_bytes().as_slice()]).unwrap();
    store.conn().execute("INSERT INTO ticket_lines(task_id,stage,stage_since,created_at,resource_version) VALUES(?1,'landed',1,1,1)",[task.id.as_bytes().as_slice()]).unwrap();
    let landing=uuid::Uuid::now_v7();store.conn().execute("INSERT INTO landings(id,workspace_id,number,route,state,state_since,actor,created_at,resource_version) VALUES(?1,?2,1,'ask','landed',1,'user',1,1)",params![landing.as_bytes().as_slice(),ws.id.as_bytes().as_slice()]).unwrap();
    store.conn().execute("INSERT INTO landing_tickets VALUES(?1,?2,1)",params![landing.as_bytes().as_slice(),task.id.as_bytes().as_slice()]).unwrap();
    let plan=conserved(&store,"inert-history");assert_eq!(plan.bindings[0].disposition,"retained-intent");
    store.engine_release(&plan,&joined(&store)).unwrap();drop(store);
    let store=Store::open_fixture(&path).unwrap();let command=input(ws.id.to_string(),task.key,"new-resource");
    assert_eq!(store.workflow_apply(&command).unwrap().result,Ok(1));
    assert_eq!(store.conn().query_row("SELECT count(*) FROM ticket_lines",[],|r|r.get::<_,usize>(0)).unwrap(),1);
    assert_eq!(store.get_task(task.id).unwrap().status,crate::TaskStatus::Done,"legacy Done remains history, not new engine gates");
}

#[test]
fn engine_cutover_exhausted_or_inconsistent_resource_counter_refuses_without_writes() {
    for value in [i64::MAX,-1] {
        let store=all_fixture();
        store.conn().execute("UPDATE workflow_resource_generations SET generation=?1",[value]).unwrap();
        store.conn().execute("UPDATE workflow_resources SET generation=?1",[value]).unwrap();
        let before=Manifest::capture(&store.conn()).unwrap();assert!(preflight(&store.conn()).is_err());
        assert_eq!(before,Manifest::capture(&store.conn()).unwrap());assert_eq!(store.engine_authority().unwrap().phase,"legacy");
    }
    let store=all_fixture();store.conn().execute("UPDATE workflow_resources SET generation=2",[]).unwrap();
    assert!(preflight(&store.conn()).is_err(),"mismatched reservation/high-water mark must not be silently repaired");
}

#[test]
fn engine_cutover_unknown_cutover_prefixed_schema_is_never_ignored() {
    let store=fixture();
    store.conn().execute_batch("CREATE TRIGGER engine_unreviewed_hook AFTER INSERT ON task_notes BEGIN SELECT 1; END;").unwrap();
    let before=Manifest::capture(&store.conn()).unwrap();
    assert!(preflight(&store.conn()).is_err(),"unknown cutover-prefixed schema must not disappear");
    assert_eq!(before,Manifest::capture(&store.conn()).unwrap());
}

fn fixture_git(repository:&std::path::Path,args:&[&str])->String {
    let result=std::process::Command::new("/usr/bin/git").env_clear().env("PATH","/usr/bin:/bin")
        .env("LC_ALL","C").env("GIT_CONFIG_GLOBAL","/dev/null").env("GIT_CONFIG_SYSTEM","/dev/null")
        .env("GIT_CONFIG_NOSYSTEM","1").env("GIT_TERMINAL_PROMPT","0").env("GIT_OPTIONAL_LOCKS","0")
        .env("GIT_AUTHOR_DATE","2000-01-01T00:00:00Z").env("GIT_COMMITTER_DATE","2000-01-01T00:00:00Z")
        .args(["-c","user.name=Fixture","-c","user.email=fixture@example.invalid","-c","commit.gpgsign=false","-c"])
        .arg(format!("core.hooksPath={}",repository.join(".fixture-hooks").display()))
        .arg("-C").arg(repository).args(args).output().unwrap();
    assert!(result.status.success(),"private git {args:?}: {}",String::from_utf8_lossy(&result.stderr));
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn engine_cutover_private_resource_roots_refuse_symlinks_and_unmapped_mode_bits() {
    let tmp=tempfile::tempdir().unwrap();let private=PrivateRehearsal::create(tmp.path(),"path-fence").unwrap();
    let store=Store::open_fixture(private.live().join("db.sqlite")).unwrap();drop(store);
    std::fs::write(private.live().join("special-mode"),b"retain original permission evidence").unwrap();
    std::fs::set_permissions(private.live().join("special-mode"),std::os::unix::fs::PermissionsExt::from_mode(0o4644)).unwrap();
    assert!(matches!(private.backup("special-must-refuse"),Err(Error::Refused(reason)) if reason.contains("special permission bits")));
    std::fs::rename(private.live(),private.root().join("original-live-evidence")).unwrap();
    let other=tempfile::tempdir().unwrap();std::fs::write(other.path().join("db.sqlite"),b"private stand-in that must never be opened").unwrap();
    std::os::unix::fs::symlink(other.path(),private.live()).unwrap();
    assert!(matches!(private.backup("path-must-refuse"),Err(Error::Refused(reason)) if reason.contains("symlink traversal refused")));
    std::fs::rename(private.root().join("journal"),private.root().join("original-journal-evidence")).unwrap();
    std::os::unix::fs::symlink(other.path(),private.root().join("journal")).unwrap();
    assert!(matches!(private.record(0,&JournalEntry {attempt:"x".into(),stage:"prepared".into(),evidence:"x".into()}),Err(Error::Refused(reason)) if reason.contains("symlink traversal refused")));
    assert_eq!(std::fs::read(other.path().join("db.sqlite")).unwrap(),b"private stand-in that must never be opened");
    assert!(!other.path().join("000000.json").exists());
}
