//! What agents spent, one row per finished turn, for reports.
//!
//! A turn row says WHERE the work was (terminal, worktree, repository,
//! workspace, and task by the existing link, all as they were when the turn
//! ended) and how long it ran; its model rows say what it spent on each
//! model, and what that cost and how the cost is known.
//!
//! The ids are copies, not foreign keys, on purpose: a report about last week
//! still has to count the turns of a terminal closed since, and a cascade
//! would erase them. A key that names something gone still groups.
//!
//! Every cost carries its provenance (`farcooler_core::usage::CostSource`): the
//! agent's own figure, an estimate stamped with the price table it came from,
//! or unknown. An aggregate keeps the three apart so a report can say "80%
//! reported" rather than present a sum of unlike things as one fact.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rusqlite::{Transaction, params};
use uuid::Uuid;

use farcooler_core::Result;
use farcooler_core::usage::{CostSource, PRICE_TABLE, TokenCounts, estimate};

use crate::error::map_err;
use crate::models::{get_uuid, uuid_blob};
use crate::store::Store;

/// Two tables and nothing else touched: additive, so a build from before it
/// reads every table it knows exactly as it did.
pub(crate) fn migration_0020_agent_turns(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE agent_turns (
            id BLOB PRIMARY KEY,
            -- The harness's own name for the turn. A turn heard twice (a
            -- shim replaying its ring, a log re-read after rotation) is one.
            turn_key TEXT NOT NULL UNIQUE,
            terminal_id BLOB,
            worktree_id BLOB,
            repository_id BLOB,
            workspace_id BLOB,
            task_id BLOB,
            -- `claude`, `codex`, or an ACP adapter's preset.
            harness TEXT NOT NULL,
            -- `chat` (a protocol the daemon speaks) or `terminal` (a log it reads).
            surface TEXT NOT NULL,
            started_at INTEGER,
            ended_at INTEGER NOT NULL,
            active_ms INTEGER,
            -- `reported`, `partial` (some calls stated none) or `not_reported`.
            usage TEXT NOT NULL
        );
        CREATE INDEX agent_turns_by_end ON agent_turns (ended_at);
        CREATE INDEX agent_turns_by_task ON agent_turns (task_id, ended_at) WHERE task_id IS NOT NULL;

        CREATE TABLE agent_turn_models (
            turn_id BLOB NOT NULL REFERENCES agent_turns(id) ON DELETE CASCADE,
            -- Empty when the harness named no model.
            model TEXT NOT NULL,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            cache_read_tokens INTEGER NOT NULL,
            cache_write_tokens INTEGER NOT NULL,
            -- Millionths of a US dollar. NULL exactly when `cost_source` is
            -- `unknown`.
            cost_micros INTEGER,
            cost_source TEXT NOT NULL,
            -- The price table an estimate came from; NULL unless estimated.
            price_table TEXT,
            PRIMARY KEY (turn_id, model)
        );
        "#,
    )
}

/// Which way a turn was heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Chat,
    Terminal,
}

impl Surface {
    fn as_str(self) -> &'static str {
        match self {
            Surface::Chat => "chat",
            Surface::Terminal => "terminal",
        }
    }
}

/// One model's share of a turn, priced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnModel {
    pub model: Option<String>,
    pub tokens: TokenCounts,
    pub cost_micros: Option<i64>,
    pub cost_source: CostSource,
    pub price_table: Option<String>,
}

impl TurnModel {
    /// The agent's own figure when it gave one; else an estimate from the
    /// price table, stamped with its date; else unknown.
    pub fn priced(model: Option<String>, tokens: TokenCounts, reported_micros: Option<i64>) -> TurnModel {
        let (cost_micros, cost_source, price_table) = match reported_micros {
            Some(micros) => (Some(micros), CostSource::Reported, None),
            None => match estimate(model.as_deref(), &tokens) {
                Some(micros) => (Some(micros), CostSource::Estimated, Some(PRICE_TABLE.to_string())),
                None => (None, CostSource::Unknown, None),
            },
        };
        TurnModel { model, tokens, cost_micros, cost_source, price_table }
    }
}

/// A finished turn, ready to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTurn {
    pub key: String,
    pub terminal_id: Option<Uuid>,
    pub worktree_id: Option<Uuid>,
    pub repository_id: Option<Uuid>,
    pub workspace_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub harness: String,
    pub surface: Surface,
    pub started_at: Option<i64>,
    pub ended_at: i64,
    pub active_ms: Option<i64>,
    /// `reported`, `partial` or `not_reported`; see the column.
    pub usage: &'static str,
    pub models: Vec<TurnModel>,
}

