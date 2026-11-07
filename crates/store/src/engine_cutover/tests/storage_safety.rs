use super::*;
use std::fs;
use std::path::Path;
use std::os::unix::{ffi::OsStrExt, fs::symlink};
use farcooler_workflow::{Command, Definition, Grant, Input};

fn enqueue(workspace: String) -> Input {
    Input {schema:1,id:"enqueue".into(),workspace,expected_revision:0,epoch:7,source:"controller".into(),source_sequence:1,now_ms:100,
        grant:Grant::Controller,busy_resources:BTreeSet::new(),command:Command::Enqueue {item:"item".into(),definition:Definition::coding(),policy_pin:"fixture-policy".into(),resource:"resource".into(),dependencies:BTreeSet::new(),initial_candidate:None}}
}
fn fifo(path: &Path) {
    let path=std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe {libc::mkfifo(path.as_ptr(),0o600)},0);
}
fn fact() -> ExternalFact {
    ExternalFact {operation:"operation".into(),request_hash:"request".into(),resource:"resource".into(),incarnation:"synthetic".into(),receipt:None}
}
fn journal_path(private: &PrivateRehearsal) -> std::path::PathBuf {
    let fact=fact();let id=farcooler_workflow::digest(&(&fact.operation,&fact.resource));
    private.root().join("journal").join(format!("effect-{}.json",id.replace(':',"-")))
}
fn replace(path: &Path, kind: &str, outside: &Path) {
    fs::rename(path,path.with_extension("original")).unwrap();
    match kind {
        "symlink" => symlink(outside,path).unwrap(),
        "fifo" => fifo(path),
        "hardlink" => fs::hard_link(outside,path).unwrap(),
        "oversized" => fs::File::create(path).unwrap().set_len(16*1024*1024+1).unwrap(),
        _ => unreachable!(),
    }
}
#[test]
fn engine_cutover_storage_metadata_restore_and_reconcile_refuse_unsafe_leaves() {
    for kind in ["symlink","fifo","hardlink","oversized"] {
        let tmp=tempfile::tempdir().unwrap();let private=PrivateRehearsal::create(tmp.path(),"metadata").unwrap();
        let store=Store::open(private.live().join("db.sqlite")).unwrap();drop(store);
        let snapshot=private.backup("backup").unwrap();
        private.retain_effect(&fact()).unwrap();
        assert!(!private.reconcile(&fact(),None).unwrap());
        let outside=tmp.path().join("outside.json");let bytes=serde_json::to_vec(&snapshot).unwrap();fs::write(&outside,&bytes).unwrap();
        replace(&private.root().join("backup/SNAPSHOT.json"),kind,&outside);
        let start=std::time::Instant::now();
        assert!(private.restore("backup","refused",&snapshot).is_err(),"restore must refuse {kind}");
        assert!(!private.root().join("refused").exists());
        assert!(start.elapsed().as_secs()<2,"metadata refusal must not block");
        let effect_bytes=serde_json::to_vec(&fact()).unwrap();let outside_effect=tmp.path().join("outside-effect.json");fs::write(&outside_effect,&effect_bytes).unwrap();
        replace(&journal_path(&private),kind,&outside_effect);
        let start=std::time::Instant::now();
        assert!(private.reconcile(&fact(),None).is_err(),"reconcile must refuse {kind}");
        assert!(start.elapsed().as_secs()<2);
        assert_eq!(fs::read(&outside).unwrap(),bytes);assert_eq!(fs::read(&outside_effect).unwrap(),effect_bytes);
        for kind in ["send","push","enter"] {assert_eq!(private.fake_counter(kind).unwrap(),0);}
    }
}
#[test]
fn engine_cutover_storage_restore_refuses_replaced_journal_and_effect_leaves() {
    for kind in ["journal","symlink","fifo","hardlink","oversized"] {
        let tmp=tempfile::tempdir().unwrap();let private=PrivateRehearsal::create(tmp.path(),"journal").unwrap();
        drop(Store::open(private.live().join("db.sqlite")).unwrap());
        let snapshot=private.backup("backup").unwrap();private.retain_effect(&fact()).unwrap();
        let outside=tmp.path().join("outside");fs::create_dir(&outside).unwrap();
        let outside_effect=outside.join(journal_path(&private).file_name().unwrap());let bytes=serde_json::to_vec(&fact()).unwrap();fs::write(&outside_effect,&bytes).unwrap();
        if kind=="journal" {
            fs::rename(private.root().join("journal"),private.root().join("journal-original")).unwrap();
            symlink(&outside,private.root().join("journal")).unwrap();
        } else {replace(&journal_path(&private),kind,&outside_effect);}
        let start=std::time::Instant::now();
        assert!(private.restore("backup","refused",&snapshot).is_err(),"restore journal must refuse {kind}");
        assert!(start.elapsed().as_secs()<2);
        if private.root().join("refused/db.sqlite").exists() {
            let db=Connection::open(private.root().join("refused/db.sqlite")).unwrap();
            assert_eq!(db.query_row("SELECT count(*) FROM engine_cutover_external",[],|r|r.get::<_,u64>(0)).unwrap(),0);
        }
        assert_eq!(fs::read(&outside_effect).unwrap(),bytes);
        for kind in ["send","push","enter"] {assert_eq!(private.fake_counter(kind).unwrap(),0);}
    }
}
fn corrupt_effect(store: &Store, mutation: &str) -> String {
    let conn=store.conn();
    let (payload,hash):(String,String)=conn.query_row("SELECT payload,payload_hash FROM workflow_outbox",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let mut raw:serde_json::Value=serde_json::from_str(&payload).unwrap();
    match mutation {
        "effect" => raw["future_authority"]=serde_json::json!({"must":"retain"}),
        "operation" => raw["operation"]["future_authority"]=serde_json::json!("retain nested"),
        "item" => raw["item"]=serde_json::json!("other-item"),
        "id" => raw["id"]=serde_json::json!("other-id"),
        "hash" => {},
        _ => unreachable!(),
    }
    let raw=serde_json::to_string(&raw).unwrap();
    let hash=if matches!(mutation,"item"|"id") {farcooler_workflow::digest(&serde_json::from_str::<farcooler_workflow::Effect>(&raw).unwrap())} else if mutation=="hash" {"wrong-hash".into()} else {hash};
    // Model a source-era stored artifact; restore the exact immutable trigger.
    let sql:String=conn.query_row("SELECT sql FROM sqlite_schema WHERE name='workflow_outbox_no_payload_update'",[],|r|r.get(0)).unwrap();
    conn.execute_batch("DROP TRIGGER workflow_outbox_no_payload_update").unwrap();
    conn.execute("UPDATE workflow_outbox SET payload=?1,payload_hash=?2",params![raw,hash]).unwrap();
    conn.execute_batch(&sql).unwrap();raw
}
#[test]
fn engine_cutover_storage_unknown_effects_preserve_raw_and_never_claim_or_admit() {
    for dispatch in ["pending","claimed","admitted"] {
        for mutation in ["effect","operation","item","id","hash"] {
            let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("effect.sqlite");
            let store=Store::open(&path).unwrap();let repo=store.register_repository_for_test("Effect");let ws=store.ensure_main_workspace(repo).unwrap();
            let command=enqueue(ws.id.to_string());let receipt=store.workflow_apply(&command).unwrap();let id=&receipt.effects[0];
            let claim=store.workflow_claim_effect(&command.workspace,id,"worker",1,7,100,200).unwrap().unwrap();
            if dispatch=="admitted" {assert!(store.workflow_admit_effect(&claim,101).unwrap());}
            if dispatch=="pending" {store.conn().execute("UPDATE workflow_outbox SET dispatch_state='pending'",[]).unwrap();}
            let raw=corrupt_effect(&store,mutation);
            let before=Manifest::capture(&store.conn()).unwrap().tables["workflow_outbox"].clone();
            assert!(store.workflow_claim_effect(&command.workspace,id,"other",1,7,300,400).unwrap().is_none(),"claim must refuse {dispatch}/{mutation}");
            let mut calls=0;
            if store.workflow_admit_effect(&claim,102).unwrap() {calls+=1;}
            assert_eq!(calls,0,"admission must refuse {dispatch}/{mutation}");
            assert_eq!(Manifest::capture(&store.conn()).unwrap().tables["workflow_outbox"],before);
            let plan=preflight(&store.conn()).unwrap();
            assert!(plan.quarantines.iter().any(|q|q.resource==format!("workspace:{}",command.workspace)),"preflight must quarantine {dispatch}/{mutation}");
            assert!(plan.quarantines.iter().any(|q|q.resource=="resource"));
            store.engine_quiesce(&joined(&store)).unwrap();store.engine_commit_for_test(&plan,&proof(&store),"storage").unwrap();store.engine_release(&plan,&joined(&store)).unwrap();drop(store);
            let store=Store::open(&path).unwrap();
            assert!(store.workflow_claim_effect(&command.workspace,id,"new",1,7,300,400).unwrap().is_none());
            assert!(!store.workflow_admit_effect(&claim,102).unwrap());
            assert_eq!(store.conn().query_row("SELECT payload FROM workflow_outbox",[],|r|r.get::<_,String>(0)).unwrap(),raw);
            assert_eq!(Manifest::capture(&store.conn()).unwrap().tables["workflow_outbox"],before);
            println!("S2 {}",serde_json::json!({"dispatch":dispatch,"mutation":mutation,"payload":raw,"before":before,"after":Manifest::capture(&store.conn()).unwrap().tables["workflow_outbox"],"calls":calls}));
        }
    }
}
#[test]
fn engine_cutover_storage_settled_unknown_effect_remains_inert_history() {
    let store=Store::open_in_memory().unwrap();let command=enqueue("workspace".into());store.workflow_apply(&command).unwrap();
    let raw=corrupt_effect(&store,"operation");store.conn().execute("UPDATE workflow_outbox SET active=0,dispatch_state='settled'",[]).unwrap();
    let plan=preflight(&store.conn()).unwrap();
    assert!(!plan.quarantines.iter().any(|q|q.reason.contains("Effect")));
    assert_eq!(store.conn().query_row("SELECT payload FROM workflow_outbox",[],|r|r.get::<_,String>(0)).unwrap(),raw);
}
#[test]
fn engine_cutover_storage_preflight_quarantines_pending_lossy_effect() {
    let store=Store::open_in_memory().unwrap();let command=enqueue("workspace".into());store.workflow_apply(&command).unwrap();
    let raw=corrupt_effect(&store,"operation");let plan=preflight(&store.conn()).unwrap();
    assert!(plan.quarantines.iter().any(|q|q.resource=="workspace:workspace"),"lossy pending Effect requires workspace quarantine");
    assert!(plan.quarantines.iter().any(|q|q.resource=="resource"));
    assert_eq!(store.conn().query_row("SELECT payload FROM workflow_outbox",[],|r|r.get::<_,String>(0)).unwrap(),raw);
}
#[test]
fn engine_cutover_storage_admit_rechecks_payload_after_valid_claim() {
    let store=Store::open_in_memory().unwrap();let command=enqueue("workspace".into());let receipt=store.workflow_apply(&command).unwrap();
    let claim=store.workflow_claim_effect(&command.workspace,&receipt.effects[0],"worker",1,7,100,200).unwrap().unwrap();
    corrupt_effect(&store,"operation");
    assert!(!store.workflow_admit_effect(&claim,101).unwrap(),"admit must recheck immutable source payload");
    assert_eq!(store.conn().query_row("SELECT dispatch_state FROM workflow_outbox",[],|r|r.get::<_,String>(0)).unwrap(),"claimed");
}
#[test]
fn engine_cutover_storage_reconcile_refuses_outside_leaf_before_readback() {
    let tmp=tempfile::tempdir().unwrap();let private=PrivateRehearsal::create(tmp.path(),"reconcile").unwrap();private.retain_effect(&fact()).unwrap();
    let outside=tmp.path().join("outside.json");let bytes=serde_json::to_vec(&fact()).unwrap();fs::write(&outside,&bytes).unwrap();
    replace(&journal_path(&private),"symlink",&outside);
    assert!(private.reconcile(&fact(),None).is_err(),"reconcile must refuse outside leaf");
    assert_eq!(fs::read(&outside).unwrap(),bytes);
}
