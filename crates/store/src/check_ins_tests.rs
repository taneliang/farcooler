
    use farcooler_core::DomainError;
    use rusqlite::params;
    use uuid::Uuid;

    use crate::models::{Actor, TaskStatus, uuid_blob};
    use crate::Store;

    #[test]
    fn destination_migration_preserves_legacy_attempt_without_inventing_proof() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);").unwrap();
        let tx = conn.transaction().unwrap();
        for (migration, _) in &crate::migrate::MIGRATIONS[..59] { migration(&tx).unwrap(); }
        tx.execute_batch("INSERT INTO check_in_occurrences
            (id, workspace_id, task_id, generation, eligibility_epoch, authorizer_source, authorized_at,
             template_version, due_at, state, process_started)
            VALUES (zeroblob(16), zeroblob(16), zeroblob(16), zeroblob(16), 0, 'local_person', 1, 1, 1, 'submitted', 1234);").unwrap();
        super::migration_check_in_destination(&tx).unwrap();
        let (state, seconds, tty, incarnation): (String, i64, Option<String>, Option<i64>) = tx.query_row(
            "SELECT state, process_started, tty, process_incarnation FROM check_in_occurrences", [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).unwrap();
        assert_eq!((state.as_str(), seconds, tty, incarnation), ("submitted", 1234, None, None));
        assert_eq!(crate::migrate::MIGRATIONS[59].1, crate::compat::Older::Refused);
    }

    fn board() -> (Store, Uuid, Uuid) {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("check-in");
        let ws = store.ensure_main_workspace(repo).unwrap().id;
        let task = store.create_task(ws, "audit", Actor::Manager).unwrap().id;
        store.conn().execute(
            "INSERT INTO check_in_schedules (workspace_id, task_id, generation, authorizer_source, authorized_at, template_version, next_due, state) VALUES (?1, ?2, ?3, 'local_person', 1, 1, 3600001, 'enabled')",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        (store, ws, task)
    }

    #[test]
    fn unclean_instance_never_rearms_and_clean_close_requires_no_unknowns() {
        let (store, _, _) = board();
        let (first, second, token) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        assert!(store.arm_check_in_instance(first).unwrap());
        assert!(!store.arm_check_in_instance(second).unwrap());
        assert!(!store.close_check_in_instance(second).unwrap());
        store.begin_check_in_cleanup(token).unwrap();
        assert!(!store.close_check_in_instance(first).unwrap());
        assert!(store.complete_check_in_cleanup(token).unwrap());
        assert!(store.close_check_in_instance(first).unwrap());
        assert!(store.arm_check_in_instance(second).unwrap());
        store.recover_check_ins_after_exclusive_start().unwrap();
        assert!(!store.arm_check_in_instance(Uuid::now_v7()).unwrap());
    }

    #[test]
    fn unknown_cleanup_survives_recovery_and_only_its_token_clears_it() {
        let (store, ws, _) = board();
        let (first, second) = (Uuid::now_v7(), Uuid::now_v7());
        store.begin_check_in_cleanup(first).unwrap();
        store.begin_check_in_cleanup(second).unwrap();
        store.recover_check_ins_after_exclusive_start().unwrap();
        assert_eq!(store.check_in_status(ws).unwrap().unwrap().reason.as_deref(), Some("cleanup_completion_unknown"));
        assert!(!store.complete_check_in_cleanup(Uuid::now_v7()).unwrap());
        assert!(store.complete_check_in_cleanup(first).unwrap());
        assert_eq!(store.check_in_status(ws).unwrap().unwrap().reason.as_deref(), Some("cleanup_completion_unknown"));
        assert!(store.complete_check_in_cleanup(second).unwrap());
        assert_ne!(store.check_in_status(ws).unwrap().unwrap().reason.as_deref(), Some("cleanup_completion_unknown"));
    }

    #[test]
    fn enable_requires_expected_generation_and_starts_one_hour_later() {
        let (store, ws, task) = board();
        assert!(matches!(store.enable_check_in(crate::EnableCheckIn { workspace: ws, task, device: None, expected_generation: None, expected_occurrence: None, acknowledge_uncertain: false, now: 100 }), Err(DomainError::ResourceConflict)));
        let generation: Vec<u8> = store.conn().query_row(
            "SELECT generation FROM check_in_schedules WHERE workspace_id = ?1", [uuid_blob(ws)], |r| r.get(0),
        ).unwrap();
        let old = Uuid::from_slice(&generation).unwrap();
        let next = store.enable_check_in(crate::EnableCheckIn { workspace: ws, task, device: None, expected_generation: Some(old), expected_occurrence: None, acknowledge_uncertain: false, now: 100 }).unwrap();
        assert_ne!(next, old);
        assert!(!store.tick_check_in(ws, 3_600_099).unwrap());
        assert!(store.tick_check_in(ws, 3_600_100).unwrap());
    }

    fn disabled_uncertain(generation: Uuid) -> (Store, Uuid, Uuid, Uuid) {
        let (store, ws, task) = board();
        let occurrence = Uuid::now_v7();
        {
            let conn = store.conn();
            conn.execute("UPDATE check_in_schedules SET generation = ?2, state = 'disabled' WHERE workspace_id = ?1", params![uuid_blob(ws), uuid_blob(generation)]).unwrap();
            conn.execute(
                "INSERT INTO check_in_occurrences (id, workspace_id, task_id, generation, eligibility_epoch, authorizer_source, authorized_at, template_version, due_at, state, reason) SELECT ?1, workspace_id, task_id, generation, eligibility_epoch, authorizer_source, authorized_at, template_version, next_due, 'uncertain', 'fixture' FROM check_in_schedules WHERE workspace_id = ?2",
                params![uuid_blob(occurrence), uuid_blob(ws)],
            ).unwrap();
        }
        (store, ws, task, occurrence)
    }

    #[test]
    fn stale_acknowledgement_cannot_resolve_another_occurrence() {
        let generation = Uuid::now_v7();
        let displayed_occurrence = Uuid::now_v7();
        let (store, ws, task, occurrence) = disabled_uncertain(generation);
        let snapshot = || {
            let conn = store.conn();
            let schedule: (Vec<u8>, String, i64, Option<String>, String, i64) = conn.query_row(
                "SELECT generation, state, eligibility_epoch, authorizer_device, authorizer_source, authorized_at FROM check_in_schedules WHERE workspace_id = ?1", [uuid_blob(ws)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            ).unwrap();
            let item: (Vec<u8>, Vec<u8>, String, Option<String>) = conn.query_row(
                "SELECT id, generation, state, reason FROM check_in_occurrences WHERE id = ?1", [uuid_blob(occurrence)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            ).unwrap();
            let counts: (i64, i64, i64) = conn.query_row(
                "SELECT (SELECT count(*) FROM check_in_schedules WHERE workspace_id = ?1), (SELECT count(*) FROM check_in_occurrences WHERE workspace_id = ?1), (SELECT count(*) FROM task_notes WHERE task_id = ?2)",
                params![uuid_blob(ws), uuid_blob(task)], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            ).unwrap();
            (schedule, item, counts)
        };
        let before = snapshot();
        assert!(matches!(store.enable_check_in(crate::EnableCheckIn { workspace: ws, task, device: None, expected_generation: Some(generation), expected_occurrence: Some(displayed_occurrence), acknowledge_uncertain: true, now: 100 }), Err(DomainError::ResourceConflict)));
        assert_eq!(snapshot(), before);
        assert!(matches!(store.enable_check_in(crate::EnableCheckIn { workspace: ws, task, device: None, expected_generation: Some(Uuid::now_v7()), expected_occurrence: Some(occurrence), acknowledge_uncertain: true, now: 100 }), Err(DomainError::ResourceConflict)));
        assert_eq!(snapshot(), before);
        assert!(matches!(store.enable_check_in(crate::EnableCheckIn { workspace: ws, task, device: None, expected_generation: Some(generation), expected_occurrence: None, acknowledge_uncertain: true, now: 100 }), Err(DomainError::ResourceConflict)));
        assert_eq!(snapshot(), before);

        // Same displayed A, independently seeded as the current snapshot.
        let (store, ws, task, occurrence) = disabled_uncertain(generation);
        let notes_before: i64 = store.conn().query_row("SELECT count(*) FROM task_notes WHERE task_id = ?1", [uuid_blob(task)], |r| r.get(0)).unwrap();
        let next = store.enable_check_in(crate::EnableCheckIn { workspace: ws, task, device: None, expected_generation: Some(generation), expected_occurrence: Some(occurrence), acknowledge_uncertain: true, now: 100 }).unwrap();
        assert_ne!(next, generation);
        let status = store.check_in_status(ws).unwrap().unwrap();
        assert_eq!(status.generation, next);
        assert_eq!(status.state, "enabled");
        let old = status.occurrence.unwrap();
        assert_eq!(old.id, occurrence);
        assert_eq!(old.state, "resolved_uncertain");
        let notes_after: i64 = store.conn().query_row("SELECT count(*) FROM task_notes WHERE task_id = ?1", [uuid_blob(task)], |r| r.get(0)).unwrap();
        assert_eq!(notes_after, notes_before + 1);
    }

    #[test]
    fn missed_hours_coalesce_without_using_message_wakes() {
        let (store, ws, _) = board();
        assert!(store.due_check_in_workspaces(3_600_000).unwrap().is_empty());
        assert_eq!(store.due_check_in_workspaces(3_600_001).unwrap(), [ws]);
        assert!(!store.tick_check_in(ws, 3_600_000).unwrap());
        assert!(store.tick_check_in(ws, 3_600_001).unwrap());
        assert_eq!(store.pending_check_ins().unwrap().len(), 1);
        assert!(!store.tick_check_in(ws, 20_000_000).unwrap());
        let status = store.check_in_status(ws).unwrap().unwrap();
        assert_eq!(status.occurrence.unwrap().state, "pending");
        let conn = store.conn();
        let count: i64 = conn.query_row("SELECT count(*) FROM check_in_occurrences", [], |r| r.get(0)).unwrap();
        let messages: i64 = conn.query_row("SELECT count(*) FROM message_wakes", [], |r| r.get(0)).unwrap();
        assert_eq!((count, messages), (1, 0));
    }

    #[test]
    fn receipt_is_bound_to_attempt_session_and_pinned_file() {
        let (store, ws, _) = board();
        let (occ, attempt, terminal) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        store.conn().execute(
            "INSERT INTO check_in_occurrences (id, workspace_id, task_id, generation, eligibility_epoch, authorizer_device, authorizer_source, authorized_at, template_version, due_at, state, attempt_id, terminal_id, thread_id, message_text, pane_id, terminal_lease, terminal_epoch, process_id, process_started, rollout_path, rollout_device, rollout_inode, rollout_offset, admitted_at, submitted_at, tty, process_incarnation)
             SELECT ?1, workspace_id, task_id, generation, eligibility_epoch, authorizer_device, authorizer_source, authorized_at, template_version, next_due, 'submitted', ?2, ?3, 'thread', 'test message', '%1', 1, 1, 123, 1, '/rollout', 1, 2, 10, 5, 6, '/dev/test', 7 FROM check_in_schedules WHERE workspace_id = ?4",
            params![uuid_blob(occ), uuid_blob(attempt), uuid_blob(terminal), uuid_blob(ws)],
        ).unwrap();
        let observation = store.check_in_observation(occ, attempt).unwrap().unwrap();
        assert_eq!((observation.path.as_str(), observation.device, observation.inode, observation.from), ("/rollout", 1, 2, 10));
        assert!(store.check_in_observation(occ, Uuid::now_v7()).unwrap().is_none());
        assert!(!store.confirm_check_in_receipt(occ, attempt, terminal, "other", 1, 2, 10, 5, 7).unwrap());
        assert!(!store.confirm_check_in_receipt(occ, attempt, terminal, "thread", 1, 3, 10, 5, 7).unwrap());
        assert!(!store.confirm_check_in_receipt(occ, attempt, terminal, "thread", 1, 2, 9, 5, 7).unwrap());
        assert!(store.confirm_check_in_receipt(occ, attempt, terminal, "thread", 1, 2, 10, 5, 7).unwrap());
        assert!(!store.confirm_check_in_receipt(occ, attempt, terminal, "thread", 1, 2, 10, 5, 7).unwrap());
        let status = store.check_in_status(ws).unwrap().unwrap();
        assert_eq!(status.next_due, 3_600_007);
        assert_eq!(status.occurrence.unwrap().state, "confirmed");
    }

    #[test]
    fn legacy_submitted_attempt_without_new_destination_proof_cannot_be_observed() {
        let (store, ws, _) = board();
        let (occ, attempt, terminal) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        store.conn().execute(
            "INSERT INTO check_in_occurrences (id, workspace_id, task_id, generation, eligibility_epoch, authorizer_source, authorized_at,
                template_version, due_at, state, attempt_id, terminal_id, thread_id, message_text, pane_id, terminal_lease, terminal_epoch,
                process_id, process_started, rollout_path, rollout_device, rollout_inode, rollout_offset, admitted_at, submitted_at)
             SELECT ?1, workspace_id, task_id, generation, eligibility_epoch, authorizer_source, authorized_at,
                template_version, next_due, 'submitted', ?2, ?3, 'thread', 'test', '%1', 1, 1, 123, 1000, '/rollout', 1, 2, 0, 5, 6
             FROM check_in_schedules WHERE workspace_id = ?4",
            params![uuid_blob(occ), uuid_blob(attempt), uuid_blob(terminal), uuid_blob(ws)],
        ).unwrap();
        assert!(store.check_in_observation(occ, attempt).unwrap().is_none());
        assert_eq!(store.check_in_status(ws).unwrap().unwrap().occurrence.unwrap().state, "submitted");
    }

    #[test]
    fn admission_vetoes_mutation_and_settles_only_its_attempt() {
        use farcooler_protocol::v1::TerminalIntent;
        let (store, ws, task) = board();
        let repo = store.get_workspace(ws).unwrap().repository_id;
        let worktree = store.create_worktree(repo, "test/check-in", "/check-in", false).unwrap();
        let terminal = store.create_terminal(worktree.id, "manager", "codex", TerminalIntent::Running, 80, 24).unwrap();
        store.conn().execute(
            "UPDATE terminals SET workspace_id = ?2, role = 2, runtime_confirmed = 1, agent_session_id = 'thread' WHERE id = ?1",
            params![uuid_blob(terminal.id), uuid_blob(ws)],
        ).unwrap();
        store.conn().execute("UPDATE check_in_schedules SET state = 'enabled' WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        assert!(store.tick_check_in(ws, 3_600_001).unwrap());
        let occ = store.pending_check_ins().unwrap()[0].id;
        let (attempt, daemon) = (Uuid::now_v7(), Uuid::now_v7());
        let cleanup = Uuid::now_v7();
        store.begin_check_in_cleanup(cleanup).unwrap();
        assert!(!store.claim_check_in(occ, attempt, daemon, terminal.id, "thread", "%1", 123, 1, 3_600_002).unwrap());
        assert!(store.complete_check_in_cleanup(cleanup).unwrap());
        assert!(store.claim_check_in(occ, attempt, daemon, terminal.id, "thread", "%1", 123, 1, 3_600_002).unwrap());
        let text = store.claimed_check_in_message(occ, attempt).unwrap().unwrap();
        assert!(text.starts_with("[Far Cooler] Scheduled hourly check-in"));
        assert!(text.contains(&occ.to_string()));
        assert!(!store.admit_check_in_enter(occ, attempt, daemon, terminal.id, "thread", "%1", 123, 1, 7, 3_600_003).unwrap());
        assert!(store.pin_check_in_rollout(crate::PinCheckInRollout { occurrence: occ, attempt, path: "/rollout", tty: "/dev/test", incarnation: 7, device: 1, inode: 2, offset: 0 }).unwrap());
        let cleanup = Uuid::now_v7();
        store.begin_check_in_cleanup(cleanup).unwrap();
        assert!(!store.admit_check_in_enter(occ, attempt, daemon, terminal.id, "thread", "%1", 123, 1, 7, 3_600_003).unwrap());
        assert!(store.complete_check_in_cleanup(cleanup).unwrap());
        let reservation = Uuid::now_v7();
        assert!(store.reserve_check_in_target_change(terminal.id, reservation, "stop", Some(1)).unwrap());
        assert!(!store.admit_check_in_enter(occ, attempt, daemon, terminal.id, "thread", "%1", 123, 1, 7, 3_600_003).unwrap());
        assert!(store.complete_check_in_target_change(reservation).unwrap());
        assert!(store.release_check_in_target_change(reservation).unwrap());
        assert!(!store.admit_check_in_enter(occ, attempt, daemon, terminal.id, "other", "%1", 123, 1, 7, 3_600_003).unwrap());
        assert!(!store.admit_check_in_enter(occ, attempt, daemon, terminal.id, "thread", "%2", 123, 1, 7, 3_600_003).unwrap());
        assert!(store.admit_check_in_enter(occ, attempt, daemon, terminal.id, "thread", "%1", 123, 1, 7, 3_600_003).unwrap());
        assert!(matches!(store.reserve_check_in_target_change(terminal.id, Uuid::now_v7(), "stop", Some(1)), Err(DomainError::ResourceConflict)));
        assert!(matches!(store.set_task_status(task, TaskStatus::Done, Actor::Manager), Err(DomainError::ResourceConflict)));
        assert!(matches!(store.disable_check_in(ws, None, None), Err(DomainError::ResourceConflict)));
        assert!(!store.settle_check_in_enter(occ, Uuid::now_v7(), 3_600_004).unwrap());
        assert!(store.settle_check_in_enter(occ, attempt, 3_600_004).unwrap());
        assert_eq!(store.expire_check_in_submissions(3_600_004 + super::CHECK_IN_CONFIRMATION_TIMEOUT_MS - 1).unwrap(), 0);
        assert_eq!(store.expire_check_in_submissions(3_600_004 + super::CHECK_IN_CONFIRMATION_TIMEOUT_MS).unwrap(), 1);
        assert!(store.disable_check_in(ws, None, None).unwrap());
        store.set_task_status(task, TaskStatus::Done, Actor::Manager).unwrap();
        let state: String = store.conn().query_row("SELECT state FROM check_in_occurrences WHERE id = ?1", [uuid_blob(occ)], |r| r.get(0)).unwrap();
        assert_eq!(state, "uncertain");
    }

    #[test]
    fn agent_host_mode_cannot_be_claimed() {
        use farcooler_protocol::v1::TerminalIntent;
        let (store, ws, _) = board();
        let repository = store.get_workspace(ws).unwrap().repository_id;
        let worktree = store.create_worktree(repository, "test/drawn", "/drawn", false).unwrap();
        let terminal = store.create_terminal(worktree.id, "manager", "codex", TerminalIntent::Running, 80, 24).unwrap();
        store.conn().execute(
            "UPDATE terminals SET workspace_id = ?2, role = 2, pane_mode = 1, runtime_confirmed = 1, agent_session_id = 'thread' WHERE id = ?1",
            params![uuid_blob(terminal.id), uuid_blob(ws)],
        ).unwrap();
        store.conn().execute("UPDATE check_in_schedules SET state = 'enabled' WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        assert!(store.tick_check_in(ws, 3_600_001).unwrap());
        let occurrence = store.pending_check_ins().unwrap()[0].id;
        assert!(!store.claim_check_in(occurrence, Uuid::now_v7(), Uuid::now_v7(), terminal.id, "thread", "%1", 123, 1, 3_600_002).unwrap());
        assert_eq!(store.pending_check_ins().unwrap().len(), 1);
    }

    #[test]
    fn uncertain_enter_keeps_its_fence_across_exclusive_recovery() {
        let (store, ws, task) = board();
        let (occ, attempt, daemon, terminal) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        store.conn().execute(
            "INSERT INTO check_in_occurrences (id, workspace_id, task_id, generation, eligibility_epoch, authorizer_device, authorizer_source, authorized_at, template_version, due_at, state, attempt_id, daemon_instance, terminal_id)
             SELECT ?1, workspace_id, task_id, generation, eligibility_epoch, authorizer_device, authorizer_source, authorized_at, template_version, next_due, 'enter_admitted', ?2, ?3, ?4 FROM check_in_schedules WHERE workspace_id = ?5",
            params![uuid_blob(occ), uuid_blob(attempt), uuid_blob(daemon), uuid_blob(terminal), uuid_blob(ws)],
        ).unwrap();
        store.conn().execute(
            "INSERT INTO check_in_enter_fences VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(terminal), uuid_blob(occ), uuid_blob(attempt), uuid_blob(daemon)],
        ).unwrap();
        assert!(store.fail_admitted_check_in(occ, attempt).unwrap());
        assert!(matches!(store.disable_check_in(ws, None, None), Err(DomainError::ResourceConflict)));
        store.recover_check_ins_after_exclusive_start().unwrap();
        assert!(matches!(store.disable_check_in(ws, None, None), Err(DomainError::ResourceConflict)));
        let status = store.check_in_status(ws).unwrap().unwrap();
        assert_eq!(status.occurrence.unwrap().state, "uncertain");
    }

    #[test]
    fn disable_after_claim_prevents_enter_admission() {
        use farcooler_protocol::v1::TerminalIntent;
        let (store, ws, _) = board();
        let repository = store.get_workspace(ws).unwrap().repository_id;
        let worktree = store.create_worktree(repository, "test/disable", "/disable", false).unwrap();
        let terminal = store.create_terminal(worktree.id, "manager", "codex", TerminalIntent::Running, 80, 24).unwrap();
        store.conn().execute(
            "UPDATE terminals SET workspace_id = ?2, role = 2, runtime_confirmed = 1, agent_session_id = 'thread' WHERE id = ?1",
            params![uuid_blob(terminal.id), uuid_blob(ws)],
        ).unwrap();
        store.conn().execute("UPDATE check_in_schedules SET state = 'enabled' WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        assert!(store.tick_check_in(ws, 3_600_001).unwrap());
        let occurrence = store.pending_check_ins().unwrap()[0].id;
        let (attempt, daemon) = (Uuid::now_v7(), Uuid::now_v7());
        assert!(store.claim_check_in(occurrence, attempt, daemon, terminal.id, "thread", "%1", 123, 1, 3_600_002).unwrap());
        assert!(store.pin_check_in_rollout(crate::PinCheckInRollout { occurrence, attempt, path: "/rollout", tty: "/dev/test", incarnation: 7, device: 1, inode: 2, offset: 0 }).unwrap());
        assert!(store.disable_check_in(ws, None, None).unwrap());
        assert!(!store.admit_check_in_enter(occurrence, attempt, daemon, terminal.id, "thread", "%1", 123, 1, 7, 3_600_003).unwrap());
        let status = store.check_in_status(ws).unwrap().unwrap();
        assert_eq!(status.state, "disabled");
        assert_eq!(status.occurrence.unwrap().state, "uncertain");
    }

    #[test]
    fn admitted_fence_refuses_a_reseat_even_when_values_are_unchanged() {
        use farcooler_protocol::v1::TerminalIntent;
        let (store, ws, task) = board();
        let repository = store.get_workspace(ws).unwrap().repository_id;
        let worktree = store.create_worktree(repository, "test/reseat", "/reseat", false).unwrap();
        let terminal = store.create_terminal(worktree.id, "manager", "codex", TerminalIntent::Running, 80, 24).unwrap();
        store.set_terminal_workspace(terminal.id, ws).unwrap();
        let terminal = store.set_terminal_role(terminal.id, crate::models::TerminalRole::Orchestrator).unwrap();
        store.conn().execute("UPDATE check_in_schedules SET state = 'enabled' WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        store.conn().execute(
            "INSERT INTO check_in_enter_fences VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(terminal.id), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        assert!(matches!(store.reseat_orchestrator(terminal.id, terminal.resource_version, &[]), Err(DomainError::ResourceConflict)));
        assert_eq!(store.get_terminal(terminal.id).unwrap().resource_version, terminal.resource_version);
        store.conn().execute("DELETE FROM check_in_enter_fences WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        let reservation = Uuid::now_v7();
        assert!(store.reserve_check_in_target_change(terminal.id, reservation, "stop", Some(1)).unwrap());
        assert!(matches!(store.reseat_orchestrator(terminal.id, terminal.resource_version, &[]), Err(DomainError::ResourceConflict)));
        assert!(store.complete_check_in_target_change(reservation).unwrap());
        assert!(store.release_check_in_target_change(reservation).unwrap());
        store.reseat_orchestrator(terminal.id, terminal.resource_version, &[]).unwrap();
    }

    #[test]
    fn orphaned_and_uncertain_target_holds_are_visible_and_cannot_be_released() {
        use farcooler_protocol::v1::TerminalIntent;
        let (store, ws, _) = board();
        let repository = store.get_workspace(ws).unwrap().repository_id;
        let worktree = store.create_worktree(repository, "test/orphan", "/orphan", false).unwrap();
        let terminal = store.create_terminal(worktree.id, "agent", "codex", TerminalIntent::Running, 80, 24).unwrap();
        let token = Uuid::now_v7();
        assert!(store.reserve_check_in_target_change(terminal.id, token, "remove", Some(1)).unwrap());
        assert!(!store.release_check_in_target_change(token).unwrap());
        assert!(store.mark_check_in_target_uncertain(token).unwrap());
        store.conn().execute("DELETE FROM terminals WHERE id = ?1", [uuid_blob(terminal.id)]).unwrap();
        store.recover_check_ins_after_exclusive_start().unwrap();
        let holds = store.check_in_target_holds(None).unwrap();
        assert_eq!(holds.len(), 1);
        assert_eq!((holds[0].token, holds[0].workspace, holds[0].terminal, holds[0].phase.as_str()), (token, None, terminal.id, "uncertain"));
        assert!(!store.complete_check_in_target_change(token).unwrap());
        assert!(!store.release_check_in_target_change(token).unwrap());
    }

    #[test]
    fn only_a_completed_target_hold_can_be_reconciled() {
        use farcooler_protocol::v1::TerminalIntent;
        let (store, ws, _) = board();
        let repository = store.get_workspace(ws).unwrap().repository_id;
        let worktree = store.create_worktree(repository, "test/completed", "/completed", false).unwrap();
        let terminal = store.create_terminal(worktree.id, "agent", "codex", TerminalIntent::Running, 80, 24).unwrap();
        let token = Uuid::now_v7();
        assert!(store.reserve_check_in_target_change(terminal.id, token, "stop", Some(1)).unwrap());
        assert!(!store.release_check_in_target_change(token).unwrap());
        assert!(store.complete_check_in_target_change(token).unwrap());
        assert_eq!(store.check_in_target_holds(None).unwrap()[0].phase, "completed");
        assert!(store.reconcile_check_in_target_change(token, None).unwrap());
        assert!(store.check_in_target_holds(None).unwrap().is_empty());
    }

    #[test]
    fn a_stopping_agent_cannot_be_promoted_to_orchestrator() {
        use farcooler_protocol::v1::TerminalIntent;
        let (store, ws, _) = board();
        let repository = store.get_workspace(ws).unwrap().repository_id;
        let worktree = store.create_worktree(repository, "test/promote", "/promote", false).unwrap();
        let terminal = store.create_terminal(worktree.id, "agent", "codex", TerminalIntent::Running, 80, 24).unwrap();
        store.set_terminal_workspace(terminal.id, ws).unwrap();
        let reservation = Uuid::now_v7();
        assert!(store.reserve_check_in_target_change(terminal.id, reservation, "stop", Some(1)).unwrap());
        assert!(matches!(store.set_terminal_role(terminal.id, crate::models::TerminalRole::Orchestrator), Err(DomainError::ResourceConflict)));
        assert!(store.complete_check_in_target_change(reservation).unwrap());
        assert!(store.release_check_in_target_change(reservation).unwrap());
        store.set_terminal_role(terminal.id, crate::models::TerminalRole::Orchestrator).unwrap();
    }

    #[test]
    fn a_stop_started_while_off_blocks_a_new_schedule_claim() {
        use farcooler_protocol::v1::TerminalIntent;
        let (store, ws, task) = board();
        let repository = store.get_workspace(ws).unwrap().repository_id;
        let worktree = store.create_worktree(repository, "test/stop-first", "/stop-first", false).unwrap();
        let terminal = store.create_terminal(worktree.id, "manager", "codex", TerminalIntent::Running, 80, 24).unwrap();
        store.conn().execute(
            "UPDATE terminals SET workspace_id = ?2, role = 2, runtime_confirmed = 1, agent_session_id = 'thread' WHERE id = ?1",
            params![uuid_blob(terminal.id), uuid_blob(ws)],
        ).unwrap();
        store.conn().execute("DELETE FROM check_in_schedules WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        let reservation = Uuid::now_v7();
        assert!(store.reserve_check_in_target_change(terminal.id, reservation, "stop", Some(1)).unwrap());
        store.recover_check_ins_after_exclusive_start().unwrap();
        store.enable_check_in(crate::EnableCheckIn { workspace: ws, task, device: None, expected_generation: None, expected_occurrence: None, acknowledge_uncertain: false, now: 1 }).unwrap();
        assert!(store.tick_check_in(ws, 3_600_001).unwrap());
        let occ = store.pending_check_ins().unwrap()[0].id;
        let (attempt, daemon) = (Uuid::now_v7(), Uuid::now_v7());
        assert!(!store.claim_check_in(occ, attempt, daemon, terminal.id, "thread", "%1", 123, 1, 3_600_002).unwrap());
        assert_eq!(store.check_in_status(ws).unwrap().unwrap().reason.as_deref(), Some("target_change_in_progress"));
        assert!(store.complete_check_in_target_change(reservation).unwrap());
        assert!(store.release_check_in_target_change(reservation).unwrap());
        assert!(store.claim_check_in(occ, attempt, daemon, terminal.id, "thread", "%1", 123, 1, 3_600_003).unwrap());
    }

    #[test]
    fn admitted_enter_vetoes_target_identity_and_parent_deletion() {
        use farcooler_protocol::v1::TerminalIntent;
        let (store, ws, task) = board();
        let repository = store.get_workspace(ws).unwrap().repository_id;
        let worktree = store.create_worktree(repository, "test/identity", "/identity", false).unwrap();
        let terminal = store.create_terminal(worktree.id, "manager", "codex", TerminalIntent::Running, 80, 24).unwrap();
        store.set_terminal_workspace(terminal.id, ws).unwrap();
        store.set_terminal_role(terminal.id, crate::models::TerminalRole::Orchestrator).unwrap();
        store.conn().execute("UPDATE check_in_schedules SET state = 'enabled' WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        store.conn().execute(
            "INSERT INTO check_in_enter_fences VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(terminal.id), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        for sql in [
            "UPDATE terminals SET role = 1 WHERE id = ?1",
            "UPDATE terminals SET pane_mode = 1 WHERE id = ?1",
            "UPDATE terminals SET epoch = epoch + 1 WHERE id = ?1",
            "UPDATE terminals SET agent_session_id = 'replacement' WHERE id = ?1",
            "DELETE FROM terminals WHERE id = ?1",
        ] {
            let result = store.conn().execute(sql, [uuid_blob(terminal.id)]).map_err(crate::error::map_err);
            assert!(matches!(&result, Err(DomainError::ResourceConflict)), "{sql}: {result:?}");
        }
        let result = store.conn().execute("DELETE FROM repositories WHERE id = ?1", [uuid_blob(repository)]).map_err(crate::error::map_err);
        assert!(matches!(result, Err(DomainError::ResourceConflict)));
        store.conn().execute("UPDATE terminals SET title = 'new title' WHERE id = ?1", [uuid_blob(terminal.id)]).unwrap();
        assert_eq!(store.get_terminal(terminal.id).unwrap().title, "new title");
    }

    #[test]
    fn admitted_enter_vetoes_wake_switch_and_task_deletion() {
        let (store, ws, task) = board();
        store.conn().execute(
            "INSERT INTO check_in_enter_fences VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        let version = store.get_workspace(ws).unwrap().resource_version;
        assert!(matches!(store.set_workspace_wake_on_answer(ws, version, false), Err(DomainError::ResourceConflict)));
        let deleted = store.conn().execute("DELETE FROM tasks WHERE id = ?1", [uuid_blob(task)]).map_err(crate::error::map_err);
        assert!(matches!(deleted, Err(DomainError::ResourceConflict)));
        assert!(store.get_workspace(ws).unwrap().wake_on_answer);
        assert_eq!(store.get_task(task).unwrap().workspace_id, ws);
    }

    #[test]
    fn fenced_task_move_rolls_back_the_whole_batch() {
        let (store, ws, audit) = board();
        let repository = store.get_workspace(ws).unwrap().repository_id;
        let other = store.create_workspace(repository, "Other", "other").unwrap().id;
        let ordinary = store.create_task(ws, "ordinary", Actor::Manager).unwrap().id;
        store.conn().execute(
            "INSERT INTO check_in_enter_fences VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![uuid_blob(ws), uuid_blob(audit), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        assert!(matches!(store.move_tasks(&[ordinary, audit], other, Actor::Manager), Err(DomainError::ResourceConflict)));
        assert_eq!(store.get_task(ordinary).unwrap().workspace_id, ws);
        assert_eq!(store.get_task(audit).unwrap().workspace_id, ws);
        store.conn().execute("DELETE FROM check_in_enter_fences WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        store.move_tasks(&[ordinary, audit], other, Actor::Manager).unwrap();
        assert_eq!(store.check_in_status(ws).unwrap().unwrap().state, "suspended");
    }

    #[test]
    fn lead_upsert_is_fenced_and_unchanged_lead_is_allowed() {
        let (store, ws, task) = board();
        store.conn().execute("INSERT INTO members (handle, name, created_at) VALUES ('alice', 'Alice', 1), ('bob', 'Bob', 1)", []).unwrap();
        store.conn().execute("INSERT INTO workspace_leads (workspace_id, handle) VALUES (?1, 'alice')", [uuid_blob(ws)]).unwrap();
        store.conn().execute("UPDATE check_in_schedules SET state = 'enabled' WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        store.conn().execute(
            "INSERT INTO check_in_enter_fences VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        store.set_workspace_lead(ws, Some("alice")).unwrap();
        assert!(matches!(store.set_workspace_lead(ws, Some("bob")), Err(DomainError::ResourceConflict)));
        assert_eq!(store.workspace_lead(ws).unwrap().as_deref(), Some("alice"));
        let cascade = store.conn().execute("DELETE FROM members WHERE handle = 'alice'", []).map_err(crate::error::map_err);
        assert!(matches!(cascade, Err(DomainError::ResourceConflict)));
        assert_eq!(store.workspace_lead(ws).unwrap().as_deref(), Some("alice"));
        store.conn().execute("DELETE FROM check_in_enter_fences WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        store.set_workspace_lead(ws, Some("bob")).unwrap();
        store.set_workspace_lead(ws, Some("alice")).unwrap();
        assert_eq!(store.check_in_status(ws).unwrap().unwrap().state, "suspended");
    }

    #[test]
    fn unchanged_owner_device_write_does_not_disturb_an_admitted_attempt() {
        let (store, ws, task) = board();
        store.conn().execute("INSERT INTO owner_devices (client_id) VALUES ('owner')", []).unwrap();
        store.conn().execute(
            "UPDATE check_in_schedules SET authorizer_device = 'owner', authorizer_source = 'owner_device' WHERE workspace_id = ?1",
            [uuid_blob(ws)],
        ).unwrap();
        store.conn().execute(
            "INSERT INTO check_in_enter_fences VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'owner')",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        store.conn().execute("UPDATE owner_devices SET state = state WHERE client_id = 'owner'", []).unwrap();
        assert_eq!(store.check_in_status(ws).unwrap().unwrap().state, "enabled");
        assert!(matches!(store.conn().execute("UPDATE owner_devices SET state = 'check' WHERE client_id = 'owner'", []).map_err(crate::error::map_err), Err(DomainError::ResourceConflict)));
    }

    #[test]
    fn an_unseated_agent_cannot_disable_a_check_in() {
        let (store, ws, _) = board();
        assert!(matches!(store.disable_check_in(ws, Some(Uuid::now_v7()), None), Err(DomainError::Conflict { what: "not_orchestrator" })));
        assert_eq!(store.check_in_status(ws).unwrap().unwrap().state, "enabled");
        assert!(store.disable_check_in(ws, None, None).unwrap());
    }

    #[test]
    fn disable_rechecks_owner_device_inside_the_write_transaction() {
        for mutation in [
            "UPDATE owner_devices SET state = 'check' WHERE client_id = 'owner'",
            "INSERT INTO device_caps (client_id, cap) VALUES ('owner', 'viewer')",
            "INSERT INTO member_devices (client_id, handle) VALUES ('owner', 'alice')",
            "INSERT INTO released_devices (client_id) VALUES ('owner')",
        ] {
            let (store, ws, _) = board();
            store.conn().execute("INSERT INTO members (handle, name, created_at) VALUES ('alice', 'Alice', 1)", []).unwrap();
            store.conn().execute("INSERT INTO owner_devices (client_id) VALUES ('owner')", []).unwrap();
            store.conn().execute(mutation, []).unwrap();
            assert!(matches!(store.disable_check_in(ws, None, Some("owner")), Err(DomainError::Conflict { what: "person_only" })), "{mutation}");
            assert_eq!(store.check_in_status(ws).unwrap().unwrap().state, "enabled");
        }
        let (store, ws, _) = board();
        store.conn().execute("INSERT INTO owner_devices (client_id) VALUES ('owner')", []).unwrap();
        assert!(store.disable_check_in(ws, None, Some("owner")).unwrap());
    }

    #[test]
    fn disable_rechecks_orchestrator_device_and_seat_in_the_write_transaction() {
        use farcooler_protocol::v1::TerminalIntent;
        let (store, ws, _) = board();
        let repo = store.get_workspace(ws).unwrap().repository_id;
        let wt = store.create_worktree(repo, "test/disable", "/disable", false).unwrap();
        let terminal = store.create_terminal(wt.id, "manager", "codex", TerminalIntent::Running, 80, 24).unwrap();
        store.conn().execute("UPDATE terminals SET workspace_id = ?2, role = 2 WHERE id = ?1", params![uuid_blob(terminal.id), uuid_blob(ws)]).unwrap();
        store.conn().execute("UPDATE check_in_schedules SET state = 'enabled' WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        store.conn().execute("INSERT INTO members (handle, name, created_at) VALUES ('alice', 'Alice', 1)", []).unwrap();
        store.conn().execute("INSERT INTO member_devices (client_id, handle) VALUES ('member', 'alice')", []).unwrap();
        store.conn().execute("INSERT INTO member_access (handle, access) VALUES ('alice', 'viewer')", []).unwrap();
        assert!(store.disable_check_in(ws, Some(terminal.id), Some("member")).is_err());
        assert_eq!(store.check_in_status(ws).unwrap().unwrap().state, "enabled");
        store.conn().execute("UPDATE member_access SET access = 'member' WHERE handle = 'alice'", []).unwrap();
        store.conn().execute("INSERT INTO device_caps (client_id, cap) VALUES ('member', 'viewer')", []).unwrap();
        assert!(store.disable_check_in(ws, Some(terminal.id), Some("member")).is_err());
        store.conn().execute("DELETE FROM device_caps WHERE client_id = 'member'", []).unwrap();
        store.conn().execute("UPDATE terminals SET role = 1 WHERE id = ?1", [uuid_blob(terminal.id)]).unwrap();
        assert!(store.disable_check_in(ws, Some(terminal.id), Some("member")).is_err());
        store.conn().execute("UPDATE terminals SET role = 2 WHERE id = ?1", [uuid_blob(terminal.id)]).unwrap();
        assert!(store.disable_check_in(ws, Some(terminal.id), Some("member")).unwrap());
    }

    #[test]
    fn admitted_fence_vetoes_a_second_connection() {
        let dir = farcooler_core::scratch::dir("check-in-two-connections");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.db");
        let first = Store::open(&path).unwrap();
        let repo = first.register_repository_for_test("check-in");
        let ws = first.ensure_main_workspace(repo).unwrap().id;
        let task = first.create_task(ws, "audit", Actor::Manager).unwrap().id;
        first.conn().execute(
            "INSERT INTO check_in_schedules (workspace_id, task_id, generation, authorizer_source, authorized_at, template_version, next_due, state) VALUES (?1, ?2, ?3, 'local_person', 1, 1, 10, 'enabled')",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        let second = Store::open(&path).unwrap();
        first.conn().execute(
            "INSERT INTO check_in_enter_fences VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        assert!(matches!(second.set_task_status(task, TaskStatus::Done, Actor::Manager), Err(DomainError::ResourceConflict)));
        assert_eq!(second.get_task(task).unwrap().status, TaskStatus::Backlog);
        first.conn().execute("DELETE FROM check_in_enter_fences WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        second.set_task_status(task, TaskStatus::Done, Actor::Manager).unwrap();
        assert_eq!(first.check_in_status(ws).unwrap().unwrap().state, "suspended");
        drop(second);
        drop(first);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn status_change_invalidates_and_does_not_revive_on_return() {
        let (store, ws, task) = board();
        store.set_task_status(task, TaskStatus::Done, Actor::Manager).unwrap();
        store.set_task_status(task, TaskStatus::Backlog, Actor::Manager).unwrap();
        let state: (String, i64) = store.conn().query_row(
            "SELECT state, eligibility_epoch FROM check_in_schedules WHERE workspace_id = ?1",
            [uuid_blob(ws)], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(state, ("suspended".into(), 1));
    }

    #[test]
    fn admitted_enter_vetoes_status_until_fence_is_released() {
        let (store, ws, task) = board();
        store.conn().execute(
            "INSERT INTO check_in_enter_fences VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        assert!(matches!(store.set_task_status(task, TaskStatus::Done, Actor::Manager), Err(DomainError::ResourceConflict)));
        assert_eq!(store.get_task(task).unwrap().status, TaskStatus::Backlog);
        let state: String = store.conn().query_row(
            "SELECT state FROM check_in_schedules WHERE workspace_id = ?1", [uuid_blob(ws)], |r| r.get(0),
        ).unwrap();
        assert_eq!(state, "enabled");
        store.conn().execute("DELETE FROM check_in_enter_fences WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        store.set_task_status(task, TaskStatus::Done, Actor::Manager).unwrap();
    }