/// What to count. Every field narrows; `None` is everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageFilter {
    /// Unix milliseconds, inclusive, on the turn's end.
    pub since: Option<i64>,
    /// Unix milliseconds, exclusive.
    pub until: Option<i64>,
    pub task_id: Option<Uuid>,
    pub worktree_id: Option<Uuid>,
    pub workspace_id: Option<Uuid>,
    pub repository_id: Option<Uuid>,
    pub harness: Option<String>,
    /// Matches a model row's name exactly; `""` is the unnamed model.
    pub model: Option<String>,
}

/// A dimension to break totals down by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GroupBy {
    Task,
    Worktree,
    Workspace,
    Repository,
    Harness,
    Model,
    /// The turn's end date, `YYYY-MM-DD`, in the caller's offset.
    Day,
    /// The Monday the turn's week began, `YYYY-MM-DD`.
    Week,
    /// `YYYY-MM`.
    Month,
}

/// Sums over a set of turns.
///
/// `turns` and `active_ms` count each turn once per group it touches, so a
/// turn on two models appears under both when grouped by model: sum them
/// across tasks or days, never across models.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageTotals {
    pub turns: u64,
    /// Turns whose tokens are a floor: some model call stated none.
    pub turns_partial: u64,
    /// Turns that stated no usage at all, counted but with no tokens.
    pub turns_not_reported: u64,
    pub active_ms: i64,
    pub tokens: TokenCounts,
    /// What the agents said it cost.
    pub cost_reported_micros: i64,
    /// What the price table says it would have cost.
    pub cost_estimated_micros: i64,
    /// Tokens on model rows whose cost is unknown: no model, or no rate.
    pub unpriced_tokens: u64,
    /// Every price table an estimate here came from, oldest first.
    pub price_tables: Vec<String>,
    pub first_ended_at: Option<i64>,
    pub last_ended_at: Option<i64>,
}

/// One group's key and totals. A key field is `Some` only where that
/// dimension was grouped AND the turn had a value for it; the unnamed model
/// is `Some("")`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UsageKey {
    pub task_id: Option<Uuid>,
    pub worktree_id: Option<Uuid>,
    pub workspace_id: Option<Uuid>,
    pub repository_id: Option<Uuid>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub period: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageGroup {
    pub key: UsageKey,
    pub totals: UsageTotals,
}

struct Row {
    id: Uuid,
    task: Option<Uuid>,
    worktree: Option<Uuid>,
    workspace: Option<Uuid>,
    repository: Option<Uuid>,
    harness: String,
    ended_at: i64,
    active_ms: Option<i64>,
    usage: String,
}

struct ModelRow {
    model: String,
    tokens: TokenCounts,
    cost_micros: Option<i64>,
    source: CostSource,
    table: Option<String>,
}

