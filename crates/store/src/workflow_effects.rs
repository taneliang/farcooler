//! Durable effect admission metadata only. This module performs no external I/O.
//! Claimed-but-not-admitted work can be reclaimed; admitted work requires reconciliation.
use farcooler_workflow::{State, Phase, owns_resource, valid_id};
use rusqlite::{Transaction, TransactionBehavior, OptionalExtension, params};
use crate::{Store, workflow::Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectClaim {
    pub workspace: String,
    pub effect: String,
    pub owner: String,
    pub lease_generation: u64,
    pub resource_generation: u64,
    pub revision: u64,
    pub epoch: u64,
    pub authority_incarnation: u64,
}

pub(crate) fn reserve(tx: &Transaction<'_>, state: &State) -> rusqlite::Result<()> {
    let mut stmt = tx.prepare("SELECT resource FROM workflow_resources WHERE workspace = ?1")?;
    let resources = stmt.query_map([&state.workspace], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for resource in resources {
        if !state.items.values().any(|i| owns_resource(i) && i.resource == resource) {
            tx.execute("DELETE FROM workflow_resources WHERE resource = ?1", [&resource])?;
        }
    }
    for (id, item) in state.items.iter().filter(|(_, i)| owns_resource(i)) {
        let existing: Option<(String, String, u64)> = tx.query_row(
            "SELECT item, run, fence FROM workflow_resources WHERE resource = ?1 AND workspace = ?2",
            params![item.resource, state.workspace], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?;
        if existing == Some((id.clone(), item.run.clone(), item.fence)) { continue; }
        tx.execute(
            "INSERT INTO workflow_resource_generations (resource, generation) VALUES (?1, 1)
             ON CONFLICT(resource) DO UPDATE SET generation = generation + 1", [&item.resource],
        )?;
        tx.execute(
            "INSERT INTO workflow_resources (resource, workspace, item, run, fence, generation)
             SELECT ?1, ?2, ?3, ?4, ?5, generation FROM workflow_resource_generations WHERE resource = ?1
             ON CONFLICT(resource) DO UPDATE SET workspace = excluded.workspace, item = excluded.item,
             run = excluded.run, fence = excluded.fence, generation = excluded.generation",
            params![item.resource, state.workspace, id, item.run, item.fence],
        )?;
    }
    Ok(())
}
/// Validate supported executable JSON without dropping unknown nested fields.
/// The raw row remains evidence; this never rewrites it or creates a receipt.
pub(crate) fn checked_effect(payload: &str, id: &str, hash: &str) -> Option<farcooler_workflow::Effect> {
    let raw: serde_json::Value=serde_json::from_str(payload).ok()?;
    let effect: farcooler_workflow::Effect=serde_json::from_value(raw.clone()).ok()?;
    (serde_json::to_value(&effect).ok()?==raw && effect.id==id
        && farcooler_workflow::digest(&effect)==hash).then_some(effect)
}
pub(crate) fn effect_binds(effect: &farcooler_workflow::Effect, state: &State, workspace: &str, revision: u64) -> bool {
    state.workspace==workspace && state.schema==1 && revision<=state.revision && effect.epoch==state.epoch
        && state.items.get(&effect.item).is_some_and(|item| item.pending.as_deref()==Some(effect.id.as_str())
            && item.run==effect.run && item.resource==effect.resource && item.fence==effect.fence
            && item.pin==effect.definition_pin && item.policy_pin==effect.policy_pin && item.candidate==effect.candidate)
}
fn executable_payload(tx: &Transaction<'_>, workspace: &str, id: &str) -> Result<bool> {
    let row: Option<(String,String,u64,String)>=tx.query_row(
        "SELECT o.payload,o.payload_hash,o.revision,s.state FROM workflow_outbox o JOIN workflow_states s ON s.workspace=o.workspace WHERE o.workspace=?1 AND o.id=?2",
        params![workspace,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((payload,hash,revision,raw_state))=row else {return Ok(false);};
    let Some(effect)=checked_effect(&payload,id,&hash) else {return Ok(false);};
    let Ok(state)=serde_json::from_str::<State>(&raw_state) else {return Ok(false);};
    Ok(effect_binds(&effect,&state,workspace,revision))
}
impl Store {
    /// Runtime admission must authenticate lease_owner and supply time. A claim is
    /// never permission to execute: only a successful subsequent admission is.
    #[allow(clippy::too_many_arguments)] // Each independent CAS/lease coordinate is explicit at admission.
    pub fn workflow_claim_effect(&self, workspace: &str, effect: &str, lease_owner: &str,
        revision: u64, epoch: u64, now_ms: i64, lease_until: i64) -> Result<Option<EffectClaim>>
    {
        if !valid_id(lease_owner) || lease_until <= now_ms
            || lease_until.saturating_sub(now_ms) > 300_000 { return Ok(None); }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !crate::engine_cutover::engine_allowed(&tx, self.engine_incarnation)? { return Ok(None); }
        let blocked: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM engine_cutover_quarantine q JOIN workflow_outbox o ON q.resource=json_extract(o.payload,'$.resource') WHERE o.workspace=?1 AND o.id=?2) OR EXISTS(SELECT 1 FROM engine_cutover_quarantine WHERE resource='workspace:'||?1)",params![workspace,effect],|r|r.get(0))?;
        if blocked || !executable_payload(&tx,workspace,effect)? { return Ok(None); }
        let state: Option<String> = tx.query_row("SELECT state FROM workflow_states WHERE workspace = ?1",
            [workspace], |r| r.get(0)).optional()?;
        let Some(state) = state else { return Ok(None); };
        let state: State = serde_json::from_str(&state)?;
        if state.revision != revision || state.epoch != epoch { return Ok(None); }
        let Some(item) = state.items.values().find(|i| i.pending.as_deref() == Some(effect)) else {
            return Ok(None);
        };
        if !matches!(item.phase, Phase::Running | Phase::Cancelling) { return Ok(None); }
        let generation: u64 = tx.query_row(
            "SELECT generation FROM workflow_resources WHERE resource = ?1 AND workspace = ?2 AND run = ?3 AND fence = ?4",
            params![item.resource, workspace, item.run, item.fence], |r| r.get(0),
        )?;
        let n = tx.execute(
            "UPDATE workflow_outbox SET dispatch_state = 'claimed', lease_owner = ?3,
                lease_generation = lease_generation + 1, lease_until = ?5, resource_generation = ?6
             WHERE workspace = ?1 AND id = ?2 AND active = 1
             AND (dispatch_state = 'pending' OR (dispatch_state = 'claimed' AND lease_until <= ?4))",
            params![workspace, effect, lease_owner, now_ms, lease_until, generation],
        )?;
        if n != 1 { return Ok(None); }
        let lease_generation = tx.query_row(
            "SELECT lease_generation FROM workflow_outbox WHERE workspace = ?1 AND id = ?2",
            params![workspace, effect], |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(Some(EffectClaim { workspace: workspace.into(), effect: effect.into(), owner: lease_owner.into(),
            lease_generation, resource_generation: generation, revision, epoch, authority_incarnation:self.engine_incarnation }))
    }

    /// Persists the uncertain-effect boundary before any adapter call. Expiry after
    /// this point never permits re-execution, even by the same owner.
    pub fn workflow_admit_effect(&self, claim: &EffectClaim, now_ms: i64) -> Result<bool> {
        let mut conn=self.conn();
        let tx=conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if claim.authority_incarnation!=self.engine_incarnation || !crate::engine_cutover::engine_allowed(&tx,self.engine_incarnation)? { return Ok(false); }
        if !executable_payload(&tx,&claim.workspace,&claim.effect)? {return Ok(false);}
        let blocked: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM engine_cutover_quarantine WHERE resource='workspace:'||?1 OR resource=(SELECT json_extract(payload,'$.resource') FROM workflow_outbox WHERE workspace=?1 AND id=?2))",params![claim.workspace,claim.effect],|r|r.get(0))?;
        if blocked {return Ok(false);}
        let n = tx.execute(
            "UPDATE workflow_outbox SET dispatch_state = 'admitted'
             WHERE workspace = ?1 AND id = ?2 AND active = 1 AND dispatch_state = 'claimed'
             AND lease_owner = ?3 AND lease_generation = ?4 AND lease_until > ?5
             AND resource_generation = ?6
             AND EXISTS (SELECT 1 FROM workflow_states s WHERE s.workspace = ?1 AND s.revision = ?7 AND s.epoch = ?8)
             AND EXISTS (SELECT 1 FROM workflow_resources r WHERE r.workspace = ?1 AND r.generation = ?6
                 AND r.resource = json_extract(workflow_outbox.payload, '$.resource')
                 AND r.run = json_extract(workflow_outbox.payload, '$.run')
                 AND r.fence = json_extract(workflow_outbox.payload, '$.fence'))",
            params![claim.workspace, claim.effect, claim.owner, claim.lease_generation, now_ms,
                claim.resource_generation, claim.revision, claim.epoch],
        )?;
        tx.commit()?;
        Ok(n == 1)
    }
}
