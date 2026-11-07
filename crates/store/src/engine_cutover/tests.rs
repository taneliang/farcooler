use super::*;
use crate::models::Actor;
use rehearsal::{PrivateRehearsal, JournalEntry, ExternalFact};
use rusqlite::params;

fn proof(store:&Store) -> Quiescence {
    let inventory:BTreeSet<String>=["legacy-scheduler","line-jobs","landing-jobs","legacy-hooks","delivery","workflow-owner","native-notices"].into_iter().map(str::to_owned).collect();
    let resources=required_resources(&store.conn()).unwrap();
    let stopped=inventory.iter().enumerate().map(|(n,identity)|StoppedWriter {identity:identity.clone(),incarnation:format!("fake-pid{n}-start2-tty3"),joined_receipt:"synthetic joined owner; no writable handles".into(),resources:if n==0 {resources.clone()} else {BTreeSet::new()}}).collect();
    Quiescence {inventory,stopped,protected_resources:resources}
}
pub(super) fn joined(_store:&Store) -> JoinedOwners { JoinedOwners::unit("unit") }
/// The real proof: every Store on `path` must already be closed.
pub(super) fn join_db(path:&std::path::Path,attempt:&str) -> JoinedOwners { JoinedOwners::acquire(path,attempt).unwrap() }
fn fixture() -> Store {
    let store=Store::open_in_memory().unwrap();
    let repo=store.register_repository_for_test("Conservation");
    let workspace=store.ensure_main_workspace(repo).unwrap();
    store.create_task(workspace.id,"history",Actor::User).unwrap();
    store
}
#[test]
fn engine_cutover_f01_typed_raw_conservation_and_atomic_workspace_coverage() {
    let store=fixture();
    let repo=store.register_repository_for_test("Other repository");
    let workspace=store.ensure_main_workspace(repo).unwrap();
    let task=store.create_task(workspace.id,"Unicode 空\nmultiline",Actor::User).unwrap();
    store.conn().execute("UPDATE tasks SET former_key='old-key',acceptance=' [ { \"id\": \"11111111-1111-1111-1111-111111111111\", \"text\": \"raw checklist\", \"met\": true } ] ',status='done' WHERE id=?1",[task.id.as_bytes().as_slice()]).unwrap();
    let other:Vec<u8>=store.conn().query_row("SELECT id FROM tasks WHERE id!=?1 LIMIT 1",[task.id.as_bytes().as_slice()],|r|r.get(0)).unwrap();
    store.conn().execute("UPDATE tasks SET status='cancelled' WHERE id=?1",[&other]).unwrap();
    store.conn().execute("INSERT INTO task_blocks VALUES(?1,?2,'cross-workspace exact dependency')",params![task.id.as_bytes().as_slice(),other]).unwrap();
    let a=uuid::Uuid::from_bytes([1;16]);let b=uuid::Uuid::from_bytes([2;16]);
    for (id,supersedes,body) in [(a,None,"legacy agent says done"),(b,Some(a),"revised legacy decision")]{
        store.conn().execute("INSERT INTO task_notes(id,task_id,kind,actor,at,body,extra,supersedes) VALUES(?1,?2,'decision','agent:11111111-1111-1111-1111-111111111111',7,?3,' { } ',?4)",params![id.as_bytes().as_slice(),task.id.as_bytes().as_slice(),body,supersedes.map(|v|v.as_bytes().to_vec())]).unwrap();
    }
    let order:Vec<_>=store.notes_for(task.id,None).unwrap().iter().map(|n|n.id).collect();
    let plan=preflight(&store.conn()).unwrap();
    assert_eq!(plan.bindings.len(),2);
    store.engine_quiesce(joined(&store)).unwrap();
    let result=store.engine_commit_for_test(&plan,&proof(&store),"f01").unwrap();
    assert_eq!(result.phase,"committed");
    assert_eq!(result.incarnation,1);
    store.engine_verify(&plan).unwrap();
    record_fixture("f01",&plan,&store);
    assert_eq!(store.get_task(task.id).unwrap().title,"Unicode 空\nmultiline");
    assert_eq!(order,store.notes_for(task.id,None).unwrap().iter().map(|n|n.id).collect::<Vec<_>>());
    assert_eq!(store.conn().query_row("SELECT count(*) FROM engine_cutover_workspaces",[],|r|r.get::<_,usize>(0)).unwrap(),plan.workspaces.len());
    assert!(store.conn().query_row("SELECT count(*) FROM engine_cutover_ledger",[],|r|r.get::<_,usize>(0)).unwrap()>0);
    assert_eq!(store.conn().query_row("SELECT count(*) FROM workflow_events",[],|r|r.get::<_,usize>(0)).unwrap(),0,"legacy history must not fabricate events");
}
#[test]
fn engine_cutover_schema_catalog() {
    let mut conn=Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut conn,65);
    let manifest=Manifest::capture(&conn).unwrap();
    if let Ok(path)=std::env::var("OV936_SCHEMA_OUTPUT") {
        std::fs::write(path,serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    }
    println!("source65 tables={} objects={} columns={} schema={}",manifest.tables.len(),manifest.objects.len(),manifest.tables.values().map(|t|t.columns.len()).sum::<usize>(),farcooler_workflow::digest(&manifest.objects));
}
#[test]
fn engine_cutover_f10_raw_invalid_wait_survives_and_ambiguous_alias_quarantines() {
    let store=fixture();
    let ws=store.list_workspaces(None).unwrap()[0].id;
    store.create_task(ws,"second",Actor::User).unwrap();
    store.conn().execute("UPDATE tasks SET former_key='AMBIG',wait_kind='future',wait_line=' raw ' WHERE workspace_id=?1",[ws.as_bytes().as_slice()]).unwrap();
    let plan=preflight(&store.conn()).unwrap();
    assert_eq!(plan.bindings.iter().filter(|b|b.disposition=="quarantined").count(),2);
    assert!(plan.quarantines.len()>=3);
    store.engine_quiesce(joined(&store)).unwrap();
    store.engine_commit_for_test(&plan,&proof(&store),"f10").unwrap();
    assert!(plan.raw.conserved_by(&Manifest::capture(&store.conn()).unwrap()));
}
#[test]
fn engine_cutover_f12_closed_backup_independent_restore_and_postbackup_receipts() {
    let tmp=tempfile::tempdir().unwrap();
    let private=PrivateRehearsal::create(tmp.path(),"f12").unwrap();
    let store=Store::open_fixture(private.live().join("db.sqlite")).unwrap();
    let repo=store.register_repository_for_test("Restore");
    store.ensure_main_workspace(repo).unwrap();
    drop(store);
    std::fs::create_dir(private.live().join("resources")).unwrap();
    std::fs::write(private.live().join("resources/dirty"),b"dirty untracked bytes\0\xff").unwrap();
    std::os::unix::fs::symlink("/synthetic/missing",private.live().join("resources/pointer")).unwrap();
    private.record(0,&JournalEntry {attempt:"f12".into(),stage:"prepared".into(),evidence:"closed synthetic only".into()}).unwrap();
    let snapshot=private.backup("baseline").unwrap();
    assert!(private.backup("baseline").is_err(),"backup collision must refuse");
    let fact=ExternalFact {operation:"push-exact-op".into(),request_hash:"request1".into(),resource:"branch".into(),incarnation:"new1".into(),receipt:Some("remote exact op receipt".into())};
    private.retain_effect(&fact).unwrap();
    let restored=private.restore("baseline","restored",&snapshot).unwrap();
    assert!(restored.join("db.sqlite").exists());
    assert!(private.reconcile(&fact,Some(&fact)).unwrap());
    assert!(!private.reconcile(&fact,None).unwrap(),"missing readback never permits replay");
    assert!(private.retain_effect(&fact).is_err(),"external operation identity cannot be repeated");
}

