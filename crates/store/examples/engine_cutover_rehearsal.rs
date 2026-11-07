//! Explicit fresh PRIVATE synthetic admin workflow; no installed-path fallback.
use std::collections::BTreeSet;
use farcooler_store::{Store, Actor};
use farcooler_store::engine_cutover::{preflight, Quiescence, StoppedWriter};
use farcooler_store::engine_cutover::rehearsal::{PrivateRehearsal, JournalEntry};

fn main() -> Result<(),Box<dyn std::error::Error>> {
    let args:Vec<_>=std::env::args().collect();
    if args.len()!=3 {return Err("usage: engine_cutover_rehearsal <explicit-private-parent> <new-name>".into());}
    let parent=std::path::Path::new(&args[1]).canonicalize()?;
    if !parent.starts_with("/private/tmp") {return Err("synthetic parent must be /private/tmp".into());}
    let private=PrivateRehearsal::create(&parent,&args[2])?;
    let path=private.live().join("db.sqlite");
    let store=Store::open(&path)?;
    let host=uuid::Uuid::now_v7();
    let root=store.create_repository_root(host,"resources/synthetic-repository",0)?;
    let repository=store.create_repository(host,root.id,"Synthetic","resources/synthetic-repository/.git","")?;
    let ws=store.ensure_main_workspace(repository.id)?;
    store.create_task(ws.id,"Retained private synthetic history",Actor::User)?;
    let conn=rusqlite::Connection::open(&path)?;let plan=preflight(&conn)?;drop(conn);
    // No schedulers/processes are launched by this executable. Each category is
    // owned here and explicitly absent; this is not live-process inventory proof.
    let inventory:BTreeSet<String>=["legacy-scheduler","line-jobs","landing-jobs","legacy-hooks","delivery","workflow-owner","native-notices"].into_iter().map(str::to_owned).collect();
    let proof=Quiescence {stopped:inventory.iter().map(|identity|StoppedWriter {identity:identity.clone(),incarnation:"synthetic-not-started".into(),joined_receipt:"owned executable never started any writer child".into(),resources:BTreeSet::new()}).collect(),inventory,protected_resources:BTreeSet::new()};
    private.record(0,&JournalEntry {attempt:"cli".into(),stage:"prepared".into(),evidence:plan.raw.hash()})?;
    store.engine_quiesce(&proof)?;drop(store);
    let backup=private.backup("baseline")?;
    let permit=private.commit_permit(&plan,&proof,"cli","baseline",&backup,1)?;
    let store=Store::open(&path)?;
    let authority=store.engine_commit(&plan,&proof,&permit)?;store.engine_verify(&plan)?;
    private.record(2,&JournalEntry {attempt:"cli".into(),stage:"verified".into(),evidence:serde_json::to_string(&authority)?})?;
    drop(store);
    let restored=private.restore("baseline","independent-restore",&backup)?;
    std::fs::copy(&path,private.root().join("db.sqlite"))?;
    println!("{}",serde_json::json!({"synthetic_only":true,"authority":authority,"baseline":plan.raw.hash(),"root":private.root(),"restored":restored,"old_reader_fixture":private.root().join("db.sqlite"),"send":private.fake_counter("send")?,"push":private.fake_counter("push")?,"enter":private.fake_counter("enter")?}));
    Ok(())
}
