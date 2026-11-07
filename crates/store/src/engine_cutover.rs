//! Bounded all-workspace authority transfer. Original legacy tables remain inert.
//! This API does not stop external processes: bootstrap must join them and supply
//! exact incarnation/resource evidence before it can authorize a private rehearsal.
use std::collections::{BTreeMap, BTreeSet};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use crate::Store;
pub mod boundary;
pub mod closing;
pub mod manifest;
pub mod rehearsal;
pub mod startup;
pub mod owners;
pub mod transfer;
mod verify;
pub(crate) use manifest::Manifest;
use manifest::Cell;
use rehearsal::CommitPermit;
pub use owners::JoinedOwners;
pub const REQUIRED_OWNERS: [&str; 7] = ["legacy-scheduler","line-jobs","landing-jobs","legacy-hooks","delivery","workflow-owner","native-notices"];

pub const SOURCE_PIN: &str = "fae3f30d0c3d1c475f9dc4463243f293fd88b007";
pub const CUTOVER_SCHEMA_HASH: &str = "wf1:7871168a56707659c4aec1ed4e548970a69f4193f5f92726d37b7286a52a5e2d";
/// The registry-60 catalog (entry 60 = check_in_destination), which predates
/// every workflow table. Its tables and rows are conserved unchanged upward.
pub const SOURCE60_SCHEMA_HASH: &str = "wf1:7e663d320ef00a1181410d4610e24f6b6ee8729ca14f558d25c70a873f39053e";
pub const SOURCE_SCHEMA_HASH: &str = "wf1:9feeae84e856a84252be1ae4d038e3c35122d1808921d9b8754a304e730d3b09";
pub const CUTOVER67_SCHEMA_HASH: &str = "wf1:322d78f8e47556c67cabfdbc1e4e0a37b568aedd766b7f12e4a1e0eba471610f";
pub const MAP_PIN: &str = "bdff40f0b2944173c2be5b5fa906dcd2af88f7bf";
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cutover storage: {0}")] Storage(#[from] rusqlite::Error),
    #[error("cutover encoding: {0}")] Encoding(#[from] serde_json::Error),
    #[error("cutover files: {0}")] Files(#[from] std::io::Error),
    #[error("cutover refused: {0}")] Refused(String),
}
pub type Result<T> = std::result::Result<T, Error>;
fn refuse<T>(why: &str) -> Result<T> { Err(Error::Refused(why.into())) }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Authority {
    pub phase: String,
    pub incarnation: u64,
    pub manifest: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub workspace: String,
    pub task: String,
    pub item: String,
    pub disposition: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quarantine {
    pub resource: String,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preflight {
    pub source_pin: String,
    pub map_pin: String,
    pub registry: u32,
    pub raw: Manifest,
    pub bindings: Vec<Binding>,
    pub quarantines: Vec<Quarantine>,
    pub workspaces: BTreeSet<String>,
}
/// Runtime-owned proof, not liveness derived from a database row. The bootstrap
/// integration supplies the complete inventory and evidence; unknown writers stop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoppedWriter {
    pub identity: String,
    pub incarnation: String,
    pub joined_receipt: String,
    pub resources: BTreeSet<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quiescence {
    pub inventory: BTreeSet<String>,
    pub stopped: Vec<StoppedWriter>,
    pub protected_resources: BTreeSet<String>,
}
impl Quiescence {
    pub(crate) fn validate(&self) -> Result<()> {
        let identities: BTreeSet<_> = self.stopped.iter().map(|s| s.identity.clone()).collect();
        let resources: BTreeSet<_> = self.stopped.iter().flat_map(|s| s.resources.iter().cloned()).collect();
        let required:BTreeSet<String>=REQUIRED_OWNERS.into_iter().map(str::to_owned).collect();
        if !required.is_subset(&self.inventory) || identities != self.inventory || identities.len() != self.stopped.len()
            || resources != self.protected_resources || self.stopped.iter().any(|s|
                s.identity.is_empty() || s.incarnation.is_empty() || s.joined_receipt.is_empty()) {
            return refuse("incomplete joined writer/incarnation/resource evidence");
        }
        Ok(())
    }
}

/// True until the engine takes over. After release the daemon's legacy loops
/// (collecting, landing on green, landing follow-up of CI) must not run: their
/// writes are fenced and would only abort (ov-947). An unreadable phase counts
/// as released, the side that does nothing.
pub fn legacy_authority(conn: &Connection) -> bool {
    authority(conn).is_ok_and(|a| a.phase == "legacy")
}
impl Store {
    /// `legacy_authority` for the daemon's loops.
    pub fn legacy_authority(&self) -> bool { legacy_authority(&self.conn()) }
}
pub fn authority(conn: &Connection) -> Result<Authority> {
    Ok(conn.query_row("SELECT phase,incarnation,manifest FROM engine_cutover_authority WHERE singleton=1", [],
        |r| Ok(Authority { phase:r.get(0)?, incarnation:r.get(1)?, manifest:r.get(2)? }))?)
}
/// Called within the same writer transaction as apply/claim/admission. Old Store
/// handles cannot acquire the new incarnation just because the DB was transferred.
pub(crate) fn engine_allowed(conn: &Connection, incarnation: u64) -> rusqlite::Result<bool> {
    conn.query_row("SELECT incarnation=?1 AND phase IN ('legacy','released') FROM engine_cutover_authority WHERE singleton=1",
        [incarnation], |r| r.get(0))
}
pub(crate) fn migration(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("CREATE TABLE engine_cutover_authority(singleton INTEGER PRIMARY KEY CHECK(singleton=1),phase TEXT NOT NULL CHECK(phase IN ('legacy','quiesced','committed','released')),incarnation INTEGER NOT NULL,manifest TEXT);
        INSERT INTO engine_cutover_authority VALUES(1,'legacy',0,NULL);
        CREATE TABLE engine_cutover_workspaces(workspace BLOB PRIMARY KEY,incarnation INTEGER NOT NULL);
        CREATE TABLE engine_cutover_items(task BLOB PRIMARY KEY,workspace TEXT NOT NULL,item TEXT NOT NULL,disposition TEXT NOT NULL,UNIQUE(workspace,item));
        CREATE TABLE engine_cutover_ledger(source_pin TEXT NOT NULL,table_name TEXT NOT NULL,row_key TEXT NOT NULL,field TEXT NOT NULL,value_hash TEXT NOT NULL,destination TEXT NOT NULL,disposition TEXT NOT NULL,PRIMARY KEY(table_name,row_key,field));
        CREATE TABLE engine_cutover_quarantine(resource TEXT PRIMARY KEY,reason TEXT NOT NULL);
        CREATE TABLE engine_cutover_external(operation TEXT NOT NULL,resource TEXT NOT NULL,request_hash TEXT NOT NULL,fact TEXT NOT NULL,outcome TEXT NOT NULL,PRIMARY KEY(operation,resource));
        CREATE TRIGGER engine_external_no_update BEFORE UPDATE ON engine_cutover_external BEGIN SELECT RAISE(ABORT,'immutable external reconciliation'); END;
        CREATE TRIGGER engine_external_no_delete BEFORE DELETE ON engine_cutover_external BEGIN SELECT RAISE(ABORT,'immutable external reconciliation'); END;
        CREATE TABLE engine_cutover_attempts(id TEXT PRIMARY KEY,manifest TEXT NOT NULL,proof TEXT NOT NULL);
        CREATE TRIGGER engine_ledger_no_update BEFORE UPDATE ON engine_cutover_ledger BEGIN SELECT RAISE(ABORT,'immutable conservation ledger'); END;
        CREATE TRIGGER engine_ledger_no_delete BEFORE DELETE ON engine_cutover_ledger BEGIN SELECT RAISE(ABORT,'immutable conservation ledger'); END;
        CREATE TRIGGER engine_attempt_no_update BEFORE UPDATE ON engine_cutover_attempts BEGIN SELECT RAISE(ABORT,'immutable cutover attempt'); END;
        CREATE TRIGGER engine_attempt_no_delete BEFORE DELETE ON engine_cutover_attempts BEGIN SELECT RAISE(ABORT,'immutable cutover attempt'); END;")?;
    // Preserve readers and history. Legacy delivery and landing writes stay frozen
    // everywhere, even through another SQLite connection or FK cascade: the engine
    // has not taken those over yet (ov-945 tickets 4 and 5). Everything else is
    // board data that works as before, fenced only for a task the engine owns
    // (workflow_owned_items). No dropping evidence.
    for table in ["landings","landing_tickets","answer_wakes","hold_wakes","message_wakes"] {
        for (suffix,event) in [("i","INSERT"),("u","UPDATE"),("d","DELETE")] {
            tx.execute_batch(&format!("CREATE TRIGGER engine_fence_{table}_{suffix} BEFORE {event} ON {table}
                WHEN (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy'
                BEGIN SELECT RAISE(ABORT,'legacy execution authority retired'); END;"))?;
        }
    }
    let owned=|task:&str| format!("EXISTS(SELECT 1 FROM workflow_owned_items WHERE task_id={task})");
    let (owned_old,owned_old_task,owned_new_task)=(owned("OLD.id"),owned("OLD.task_id"),owned("NEW.task_id"));
    // Insert and update of these are already refused for an owned task by
    // workflow_excludes_*; deleting what the engine's card owns is refused here.
    for table in ["ticket_lines","ticket_work","task_blocks"] {
        tx.execute_batch(&format!("CREATE TRIGGER engine_fence_{table}_d BEFORE DELETE ON {table}
            WHEN {owned_old_task} AND (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy'
            BEGIN SELECT RAISE(ABORT,'task owned by workflow engine'); END;"))?;
    }
    tx.execute_batch(&format!("CREATE TRIGGER engine_fence_task_blocks_i BEFORE INSERT ON task_blocks
        WHEN {owned_new_task} AND (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy'
        BEGIN SELECT RAISE(ABORT,'task owned by workflow engine'); END;
        CREATE TRIGGER engine_fence_task_blocks_u BEFORE UPDATE ON task_blocks
        WHEN ({owned_old_task} OR {owned_new_task}) AND (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy'
        BEGIN SELECT RAISE(ABORT,'task owned by workflow engine'); END;
        CREATE TRIGGER engine_task_execution_fence BEFORE UPDATE OF status,status_since,wait_kind,wait_line,wait_rank,wait_until,wait_event,wait_since ON tasks
        WHEN {owned_old} AND (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy'
        BEGIN SELECT RAISE(ABORT,'legacy task execution authority retired'); END;
        CREATE TRIGGER engine_task_delete_fence BEFORE DELETE ON tasks
        WHEN {owned_old} AND (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy'
        BEGIN SELECT RAISE(ABORT,'retain migrated task identity'); END;"))?;
    for table in ["workflow_states","workflow_events","workflow_receipts","workflow_outbox","workflow_wire_receipts","workflow_definitions","workflow_configurations","workflow_bundles","workflow_bundle_selections"] {
        for (suffix,event) in [("i","INSERT"),("u","UPDATE"),("d","DELETE")] {
            tx.execute_batch(&format!("CREATE TRIGGER engine_freeze_{table}_{suffix} BEFORE {event} ON {table}
                WHEN (SELECT phase FROM engine_cutover_authority WHERE singleton=1) IN ('quiesced','committed')
                BEGIN SELECT RAISE(ABORT,'engine dispatch frozen for cutover'); END;"))?;
        }
    }
    tx.execute_batch(&format!("CREATE TRIGGER engine_task_identity_fence BEFORE UPDATE OF id,workspace_id,repository_id,key,former_key,worktree_id ON tasks
        WHEN (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy' AND {owned_old}
        AND NOT(OLD.id IS NEW.id AND OLD.workspace_id IS NEW.workspace_id AND OLD.repository_id IS NEW.repository_id AND OLD.key IS NEW.key AND OLD.former_key IS NEW.former_key AND OLD.worktree_id IS NEW.worktree_id)
        BEGIN SELECT RAISE(ABORT,'migrated task identity is fenced'); END;
        CREATE TRIGGER engine_terminal_task_insert_fence BEFORE INSERT ON terminals
        WHEN {owned_new_task} AND (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy'
        BEGIN SELECT RAISE(ABORT,'legacy task-bound terminal admission retired'); END;
        CREATE TRIGGER engine_terminal_task_delete_fence BEFORE DELETE ON terminals
        WHEN {owned_old_task} AND (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy'
        BEGIN SELECT RAISE(ABORT,'retain migrated terminal evidence'); END;
        CREATE TRIGGER engine_terminal_task_update_fence BEFORE UPDATE OF task_id,worktree_id,workspace_id,command_preset,intent,runtime_confirmed,lease_generation,epoch,agent_session_id,role,pane_mode ON terminals
        WHEN ({owned_old_task} OR {owned_new_task}) AND (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy'
        AND NOT(OLD.task_id IS NEW.task_id AND OLD.worktree_id IS NEW.worktree_id AND OLD.workspace_id IS NEW.workspace_id AND OLD.command_preset IS NEW.command_preset AND OLD.intent IS NEW.intent AND OLD.runtime_confirmed IS NEW.runtime_confirmed AND OLD.lease_generation IS NEW.lease_generation AND OLD.epoch IS NEW.epoch AND OLD.agent_session_id IS NEW.agent_session_id AND OLD.role IS NEW.role AND OLD.pane_mode IS NEW.pane_mode)
        BEGIN SELECT RAISE(ABORT,'legacy task-bound terminal restart retired'); END;"))?;
    Ok(())
}

/// Store::init and historical fixture helpers have these two equivalent meta
/// spellings. Recognize only these exact declarations, not arbitrary SQL.
pub(crate) fn catalog_hash(raw:&Manifest) -> String {
    let mut objects=raw.objects.clone();
    if let Some((_,_,sql))=objects.get_mut("meta") {
        if sql.as_deref()==Some("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)") {
            *sql=Some("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT)".into());
        }
    }
    farcooler_workflow::digest(&objects)
}
fn field<'a>(table: &'a manifest::Table, row: &'a [Cell], name: &str) -> Result<&'a Cell> {
    table.columns.iter().position(|c| c.name==name).and_then(|i| row.get(i))
        .ok_or_else(|| Error::Refused(format!("missing source field {name}")))
}
fn uuid(value: &Cell) -> Result<String> {
    value.bytes().and_then(|b| uuid::Uuid::from_slice(b).ok()).map(|v| v.to_string())
        .ok_or_else(|| Error::Refused("invalid UUID binding".into()))
}
/// Capture before Store::open: no waits sweep or schema stamp can erase raw facts.
pub fn preflight(conn: &Connection) -> Result<Preflight> {
    let registry = crate::migrate::read_schema_version(conn).map_err(|e| Error::Refused(e.to_string()))?;
    if !matches!(registry,60|65..=67) { return refuse("expected exact raw schema60, composed schema65, cutover schema66 or manual schema67"); }
    let unsafe_generation:bool=registry>=65 && conn.query_row("SELECT EXISTS(SELECT 1 FROM workflow_resource_generations WHERE typeof(generation)!='integer' OR generation<0 OR generation=9223372036854775807)
        OR EXISTS(SELECT 1 FROM workflow_resources r LEFT JOIN workflow_resource_generations g ON g.resource=r.resource WHERE g.resource IS NULL OR typeof(r.generation)!='integer' OR r.generation!=g.generation)",[],|r|r.get(0))?;
    if unsafe_generation {return refuse("resource generation is invalid, exhausted or inconsistent; no authority transfer");}
    let mut raw = Manifest::capture(conn)?;
    let cutover_objects:BTreeMap<_,_>=raw.objects.iter().filter(|(name,(_,owner,_))|name.starts_with("engine_")||owner.starts_with("engine_cutover_")).map(|(name,value)|(name.clone(),value.clone())).collect();
    if registry>=66 && farcooler_workflow::digest(&cutover_objects)!=if registry==66 {CUTOVER_SCHEMA_HASH} else {CUTOVER67_SCHEMA_HASH} {return Err(Error::Refused(format!("cutover schema objects differ from the pinned schema; preserve and stop ({})",farcooler_workflow::digest(&cutover_objects))));}
    // Only the exact signed 103-object namespace may be excluded as migration
    // metadata. Unknown same-prefix tables/triggers can never hide in a prefix.
    if registry>=66 {
        raw.tables.retain(|name,_| !name.starts_with("engine_cutover_") && !name.starts_with("engine_manual_") && !name.starts_with("engine_notice_"));
        raw.objects.retain(|name,(_,owner,_)| !name.starts_with("engine_") && !owner.starts_with("engine_cutover_") && !owner.starts_with("engine_manual_") && !owner.starts_with("engine_notice_"));
    }
    let mut result = Preflight { source_pin:SOURCE_PIN.into(), map_pin:MAP_PIN.into(), registry,
        raw, bindings:Vec::new(), quarantines:Vec::new(), workspaces:BTreeSet::new() };
    let workspaces = result.raw.tables.get("workspaces").ok_or_else(|| Error::Refused("missing workspaces".into()))?;
    for row in &workspaces.rows { result.workspaces.insert(uuid(field(workspaces,row,"id")?)?); }
    let catalog=catalog_hash(&result.raw);
    // The raw60 path reads no table until the catalog is the pinned one.
    if registry==60 && catalog!=SOURCE60_SCHEMA_HASH { return Err(Error::Refused(format!("schema60 catalog differs from the pinned registry prefix: {catalog}"))); }
    let schema_unknown=registry!=60 && catalog!=SOURCE_SCHEMA_HASH;
    if schema_unknown { result.quarantines.push(Quarantine {resource:"schema:unknown".into(),reason:"composed source schema differs from pinned schema65 catalog; retain every raw field, block release".into()}); }
    let tasks = &result.raw.tables["tasks"];
    let mut aliases: BTreeMap<(String,String),Vec<String>> = BTreeMap::new();
    for row in &tasks.rows {
        let task=uuid(field(tasks,row,"id")?)?;
        let workspace=uuid(field(tasks,row,"workspace_id")?)?;
        if !result.workspaces.contains(&workspace) { return refuse("task outside source workspace set"); }
        for name in ["key","former_key"] {
            if let Some(alias)=field(tasks,row,name)?.text() {
                aliases.entry((workspace.clone(),alias.to_ascii_lowercase())).or_default().push(task.clone());
            }
        }
        // UUID is the inert import identity. No reducer commands, grants, gates,
        // receipts, active runs or default definitions are invented here.
        result.bindings.push(Binding { workspace, item:task.clone(), task, disposition:if schema_unknown {"quarantined"} else {"retained-intent"}.into() });
    }
    for ((workspace,alias), ids) in aliases {
        if ids.iter().collect::<BTreeSet<_>>().len()>1 {
            result.quarantines.push(Quarantine { resource:format!("alias:{workspace}:{alias}"), reason:"ambiguous task alias; no arbitrary winner".into() });
            for binding in &mut result.bindings { if ids.contains(&binding.task) { binding.disposition="quarantined".into(); } }
        }
    }
    // Every original table/column remains represented by the typed raw manifest.
    // No unknown is dropped or interpreted as a successful engine operation.
    for (name,table) in &result.raw.tables {
        if name.starts_with("engine_cutover_") { continue; }
        for row in &table.rows {
            let uncertain = match name.as_str() {
                "workflow_outbox" => matches!(field(table,row,"dispatch_state")?.text(),Some("admitted"|"uncertain")),
                "check_in_occurrences" => matches!(field(table,row,"state")?.text(),Some("claimed"|"enter_admitted"|"submitted"|"uncertain")),
                "answer_wakes"|"hold_wakes"|"message_wakes"|"compose_holds" => !matches!(field(table,row,"claimed_at")?,Cell::Null),
                "ticket_work" => matches!(field(table,row,"ended_at")?,Cell::Null),
                _ => false,
            };
            if uncertain {
                let key=farcooler_workflow::digest(row);
                let resource=if name=="workflow_outbox" {
                    let payload=field(table,row,"payload")?.text().unwrap_or("");
                    serde_json::from_str::<serde_json::Value>(payload).ok().and_then(|v| v.get("resource").and_then(|v|v.as_str()).map(str::to_owned)).unwrap_or_else(||format!("{name}:{key}"))
                } else { format!("{name}:{key}") };
                if let Ok(task)=field(table,row,"task_id").and_then(uuid) {
                    for binding in &mut result.bindings { if binding.task==task { binding.disposition="quarantined".into(); } }
                }
                result.quarantines.push(Quarantine {resource,reason:format!("retained unresolved {name}; absence of receipt is not retry permission")});
            }
        }
    }
    if registry>=65 {
    let states=&result.raw.tables["workflow_states"];
    for row in &states.rows {
        let workspace=field(states,row,"workspace")?.text().unwrap_or("");
        let original=field(states,row,"state")?.text().and_then(|raw|serde_json::from_str::<serde_json::Value>(raw).ok());
        let typed=original.clone().and_then(|value|serde_json::from_value::<farcooler_workflow::State>(value).ok());
        let valid=typed.as_ref().is_some_and(|state|state.schema==1 && state.workspace==workspace
            && serde_json::to_value(state).ok()==original
            && field(states,row,"revision").ok()==Some(&Cell::Integer(state.revision as i64))
            && field(states,row,"epoch").ok()==Some(&Cell::Integer(state.epoch as i64)));
        if !valid {
            result.quarantines.push(Quarantine {resource:format!("workspace:{workspace}"),reason:"unknown/conflicting engine state fields; exact raw bytes retained, no running-definition rewrite".into()});
            for binding in &mut result.bindings {if binding.workspace==workspace {binding.disposition="quarantined".into();}}
        }
    }
    let outbox=&result.raw.tables["workflow_outbox"];
    for row in &outbox.rows {
        if !matches!(field(outbox,row,"dispatch_state")?.text(),Some("pending"|"claimed"|"admitted")) {continue;}
        let workspace=field(outbox,row,"workspace")?.text().unwrap_or("");
        let payload=field(outbox,row,"payload")?.text().unwrap_or("");
        let effect=crate::workflow_effects::checked_effect(payload,field(outbox,row,"id")?.text().unwrap_or(""),field(outbox,row,"payload_hash")?.text().unwrap_or(""));
        let bound=effect.as_ref().is_some_and(|effect| {
            if field(outbox,row,"active").ok()==Some(&Cell::Integer(0)) {return true;}
            states.rows.iter().any(|state_row| {
                field(states,state_row,"workspace").ok().and_then(Cell::text)==Some(workspace)
                    && field(states,state_row,"state").ok().and_then(Cell::text)
                        .and_then(|raw|serde_json::from_str::<farcooler_workflow::State>(raw).ok())
                        .is_some_and(|state| match field(outbox,row,"revision") {
                            Ok(Cell::Integer(revision)) if *revision>=0 => crate::workflow_effects::effect_binds(effect,&state,workspace,*revision as u64),
                            _ => false,
                        })
            })
        });
        if !bound {
            result.quarantines.push(Quarantine {resource:format!("workspace:{workspace}"),reason:"unknown/lossy or conflicting executable Effect; raw payload retained".into()});
            if let Some(resource)=serde_json::from_str::<serde_json::Value>(payload).ok().and_then(|v|v.get("resource").and_then(|r|r.as_str()).map(str::to_owned)) {
                result.quarantines.push(Quarantine {resource,reason:"uninterpretable executable Effect resource".into()});
            }
            for binding in &mut result.bindings {if binding.workspace==workspace {binding.disposition="quarantined".into();}}
        }
    }
    }
    // Unsupported legacy execution policies remain inert and explicitly block
    // conflicting fresh admission rather than being replaced by engine defaults.
    for name in ["task_blocks","ticket_lines","plan_budgets","check_in_schedules","check_in_enter_fences"] {
        let table=&result.raw.tables[name];
        for row in &table.rows {
            if name=="ticket_lines" && matches!(field(table,row,"stage")?.text(),Some("landed"|"dropped")) {continue;}
            if name=="check_in_schedules" && matches!(field(table,row,"state")?.text(),Some("disabled"|"suspended")) {continue;}
            if let Ok(task)=field(table,row,"task_id").and_then(uuid) {
                for binding in &mut result.bindings { if binding.task==task {binding.disposition="quarantined".into();} }
                result.quarantines.push(Quarantine {resource:format!("task:{task}:{name}"),reason:format!("retained {name} requires reviewed policy/constraint mapping before new admission")});
            }
        }
    }
    let budgets=&result.raw.tables["plan_budgets"];let themes=&result.raw.tables["board_theme_tasks"];
    for row in &budgets.rows {
        let theme=field(budgets,row,"theme_id")?;
        if matches!(theme,Cell::Null) {continue;}
        for membership in &themes.rows {
            if field(themes,membership,"theme_id")?!=theme {continue;}
            let task=uuid(field(themes,membership,"task_id")?)?;
            for binding in &mut result.bindings {if binding.task==task {binding.disposition="quarantined".into();}}
            result.quarantines.push(Quarantine {resource:format!("task:{task}:theme-budget"),reason:"retained theme budget enforcement requires explicit mapping".into()});
        }
    }
    let policies=&result.raw.tables["workspace_landing"];
    for row in &policies.rows {
        let workspace=uuid(field(policies,row,"workspace_id")?)?;
        for binding in &mut result.bindings {if binding.workspace==workspace {binding.disposition="quarantined".into();}}
        result.quarantines.push(Quarantine {resource:format!("policy:{workspace}:landing"),reason:"retained workspace landing/CI/person policy needs selected immutable bundle mapping".into()});
    }
    let landings=&result.raw.tables["landings"];
    let membership=&result.raw.tables["landing_tickets"];
    for row in &membership.rows {
        let landing=field(membership,row,"landing_id")?;
        let settled=landings.rows.iter().any(|parent|field(landings,parent,"id").ok()==Some(landing) && matches!(field(landings,parent,"state").ok().and_then(Cell::text),Some("landed"|"dropped")));
        if !settled {let task=uuid(field(membership,row,"task_id")?)?;
            for binding in &mut result.bindings {if binding.task==task {binding.disposition="quarantined".into();}}
            result.quarantines.push(Quarantine {resource:format!("task:{task}:landing"),reason:"unsettled legacy landing membership retained; exact push/route reconciliation required".into()});
        }
    }
    // Pointer bytes are always retained. Preflight never follows a DB value into
    // owner resources or substitutes missing targets with an empty resource set.
    for (name,columns) in [("repository_roots",vec!["path"]),("repositories",vec!["canonical_git_dir"]),("worktrees",vec!["worktree_path"]),("agents",vec!["session_cwd"]),("terminal_logins",vec!["config_dir"]),("check_in_occurrences",vec!["rollout_path"])] {
        let table=&result.raw.tables[name];
        for row in &table.rows { for column in &columns {
            if field(table,row,column)?.text().is_some_and(|v|!v.is_empty()) {
                result.quarantines.push(Quarantine {resource:format!("pointer:{name}:{}:{column}",verify::row_key(table,row)),reason:"unverified source pointer retained verbatim; filesystem traversal prohibited until explicit private resource mapping".into()});
            }
        }}
    }
    for row in &tasks.rows {
        let status=field(tasks,row,"status")?.text();
        let wait=field(tasks,row,"wait_kind")?.text();
        if !matches!(status,Some("backlog"|"todo"|"needs_decision"|"in_progress"|"in_review"|"done"|"cancelled")) || wait.is_some() {
            let task=uuid(field(tasks,row,"id")?)?;
            for binding in &mut result.bindings { if binding.task==task { binding.disposition="quarantined".into(); } }
            result.quarantines.push(Quarantine {resource:format!("task:{task}"),reason:"legacy wait/status needs explicit semantic disposition; raw bytes retained".into()});
        }
    }
    Ok(result)
}

impl Store {
    pub fn engine_authority(&self) -> Result<Authority> { authority(&self.conn()) }
    /// Resources a join proof must protect (workflow, terminal, worktree identities).
    pub fn engine_protected_resources(&self) -> Result<BTreeSet<String>> { required_resources(&self.conn()) }
    pub fn engine_quiesce(&self, owners: JoinedOwners) -> Result<()> {
        let mut conn=self.conn();
        owners.check_store(&conn)?;
        let tx=conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        frozen_by(&tx,owners.attempt())?;
        match authority(&tx)?.phase.as_str() {
            "quiesced" => return Ok(()), // transfer::begin already froze it atomically with the upgrade
            "legacy" => {}
            _ => return refuse("authority is not legacy"),
        }
        tx.execute("UPDATE engine_cutover_authority SET phase='quiesced' WHERE singleton=1",[])?;
        tx.commit()?;
        Ok(())
    }
    /// Transaction truth is the recovery oracle. No workspace routing switch,
    /// no legacy row deletion, and no external operation occurs in this boundary.
    pub fn engine_commit(&self, plan: &Preflight, owners: JoinedOwners, permit: &CommitPermit) -> Result<Authority> {
        let proof=permit.evidence();
        proof.validate()?;
        permit.validate(plan)?;
        let attempt=permit.attempt();
        if owners.attempt()!=attempt { return refuse("join proof belongs to another attempt"); }
        if attempt.is_empty() || plan.source_pin!=SOURCE_PIN || plan.map_pin!=MAP_PIN { return refuse("unbound cutover identity"); }
        let mut conn=self.conn();
        owners.check_store(&conn)?;
        let tx=conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        frozen_by(&tx,attempt)?;
        if required_resources(&tx)?!=*proof.protected() { return refuse("protected resource inventory changed"); }
        if authority(&tx)?.phase!="quiesced" { return refuse("cutover not quiesced"); }
        let current=Manifest::capture(&tx)?;
        if !plan.raw.conserved_by(&current) { return refuse("source changed after raw preflight"); }
        let mut actual=BTreeSet::new();
        for row in &current.tables["workspaces"].rows { actual.insert(uuid(field(&current.tables["workspaces"],row,"id")?)?); }
        if actual!=plan.workspaces { return refuse("all-workspace coverage changed"); }
        // Plan fields are recomputed from raw rather than trusting caller supplied
        // bindings or quarantine edits, even though the entrypoint is admin-only.
        let derived=preflight(&tx)?;
        if !verify::catalog_matches(plan,&derived.raw) || derived.bindings!=plan.bindings || derived.quarantines!=plan.quarantines { return refuse("mapper disposition changed"); }
        let incarnation=authority(&tx)?.incarnation.checked_add(1).ok_or_else(|| Error::Refused("incarnation exhausted".into()))?;
        for workspace in &plan.workspaces {
            let id=uuid::Uuid::parse_str(workspace).map_err(|_| Error::Refused("bad workspace".into()))?;
            tx.execute("INSERT INTO engine_cutover_workspaces VALUES(?1,?2)",params![id.as_bytes().as_slice(),incarnation])?;
        }
        for binding in &plan.bindings {
            let id=uuid::Uuid::parse_str(&binding.task).map_err(|_| Error::Refused("bad task".into()))?;
            tx.execute("INSERT INTO engine_cutover_items VALUES(?1,?2,?3,?4)",params![id.as_bytes().as_slice(),binding.workspace,binding.item,binding.disposition])?;
        }
        for (name,table) in &plan.raw.tables {
            if name.starts_with("engine_cutover_") { continue; }
            for row in &table.rows {
                let row_key=verify::row_key(table,row);
                if let Some(ids)=table.rowids.as_ref().and_then(|ids|ids.get(&row_key)) {
                    tx.execute("INSERT INTO engine_cutover_ledger VALUES(?1,?2,?3,'@rowid',?4,?5,'P')",params![SOURCE_PIN,name,row_key,farcooler_workflow::digest(ids),format!("{name}/{row_key}/@rowid")])?;
                }
                for (column,value) in table.columns.iter().zip(row) {
                    let disposition=if name=="meta" && matches!(row.first().and_then(Cell::text),Some("schema_version"|"compatible_down_to")) && column.name=="value" {"M:registry66"} else {"P"};
                    tx.execute("INSERT INTO engine_cutover_ledger VALUES(?1,?2,?3,?4,?5,?6,?7)",params![SOURCE_PIN,name,row_key,column.name,farcooler_workflow::digest(value),format!("{name}/{row_key}/{}",column.name),disposition])?;
                }
            }
        }
        for q in &plan.quarantines {
            tx.execute("INSERT INTO engine_cutover_quarantine VALUES(?1,?2) ON CONFLICT(resource) DO NOTHING",params![q.resource,q.reason])?;
        }
        tx.execute("INSERT INTO engine_cutover_attempts VALUES(?1,?2,?3)",params![attempt,plan.raw.hash(),serde_json::to_string(&(proof,permit.backup_hash()))?])?;
        // Preserve immutable event/state/receipt bytes. New resource generations
        // fence claims; admitted/uncertain effects remain non-retryable as-is.
        tx.execute("UPDATE workflow_resource_generations SET generation=generation+1",[])?;
        tx.execute("UPDATE workflow_resources SET generation=(SELECT generation FROM workflow_resource_generations g WHERE g.resource=workflow_resources.resource)",[])?;
        crate::engine_manual::record_notice_floor(&tx,incarnation)?;
        tx.execute("UPDATE engine_cutover_authority SET phase='committed',incarnation=?1,manifest=?2 WHERE singleton=1",params![incarnation,plan.raw.hash()])?;
        crate::compat::stamp_compatible_down_to(&tx).map_err(|e|Error::Refused(e.to_string()))?;
        #[cfg(test)]
        if let Some(hook)=BEFORE_SEAL.with(|h|h.borrow_mut().take()) { hook(&tx); }
        // A commit that could not verify must stay frozen, where Recover still
        // works: past this transaction the attempt only goes forward.
        verify::check(plan,&tx)?;
        boundary::reach("commit.in-mapping");
        tx.commit()?;
        boundary::reach("commit.after-mapping");
        authority(&conn)
    }
}

#[cfg(test)]
type SealHook=Box<dyn FnOnce(&Connection)>;
#[cfg(test)]
thread_local! { pub(crate) static BEFORE_SEAL: std::cell::RefCell<Option<SealHook>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
mod tests;

/// A transfer's frozen attempt binds every later stage to it. A database with
/// none was frozen outside a transfer, by the private rehearsal.
fn frozen_by(conn:&Connection,attempt:&str) -> Result<()> {
    let frozen:Option<String>=conn.query_row("SELECT id FROM engine_cutover_attempts WHERE id LIKE 'frozen:%'",[],|r|r.get(0)).optional()?;
    if frozen.is_some_and(|id|id!=format!("frozen:{attempt}")) { return refuse("join proof belongs to another attempt"); }
    Ok(())
}
fn has_table(conn:&Connection,name:&str) -> Result<bool> {
    Ok(conn.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",[name],|_|Ok(())).optional()?.is_some())
}
/// Every protected resource and its generation, where the source has one.
fn resource_generations(conn:&Connection) -> Result<BTreeMap<String,Option<i64>>> {
    let mut resources:BTreeMap<String,Option<i64>>=required_resources(conn)?.into_iter().map(|r|(r,None)).collect();
    if has_table(conn,"workflow_resource_generations")? {
        let mut query=conn.prepare("SELECT resource,generation FROM workflow_resource_generations")?;
        for row in query.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))? {
            let (resource,generation)=row?;
            resources.insert(resource,Some(generation));
        }
    }
    Ok(resources)
}
fn required_resources(conn:&Connection) -> Result<BTreeSet<String>> {
    let mut resources:BTreeSet<String>=BTreeSet::new();
    if has_table(conn,"workflow_resources")? {
        resources=conn.prepare("SELECT resource FROM workflow_resources")?.query_map([],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
    }
    for table in ["terminals","worktrees"] {
        let mut query=conn.prepare(&format!("SELECT id FROM {table}"))?;
        for row in query.query_map([],|r|r.get::<_,Vec<u8>>(0))? {
            let id=uuid::Uuid::from_slice(&row?).map_err(|_|Error::Refused("invalid protected resource identity".into()))?;
            resources.insert(format!("{table}:{id}"));
        }
    }
    Ok(resources)
}

#[cfg(test)]
impl Store {
    // Unit boundary tests intentionally do not claim closed file backup proof.
    // The public API has no unchecked permit constructor; CLI uses real backup.
    fn engine_commit_for_test(&self,plan:&Preflight,proof:&Quiescence,attempt:&str) -> Result<Authority> {
        self.engine_commit(plan,JoinedOwners::unit(attempt),&CommitPermit::unit(plan,proof,attempt))
    }
}