impl Store {
    /// Record one finished turn. False when its key was already recorded,
    /// which is a turn heard twice and not an error.
    pub fn record_turn(&self, turn: &NewTurn) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let id = Uuid::now_v7();
        let opt = |id: Option<Uuid>| id.map(uuid_blob);
        let inserted = tx
            .execute(
                "INSERT OR IGNORE INTO agent_turns (id, turn_key, terminal_id, worktree_id, repository_id,
                     workspace_id, task_id, harness, surface, started_at, ended_at, active_ms, usage)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    uuid_blob(id),
                    turn.key,
                    opt(turn.terminal_id),
                    opt(turn.worktree_id),
                    opt(turn.repository_id),
                    opt(turn.workspace_id),
                    opt(turn.task_id),
                    turn.harness,
                    turn.surface.as_str(),
                    turn.started_at,
                    turn.ended_at,
                    turn.active_ms,
                    turn.usage,
                ],
            )
            .map_err(map_err)?;
        if inserted == 0 {
            return Ok(false);
        }
        for m in &turn.models {
            let t = &m.tokens;
            // Two rows for one model (a harness repeating a name) add up
            // rather than colliding on the key.
            tx.execute(
                "INSERT INTO agent_turn_models (turn_id, model, input_tokens, output_tokens,
                     cache_read_tokens, cache_write_tokens, cost_micros, cost_source, price_table)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (turn_id, model) DO UPDATE SET
                     input_tokens = input_tokens + excluded.input_tokens,
                     output_tokens = output_tokens + excluded.output_tokens,
                     cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
                     cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens,
                     cost_micros = CASE WHEN cost_source = excluded.cost_source
                                        THEN cost_micros + excluded.cost_micros END,
                     cost_source = CASE WHEN cost_source = excluded.cost_source
                                        THEN cost_source ELSE 'unknown' END",
                params![
                    uuid_blob(id),
                    m.model.as_deref().unwrap_or_default(),
                    t.input as i64,
                    t.output as i64,
                    t.cache_read as i64,
                    t.cache_write as i64,
                    m.cost_micros,
                    m.cost_source.as_str(),
                    m.price_table,
                ],
            )
            .map_err(map_err)?;
        }
        tx.commit().map_err(map_err)?;
        Ok(true)
    }

    /// Totals over the turns `filter` admits, one per distinct combination
    /// of `group_by`'s dimensions, in key order. No dimensions is one group.
    /// `utc_offset_minutes` places a period's boundaries in the caller's day.
    pub fn usage_summary(
        &self,
        filter: &UsageFilter,
        group_by: &[GroupBy],
        utc_offset_minutes: i32,
    ) -> Result<Vec<UsageGroup>> {
        let (turns, models) = self.usage_rows(filter)?;
        let mut groups: BTreeMap<UsageKey, (UsageTotals, BTreeSet<String>)> = BTreeMap::new();
        for turn in &turns {
            let rows: Vec<&ModelRow> = models
                .get(&turn.id)
                .map(|rows| {
                    rows.iter().filter(|r| filter.model.as_ref().is_none_or(|m| *m == r.model)).collect()
                })
                .unwrap_or_default();
            if filter.model.is_some() && rows.is_empty() {
                continue;
            }
            let key_for = |model: Option<&str>| key(turn, model, group_by, utc_offset_minutes);
            // Turn-level sums once per group this turn reaches.
            let mut reached: BTreeSet<UsageKey> = BTreeSet::new();
            if rows.is_empty() {
                reached.insert(key_for(None));
            }
            for row in &rows {
                let k = key_for(Some(&row.model));
                let (totals, tables) = groups.entry(k.clone()).or_default();
                add_model(totals, tables, row);
                reached.insert(k);
            }
            for k in reached {
                let (totals, _) = groups.entry(k).or_default();
                add_turn(totals, turn);
            }
        }
        Ok(groups
            .into_iter()
            .map(|(key, (mut totals, tables))| {
                totals.price_tables = tables.into_iter().collect();
                UsageGroup { key, totals }
            })
            .collect())
    }

    /// One task's totals, and the same broken down by harness and model.
    /// Indexed by task, so it is cheap enough to ask whenever a task opens.
    pub fn task_usage(&self, task: Uuid) -> Result<(UsageTotals, Vec<UsageGroup>)> {
        let filter = UsageFilter { task_id: Some(task), ..Default::default() };
        let total = self.usage_summary(&filter, &[], 0)?.pop().map(|g| g.totals).unwrap_or_default();
        let split = self.usage_summary(&filter, &[GroupBy::Harness, GroupBy::Model], 0)?;
        Ok((total, split))
    }

    fn usage_rows(&self, f: &UsageFilter) -> Result<(Vec<Row>, HashMap<Uuid, Vec<ModelRow>>)> {
        let conn = self.conn();
        let blob = |id: Option<Uuid>| id.map(uuid_blob);
        let mut stmt = conn
            .prepare(
                "SELECT id, task_id, worktree_id, workspace_id, repository_id, harness, ended_at, active_ms, usage
                   FROM agent_turns
                  WHERE (?1 IS NULL OR ended_at >= ?1) AND (?2 IS NULL OR ended_at < ?2)
                    AND (?3 IS NULL OR task_id = ?3) AND (?4 IS NULL OR worktree_id = ?4)
                    AND (?5 IS NULL OR workspace_id = ?5) AND (?6 IS NULL OR repository_id = ?6)
                    AND (?7 IS NULL OR harness = ?7)
                  ORDER BY ended_at, rowid",
            )
            .map_err(map_err)?;
        let opt_uuid = |r: &rusqlite::Row, i: usize| -> rusqlite::Result<Option<Uuid>> {
            let raw: Option<Vec<u8>> = r.get(i)?;
            Ok(raw.and_then(|b| Uuid::from_slice(&b).ok()))
        };
        let turns = stmt
            .query_map(
                params![
                    f.since,
                    f.until,
                    blob(f.task_id),
                    blob(f.worktree_id),
                    blob(f.workspace_id),
                    blob(f.repository_id),
                    f.harness
                ],
                |r| {
                    Ok(Row {
                        id: get_uuid(r, 0)?,
                        task: opt_uuid(r, 1)?,
                        worktree: opt_uuid(r, 2)?,
                        workspace: opt_uuid(r, 3)?,
                        repository: opt_uuid(r, 4)?,
                        harness: r.get(5)?,
                        ended_at: r.get(6)?,
                        active_ms: r.get(7)?,
                        usage: r.get(8)?,
                    })
                },
            )
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;

        let mut models: HashMap<Uuid, Vec<ModelRow>> = HashMap::new();
        let mut stmt = conn
            .prepare(
                "SELECT turn_id, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                        cost_micros, cost_source, price_table
                   FROM agent_turn_models WHERE turn_id = ?1",
            )
            .map_err(map_err)?;
        for turn in &turns {
            let rows = stmt
                .query_map(params![uuid_blob(turn.id)], |r| {
                    let n = |i: usize| -> rusqlite::Result<u64> { Ok(r.get::<_, i64>(i)?.max(0) as u64) };
                    let source: String = r.get(7)?;
                    Ok(ModelRow {
                        model: r.get(1)?,
                        tokens: TokenCounts {
                            input: n(2)?,
                            output: n(3)?,
                            cache_read: n(4)?,
                            cache_write: n(5)?,
                            cache_write_1h: 0,
                        },
                        cost_micros: r.get(6)?,
                        source: CostSource::parse(&source),
                        table: r.get(8)?,
                    })
                })
                .map_err(map_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_err)?;
            if !rows.is_empty() {
                models.insert(turn.id, rows);
            }
        }
        Ok((turns, models))
    }
}

