//! Verify every retained field plus the exact finite ledger and named deltas.
use super::*;

pub(super) fn row_key(table:&manifest::Table,row:&[Cell]) -> String {
    manifest::identity(&table.columns,row)
}
/// A source at the registry keeps its exact catalog. A raw60 source grows into
/// exactly the pinned later catalog, and every table it grew starts empty: the
/// upgrade from 60 invents no row.
pub(super) fn catalog_matches(plan:&Preflight, after:&manifest::Manifest) -> bool {
    if plan.registry!=60 {
        return plan.raw.tables.keys().eq(after.tables.keys()) && plan.raw.objects.keys().eq(after.objects.keys());
    }
    catalog_hash(after)==SOURCE_SCHEMA_HASH
        && after.tables.iter().all(|(name,table)|plan.raw.tables.contains_key(name)||table.rows.is_empty())
}
fn check(plan:&Preflight, conn:&Connection) -> Result<()> {
    let mut after=preflight(conn)?.raw;
    // These are named transfer deltas, checked for exact +1. Never ignore an
    // entire column or table; every other value in the row must match the source.
    for name in ["workflow_resources","workflow_resource_generations"] {
        let Some(before)=plan.raw.tables.get(name) else { continue };
        let current=after.tables.get_mut(name).ok_or_else(||Error::Refused("resource table lost".into()))?;
        let generation=before.columns.iter().position(|c|c.name=="generation").ok_or_else(||Error::Refused("resource generation missing".into()))?;
        if before.rows.len()!=current.rows.len() { return refuse("resource set changed"); }
        for row in &mut current.rows {
            let Some(old)=before.rows.iter().find(|old| row_key(before,old)==row_key(before,row)) else { return refuse("resource key changed"); };
            match (&old[generation],&row[generation]) {
                (Cell::Integer(old),Cell::Integer(new)) if old.checked_add(1)==Some(*new) => row[generation]=Cell::Integer(*old),
                _ => return refuse("resource generation delta is not exactly +1"),
            }
        }
        current.rows.sort_by_cached_key(|row|serde_json::to_vec(row).expect("closed cells"));
    }
    if !plan.raw.conserved_by(&after) || !catalog_matches(plan,&after) {
        return refuse("per-field/schema/relationship/allocator conservation failed");
    }
    let schema:String=conn.query_row("SELECT value FROM meta WHERE key='schema_version'",[],|r|r.get(0))?;
    let floor:String=conn.query_row("SELECT value FROM meta WHERE key='compatible_down_to'",[],|r|r.get(0))?;
    let current=crate::migrate::CURRENT_SCHEMA_VERSION.to_string();
    if schema!=current || floor!=current { return refuse("authority compatibility stamp differs from the current registry"); }
    let mut expected=BTreeMap::new();
    for (name,table) in &plan.raw.tables {
        for row in &table.rows {
            let key=row_key(table,row);
            if let Some(ids)=table.rowids.as_ref().and_then(|ids|ids.get(&key)) {
                expected.insert((name.clone(),key.clone(),"@rowid".into()),farcooler_workflow::digest(ids));
            }
            for (column,value) in table.columns.iter().zip(row) {
                expected.insert((name.clone(),key.clone(),column.name.clone()),farcooler_workflow::digest(value));
            }
        }
    }
    let mut statement=conn.prepare("SELECT source_pin,table_name,row_key,field,value_hash,destination FROM engine_cutover_ledger")?;
    let mut actual=BTreeMap::new();
    for row in statement.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?)))? {
        let (source,table,key,field,hash,destination)=row?;
        if source!=SOURCE_PIN || destination!=format!("{table}/{key}/{field}") { return refuse("ledger source/destination binding lost"); }
        actual.insert((table,key,field),hash);
    }
    if expected!=actual { return refuse("per-field ledger injectivity/coverage failed"); }
    let current=authority(conn)?;
    if current.manifest.as_deref()!=Some(plan.raw.hash().as_str()) { return refuse("authority baseline identity changed"); }
    let workspace_count:u64=conn.query_row("SELECT count(*) FROM engine_cutover_workspaces WHERE incarnation=?1",[current.incarnation],|r|r.get(0))?;
    if workspace_count!=plan.workspaces.len() as u64 { return refuse("workspace authority coverage lost"); }
    Ok(())
}
impl Store {
    pub fn engine_verify(&self,plan:&Preflight) -> Result<()> { check(plan,&self.conn()) }
    /// Explicit private-admin release after bootstrap has supplied joined-writer
    /// proof. Scoped unknown effects stay quarantined; unknown schema blocks all.
    pub fn engine_release(&self,plan:&Preflight,owners:super::JoinedOwners) -> Result<()> {
        let mut conn=self.conn();
        owners.check_store(&conn)?;
        let tx=conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        frozen_by(&tx,owners.attempt())?;
        if authority(&tx)?.phase!="committed" { return refuse("not a committed authority"); }
        check(plan,&tx)?;
        let unknown:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM engine_cutover_quarantine WHERE resource='schema:unknown')",[],|r|r.get(0))?;
        if unknown { return refuse("unreviewed source schema stays frozen"); }
        tx.execute("UPDATE engine_cutover_authority SET phase='released' WHERE singleton=1",[])?;
        tx.commit()?;
        Ok(())
    }
}