#[test]
fn engine_cutover_f08_every_current_table_and_column_is_seeded_and_conserved() {
    let mut conn=Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut conn,65);
    let tx=conn.transaction().unwrap();
    tx.execute_batch(include_str!("../../fixtures/engine_cutover_all_columns.sql")).unwrap();
    tx.commit().unwrap();
    let raw=Manifest::capture(&conn).unwrap();
    for (name,table) in &raw.tables { assert!(!table.rows.is_empty(),"unseeded table {name}"); }
    assert_eq!(raw.tables.len(),87);
    assert_eq!(raw.tables.values().map(|t|t.columns.len()).sum::<usize>(),540);
    assert_eq!(conn.prepare("PRAGMA foreign_key_check").unwrap().query_map([],|_|Ok(())).unwrap().count(),0);
    let plan=preflight(&conn).unwrap();
    crate::migrate::migrate(&mut conn,65).unwrap();
    let store=Store::from_engine_cutover_connection(conn,None).unwrap();
    store.engine_quiesce(joined(&store)).unwrap();
    store.engine_commit_for_test(&plan,&proof(&store),"f08").unwrap();
    // Existing resource generations are the sole documented non-meta field delta.
    let mut after=Manifest::capture(&store.conn()).unwrap();
    for name in ["workflow_resources","workflow_resource_generations"] {
        let table=after.tables.get_mut(name).unwrap();
        let index=table.columns.iter().position(|c|c.name=="generation").unwrap();
        for row in &mut table.rows { if let Cell::Integer(v)=&mut row[index] { assert_eq!(*v,2); *v-=1; } }
    }
    assert!(raw.conserved_by(&after));
    record_fixture("f08",&plan,&store);
    let mut corrupted=after.clone();
    corrupted.tables.get_mut("tasks").unwrap().rows[0][3]=Cell::Text(b"corrupt title".to_vec());
    assert!(!raw.conserved_by(&corrupted),"same counts must never pass field corruption");
}

