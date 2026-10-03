use super::*;

const OPUS: TokenCounts =
    TokenCounts { input: 2, output: 4, cache_read: 19912, cache_write: 7018, cache_write_1h: 7018 };

fn turn(key: &str, task: Option<Uuid>, harness: &str, ended_at: i64, models: Vec<TurnModel>) -> NewTurn {
    NewTurn {
        key: key.to_string(),
        terminal_id: Some(Uuid::from_u128(9)),
        worktree_id: Some(Uuid::from_u128(8)),
        repository_id: Some(Uuid::from_u128(7)),
        workspace_id: Some(Uuid::from_u128(6)),
        task_id: task,
        harness: harness.to_string(),
        surface: Surface::Chat,
        started_at: Some(ended_at - 1000),
        ended_at,
        active_ms: Some(1000),
        usage: if models.is_empty() { "not_reported" } else { "reported" },
        models,
    }
}

/// Each provenance, decided once when the row is written.
#[test]
fn a_cost_is_reported_estimated_or_unknown_and_says_which() {
    let reported = TurnModel::priced(Some("claude-opus-5[1m]".into()), OPUS, Some(80_246));
    assert_eq!((reported.cost_source, reported.cost_micros, reported.price_table.as_deref()), (CostSource::Reported, Some(80_246), None));

    let estimated = TurnModel::priced(Some("claude-opus-5".into()), OPUS, None);
    assert_eq!(estimated.cost_source, CostSource::Estimated);
    assert_eq!(estimated.cost_micros, Some(80_246), "the table agrees with the agent here");
    assert_eq!(estimated.price_table.as_deref(), Some(PRICE_TABLE), "stamped with the table it came from");

    let unknown = TurnModel::priced(Some("gpt-5.6-luna".into()), OPUS, None);
    assert_eq!((unknown.cost_source, unknown.cost_micros), (CostSource::Unknown, None));
    let unnamed = TurnModel::priced(None, OPUS, None);
    assert_eq!(unnamed.cost_source, CostSource::Unknown);
}

/// A turn heard twice is recorded once.
#[test]
fn the_same_turn_key_is_recorded_once() {
    let s = Store::open_in_memory().unwrap();
    let t = turn("claude:x", None, "claude", 10_000, vec![TurnModel::priced(None, OPUS, Some(5))]);
    assert!(s.record_turn(&t).unwrap());
    assert!(!s.record_turn(&t).unwrap());
    let all = s.usage_summary(&UsageFilter::default(), &[], 0).unwrap();
    assert_eq!(all[0].totals.turns, 1);
    assert_eq!(all[0].totals.tokens.cache_read, 19912);
}

/// "$X, n% reported, m% estimated": the aggregate keeps the mix apart, and
/// tokens nobody could price are counted, not dropped.
#[test]
fn an_aggregate_keeps_its_provenance_mix() {
    let s = Store::open_in_memory().unwrap();
    let task = Uuid::from_u128(1);
    let models = vec![
        TurnModel::priced(Some("claude-opus-5[1m]".into()), OPUS, Some(80_246)),
        TurnModel::priced(Some("claude-haiku-4-5-20251001".into()), TokenCounts { input: 1169, output: 13, ..Default::default() }, None),
    ];
    s.record_turn(&turn("a", Some(task), "claude", 10_000, models)).unwrap();
    let codex = vec![TurnModel::priced(Some("gpt-5.5".into()), TokenCounts { input: 100, output: 6, ..Default::default() }, None)];
    s.record_turn(&turn("b", Some(task), "codex", 20_000, codex)).unwrap();
    s.record_turn(&turn("c", Some(task), "gemini", 30_000, vec![])).unwrap();

    let (total, split) = s.task_usage(task).unwrap();
    assert_eq!(total.turns, 3);
    assert_eq!(total.turns_not_reported, 1);
    assert_eq!(total.active_ms, 3000);
    assert_eq!(total.cost_reported_micros, 80_246);
    assert_eq!(total.cost_estimated_micros, 1234);
    assert_eq!(total.unpriced_tokens, 106);
    assert_eq!(total.price_tables, vec![PRICE_TABLE.to_string()]);

    let names: Vec<(Option<&str>, Option<&str>, u64)> = split
        .iter()
        .map(|g| (g.key.harness.as_deref(), g.key.model.as_deref(), g.totals.turns))
        .collect();
    assert_eq!(
        names,
        vec![
            (Some("claude"), Some("claude-haiku-4-5-20251001"), 1),
            (Some("claude"), Some("claude-opus-5[1m]"), 1),
            (Some("codex"), Some("gpt-5.5"), 1),
            (Some("gemini"), Some(""), 1),
        ],
        "per harness and model, the unreported turn under the unnamed model"
    );
}

#[test]
fn totals_break_down_by_task_and_by_day_in_the_callers_offset() {
    let s = Store::open_in_memory().unwrap();
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    // 2026-10-02T23:30Z and 2026-10-03T00:30Z.
    let late = 1_790_983_800_000;
    let early = late + 3_600_000;
    let m = || vec![TurnModel::priced(Some("claude-opus-5".into()), OPUS, None)];
    s.record_turn(&turn("1", Some(a), "claude", late, m())).unwrap();
    s.record_turn(&turn("2", Some(b), "claude", early, m())).unwrap();

    let by_task = s.usage_summary(&UsageFilter::default(), &[GroupBy::Task], 0).unwrap();
    assert_eq!(by_task.iter().map(|g| g.key.task_id).collect::<Vec<_>>(), vec![Some(a), Some(b)]);

    let utc = s.usage_summary(&UsageFilter::default(), &[GroupBy::Day], 0).unwrap();
    assert_eq!(utc.iter().map(|g| g.key.period.clone().unwrap()).collect::<Vec<_>>(), ["2026-10-02", "2026-10-03"]);
    let singapore = s.usage_summary(&UsageFilter::default(), &[GroupBy::Day], 480).unwrap();
    assert_eq!(singapore.len(), 1, "both are the morning of the 3rd at +8");
    assert_eq!(singapore[0].key.period.as_deref(), Some("2026-10-03"));
    let week = s.usage_summary(&UsageFilter::default(), &[GroupBy::Week], 0).unwrap();
    assert_eq!(week[0].key.period.as_deref(), Some("2026-09-28"), "the Monday");

    let since = UsageFilter { since: Some(early), ..Default::default() };
    assert_eq!(s.usage_summary(&since, &[], 0).unwrap()[0].totals.turns, 1);
}

/// A database a build before this one left behind gains the two tables and
/// keeps everything else.
#[test]
fn the_migration_only_adds() {
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut conn, 19);
    let before: Vec<String> = tables(&conn);
    crate::migrate::migrate(&mut conn, 19).unwrap();
    let after = tables(&conn);
    let added: Vec<&String> = after.iter().filter(|t| !before.contains(t)).collect();
    assert_eq!(added, ["agent_turn_models", "agent_turns"]);
    assert!(before.iter().all(|t| after.contains(t)));
}

fn tables(conn: &rusqlite::Connection) -> Vec<String> {
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").unwrap();
    stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
}