fn key(turn: &Row, model: Option<&str>, group_by: &[GroupBy], offset_minutes: i32) -> UsageKey {
    let mut k = UsageKey::default();
    for g in group_by {
        match g {
            GroupBy::Task => k.task_id = turn.task,
            GroupBy::Worktree => k.worktree_id = turn.worktree,
            GroupBy::Workspace => k.workspace_id = turn.workspace,
            GroupBy::Repository => k.repository_id = turn.repository,
            GroupBy::Harness => k.harness = Some(turn.harness.clone()),
            GroupBy::Model => k.model = Some(model.unwrap_or_default().to_string()),
            GroupBy::Day | GroupBy::Week | GroupBy::Month => {
                k.period = Some(period(*g, turn.ended_at, offset_minutes));
            }
        }
    }
    k
}

fn add_turn(t: &mut UsageTotals, turn: &Row) {
    t.turns += 1;
    match turn.usage.as_str() {
        "partial" => t.turns_partial += 1,
        "not_reported" => t.turns_not_reported += 1,
        _ => {}
    }
    t.active_ms += turn.active_ms.unwrap_or(0);
    t.first_ended_at = Some(t.first_ended_at.map_or(turn.ended_at, |a| a.min(turn.ended_at)));
    t.last_ended_at = Some(t.last_ended_at.map_or(turn.ended_at, |a| a.max(turn.ended_at)));
}

fn add_model(t: &mut UsageTotals, tables: &mut BTreeSet<String>, row: &ModelRow) {
    t.tokens.add(&row.tokens);
    match (row.source, row.cost_micros) {
        (CostSource::Reported, Some(c)) => t.cost_reported_micros += c,
        (CostSource::Estimated, Some(c)) => {
            t.cost_estimated_micros += c;
            tables.extend(row.table.clone());
        }
        _ => {
            let k = &row.tokens;
            t.unpriced_tokens += k.input + k.output + k.cache_read + k.cache_write;
        }
    }
}

/// The period a turn's end falls in, as a date in the caller's offset.
fn period(g: GroupBy, ended_at_ms: i64, offset_minutes: i32) -> String {
    let local = ended_at_ms.div_euclid(1000) + offset_minutes as i64 * 60;
    let days = local.div_euclid(86_400);
    match g {
        GroupBy::Week => {
            // 1970-01-01 was a Thursday: Monday is three days earlier.
            let monday = days - (days + 3).rem_euclid(7);
            let (y, m, d) = civil(monday);
            format!("{y:04}-{m:02}-{d:02}")
        }
        GroupBy::Month => {
            let (y, m, _) = civil(days);
            format!("{y:04}-{m:02}")
        }
        _ => {
            let (y, m, d) = civil(days);
            format!("{y:04}-{m:02}-{d:02}")
        }
    }
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