fn all_fixture() -> Store {all_fixture_schedule("disabled")}
fn all_fixture_schedule(schedule:&str) -> Store {
    let mut conn=Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut conn,65);
    let tx=conn.transaction().unwrap();
    let sql=include_str!("../../fixtures/engine_cutover_all_columns.sql").replace("'disabled'",&format!("'{schedule}'"));
    tx.execute_batch(&sql).unwrap();
    tx.commit().unwrap();
    crate::migrate::migrate(&mut conn,65).unwrap();
    Store::from_engine_cutover_connection(conn,None).unwrap()
}
fn conserved(store:&Store, label:&str) -> Preflight {
    let plan=preflight(&store.conn()).unwrap();
    let evidence=proof(store);
    store.engine_quiesce(JoinedOwners::unit("unit")).unwrap();
    store.engine_commit_for_test(&plan,&evidence,label).unwrap();
    store.engine_verify(&plan).unwrap();
    record_fixture(label,&plan,store);
    plan
}
mod cases;

fn record_fixture(label:&str,plan:&Preflight,store:&Store) {
    let Ok(destination)=std::env::var("OV936_EVIDENCE_OUTPUT") else {return;};
    assert!(label.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-'||b==b'_'));
    let directory=std::path::Path::new(&destination);std::fs::create_dir_all(directory).unwrap();
    let conn=store.conn();
    let mut statement=conn.prepare("SELECT source_pin,table_name,row_key,field,value_hash,destination,disposition FROM engine_cutover_ledger ORDER BY table_name,row_key,field").unwrap();
    let ledger:Vec<Vec<String>>=statement.query_map([],|r|(0..7).map(|index|r.get(index)).collect()).unwrap().map(|r|r.unwrap()).collect();
    let after=Manifest::capture(&conn).unwrap();
    let result_hash=after.hash();
    let bytes=serde_json::to_vec_pretty(&serde_json::json!({"label":label,"plan":plan,"after":after,"ledger":ledger})).unwrap();
    let path=directory.join(format!("{label}-{}-{}.json",plan.raw.hash().replace(':',"-"),result_hash.replace(':',"-")));
    use std::io::Write;
    match std::fs::OpenOptions::new().create_new(true).write(true).open(&path) {
        Ok(mut file)=>{file.write_all(&bytes).unwrap();file.sync_all().unwrap();},
        Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>assert_eq!(std::fs::read(path).unwrap(),bytes,"same fixture identity must retain identical evidence"),
        Err(error)=>panic!("fixture evidence: {error}"),
    }
}

fn save_bytes(label:&str,extension:&str,bytes:&[u8]) {
    let Ok(destination)=std::env::var("OV936_EVIDENCE_OUTPUT") else {return;};
    let directory=std::path::Path::new(&destination);std::fs::create_dir_all(directory).unwrap();
    let path=directory.join(format!("{label}-{}.{extension}",farcooler_workflow::digest(&bytes).replace(':',"-")));
    use std::io::Write;
    match std::fs::OpenOptions::new().create_new(true).write(true).open(&path) {
        Ok(mut file)=>{file.write_all(bytes).unwrap();file.sync_all().unwrap();},
        Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>assert_eq!(std::fs::read(path).unwrap(),bytes),
        Err(error)=>panic!("private fixture receipt: {error}"),
    }
}
fn save_json(label:&str,value:&serde_json::Value) {save_bytes(label,"json",&serde_json::to_vec_pretty(value).unwrap());}
fn save_db(label:&str,path:&std::path::Path) {
    assert!(std::fs::symlink_metadata(path).unwrap().file_type().is_file());
    let canonical=path.canonicalize().unwrap();
    assert!(canonical.starts_with(std::env::temp_dir().canonicalize().unwrap()) || canonical.starts_with("/private/tmp"));
    save_bytes(label,"sqlite",&std::fs::read(path).unwrap());
}

mod storage_safety;
mod startup_gate;
mod manual_delivery;
mod engine_delivery;
mod recovery;
mod commit_closed;
mod released_board;
mod source60;
